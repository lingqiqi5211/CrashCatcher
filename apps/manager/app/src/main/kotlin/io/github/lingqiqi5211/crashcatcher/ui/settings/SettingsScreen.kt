package io.github.lingqiqi5211.crashcatcher.ui.settings

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import io.github.lingqiqi5211.crashcatcher.R
import io.github.lingqiqi5211.crashcatcher.data.daemon.NotifyMode
import io.github.lingqiqi5211.crashcatcher.ui.components.SettingsNavigationRow
import io.github.lingqiqi5211.crashcatcher.ui.components.SettingsSection

/**
 * The settings tab: a table of contents, not a control panel.
 *
 * Every group of controls is one row leading to its own page. The previous version put
 * five sections and a dozen switches at this level, which meant finding any single
 * setting required reading all of them, and left no room to say what a group was *for* —
 * every row was a bare label. A row here can carry a sentence.
 *
 * 通用 comes first because it is the only group that works when the daemon does not.
 * Everything else reads or writes daemon state, and 重连守护进程 is exactly what someone
 * needs when that has failed, so burying it under greyed-out rows would be backwards.
 *
 * Emits sections only — the shell supplies the surrounding `MeowPreferenceScreen`, so
 * this composes identically whether it is a tab or a pushed page.
 */
@Composable
internal fun SettingsScreenContent(
    state: SettingsUiState,
    actions: SettingsActions,
) {
    val reachable = state.value != null

    SettingsSection(
        title = stringResource(R.string.settings_section_general),
        testTag = "crashcatcher.settings.general",
    ) {
        SettingsNavigationRow(
            title = stringResource(R.string.settings_section_appearance),
            description = stringResource(R.string.settings_appearance_summary),
            onClick = actions.onOpenAppearance,
            modifier = Modifier.testTag("crashcatcher.settings.appearance"),
        )
        SettingsNavigationRow(
            title = stringResource(R.string.settings_reconnect),
            description = stringResource(R.string.settings_reconnect_summary),
            onClick = actions.onReconnect,
            modifier = Modifier.testTag("crashcatcher.settings.reconnect"),
        )
        // In 通用 with 重连守护进程 for the same reason: this is the group that has to be
        // reachable when the daemon is the thing that is broken.
        SettingsNavigationRow(
            title = stringResource(R.string.settings_section_diagnostics),
            description = stringResource(R.string.settings_diagnostics_summary),
            onClick = actions.onOpenDiagnostics,
            modifier = Modifier.testTag("crashcatcher.settings.diagnostics"),
        )
        SettingsNavigationRow(
            title = stringResource(R.string.settings_section_about),
            description = stringResource(R.string.settings_about_summary),
            onClick = actions.onOpenAbout,
            modifier = Modifier.testTag("crashcatcher.settings.about"),
        )
    }

    SettingsSection(
        title = stringResource(R.string.settings_section_behaviour),
        testTag = "crashcatcher.settings.behaviour",
    ) {
        SettingsNavigationRow(
            title = stringResource(R.string.settings_section_capture),
            description = stringResource(R.string.settings_capture_summary),
            onClick = actions.onOpenCapture,
            enabled = reachable,
            modifier = Modifier.testTag("crashcatcher.settings.capture"),
        )
        SettingsNavigationRow(
            title = stringResource(R.string.settings_section_notify),
            description = stringResource(R.string.settings_notify_summary),
            onClick = actions.onOpenNotify,
            enabled = reachable,
            modifier = Modifier.testTag("crashcatcher.settings.notify"),
        )
        SettingsNavigationRow(
            title = stringResource(R.string.settings_section_dialog),
            description = stringResource(R.string.settings_dialog_summary),
            onClick = actions.onOpenDialog,
            enabled = reachable,
            modifier = Modifier.testTag("crashcatcher.settings.dialog"),
        )
    }

    SettingsSection(
        title = stringResource(R.string.settings_section_storage),
        testTag = "crashcatcher.settings.storage",
    ) {
        SettingsNavigationRow(
            title = stringResource(R.string.settings_storage_manage),
            description = stringResource(R.string.settings_storage_summary),
            onClick = actions.onOpenStorage,
            enabled = reachable,
            modifier = Modifier.testTag("crashcatcher.settings.storage.open"),
        )
    }
}

internal data class SettingsActions(
    val onCaptureJavaChange: (Boolean) -> Unit,
    val onCaptureAnrChange: (Boolean) -> Unit,
    val onCaptureNativeChange: (Boolean) -> Unit,
    val onCaptureSelfHandledChange: (Boolean) -> Unit,
    val onNotifyModeChange: (NotifyMode) -> Unit,
    val onOnlyForegroundChange: (Boolean) -> Unit,
    val onOnlyMainProcessChange: (Boolean) -> Unit,
    val onIncludeSystemAppsChange: (Boolean) -> Unit,
    val onDebugLoggingChange: (Boolean) -> Unit,
    val onDialogTakeoverChange: (Boolean) -> Unit,
    val onRetentionDaysChange: (Int) -> Unit,
    val onMaxRecordsTotalChange: (Int) -> Unit,
    val onDeleteAll: () -> Unit,
    val onReconnect: () -> Unit,
    val onOpenAppearance: () -> Unit,
    val onOpenAbout: () -> Unit,
    val onOpenCapture: () -> Unit,
    val onOpenNotify: () -> Unit,
    val onOpenDialog: () -> Unit,
    val onOpenStorage: () -> Unit,
    val onOpenDiagnostics: () -> Unit,
)

/** Offered values, all inside the daemon's clamp range so nothing is silently corrected. */
internal val RETENTION_DAY_OPTIONS = listOf(7, 14, 30, 60, 90, 180, 365)
internal val RECORD_TOTAL_OPTIONS = listOf(200, 500, 1_000, 2_000, 5_000, 10_000)
