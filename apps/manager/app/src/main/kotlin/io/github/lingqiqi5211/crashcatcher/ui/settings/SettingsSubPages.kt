package io.github.lingqiqi5211.crashcatcher.ui.settings

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import io.github.lingqiqi5211.crashcatcher.R
import io.github.lingqiqi5211.crashcatcher.ui.components.AppIcon
import io.github.lingqiqi5211.meowui.theme.MeowTheme
import io.github.lingqiqi5211.crashcatcher.data.daemon.NotifyMode
import io.github.lingqiqi5211.crashcatcher.domain.model.DeviceInfo
import io.github.lingqiqi5211.crashcatcher.domain.model.valueOrNull
import io.github.lingqiqi5211.crashcatcher.ui.components.SettingsDropdownRow
import io.github.lingqiqi5211.crashcatcher.ui.components.SettingsNavigationRow
import io.github.lingqiqi5211.crashcatcher.ui.components.SettingsSection
import io.github.lingqiqi5211.crashcatcher.ui.components.SettingsSwitchRow
import io.github.lingqiqi5211.crashcatcher.ui.components.WarningCard
import io.github.lingqiqi5211.crashcatcher.ui.home.formatBytes
import io.github.lingqiqi5211.meowui.component.MeowAlertDialog
import io.github.lingqiqi5211.meowui.component.MeowAlertStyle
import io.github.lingqiqi5211.meowui.component.MeowPreferencePage
import io.github.lingqiqi5211.meowui.component.MeowTipStyle

/*
 * The settings sub-pages.
 *
 * One page per group of related controls, reached from the settings tab's table of
 * contents. Each is a `MeowPreferencePage`, so it brings its own top bar and slides in
 * as a whole surface — the same treatment the appearance page already had.
 */

@Composable
internal fun CaptureSettingsPage(
    state: SettingsUiState,
    actions: SettingsActions,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val config = state.value

    MeowPreferencePage(
        // No subtitle: it would repeat the sentence on the row that led here, which the
        // reader has just tapped and is still holding in mind.
        title = stringResource(R.string.settings_section_capture),
        modifier = modifier,
        onBackClick = onBack,
    ) {
        SettingsSection(
            title = stringResource(R.string.settings_capture_kinds),
            testTag = "crashcatcher.capture.kinds",
        ) {
            SettingsSwitchRow(
                title = stringResource(R.string.settings_capture_java),
                description = "",
                checked = config?.captureJava ?: true,
                onCheckedChange = actions.onCaptureJavaChange,
                enabled = config != null,
                modifier = Modifier.testTag("crashcatcher.capture.java"),
            )
            SettingsSwitchRow(
                title = stringResource(R.string.settings_capture_anr),
                description = "",
                checked = config?.captureAnr ?: true,
                onCheckedChange = actions.onCaptureAnrChange,
                enabled = config != null,
                modifier = Modifier.testTag("crashcatcher.capture.anr"),
            )
            SettingsSwitchRow(
                title = stringResource(R.string.settings_capture_native),
                description = "",
                checked = config?.captureNative ?: true,
                onCheckedChange = actions.onCaptureNativeChange,
                enabled = config != null,
                modifier = Modifier.testTag("crashcatcher.capture.native"),
            )
            SettingsSwitchRow(
                title = stringResource(R.string.settings_capture_self_handled),
                description = "",
                checked = config?.captureSelfHandled ?: true,
                onCheckedChange = actions.onCaptureSelfHandledChange,
                enabled = config != null,
                modifier = Modifier.testTag("crashcatcher.capture.selfhandled"),
            )
        }

        SettingsSection(
            title = stringResource(R.string.settings_capture_scope),
            testTag = "crashcatcher.capture.scope",
        ) {
            SettingsSwitchRow(
                title = stringResource(R.string.settings_include_system_apps),
                description = stringResource(R.string.settings_include_system_apps_summary),
                checked = config?.includeSystemApps ?: false,
                onCheckedChange = actions.onIncludeSystemAppsChange,
                enabled = config != null,
                modifier = Modifier.testTag("crashcatcher.capture.systemapps"),
            )
        }
    }
}

