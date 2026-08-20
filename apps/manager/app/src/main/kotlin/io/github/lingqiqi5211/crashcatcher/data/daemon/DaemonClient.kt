package io.github.lingqiqi5211.crashcatcher.data.daemon

import io.github.lingqiqi5211.crashcatcher.domain.model.DomainError
import io.github.lingqiqi5211.crashcatcher.domain.model.DomainErrorCode
import io.github.lingqiqi5211.crashcatcher.domain.model.DomainErrorKind
import java.io.FileDescriptor
import java.io.IOException
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/** A response plus any descriptor that came with it. */
data class DaemonReply(
    val response: WireResponse,
    val fileDescriptors: Array<FileDescriptor>? = null,
) {
    // Array fields break data-class equality; identity is all callers need here.
    override fun equals(other: Any?): Boolean = this === other
    override fun hashCode(): Int = System.identityHashCode(this)
}

/**
 * Request/response over one control channel.
 *
 * Calls are serialised by a mutex rather than routed by sequence number. The manager
 * never needs pipelining — every screen awaits its own answer — and one-at-a-time
 * makes reply pairing correct by construction instead of by a router that has to
 * handle out-of-order, duplicated and unsolicited frames. Sequence numbers are still
 * sent and verified, so a daemon that answers the wrong one is caught rather than
 * silently believed.
 */
class DaemonClient(
    private val transport: DaemonTransport,
    private val clientVersion: String,
    private val trace: DaemonTrace = NoopDaemonTrace,
) {
    private val mutex = Mutex()
    private var channel: DaemonChannel? = null
    private var nextSeq = 1L

    private val connectedState = MutableStateFlow(false)

    /**
     * Whether a channel is up right now.
     *
     * Nothing polls the daemon, so without this the only screen that learns it went away is
     * whichever one happens to ask for something next — the overview goes on saying 运行中 while
     * the log page two taps away is reporting a failed read. Published from here because this is
     * the one place that finds out, whoever's request it was.
     */
    val connected: StateFlow<Boolean> = connectedState.asStateFlow()

    /** Protocol version reported by the daemon, once the handshake has run. */
    var daemonVersion: String? = null
        private set

    suspend fun connect(): Unit = mutex.withLock {
        if (channel != null) return
        openLocked()
    }

    /** Drops the connection so the next call reconnects. */
    suspend fun disconnect(): Unit = mutex.withLock {
        closeLocked()
    }

    suspend fun request(request: WireRequest): DaemonReply = mutex.withLock {
        // One transparent reconnect: the daemon restarting between two screens is
        // ordinary, including when a stale pending socket is picked on the first request.
        // Making every caller handle it would spread the retry everywhere. A rejection is not
        // retried — it would fail identically.
        //
        // Any exception, not only `DaemonException`. Writing to a socket whose peer has gone
        // raises a plain `IOException` (EPIPE, ECONNRESET), and while that was slipping past
        // this handler the dead channel stayed installed: no reconnect, and the client went on
        // reporting itself connected, so every later request failed the same way and only the
        // 重新连接 button could recover it.
        try {
            // Connect on first use rather than making every caller remember to. This stays inside
            // the retry boundary because a daemon restart can leave one closed pending channel
            // ahead of its fresh replacement in the listener queue.
            if (channel == null) openLocked()
            exchangeLocked(request)
        } catch (cause: CancellationException) {
            throw cause
        } catch (cause: Exception) {
            if (cause is DaemonException.Rejected) {
                trace.failure("request rejected without retry request=${request.traceName}", cause)
                throw cause
            }
            trace.failure("request failed; reconnecting once request=${request.traceName}", cause)
            closeLocked()
            // A failed retry leaves nothing usable behind, so it closes rather than keeping a
            // channel that only the next caller would discover is dead.
            try {
                openLocked()
                exchangeLocked(request)
            } catch (retry: CancellationException) {
                throw retry
            } catch (retry: Exception) {
                trace.failure("request retry failed request=${request.traceName}", retry)
                closeLocked()
                throw retry
            }
        }
    }

    private suspend fun openLocked() {
        trace.event("control channel open begin")
        val opened = try {
            withContext(Dispatchers.IO) { transport.open(ChannelHello.Control) }
        } catch (cause: Throwable) {
            trace.failure("control channel open failed", cause)
            throw cause
        }
        channel = opened
        nextSeq = 1
        trace.event(
            "handshake begin manager_protocol=${DaemonConstants.PROTOCOL_VERSION} " +
                "manager_version=$clientVersion",
        )

        val handshake = try {
            val reply = exchangeLocked(
                WireRequest.Handshake(
                    protocolVersion = DaemonConstants.PROTOCOL_VERSION,
                    clientVersion = clientVersion,
                ),
            )
            reply.response as? WireResponse.Handshake
                ?: throw DaemonException.ProtocolViolation(
                    "expected a handshake reply, got ${reply.response::class.simpleName}",
                )
        } catch (cause: Throwable) {
            trace.failure("handshake failed", cause)
            closeLocked()
            throw cause
        }
        if (handshake.protocolVersion != DaemonConstants.PROTOCOL_VERSION) {
            trace.event(
                "handshake version mismatch daemon_protocol=${handshake.protocolVersion} " +
                    "manager_protocol=${DaemonConstants.PROTOCOL_VERSION}",
            )
            closeLocked()
            throw DaemonException.Rejected(
                WireError(
                    WireErrorCode.VersionMismatch,
                    "daemon speaks protocol ${handshake.protocolVersion}, " +
                        "this manager speaks ${DaemonConstants.PROTOCOL_VERSION}",
                ),
            )
        }
        daemonVersion = handshake.daemonVersion
        connectedState.value = true
        trace.event(
            "handshake accepted daemon_protocol=${handshake.protocolVersion} " +
                "daemon_version=${handshake.daemonVersion}",
        )
    }

    private fun closeLocked() {
        if (channel != null) trace.event("control channel closing")
        channel?.close()
        channel = null
        daemonVersion = null
        connectedState.value = false
    }

    private suspend fun exchangeLocked(request: WireRequest): DaemonReply {
        val active = channel ?: throw DaemonException.ConnectionClosed("not connected")
        val seq = nextSeq++

        return withContext(Dispatchers.IO) {
            val envelope = RequestEnvelope(seq = seq, request = request)
            active.writeFrame(DaemonJson.encodeToString(envelope).encodeToByteArray())

            val body = active.readFrame()
            val descriptors = active.takeFileDescriptors()
            val response = try {
                DaemonJson.decodeFromString<ResponseEnvelope>(body.decodeToString())
            } catch (cause: Exception) {
                throw DaemonException.MalformedFrame("response did not parse: ${cause.message}")
            }
            if (response.seq != seq) {
                throw DaemonException.ProtocolViolation(
                    "asked for seq $seq, daemon answered seq ${response.seq}",
                )
            }
            response.err?.let { error ->
                trace.event("request rejected seq=$seq code=${error.code}")
            }
            val result = response.result()
            DaemonReply(result, descriptors)
        }
    }

    override fun toString(): String = "DaemonClient(daemonVersion=$daemonVersion)"
}

