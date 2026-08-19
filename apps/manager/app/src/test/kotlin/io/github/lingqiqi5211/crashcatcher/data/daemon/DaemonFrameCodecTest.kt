package io.github.lingqiqi5211.crashcatcher.data.daemon

import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class DaemonFrameCodecTest {

    @Test
    fun `frames round trip`() {
        val body = """{"seq":1}""".encodeToByteArray()
        val frame = DaemonFrameCodec.encodeFrame(body)
        assertArrayEquals(body, DaemonFrameCodec.decodeFrame(frame))
    }

    @Test
    fun `an empty body is legal`() {
        val frame = DaemonFrameCodec.encodeFrame(ByteArray(0))
        assertArrayEquals(byteArrayOf(0, 0, 0, 0), frame)
        assertEquals(0, DaemonFrameCodec.decodeFrame(frame).size)
    }

    @Test
    fun `the length prefix is big endian`() {
        // 0x0102 is 258. A little-endian writer would emit 02 01 00 00, which the
        // daemon would read as a plausible but wrong length and then block forever.
        val frame = DaemonFrameCodec.encodeFrame(ByteArray(258))
        assertArrayEquals(byteArrayOf(0x00, 0x00, 0x01, 0x02), frame.copyOfRange(0, 4))
    }

    @Test
    fun `oversized bodies are refused before allocating`() {
        val tooBig = ByteArray(DaemonConstants.MAX_FRAME_BODY_BYTES + 1)
        val encodeFailure = runCatching { DaemonFrameCodec.encodeFrame(tooBig) }.exceptionOrNull()
        assertTrue(encodeFailure is DaemonException.PayloadTooLarge)

        // A prefix claiming more than the limit must be rejected without trying to
        // allocate what it asked for.
        val lyingPrefix = byteArrayOf(0x7f, 0x7f, 0x7f, 0x7f)
        val readFailure = runCatching {
            DaemonFrameCodec.readFrame(ByteArrayInputStream(lyingPrefix))
        }.exceptionOrNull()
        assertTrue(readFailure is DaemonException.PayloadTooLarge)
    }

    @Test
    fun `truncated and mismatched frames are rejected`() {
        assertTrue(
            runCatching { DaemonFrameCodec.decodeFrame(byteArrayOf(0, 0)) }
                .exceptionOrNull() is DaemonException.MalformedFrame,
        )
        // Declares four bytes, carries two.
        assertTrue(
            runCatching { DaemonFrameCodec.decodeFrame(byteArrayOf(0, 0, 0, 4, 1, 2)) }
                .exceptionOrNull() is DaemonException.MalformedFrame,
        )
    }

    @Test
    fun `a stream that ends mid-frame reports a closed connection`() {
        // Distinct from a malformed frame: the fix is to reconnect, not to give up on
        // the protocol.
        val partial = byteArrayOf(0, 0, 0, 8, 1, 2, 3)
        val failure = runCatching {
            DaemonFrameCodec.readFrame(ByteArrayInputStream(partial))
        }.exceptionOrNull()
        assertTrue(failure is DaemonException.ConnectionClosed)
    }

    @Test
    fun `frames stream back to back`() {
        val output = ByteArrayOutputStream()
        val bodies = listOf("first", "second", "third").map { it.encodeToByteArray() }
        bodies.forEach { DaemonFrameCodec.writeFrame(output, it) }

        val input = ByteArrayInputStream(output.toByteArray())
        bodies.forEach { expected ->
            assertArrayEquals(expected, DaemonFrameCodec.readFrame(input))
        }
    }

    @Test
    fun `a body at the exact limit is accepted`() {
        val body = ByteArray(DaemonConstants.MAX_FRAME_BODY_BYTES)
        val frame = DaemonFrameCodec.encodeFrame(body)
        assertEquals(DaemonConstants.MAX_FRAME_BODY_BYTES, DaemonFrameCodec.decodeFrame(frame).size)
    }
}