@Composable
internal fun NotifySettingsPage(
    state: SettingsUiState,
    actions: SettingsActions,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val config = state.value

    // Resolved up front: `optionLabel` is a plain lambda called during layout, not a
    // composable scope, so `stringResource` cannot be used inside it.
    val notifyModeLabels = NotifyMode.entries.associateWith { mode ->
        stringResource(
            when (mode) {
                NotifyMode.Dialog -> R.string.notify_mode_dialog
                NotifyMode.Notification -> R.string.notify_mode_notification
                NotifyMode.Toast -> R.string.notify_mode_toast
                NotifyMode.Nothing -> R.string.notify_mode_nothing
            },
        )
    }

    MeowPreferencePage(
        title = stringResource(R.string.settings_section_notify),
        modifier = modifier,
        onBackClick = onBack,
    ) {
        SettingsSection(
            title = stringResource(R.string.settings_notify_mode),
            testTag = "crashcatcher.notify.mode.section",
        ) {
            SettingsDropdownRow(
                title = stringResource(R.string.settings_notify_mode),
                selected = config?.notifyMode ?: NotifyMode.Notification,
                options = NotifyMode.entries,
                optionLabel = { mode -> notifyModeLabels.getValue(mode) },
                onSelected = actions.onNotifyModeChange,
                enabled = config != null,
                modifier = Modifier.testTag("crashcatcher.notify.mode"),
            )
        }

        SettingsSection(
            title = stringResource(R.string.settings_notify_scope),
            testTag = "crashcatcher.notify.scope",
        ) {
            SettingsSwitchRow(
                title = stringResource(R.string.settings_only_foreground),
                description = stringResource(R.string.settings_only_foreground_summary),
                checked = config?.onlyForeground ?: false,
                onCheckedChange = actions.onOnlyForegroundChange,
                enabled = config != null,
                modifier = Modifier.testTag("crashcatcher.notify.foreground"),
            )
            SettingsSwitchRow(
                title = stringResource(R.string.settings_only_main_process),
                description = stringResource(R.string.settings_only_main_process_summary),
                checked = config?.onlyMainProcess ?: false,
                onCheckedChange = actions.onOnlyMainProcessChange,
                enabled = config != null,
                modifier = Modifier.testTag("crashcatcher.notify.mainprocess"),
            )
        }
    }
}

@Composable
internal fun DialogSettingsPage(
    state: SettingsUiState,
    actions: SettingsActions,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val config = state.value

    MeowPreferencePage(
        title = stringResource(R.string.settings_section_dialog),
        modifier = modifier,
        onBackClick = onBack,
    ) {
        SettingsSection(
            title = stringResource(R.string.settings_section_dialog),
            testTag = "crashcatcher.dialog.section",
        ) {
            SettingsSwitchRow(
                title = stringResource(R.string.settings_takeover_dialog),
                description = stringResource(R.string.settings_takeover_dialog_warning),
                // The daemon's reply wins over the stored config, and says what actually
                // took effect rather than what was asked for.
                checked = state.dialogTakeover?.effective
                    ?: config?.takeoverSystemDialog
                    ?: false,
                onCheckedChange = actions.onDialogTakeoverChange,
                enabled = config != null && state.dialogTakeover?.unsupportedReason == null,
                modifier = Modifier.testTag("crashcatcher.dialog.takeover"),
            )

            // Two things can make this setting look broken while working exactly as
            // designed, so both get said out loud rather than left to be discovered.
            state.dialogTakeover?.unsupportedReason?.let { reason ->
                item(key = "takeover-unsupported") {
                    WarningCard(
                        title = stringResource(R.string.settings_takeover_unsupported),
                        body = reason,
                        style = MeowTipStyle.Warning,
                        modifier = Modifier.testTag("crashcatcher.dialog.unsupported"),
                    )
                }
            }

            if (state.dialogTakeover?.anrShowBackgroundConflict == true) {
                item(key = "takeover-conflict") {
                    WarningCard(
                        title = stringResource(R.string.settings_takeover_conflict),
                        body = stringResource(R.string.settings_takeover_conflict_body),
                        style = MeowTipStyle.Warning,
                        modifier = Modifier.testTag("crashcatcher.dialog.conflict"),
                    )
                }
            }
        }
    }
}

