package io.github.lingqiqi5211.crashcatcher.ui.detail

import android.content.Intent
import android.os.Bundle
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.annotation.StringRes
import androidx.activity.compose.setContent
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.lifecycleScope
import io.github.lingqiqi5211.crashcatcher.CrashCatcherApplication
import io.github.lingqiqi5211.crashcatcher.MainActivity
import io.github.lingqiqi5211.crashcatcher.R
import io.github.lingqiqi5211.crashcatcher.data.daemon.MuteScope
import io.github.lingqiqi5211.crashcatcher.data.daemon.RecordId
import io.github.lingqiqi5211.crashcatcher.ui.theme.ManagerTheme
import kotlinx.coroutines.launch

/**
 * The crash alert, and the landing point for notification taps.
 *
 * Two jobs the app previously had no home for:
 *
 * - `NotifyMode.Dialog` asks the daemon's privileged bridge to start
 *   `.ui.detail.CrashDetailActivity`. Nothing declared that name, so `startActivityAsUser`
 *   failed silently and choosing 弹窗 appeared to do nothing at all.
 * - The bridge's notification tap and its action buttons fire an intent with the action
 *   [BRIDGE_ACTION]. Nothing declared that filter either, so tapping a notification did
 *   nothing.
 *
 * Themed as a dialog rather than a full screen: this is what replaces the system's
 * "已停止运行" box, and it appears over whatever the user was doing, so it has to read as
 * an interruption that can be dismissed — not as the app having been opened.
 */
class CrashDetailActivity : ComponentActivity() {

    /** The alert may start the process, but it shares its one daemon listener with the manager. */
    private val container by lazy {
        (application as CrashCatcherApplication).container
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        handleIntent(intent)
    }

    /**
     * A relaunch while already showing replaces the contents.
     *
     * `singleTask` means a second crash arriving reuses this instance, and without this
     * the dialog would keep describing the first one.
     */
    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleIntent(intent)
    }

    private fun handleIntent(intent: Intent) {
        val recordId = intent.getStringExtra(EXTRA_RECORD_ID)?.takeIf { it.isNotBlank() }
        if (recordId == null) {
            // Nothing to show. Reached by a stale notification whose record has since been
            // deleted; opening the app is more useful than an empty dialog.
            openManager(recordId = null)
            return
        }

        // The buttons on a notification are requests to act, not to be shown a dialog.
        //
        // Each one takes the notification down with it. Nothing else here would: it was
        // posted by the privileged bridge, `setAutoCancel` only covers the body tap, and a
        // notification still sitting there afterwards is what made these buttons look like
        // they did nothing.
        val action = intent.getStringExtra(EXTRA_BRIDGE_ACTION)
        if (action != null) {
            dismissNotification(RecordId(recordId))
        }

        when (action) {
            BRIDGE_ACTION_REOPEN -> {
                reopenCrashedApp(RecordId(recordId))
                return
            }

            BRIDGE_ACTION_MUTE -> {
                muteCrashedApp(RecordId(recordId))
                return
            }

            BRIDGE_ACTION_OPEN_DETAILS -> {
                openManager(recordId)
                return
            }
        }

        showDialog(
            recordId = RecordId(recordId),
            packageName = intent.getStringExtra(EXTRA_PACKAGE_NAME)?.takeIf { it.isNotBlank() },
        )
    }

    private fun showDialog(recordId: RecordId, packageName: String?) {
        val viewModel = CrashAlertViewModel(container.crashes, packageName)
        lifecycleScope.launch { viewModel.load(recordId) }

        setContent {
            val appearance by container.appearance.appearance.collectAsStateWithLifecycle()
            val state by viewModel.uiState.collectAsStateWithLifecycle()
            var dismissed by remember { mutableStateOf(false) }

            ManagerTheme(appearance = appearance) {
                CrashAlertDialog(
                    show = !dismissed,
                    state = state,
                    onOpenDetails = {
                        dismissed = true
                        openManager(recordId.value)
                    },
                    onReopen = {
                        dismissed = true
                        reopenCrashedApp(recordId)
                    },
                    onMute = {
                        dismissed = true
                        muteCrashedApp(recordId)
                    },
                    onDismiss = {
                        dismissed = true
                        finish()
                    },
                )
            }
        }
    }

    /** Hands the record to the manager's own navigation and closes this alert. */
    private fun openManager(recordId: String?) {
        startActivity(
            Intent(this, MainActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
                .putExtra(EXTRA_RECORD_ID, recordId),
        )
        finish()
    }

    /**
     * Asks the daemon to start the crashed app again.
     *
     * Goes through the daemon because only it can launch another package's activity as
     * that user; this process is an ordinary app.
     *
     * Says so when it could not. A relaunch that works announces itself — the app appears —
     * but one that fails is otherwise indistinguishable from a button that does nothing,
     * and this activity has no UI of its own to report into.
     */
    private fun reopenCrashedApp(recordId: RecordId) {
        lifecycleScope.launch {
            val launched = container.crashes.getRecord(recordId).mapCatching { detail ->
                container.apps.reopen(detail.group.packageName, detail.group.userId)
                    .getOrThrow()
            }.getOrDefault(false)

            if (!launched) toast(R.string.alert_reopen_failed)
            finish()
        }
    }

    /**
     * Silences the crashing app until the screen is next unlocked.
     *
     * Always reports: unlike the other two actions there is nothing to see afterwards —
     * that is the entire point of it — so without a word this is a button that swallows the
     * press.
     */
    private fun muteCrashedApp(recordId: RecordId) {
        lifecycleScope.launch {
            val muted = container.crashes.getRecord(recordId).mapCatching { detail ->
                container.config.mute(detail.group.packageName, MuteScope.UntilUnlock)
                    .getOrThrow()
            }.isSuccess

            toast(if (muted) R.string.alert_muted else R.string.alert_mute_failed)
            finish()
        }
    }

    /** Best effort: the action the user asked for matters more than its notification. */
    private fun dismissNotification(recordId: RecordId) {
        lifecycleScope.launch { container.crashes.dismissNotification(recordId) }
    }

    private fun toast(@StringRes message: Int) {
        Toast.makeText(this, message, Toast.LENGTH_SHORT).show()
    }

    internal companion object {
        /** Matches the extras the daemon and the bridge already put on their intents. */
        const val EXTRA_RECORD_ID = "record_id"
        const val EXTRA_PACKAGE_NAME = "package_name"
        const val EXTRA_BRIDGE_ACTION = "bridge_action"

        /** Must match `BridgeAction`'s wire names in `cch_wire`. */
        private const val BRIDGE_ACTION_OPEN_DETAILS = "open_details"
        private const val BRIDGE_ACTION_REOPEN = "reopen_app"
        private const val BRIDGE_ACTION_MUTE = "mute_until_unlock"
    }
}
