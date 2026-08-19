package io.github.lingqiqi5211.crashcatcher.ui.apps

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import io.github.lingqiqi5211.crashcatcher.R
import io.github.lingqiqi5211.crashcatcher.data.daemon.GroupSummary
import io.github.lingqiqi5211.crashcatcher.data.daemon.MuteScope
import io.github.lingqiqi5211.crashcatcher.ui.components.CrashCatcherLoadingState
import io.github.lingqiqi5211.crashcatcher.ui.components.rememberAppLabel
import io.github.lingqiqi5211.crashcatcher.ui.components.SettingsDropdownRow
import io.github.lingqiqi5211.crashcatcher.ui.components.SettingsNavigationRow
import io.github.lingqiqi5211.crashcatcher.ui.components.SettingsSection
import io.github.lingqiqi5211.crashcatcher.ui.components.SettingsSwitchRow
import io.github.lingqiqi5211.crashcatcher.ui.util.formatTimestamp
import io.github.lingqiqi5211.meowui.component.MeowPreferencePage

/**
 * One app's settings, plus its crash history.
 *
 * The notify row offers five choices, not four: "follow global" is a real state and
 * has to be selectable, or turning an override off would be impossible once it was
 * turned on.
 */
@Composable
internal fun AppDetailScreen(
    state: AppDetailUiState,
    actions: AppDetailActions,
    modifier: Modifier = Modifier,
) {
    if (state.isLoading) {
        CrashCatcherLoadingState(
            testTag = "crashcatcher.appdetail.loading",
            modifier = modifier.fillMaxSize(),
        )
        return
    }

    val notifyLabels = AppNotifyChoice.entries.associateWith { choice ->
        stringResource(choice.labelRes)
    }
    val muteLabels = MuteScope.entries.associateWith { scope -> stringResource(scope.labelRes) }

    // The app's name as the heading, its package underneath. A package name set at
    // display size wraps mid-segment and reads as a path rather than as "which app this
    // page is about"; the package is still needed, just not as the headline.
    val label = rememberAppLabel(state.packageName)

    MeowPreferencePage(
        title = label ?: state.packageName,
        modifier = modifier,
        subtitle = if (label == null) "" else state.packageName,
        onBackClick = actions.onBack,
    ) {
        SettingsSection(
            title = stringResource(R.string.app_section_behaviour),
            testTag = "crashcatcher.appdetail.behaviour",
        ) {
            SettingsDropdownRow(
                title = stringResource(R.string.settings_notify_mode),
                selected = state.notifyChoice,
                options = AppNotifyChoice.entries,
                optionLabel = { choice -> notifyLabels.getValue(choice) },
                onSelected = actions.onNotifyChoiceChange,
                modifier = Modifier.testTag("crashcatcher.appdetail.notify"),
            )
            SettingsSwitchRow(
                title = stringResource(R.string.app_ignore),
                description = stringResource(R.string.app_ignore_summary),
                checked = state.config.ignore,
                onCheckedChange = actions.onIgnoreChange,
                modifier = Modifier.testTag("crashcatcher.appdetail.ignore"),
            )
            SettingsDropdownRow(
                title = stringResource(R.string.app_mute),
                selected = state.config.mute,
                options = MuteScope.entries,
                optionLabel = { scope -> muteLabels.getValue(scope) },
                onSelected = actions.onMuteChange,
                enabled = !state.config.ignore,
                modifier = Modifier.testTag("crashcatcher.appdetail.mute"),
            )
        }

        SettingsSection(
            title = stringResource(R.string.app_section_actions),
            testTag = "crashcatcher.appdetail.actions",
        ) {
            SettingsNavigationRow(
                title = stringResource(R.string.app_reopen),
                description = stringResource(R.string.app_reopen_summary),
                onClick = actions.onReopen,
                modifier = Modifier.testTag("crashcatcher.appdetail.reopen"),
            )
        }

        if (state.groups.isNotEmpty()) {
            SettingsSection(
                title = stringResource(R.string.app_section_history),
                testTag = "crashcatcher.appdetail.history",
            ) {
                state.groups.forEach { group ->
                    SettingsNavigationRow(
                        title = group.summaryClass ?: group.processName,
                        description = groupSummaryLine(group),
                        onClick = { actions.onOpenGroup(group) },
                        modifier = Modifier.testTag(
                            "crashcatcher.appdetail.group.${group.groupId}",
                        ),
                    )
                }
            }
        }
    }
}

@Composable
private fun groupSummaryLine(group: GroupSummary): String = stringResource(
    R.string.app_group_summary,
    group.occurrence,
    formatTimestamp(group.lastSeenMs),
)

private val AppNotifyChoice.labelRes: Int
    get() = when (this) {
        AppNotifyChoice.FollowGlobal -> R.string.app_notify_follow_global
        AppNotifyChoice.Dialog -> R.string.notify_mode_dialog
        AppNotifyChoice.Notification -> R.string.notify_mode_notification
        AppNotifyChoice.Toast -> R.string.notify_mode_toast
        AppNotifyChoice.Nothing -> R.string.notify_mode_nothing
    }

private val MuteScope.labelRes: Int
    get() = when (this) {
        MuteScope.None -> R.string.app_mute_none
        MuteScope.UntilUnlock -> R.string.app_mute_until_unlock
        MuteScope.UntilRestart -> R.string.app_mute_until_restart
    }

internal data class AppDetailActions(
    val onBack: () -> Unit,
    val onNotifyChoiceChange: (AppNotifyChoice) -> Unit,
    val onIgnoreChange: (Boolean) -> Unit,
    val onMuteChange: (MuteScope) -> Unit,
    val onReopen: () -> Unit,
    val onOpenGroup: (GroupSummary) -> Unit,
)
