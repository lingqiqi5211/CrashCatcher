package io.github.lingqiqi5211.crashcatcher.ui.settings

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import io.github.lingqiqi5211.crashcatcher.R
import io.github.lingqiqi5211.crashcatcher.domain.model.DeviceInfo
import io.github.lingqiqi5211.crashcatcher.ui.components.SettingsNavigationRow
import io.github.lingqiqi5211.crashcatcher.ui.components.SettingsSection
import io.github.lingqiqi5211.crashcatcher.ui.components.SettingsSwitchRow
import io.github.lingqiqi5211.crashcatcher.ui.components.WarningCard
import io.github.lingqiqi5211.crashcatcher.ui.home.formatBytes
import io.github.lingqiqi5211.meowui.component.MeowPreferencePage
import io.github.lingqiqi5211.meowui.component.MeowTipStyle

/**
 * Why the module is not working.
 *
 * Every row here reads, so the page still works when the daemon does not. That is when it gets
 * opened: the other settings pages all write, and grey out.
 *
 * The chain says which link is down, the log says why, the report is what goes into an issue.
 */
@Composable
internal fun DiagnosticsPage(
    state: SettingsUiState,
    log: RuntimeLogUiState,
    device: DeviceInfo,
    actions: DiagnosticsActions,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val status = state.moduleStatus
    val config = state.value

    MeowPreferencePage(
        title = stringResource(R.string.settings_section_diagnostics),
        modifier = modifier,
        onBackClick = onBack,
    ) {
        // The most common finding on this page, so it is stated rather than left as blanks.
        //
        // Outside the section below, not an item in it: a preference group draws label-and-value
        // rows on one card, and a tip dropped in among them arrives with its own fill and icon —
        // one row in the stack looking like it belongs to another screen.
        if (status == null) {
            WarningCard(
                title = stringResource(R.string.diagnostics_daemon_unreachable),
                body = stringResource(R.string.diagnostics_daemon_unreachable_body),
                style = MeowTipStyle.Warning,
                modifier = Modifier.testTag("crashcatcher.diagnostics.unreachable"),
            )
        }

        SettingsSection(
            title = stringResource(R.string.diagnostics_section_chain),
            testTag = "crashcatcher.diagnostics.chain",
        ) {
            SettingsNavigationRow(
                title = stringResource(R.string.diagnostics_daemon),
                description = status?.let {
                    stringResource(
                        R.string.diagnostics_daemon_value,
                        it.daemonVersion,
                        it.runtime.pid,
                        it.runtime.abi,
                    )
                } ?: stringResource(R.string.about_daemon_unreachable),
            )
            SettingsNavigationRow(
                title = stringResource(R.string.diagnostics_bridge),
                // The bridge posts every notification, so disconnected means silence. Its
                // version catches a stale dex.
                description = status?.runtime?.bridge?.let { bridge ->
                    if (bridge.connected) {
                        stringResource(
                            R.string.diagnostics_bridge_connected,
                            bridge.version ?: "?",
                        )
                    } else {
                        stringResource(R.string.diagnostics_bridge_disconnected)
                    }
                } ?: stringResource(R.string.about_daemon_unreachable),
            )
            SettingsNavigationRow(
                title = stringResource(R.string.diagnostics_package_index),
                // Incomplete means every app looks third-party, which makes the whole
                // system-app filter behave as if it were off.
                description = status?.runtime?.packageIndex?.let { index ->
                    if (index.systemFlagsKnown) {
                        stringResource(R.string.diagnostics_index_complete, index.packageCount)
                    } else {
                        stringResource(R.string.diagnostics_index_incomplete, index.packageCount)
                    }
                } ?: stringResource(R.string.about_daemon_unreachable),
            )
            SettingsNavigationRow(
                title = stringResource(R.string.diagnostics_selinux),
                description = status?.runtime?.selinux
                    ?: stringResource(R.string.about_daemon_unreachable),
            )
            SettingsNavigationRow(
                title = stringResource(R.string.diagnostics_active_mutes),
                description = status?.runtime?.activeMutes?.toString()
                    ?: stringResource(R.string.about_daemon_unreachable),
            )
        }

        SettingsSection(
            title = stringResource(R.string.diagnostics_section_log),
            testTag = "crashcatcher.diagnostics.log",
        ) {
            SettingsSwitchRow(
                title = stringResource(R.string.diagnostics_debug_logging),
                description = stringResource(R.string.diagnostics_debug_logging_summary),
                checked = config?.debugLogging ?: false,
                onCheckedChange = actions.onDebugLoggingChange,
                enabled = config != null,
                modifier = Modifier.testTag("crashcatcher.diagnostics.debug"),
            )
            // A row leading to its own page, not the log itself: a preference list lays out
            // label-and-value rows, and a log is fixed-width text that has to pan sideways —
            // nested here, that gesture fights the list's own scroll.
            SettingsNavigationRow(
                title = stringResource(R.string.diagnostics_open_log),
                // Every file, not the one that happens to be selected: rotation means there are
                // up to eighteen, and this row is the answer to "how much are the logs".
                description = if (log.allBytes > 0) {
                    stringResource(
                        R.string.diagnostics_log_size,
                        log.files.size,
                        formatBytes(log.allBytes),
                    )
                } else {
                    stringResource(R.string.diagnostics_open_log_summary)
                },
                onClick = actions.onOpenLog,
                modifier = Modifier.testTag("crashcatcher.diagnostics.openlog"),
            )
        }

        SettingsSection(
            title = stringResource(R.string.diagnostics_section_report),
            testTag = "crashcatcher.diagnostics.report",
        ) {
            SettingsNavigationRow(
                title = stringResource(R.string.diagnostics_share_report),
                // The logs go with it: a report saying which link is down without the lines
                // that say why is the half of the answer that is easy to guess.
                onClick = { actions.onShareReport(buildReport(state, device)) },
                modifier = Modifier.testTag("crashcatcher.diagnostics.share"),
            )
        }
    }
}

private fun buildReport(state: SettingsUiState, device: DeviceInfo): String =
    buildDiagnosticsReport(
        status = state.moduleStatus,
        device = device,
        connected = state.moduleStatus != null,
    )

internal data class DiagnosticsActions(
    val onDebugLoggingChange: (Boolean) -> Unit,
    val onOpenLog: () -> Unit,
    val onShareReport: (String) -> Unit,
)
