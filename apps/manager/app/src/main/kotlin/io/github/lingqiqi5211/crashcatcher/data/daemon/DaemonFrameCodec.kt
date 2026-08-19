package io.github.lingqiqi5211.crashcatcher.data.daemon

import java.io.InputStream
import java.io.OutputStream

/**
 * Length-prefixed framing, mirroring `cch_wire::frame`.
 *
 * Four bytes of big-endian length then the body. Big-endian because that is what
 * the daemon writes; a little-endian reader would see a plausible-looking but wrong
 * length and then block forever waiting for bytes that are not coming.
 */
object DaemonFrameCodec {

    fun encodeFrame(body: ByteArray): ByteArray {
        if (body.size > DaemonConstants.MAX_FRAME_BODY_BYTES) {
            throw DaemonException.PayloadTooLarge(body.size.toLong())
        }
        val frame = ByteArray(DaemonConstants.LENGTH_PREFIX_BYTES + body.size)
        writeLength(body.size, frame)
        body.copyInto(frame, destinationOffset = DaemonConstants.LENGTH_PREFIX_BYTES)
        return frame
    }

    fun decodeFrame(frame: ByteArray): ByteArray {
        if (frame.size < DaemonConstants.LENGTH_PREFIX_BYTES) {
            throw DaemonException.MalformedFrame("frame is shorter than its length prefix")
        }
        val declared = readLength(frame)
        if (declared > DaemonConstants.MAX_FRAME_BODY_BYTES.toLong()) {
            throw DaemonException.PayloadTooLarge(declared)
        }
        val actual = (frame.size - DaemonConstants.LENGTH_PREFIX_BYTES).toLong()
        if (actual != declared) {
            throw DaemonException.MalformedFrame(
                "frame body length mismatch: declared $declared bytes, got $actual bytes",
            )
        }
        return frame.copyOfRange(DaemonConstants.LENGTH_PREFIX_BYTES, frame.size)
    }

    /** Reads exactly one frame body, blocking until it is complete. */
    fun readFrame(input: InputStream): ByteArray {
        val prefix = input.readExactly(DaemonConstants.LENGTH_PREFIX_BYTES)
        val declared = readLength(prefix)
        if (declared > DaemonConstants.MAX_FRAME_BODY_BYTES.toLong()) {
            // Refuse before allocating: a bogus prefix must not be able to ask for
            // an arbitrary allocation.
            throw DaemonException.PayloadTooLarge(declared)
        }
        return input.readExactly(declared.toInt())
    }

    fun writeFrame(output: OutputStream, body: ByteArray) {
        output.write(encodeFrame(body))
        output.flush()
    }

    private fun readLength(bytes: ByteArray): Long =
        ((bytes[0].toLong() and 0xff) shl 24) or
            ((bytes[1].toLong() and 0xff) shl 16) or
            ((bytes[2].toLong() and 0xff) shl 8) or
            (bytes[3].toLong() and 0xff)

    private fun writeLength(length: Int, frame: ByteArray) {
        frame[0] = (length ushr 24).toByte()
        frame[1] = (length ushr 16).toByte()
        frame[2] = (length ushr 8).toByte()
        frame[3] = length.toByte()
    }

    private fun InputStream.readExactly(length: Int): ByteArray {
        val bytes = ByteArray(length)
        var offset = 0
        while (offset < length) {
            val read = read(bytes, offset, length - offset)
            if (read < 0) {
                throw DaemonException.ConnectionClosed(
                    "stream ended after $offset of $length bytes",
                )
            }
            offset += read
        }
        return bytes
    }
}
