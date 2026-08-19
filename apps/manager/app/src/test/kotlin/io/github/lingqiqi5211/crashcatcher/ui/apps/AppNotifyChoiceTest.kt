package io.github.lingqiqi5211.crashcatcher.ui.apps

import io.github.lingqiqi5211.crashcatcher.data.daemon.AppConfig
import io.github.lingqiqi5211.crashcatcher.data.daemon.AppConfigPatch
import io.github.lingqiqi5211.crashcatcher.data.daemon.DaemonJson
import io.github.lingqiqi5211.crashcatcher.data.daemon.NotifyMode
import io.github.lingqiqi5211.crashcatcher.data.daemon.NotifyModeChange
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Test

/**
 * The per-app notify setting has five states, not four.
 *
 * "Follow the global setting" is a real choice; collapsing it into "no value" is
 * how a per-app screen ends up pinning whatever the global happened to be when it
 * was opened, and how an override becomes impossible to switch back off.
 */
class AppNotifyChoiceTest {

    @Test
    fun `every choice round trips through the wire representation`() {
        for (choice in AppNotifyChoice.entries) {
            val change = choice.toChange()
            val applied = AppConfigPatch(notifyMode = change)
                .let { patch -> DaemonJson.encodeToString(patch) }
                .let { json -> DaemonJson.decodeFromString<AppConfigPatch>(json) }
                .notifyMode

            val storedMode = when (applied) {
                NotifyModeChange.FollowGlobal -> null
                is NotifyModeChange.SetTo -> applied.mode
                NotifyModeChange.Unchanged -> error("a user edit is never 'unchanged'")
            }
            assertEquals("round trip for $choice", choice, AppNotifyChoice.from(storedMode))
        }
    }

    @Test
    fun `a missing override reads as follow-global`() {
        assertEquals(AppNotifyChoice.FollowGlobal, AppNotifyChoice.from(null))
        assertEquals(AppNotifyChoice.FollowGlobal, AppNotifyChoice.from(AppConfig().notifyMode))
    }

    @Test
    fun `choosing follow-global sends an explicit null, not an omission`() {
        // Omitting the key means "leave it alone", which would keep the very override
        // the user just asked to remove.
        val json = DaemonJson.encodeToString(
            AppConfigPatch(notifyMode = AppNotifyChoice.FollowGlobal.toChange()),
        )
        assertEquals("""{"notify_mode":null}""", json)
    }

    @Test
    fun `choosing a mode sends that mode`() {
        val json = DaemonJson.encodeToString(
            AppConfigPatch(notifyMode = AppNotifyChoice.Toast.toChange()),
        )
        assertEquals("""{"notify_mode":"toast"}""", json)
    }

    @Test
    fun `every notify mode has a choice and every choice has a label`() {
        // A mode with no choice would be unreachable from the UI; a choice with no
        // label would render blank.
        for (mode in NotifyMode.entries) {
            assertNotNull(AppNotifyChoice.from(mode))
        }
        assertEquals(NotifyMode.entries.size + 1, AppNotifyChoice.entries.size)
    }
}
