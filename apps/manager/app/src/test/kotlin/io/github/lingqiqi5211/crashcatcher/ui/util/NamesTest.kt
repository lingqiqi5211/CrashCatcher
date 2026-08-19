package io.github.lingqiqi5211.crashcatcher.ui.util

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Not everything that crashes is an app.
 *
 * A tombstone identifies its process, so platform binaries reach the manager with a path where a
 * package name belongs. These two decide how one is named on screen, and both are consulted
 * before the daemon's own verdict has arrived — so their edges are what the first frame shows.
 */
class NamesTest {

    @Test
    fun `a path or a dotless name cannot be a package`() {
        // Android refuses a package name containing `/`, and requires at least one `.`.
        assertFalse(couldBePackageName("/vendor/bin/hw/android.hardware.audio.service_64"))
        assertFalse(couldBePackageName("./bluetooth_audio_provider_session_pcm192_probe"))
        assertFalse(couldBePackageName("system_server"))
        assertFalse(couldBePackageName("surfaceflinger"))
        assertFalse(couldBePackageName(""))
    }

    @Test
    fun `an ordinary package name is recognised`() {
        assertTrue(couldBePackageName("com.example.app"))
        assertTrue(couldBePackageName("com.example.app:remote"))
        // Two segments is the minimum Android accepts, and this has them.
        assertTrue(couldBePackageName("a.b"))
    }

    @Test
    fun `a process is named by its binary, not its directory`() {
        assertEquals(
            "android.hardware.audio.service_64",
            processDisplayName("/vendor/bin/hw/android.hardware.audio.service_64"),
        )
        assertEquals(
            "bluetooth_audio_provider_session_pcm192_probe",
            processDisplayName("./bluetooth_audio_provider_session_pcm192_probe"),
        )
    }

    @Test
    fun `a name with no directory part is left alone`() {
        assertEquals("system_server", processDisplayName("system_server"))
        assertEquals("com.example.app", processDisplayName("com.example.app"))
    }

    /**
     * Trailing slashes and a bare `/` would otherwise yield an empty heading — a row with no
     * name at all, which reads as a rendering fault rather than as an odd process.
     */
    @Test
    fun `degenerate paths still produce a name`() {
        assertEquals("hw", processDisplayName("/vendor/bin/hw/"))
        assertEquals("/", processDisplayName("/"))
        assertEquals("  ", processDisplayName("  "))
    }
}
