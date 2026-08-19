package io.github.lingqiqi5211.crashcatcher.data.daemon

import io.github.lingqiqi5211.crashcatcher.domain.model.DomainErrorCode
import io.github.lingqiqi5211.crashcatcher.domain.model.DomainErrorKind
import java.io.FileDescriptor
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The client's protocol behaviour, exercised through a fake transport.
 *
 * No device needed: sequence pairing, handshake validation, reconnection and error
 * mapping are where the bugs are, and none of them require a real socket.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class DaemonClientTest {

    @Test
    fun `the handshake runs before any other request`() = runTest {
        val transport = FakeTransport()
        transport.script += handshakeReply(seq = 1)

        val client = DaemonClient(transport, clientVersion = "0.1.0")
        client.connect()

        assertEquals(listOf(ChannelHello.Control), transport.openedLanes)
        val first = transport.sentRequests.first()
        assertTrue(first.request is WireRequest.Handshake)
        assertEquals("0.1.0", client.daemonVersion)
    }

    @Test
    fun `a protocol version mismatch is refused rather than tolerated`() = runTest {
        val transport = FakeTransport()
        transport.script += """{"seq":1,"ok":{"response":"handshake_result","protocol_version":99,"daemon_version":"9.9.9"}}"""

        val client = DaemonClient(transport, clientVersion = "0.1.0")
        val failure = runCatching { client.connect() }.exceptionOrNull()

        assertTrue(failure is DaemonException.Rejected)
        assertEquals(
            WireErrorCode.VersionMismatch,
            (failure as DaemonException.Rejected).error.code,
        )
    }

    @Test
    fun `requests carry increasing sequence numbers`() = runTest {
        val transport = FakeTransport()
        transport.script += handshakeReply(seq = 1)
        transport.script += """{"seq":2,"ok":{"response":"closed"}}"""
        transport.script += """{"seq":3,"ok":{"response":"closed"}}"""

        val client = DaemonClient(transport, clientVersion = "0.1.0")
        client.request(WireRequest.ClosePayload(handle = 1))
        client.request(WireRequest.ClosePayload(handle = 2))

        assertEquals(listOf(1L, 2L, 3L), transport.sentRequests.map { it.seq })
    }

    @Test
    fun `a reply for the wrong sequence number is caught`() = runTest {
        val transport = FakeTransport()
        transport.script += handshakeReply(seq = 1)
        // The daemon answers a seq we never sent.
        transport.script += """{"seq":99,"ok":{"response":"closed"}}"""
        // The client reconnects once and tries again; make that attempt fail the same
        // way so the failure surfaces instead of being papered over.
        transport.script += handshakeReply(seq = 1)
        transport.script += """{"seq":99,"ok":{"response":"closed"}}"""

        val client = DaemonClient(transport, clientVersion = "0.1.0")
        val failure = runCatching { client.request(WireRequest.ClosePayload(1)) }.exceptionOrNull()

        assertTrue(
            "a mismatched seq must not be believed",
            failure is DaemonException.ProtocolViolation,
        )
    }

    @Test
    fun `a dropped connection is retried once transparently`() = runTest {
        val transport = FakeTransport()
        transport.script += handshakeReply(seq = 1)
        transport.script += FakeTransport.CLOSE_MARKER
        transport.script += handshakeReply(seq = 1)
        transport.script += """{"seq":2,"ok":{"response":"closed"}}"""

        val client = DaemonClient(transport, clientVersion = "0.1.0")
        val reply = client.request(WireRequest.ClosePayload(handle = 1))

        assertEquals(WireResponse.Closed, reply.response)
        assertEquals(
            "the daemon restarting between two screens is ordinary",
            2,
            transport.openedLanes.size,
        )
    }

    @Test
    fun `a rejection is surfaced rather than retried`() = runTest {
        val transport = FakeTransport()
        transport.script += handshakeReply(seq = 1)
        transport.script += """{"seq":2,"err":{"code":"not_found","message":"gone"}}"""

        val client = DaemonClient(transport, clientVersion = "0.1.0")
        val failure = runCatching {
            client.request(WireRequest.GetRecord(RecordId("0".repeat(26))))
        }.exceptionOrNull()

        assertTrue(failure is DaemonException.Rejected)
        // Retrying a rejection would just fail again, so it must not reconnect.
        assertEquals(1, transport.openedLanes.size)
    }

    @Test
    fun `descriptors travel with the frame that carried them`() = runTest {
        val transport = FakeTransport()
        transport.script += handshakeReply(seq = 1)
        transport.script += """{"seq":2,"ok":{"response":"payload_opened","payload":{"total_bytes":10,"state":"present","codec_on_disk":"zstd","fd_attached":true}}}"""
        transport.descriptorsForFrame = arrayOf(FileDescriptor())

        val client = DaemonClient(transport, clientVersion = "0.1.0")
        val reply = client.request(WireRequest.OpenPayload(RecordId("0".repeat(26))))

        val opened = reply.response as WireResponse.PayloadOpenedResponse
        assertTrue(opened.payload.fdAttached)
        assertEquals(1, reply.fileDescriptors?.size)
    }

    @Test
    fun `daemon errors map onto the domain vocabulary`() {
        val rejected = DaemonException.Rejected(
            WireError(WireErrorCode.CursorInvalidated, "stale"),
        ).toDomainError()
        // Its own code, because the fix is to restart the query rather than retry.
        assertEquals(DomainErrorCode.CursorInvalidated, rejected.code)
        assertEquals(DomainErrorKind.DaemonRejected, rejected.kind)

        val closed = DaemonException.ConnectionClosed("gone").toDomainError()
        assertEquals(DomainErrorKind.ConnectionLost, closed.kind)

        val malformed = DaemonException.MalformedFrame("bad").toDomainError()
        assertEquals(DomainErrorKind.ProtocolError, malformed.kind)

        val mismatch = DaemonException.Rejected(
            WireError(WireErrorCode.VersionMismatch, "old"),
        ).toDomainError()
        assertEquals(DomainErrorKind.VersionMismatch, mismatch.kind)
    }

    private fun handshakeReply(seq: Long) =
        """{"seq":$seq,"ok":{"response":"handshake_result","protocol_version":${DaemonConstants.PROTOCOL_VERSION},"daemon_version":"0.1.0"}}"""
}

