package io.github.lingqiqi5211.crashcatcher.ui.settings

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import io.github.lingqiqi5211.crashcatcher.R
import io.github.lingqiqi5211.crashcatcher.domain.model.DeviceInfo
import io.github.lingqiqi5211.crashcatcher.ui.components.SettingsNavigationRow
import io.github.lingqiqi5211.crashcatcher.ui.components.SettingsSection
import io.github.lingqiqi5211.crashcatcher.ui.components.SettingsSwitchRow
import io.github.lingqiqi5211.crashcatcher.ui.components.WarningCard
import io.github.lingqiqi5211.crashcatcher.ui.home.formatBytes
import io.github.lingqiqi5211.crashcatcher.ui.util.errorDescription
import io.github.lingqiqi5211.crashcatcher.ui.util.errorTitle
import io.github.lingqiqi5211.meowui.component.MeowPreferencePage
import io.github.lingqiqi5211.meowui.component.MeowTipStyle
import io.github.lingqiqi5211.meowui.theme.MeowTheme

/**
 * Why the module is not doing what it should.
 *
 * Reachable when the daemon is not, which is the whole point: the switches elsewhere are all
 * writes, and none of them can be reached to *ask* a question. What is here reads.
 *
 * Three parts, in the order they get used. The chain — whether each link is up — answers "which
 * link is down". The log answers "why". The report is what gets pasted into an issue, because
 * the answer to that question is usually a combination and retyping a screen of facts loses
 * exactly the one that mattered.
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
        SettingsSection(
            title = stringResource(R.string.diagnostics_section_chain),
            testTag = "crashcatcher.diagnostics.chain",
        ) {
            // Unreachable is a finding, not an absence of findings — and the most common one on
            // this page. Said plainly, next to the number that is half of a version mismatch.
            if (status == null) {
                item(key = "daemon-unreachable") {
                    WarningCard(
                        title = stringResource(R.string.diagnostics_daemon_unreachable),
                        body = stringResource(R.string.diagnostics_daemon_unreachable_body),
                        style = MeowTipStyle.Warning,
                        modifier = Modifier.testTag("crashcatcher.diagnostics.unreachable"),
                    )
                }
            }

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
                // The bridge posts every notification, so "connected" is the difference between
                // silence and working alerts — and its own version catches a stale dex.
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
            SettingsNavigationRow(
                title = stringResource(R.string.diagnostics_refresh_log),
                description = if (log.totalBytes > 0) {
                    stringResource(R.string.diagnostics_log_size, formatBytes(log.totalBytes))
                } else {
                    stringResource(R.string.diagnostics_refresh_log_summary)
                },
                onClick = actions.onRefreshLog,
                modifier = Modifier.testTag("crashcatcher.diagnostics.refresh"),
            )

            item(key = "runtime-log") {
                RuntimeLogBlock(log)
            }
        }

        SettingsSection(
            title = stringResource(R.string.diagnostics_section_report),
            testTag = "crashcatcher.diagnostics.report",
        ) {
            SettingsNavigationRow(
                title = stringResource(R.string.diagnostics_copy_report),
                description = stringResource(R.string.diagnostics_copy_report_summary),
                onClick = { actions.onCopyReport(buildReport(state, device)) },
                modifier = Modifier.testTag("crashcatcher.diagnostics.copy"),
            )
            SettingsNavigationRow(
                title = stringResource(R.string.diagnostics_share_report),
                description = stringResource(R.string.diagnostics_share_report_summary),
                // The log goes with it: a report saying which link is down without the lines
                // that say why is the half of the answer that is easy to guess.
                onClick = {
                    actions.onShareReport(buildReport(state, device) + "\n" + log.text)
                },
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

/**
 * The log itself, monospaced and never wrapped.
 *
 * A tracing line is a timestamp, a level, a target and a message; wrapped, the columns stop
 * lining up and the level — the thing being scanned for — is no longer at a fixed offset. One
 * scroll container for the whole block, since a per-line one would fight over `maxValue`.
 */
@Composable
private fun RuntimeLogBlock(log: RuntimeLogUiState) {
    val body = when {
        log.isLoading && log.text.isEmpty() -> stringResource(R.string.loading)
        log.error != null -> "${errorTitle(log.error)}\n${errorDescription(log.error)}"
        log.isEmpty -> stringResource(R.string.diagnostics_log_empty)
        else -> log.text
    }

    Column(modifier = Modifier.fillMaxWidth()) {
        if (log.truncated) {
            Text(
                text = stringResource(R.string.diagnostics_log_truncated),
                style = MeowTheme.typography.summary,
                color = MeowTheme.colors.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
            )
        }
        SelectionContainer {
            Text(
                text = body,
                style = MeowTheme.typography.summary,
                fontFamily = FontFamily.Monospace,
                color = MeowTheme.colors.onSurfaceVariant,
                modifier = Modifier
                    .fillMaxWidth()
                    .horizontalScroll(rememberScrollState())
                    .padding(horizontal = 16.dp, vertical = 8.dp)
                    .testTag("crashcatcher.diagnostics.logtext"),
            )
        }
    }
}

internal data class DiagnosticsActions(
    val onDebugLoggingChange: (Boolean) -> Unit,
    val onRefreshLog: () -> Unit,
    val onCopyReport: (String) -> Unit,
    val onShareReport: (String) -> Unit,
)