private val WireRequest.traceName: String
    get() = this::class.simpleName ?: "UnknownRequest"

/**
 * Turns a transport-level or daemon-level failure into something the UI can act on.
 *
 * The mapping is deliberate about one case: `CursorInvalidated` keeps its own code
 * because the right response is to restart the query, not to retry the request that
 * would fail the same way.
 */
fun Throwable.toDomainError(): DomainError = when (this) {
    is DaemonException.Rejected -> DomainError(
        kind = when (error.code) {
            WireErrorCode.VersionMismatch -> DomainErrorKind.VersionMismatch
            WireErrorCode.Unavailable -> DomainErrorKind.Unavailable
            WireErrorCode.MalformedFrame -> DomainErrorKind.ProtocolError
            else -> DomainErrorKind.DaemonRejected
        },
        message = error.message,
        code = error.code.toDomainCode(),
    )

    // `IOException` alongside the two named cases: a broken pipe or a reset from the socket is
    // a lost connection, and calling it Unknown put "出错了，稍后重试" on screen for the one
    // failure whose cause is worth naming.
    is DaemonException.ConnectionClosed, is DaemonException.Timeout, is IOException -> DomainError(
        kind = DomainErrorKind.ConnectionLost,
        message = message ?: "connection lost",
    )

    is DaemonException.MalformedFrame,
    is DaemonException.ProtocolViolation,
    is DaemonException.PayloadTooLarge,
    -> DomainError(
        kind = DomainErrorKind.ProtocolError,
        message = message ?: "protocol error",
    )

    else -> DomainError(
        kind = DomainErrorKind.Unknown,
        message = message ?: this::class.simpleName.orEmpty(),
    )
}

private fun WireErrorCode.toDomainCode(): DomainErrorCode? = when (this) {
    WireErrorCode.Unauthorized -> DomainErrorCode.Unauthorized
    WireErrorCode.NotFound -> DomainErrorCode.NotFound
    WireErrorCode.InvalidRequest -> DomainErrorCode.InvalidRequest
    WireErrorCode.PayloadTooLarge -> DomainErrorCode.PayloadTooLarge
    WireErrorCode.CursorInvalidated -> DomainErrorCode.CursorInvalidated
    WireErrorCode.Internal -> DomainErrorCode.Internal
    WireErrorCode.MalformedFrame, WireErrorCode.Unavailable, WireErrorCode.VersionMismatch -> null
}
