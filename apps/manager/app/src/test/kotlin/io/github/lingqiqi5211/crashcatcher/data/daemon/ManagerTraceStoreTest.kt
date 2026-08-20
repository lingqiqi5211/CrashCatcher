package io.github.lingqiqi5211.crashcatcher.data.daemon

import java.nio.file.Files
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ManagerTraceStoreTest {

    @Test
    fun `events and failures are available without logcat`() {
        val directory = Files.createTempDirectory("manager-trace").toFile()
        try {
            val store = ManagerTraceStore(directory, nowMillis = { 0L })

            store.event("socket connect begin")
            store.failure("socket connect failed", IllegalStateException("permission denied"))

            val text = store.readAll().getValue("manager.log")
            assertTrue(text.contains("1970-01-01T00:00:00Z DEBUG socket connect begin"))
            assertTrue(text.contains("ERROR socket connect failed"))
            assertTrue(text.contains("IllegalStateException: permission denied"))
        } finally {
            directory.deleteRecursively()
        }
    }

    @Test
    fun `the trace rotates instead of growing without bound`() {
        val directory = Files.createTempDirectory("manager-trace").toFile()
        try {
            val store = ManagerTraceStore(
                directory = directory,
                nowMillis = { 0L },
                maxBytes = 48,
            )

            store.event("first connection attempt")
            store.event("second connection attempt")

            val logs = store.readAll()
            assertEquals(
                setOf("manager-previous.log", "manager.log"),
                logs.keys,
            )
            assertTrue(logs.getValue("manager-previous.log").contains("first connection"))
            assertTrue(logs.getValue("manager.log").contains("second connection"))
        } finally {
            directory.deleteRecursively()
        }
    }

    @Test
    fun `legacy routine frame lines are omitted from an exported trace`() {
        val directory = Files.createTempDirectory("manager-trace").toFile()
        try {
            val store = ManagerTraceStore(directory, nowMillis = { 0L })
            store.event("frame send seq=2 request=ListGroups")
            store.event("frame received seq=2 response=Groups")
            store.event("handshake accepted daemon_protocol=5")

            val text = store.readAll().getValue("manager.log")

            assertFalse(text.contains("frame send"))
            assertFalse(text.contains("frame received"))
            assertTrue(text.contains("handshake accepted"))
        } finally {
            directory.deleteRecursively()
        }
    }
}
