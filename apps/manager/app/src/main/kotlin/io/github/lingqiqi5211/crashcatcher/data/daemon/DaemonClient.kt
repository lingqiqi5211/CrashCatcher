package io.github.lingqiqi5211.crashcatcher.data.daemon

import io.github.lingqiqi5211.crashcatcher.domain.model.DomainError
import io.github.lingqiqi5211.crashcatcher.domain.model.DomainErrorCode
import io.github.lingqiqi5211.crashcatcher.domain.model.DomainErrorKind
import java.io.FileDescriptor
import kotlinx.coroutines.Dispatchers
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
) {
    private val mutex = Mutex()
    private var channel: DaemonChannel? = null
    private var nextSeq = 1L

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
        // Connect on first use rather than making every caller remember to. The
        // retry below is then only about a genuine mid-conversation drop, not about
        // never having been connected.
        if (channel == null) openLocked()

        // One transparent reconnect: the daemon restarting between two screens is
        // ordinary, and making every caller handle it would spread the retry
        // everywhere. A rejection is not retried — it would fail identically.
        try {
            exchangeLocked(request)
        } catch (cause: DaemonException) {
            if (cause is DaemonException.Rejected) throw cause
            closeLocked()
            openLocked()
            exchangeLocked(request)
        }
    }

    private suspend fun openLocked() {
        val opened = withContext(Dispatchers.IO) { transport.open(ChannelHello.Control) }
        channel = opened
        nextSeq = 1

        val reply = exchangeLocked(
            WireRequest.Handshake(
                protocolVersion = DaemonConstants.PROTOCOL_VERSION,
                clientVersion = clientVersion,
            ),
        )
        val handshake = reply.response as? WireResponse.Handshake
            ?: throw DaemonException.ProtocolViolation(
                "expected a handshake reply, got ${reply.response::class.simpleName}",
            )
        if (handshake.protocolVersion != DaemonConstants.PROTOCOL_VERSION) {
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
    }

    private fun closeLocked() {
        channel?.close()
        channel = null
        daemonVersion = null
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
            DaemonReply(response.result(), descriptors)
        }
    }

    override fun toString(): String = "DaemonClient(daemonVersion=$daemonVersion)"
}

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

    is DaemonException.ConnectionClosed, is DaemonException.Timeout -> DomainError(
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
