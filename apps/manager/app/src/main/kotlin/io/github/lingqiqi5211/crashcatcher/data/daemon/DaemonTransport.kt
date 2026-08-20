package io.github.lingqiqi5211.crashcatcher.data.daemon

import android.net.LocalSocket
import android.net.LocalServerSocket
import java.io.Closeable
import java.io.FileDescriptor
import java.io.IOException
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.TimeUnit
import kotlin.concurrent.thread

/**
 * One framed, bidirectional conversation with the daemon.
 *
 * An interface rather than a concrete socket so the protocol logic above it can be
 * tested on the JVM. `LocalSocket` needs a device; frame pairing, sequence handling
 * and error mapping do not, and those are where the bugs live.
 */
interface DaemonChannel : Closeable {
    fun writeFrame(body: ByteArray)

    fun readFrame(): ByteArray

    /**
     * Descriptors that arrived with the last frame, if any.
     *
     * How a payload is handed over: the daemon sends a read-only descriptor over
     * `SCM_RIGHTS` and the caller streams it directly, skipping framing and JSON
     * escaping entirely.
     */
    fun takeFileDescriptors(): Array<FileDescriptor>?
}

/** Acquires channels. Separate from [DaemonClient] so reconnection is a transport concern. */
interface DaemonTransport {
    fun open(lane: ChannelHello): DaemonChannel
}

/**
 * Listens for channels the root daemon connects into.
 *
 * The direction is deliberate: some ROMs deny an app-domain process connecting to a root-domain
 * Unix socket before either side can authenticate. The daemon connecting to this app-domain
 * listener keeps the same bidirectional stream and descriptor passing without widening SELinux.
 * Both sides still fail closed: this side accepts only uid 0, while the daemon verifies this
 * process's uid and APK certificate before it reads the lane frame.
 */
class LocalSocketTransport(
    private val socketName: String = DaemonConstants.ABSTRACT_SOCKET_NAME,
    private val trace: DaemonTrace = NoopDaemonTrace,
) : DaemonTransport {
    private val incoming = IncomingDaemonChannels()

    init {
        thread(
            start = true,
            isDaemon = true,
            name = "cch-manager-listener",
            block = ::listen,
        )
    }

    override fun open(lane: ChannelHello): DaemonChannel {
        trace.event("daemon channel wait begin name=@$socketName lane=$lane")
        val channel = try {
            incoming.take(CHANNEL_WAIT_MILLIS)
        } catch (cause: InterruptedException) {
            Thread.currentThread().interrupt()
            throw DaemonException.ConnectionClosed("interrupted while waiting for daemon")
        } ?: throw DaemonException.Timeout("waiting for daemon at @$socketName")
        trace.event("daemon channel acquired name=@$socketName lane=$lane")

        // The lane is still chosen by the Manager. The daemon maintains two authenticated,
        // unassigned connections so control and subscribe can be opened independently.
        try {
            channel.writeFrame(DaemonJson.encodeToString(lane).encodeToByteArray())
        } catch (cause: Exception) {
            channel.close()
            trace.failure("socket lane write failed lane=$lane", cause)
            throw cause
        }
        trace.event("socket lane sent lane=$lane")
        return channel
    }

    private fun listen() {
        while (!Thread.currentThread().isInterrupted) {
            var listener: LocalServerSocket? = null
            try {
                listener = LocalServerSocket(socketName)
                trace.event("manager socket listener ready name=@$socketName")
                while (!Thread.currentThread().isInterrupted) {
                    accept(listener)
                }
            } catch (cause: InterruptedException) {
                Thread.currentThread().interrupt()
            } catch (cause: Exception) {
                trace.failure("manager socket listener failed name=@$socketName", cause)
            } finally {
                runCatching { listener?.close() }
            }
            if (!Thread.currentThread().isInterrupted) {
                try {
                    Thread.sleep(LISTENER_RETRY_MILLIS)
                } catch (_: InterruptedException) {
                    Thread.currentThread().interrupt()
                }
            }
        }
    }

    private fun accept(listener: LocalServerSocket) {
        val socket = listener.accept()
        val credentials = try {
            socket.peerCredentials
                ?: throw IOException("accepted local socket has no peer credentials")
        } catch (cause: Exception) {
            runCatching { socket.close() }
            trace.failure("daemon peer credentials failed", cause)
            return
        }
        if (credentials.uid != ROOT_UID) {
            runCatching { socket.close() }
            trace.event(
                "daemon peer rejected uid=${credentials.uid} pid=${credentials.pid}",
            )
            return
        }

        val channel = LocalSocketChannel(socket)
        incoming.offer(channel)
        trace.event("daemon peer accepted uid=${credentials.uid} pid=${credentials.pid}")
    }

    private companion object {
        const val ROOT_UID = 0
        const val CHANNEL_WAIT_MILLIS = 5_000L
        const val LISTENER_RETRY_MILLIS = 1_000L
    }
}

/** Two daemon connections, matching the protocol's control and subscribe lanes. */
internal class IncomingDaemonChannels(
    capacity: Int = 2,
) {
    private val channels = ArrayBlockingQueue<DaemonChannel>(capacity)

    /** Keeps the newest channels so a daemon restart cannot leave closed ones blocking the pool. */
    fun offer(channel: DaemonChannel) {
        while (!channels.offer(channel)) {
            channels.poll()?.close()
        }
    }

    @Throws(InterruptedException::class)
    fun take(timeoutMillis: Long): DaemonChannel? =
        channels.poll(timeoutMillis, TimeUnit.MILLISECONDS)
}

private class LocalSocketChannel(private val socket: LocalSocket) : DaemonChannel {
    private val input = socket.inputStream
    private val output = socket.outputStream
    private var pendingDescriptors: Array<FileDescriptor>? = null

    override fun writeFrame(body: ByteArray) {
        DaemonFrameCodec.writeFrame(output, body)
    }

    override fun readFrame(): ByteArray {
        val body = DaemonFrameCodec.readFrame(input)
        // Ancillary descriptors belong to the frame that carried them, so they are
        // collected here and not on some later read.
        pendingDescriptors = socket.ancillaryFileDescriptors
        return body
    }

    override fun takeFileDescriptors(): Array<FileDescriptor>? {
        val descriptors = pendingDescriptors
        pendingDescriptors = null
        return descriptors
    }

    override fun close() {
        runCatching { socket.close() }
    }
}