/**
 * Retention limits and the one place records get cleared.
 *
 * Deleting everything lives here rather than on the crash list: it is not a per-row
 * action, and a destructive control sitting next to the rows it destroys is easy to hit
 * by accident. Putting it behind two taps and a confirmation, on the page that also
 * shows how much is stored and what the limits are, means the decision is made with the
 * numbers in view.
 */
@Composable
internal fun StorageSettingsPage(
    state: SettingsUiState,
    actions: SettingsActions,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val config = state.value
    val storage = state.moduleStatus?.storage
    var confirmingDeleteAll by remember { mutableStateOf(false) }

    MeowPreferencePage(
        title = stringResource(R.string.settings_storage_manage),
        modifier = modifier,
        onBackClick = onBack,
    ) {
        if (storage != null) {
            SettingsSection(
                title = stringResource(R.string.settings_storage_usage),
                testTag = "crashcatcher.storage.usage",
            ) {
                SettingsNavigationRow(
                    title = stringResource(R.string.storage_records),
                    description = stringResource(
                        R.string.settings_storage_records_summary,
                        storage.recordCount,
                        storage.groupCount,
                    ),
                )
                SettingsNavigationRow(
                    title = stringResource(R.string.storage_total_bytes),
                    description = formatBytes(storage.payloadBytes + storage.databaseBytes),
                )
            }
        }

        SettingsSection(
            title = stringResource(R.string.settings_storage_limits),
            testTag = "crashcatcher.storage.limits",
        ) {
            SettingsDropdownRow(
                title = stringResource(R.string.settings_retention_days),
                description = stringResource(R.string.settings_retention_days_summary),
                selected = config?.retention?.retentionDays ?: 30,
                options = RETENTION_DAY_OPTIONS,
                optionLabel = { days -> "$days" },
                onSelected = actions.onRetentionDaysChange,
                enabled = config != null,
                modifier = Modifier.testTag("crashcatcher.storage.days"),
            )
            SettingsDropdownRow(
                title = stringResource(R.string.settings_max_records_total),
                description = stringResource(R.string.settings_max_records_total_summary),
                selected = config?.retention?.maxRecordsTotal ?: 2_000,
                options = RECORD_TOTAL_OPTIONS,
                optionLabel = { records -> "$records" },
                onSelected = actions.onMaxRecordsTotalChange,
                enabled = config != null,
                modifier = Modifier.testTag("crashcatcher.storage.records"),
            )
        }

        SettingsSection(
            title = stringResource(R.string.settings_storage_cleanup),
            testTag = "crashcatcher.storage.cleanup",
        ) {
            SettingsNavigationRow(
                title = stringResource(R.string.settings_delete_all),
                description = stringResource(R.string.settings_delete_all_summary),
                enabled = config != null && (storage?.recordCount ?: 0) > 0,
                onClick = { confirmingDeleteAll = true },
                modifier = Modifier.testTag("crashcatcher.storage.deleteall"),
            )
        }
    }

    MeowAlertDialog(
        show = confirmingDeleteAll,
        title = stringResource(R.string.settings_delete_all),
        message = stringResource(R.string.settings_delete_all_confirm),
        confirmText = stringResource(R.string.action_confirm),
        cancelText = stringResource(R.string.action_cancel),
        // Irreversible, so the confirm button is styled as a warning rather than looking
        // like an ordinary OK.
        style = MeowAlertStyle.Warning,
        onConfirm = {
            confirmingDeleteAll = false
            actions.onDeleteAll()
        },
        onDismissRequest = { confirmingDeleteAll = false },
    )
}

/**
 * The about page: what this project is, not what the device is.
 *
 * The device facts live on the overview, where a crash is actually read against them.
 * What is left here is identity — the icon and name, the versions a bug report needs,
 * where the source is, and what it is built on.
 */
