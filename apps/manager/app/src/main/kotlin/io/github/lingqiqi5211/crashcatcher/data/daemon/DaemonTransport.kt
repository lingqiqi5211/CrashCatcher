package io.github.lingqiqi5211.crashcatcher.data.daemon

import android.net.LocalSocket
import android.net.LocalSocketAddress
import java.io.Closeable
import java.io.FileDescriptor

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

/** Opens channels. Separate from [DaemonClient] so reconnection is a transport concern. */
interface DaemonTransport {
    fun open(lane: ChannelHello): DaemonChannel
}

/**
 * Connects to the daemon's abstract-namespace socket.
 *
 * Abstract namespace, so there is no filesystem object and no path permissions to
 * get wrong; the daemon authenticates us from `SO_PEERCRED` and our signing
 * certificate instead.
 */
class LocalSocketTransport(
    private val socketName: String = DaemonConstants.ABSTRACT_SOCKET_NAME,
) : DaemonTransport {

    override fun open(lane: ChannelHello): DaemonChannel {
        val socket = LocalSocket(LocalSocket.SOCKET_STREAM)
        try {
            socket.connect(LocalSocketAddress(socketName, LocalSocketAddress.Namespace.ABSTRACT))
        } catch (cause: Exception) {
            runCatching { socket.close() }
            throw DaemonException.ConnectionClosed(
                "could not connect to @$socketName: ${cause.message}",
            )
        }

        val channel = LocalSocketChannel(socket)
        // The lane is decided by the very first frame, before anything else is sent.
        channel.writeFrame(DaemonJson.encodeToString(lane).encodeToByteArray())
        return channel
    }
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