/** A transport that replays a scripted sequence of reply frames. */
private class FakeTransport : DaemonTransport {
    val script = mutableListOf<String>()
    val openedLanes = mutableListOf<ChannelHello>()
    val sentRequests = mutableListOf<RequestEnvelope>()
    var descriptorsForFrame: Array<FileDescriptor>? = null

    private var cursor = 0

    override fun open(lane: ChannelHello): DaemonChannel {
        openedLanes += lane
        return object : DaemonChannel {
            private var pending: Array<FileDescriptor>? = null

            override fun writeFrame(body: ByteArray) {
                val text = body.decodeToString()
                // The lane hello is not a request envelope; skip it.
                if (text.contains("\"kind\"")) return
                sentRequests += DaemonJson.decodeFromString<RequestEnvelope>(text)
            }

            override fun readFrame(): ByteArray {
                if (cursor >= script.size) {
                    throw DaemonException.ConnectionClosed("script exhausted")
                }
                val next = script[cursor++]
                if (next == CLOSE_MARKER) {
                    throw DaemonException.ConnectionClosed("scripted disconnect")
                }
                pending = descriptorsForFrame
                return next.encodeToByteArray()
            }

            override fun takeFileDescriptors(): Array<FileDescriptor>? {
                val taken = pending
                pending = null
                return taken
            }

            override fun close() = Unit
        }
    }

    companion object {
        const val CLOSE_MARKER = "<<close>>"
    }
}
