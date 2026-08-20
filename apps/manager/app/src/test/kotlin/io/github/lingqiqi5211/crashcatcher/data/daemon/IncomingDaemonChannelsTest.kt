package io.github.lingqiqi5211.crashcatcher.data.daemon

import java.io.FileDescriptor
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

class IncomingDaemonChannelsTest {

    @Test
    fun `a daemon restart replaces the oldest pending connection`() {
        val incoming = IncomingDaemonChannels(capacity = 2)
        val stale = FakeChannel()
        val firstFresh = FakeChannel()
        val secondFresh = FakeChannel()

        incoming.offer(stale)
        incoming.offer(firstFresh)
        incoming.offer(secondFresh)

        assertTrue(stale.closed)
        assertSame(firstFresh, incoming.take(timeoutMillis = 0))
        assertSame(secondFresh, incoming.take(timeoutMillis = 0))
    }
}

private class FakeChannel : DaemonChannel {
    var closed = false

    override fun writeFrame(body: ByteArray) = Unit

    override fun readFrame(): ByteArray = ByteArray(0)

    override fun takeFileDescriptors(): Array<FileDescriptor>? = null

    override fun close() {
        closed = true
    }
}
