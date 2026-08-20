package io.github.lingqiqi5211.crashcatcher

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.lingqiqi5211.crashcatcher.data.daemon.RecordId
import io.github.lingqiqi5211.crashcatcher.ui.detail.CrashDetailActivity
import io.github.lingqiqi5211.crashcatcher.ui.shell.CrashCatcherApp
import io.github.lingqiqi5211.crashcatcher.ui.theme.LocalCrashCatcherFloatingNavigationBar
import io.github.lingqiqi5211.crashcatcher.ui.theme.LocalCrashCatcherPredictiveBack
import io.github.lingqiqi5211.crashcatcher.ui.theme.ManagerTheme

class MainActivity : ComponentActivity() {

    /**
     * A record this was asked to open, until the shell has navigated to it.
     *
     * The crash alert's 查看日志 and the notification's 查看详情 both arrive here carrying a
     * record id. Ignoring it is what made them look broken: they did start the app, but on
     * the overview tab — and when the app was already open on that tab, pressing the button
     * changed nothing visible at all.
     */
    private var pendingRecord by mutableStateOf<RecordId?>(null)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()

        pendingRecord = intent.recordId()
        val container = (application as CrashCatcherApplication).container

        setContent {
            val appearance by container.appearance.appearance.collectAsStateWithLifecycle()

            ManagerTheme(appearance = appearance) {
                CompositionLocalProvider(
                    LocalCrashCatcherFloatingNavigationBar provides appearance.floatingNavigationBar,
                    LocalCrashCatcherPredictiveBack provides appearance.predictiveBackEnabled,
                ) {
                    CrashCatcherApp(
                        container = container,
                        pendingRecord = pendingRecord,
                        onPendingRecordOpened = { pendingRecord = null },
                    )
                }
            }
        }
    }

    /**
     * A second notification while the app is already open.
     *
     * `singleTop` in the manifest is what routes it here instead of building another
     * instance, which would otherwise leave the first one — the one the user is looking at —
     * showing the previous crash.
     */
    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        pendingRecord = intent.recordId()
    }
}

/** The record a crash alert or notification asked this to open, if any. */
private fun Intent.recordId(): RecordId? =
    getStringExtra(CrashDetailActivity.EXTRA_RECORD_ID)
        ?.takeIf { it.isNotBlank() }
        ?.let(::RecordId)