@Composable
internal fun AboutPage(
    state: SettingsUiState,
    deviceInfo: DeviceInfo,
    onOpenUrl: (String) -> Unit,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val status = state.moduleStatus

    MeowPreferencePage(
        title = stringResource(R.string.settings_section_about),
        modifier = modifier,
        onBackClick = onBack,
    ) {
        AboutHeader(deviceInfo)

        SettingsSection(
            title = stringResource(R.string.about_section_versions),
            testTag = "crashcatcher.about.versions",
        ) {
            SettingsNavigationRow(
                title = stringResource(R.string.about_daemon_version),
                description = status?.daemonVersion
                    ?: stringResource(R.string.about_daemon_unreachable),
            )
            SettingsNavigationRow(
                title = stringResource(R.string.about_protocol_version),
                description = status?.protocolVersion?.toString() ?: "—",
            )
            SettingsNavigationRow(
                title = stringResource(R.string.about_module_id),
                description = MODULE_ID,
            )
        }

        SettingsSection(
            title = stringResource(R.string.about_section_source),
            testTag = "crashcatcher.about.source",
        ) {
            SettingsNavigationRow(
                title = stringResource(R.string.about_repository),
                description = REPOSITORY_URL.removePrefix("https://"),
                onClick = { onOpenUrl(REPOSITORY_URL) },
                modifier = Modifier.testTag("crashcatcher.about.repository"),
            )
            SettingsNavigationRow(
                title = stringResource(R.string.about_license),
                description = stringResource(R.string.about_license_value),
            )
        }

        // Credit where it is due, and a practical answer to "what is in this binary".
        // The reference implementation is listed too: this project exists because of it,
        // and its licence still governs what was learned from reading it.
        SettingsSection(
            title = stringResource(R.string.about_section_credits),
            testTag = "crashcatcher.about.credits",
        ) {
            CREDITS.forEach { credit ->
                SettingsNavigationRow(
                    title = credit.name,
                    description = "${credit.licence} · ${credit.role}",
                    onClick = { onOpenUrl(credit.url) },
                )
            }
        }
    }
}

/**
 * The app's own icon and name, so the page opens with what it is about.
 *
 * The icon is read through the same loader the crash rows use, rather than the launcher
 * resource directly, so it looks exactly like the app looks everywhere else.
 */
@Composable
private fun AboutHeader(deviceInfo: DeviceInfo) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 20.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        AppIcon(
            packageName = MANAGER_PACKAGE,
            label = stringResource(R.string.app_name),
            size = 72.dp,
        )
        Text(
            text = stringResource(R.string.app_name),
            style = MeowTheme.typography.sectionTitle,
            fontWeight = FontWeight.SemiBold,
            color = MeowTheme.colors.onSurface,
        )
        Text(
            text = "${deviceInfo.managerVersionName} (${deviceInfo.managerVersionCode})",
            style = MeowTheme.typography.summary,
            color = MeowTheme.colors.onSurfaceVariant,
        )
    }
}

/** Shown on the about page so a bug report can name the module without guessing. */
private const val MODULE_ID = "crash.catcher"
private const val MANAGER_PACKAGE = "io.github.lingqiqi5211.crashcatcher"
private const val REPOSITORY_URL = "https://github.com/lingqiqi5211/CrashCatcher"

/** One acknowledged dependency. */
private data class Credit(
    val name: String,
    val licence: String,
    val role: String,
    val url: String,
)

private val CREDITS = listOf(
    Credit(
        name = "MeowUI",
        licence = "Apache-2.0",
        role = "Compose UI",
        url = "https://github.com/lingqiqi5211/MeowUI",
    ),
    Credit(
        name = "miuix",
        licence = "Apache-2.0",
        role = "Miuix 风格组件",
        url = "https://github.com/miuix-kotlin-multiplatform/miuix",
    ),
    Credit(
        name = "rusqlite",
        licence = "MIT",
        role = "SQLite 索引",
        url = "https://github.com/rusqlite/rusqlite",
    ),
    Credit(
        name = "zstd",
        licence = "BSD-3-Clause",
        role = "日志压缩",
        url = "https://github.com/facebook/zstd",
    ),
    Credit(
        name = "AppErrorsTracking",
        licence = "AGPL-3.0",
        role = "参考实现",
        url = "https://github.com/KitsunePie/AppErrorsTracking",
    ),
)

private val SettingsUiState.moduleStatus
    get() = moduleStatusState.valueOrNull
