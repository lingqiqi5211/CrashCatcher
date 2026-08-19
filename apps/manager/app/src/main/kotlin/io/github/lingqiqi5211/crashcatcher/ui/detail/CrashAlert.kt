package io.github.lingqiqi5211.crashcatcher.ui.detail

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import io.github.lingqiqi5211.crashcatcher.R
import io.github.lingqiqi5211.crashcatcher.data.daemon.GroupSummary
import io.github.lingqiqi5211.crashcatcher.data.daemon.RecordId
import io.github.lingqiqi5211.crashcatcher.ui.components.CrashCatcherButton
import io.github.lingqiqi5211.crashcatcher.ui.components.CrashCatcherTextButton
import io.github.lingqiqi5211.crashcatcher.ui.components.rememberAppLabel
import io.github.lingqiqi5211.crashcatcher.ui.util.shortTypeName
import io.github.lingqiqi5211.meowui.component.MeowCard
import io.github.lingqiqi5211.meowui.theme.MeowTheme

/** What the alert knows about the crash it is announcing. */
internal data class CrashAlertUiState(val group: GroupSummary? = null)

/**
 * The crash alert itself.
 *
 * A stack of actions rather than a confirm/cancel pair, for the same reason the tool being
 * replaced lays its own out that way: everything here is something to *do*, and a two-slot
 * alert has only one place to put the second one. 重新打开 sat in the cancel slot, which is
 * where a dialog puts "never mind" — so the button that relaunches the app was drawn as the
 * one that dismisses, and there was no plain way to dismiss at all.
 *
 * The actions match the notification's, so the same crash offers the same choices however
 * it was announced.
 *
 * Renders before the record has loaded rather than showing a spinner: the app name comes
 * from the intent's own package resolution, so the dialog can say the useful half
 * immediately and fill in the exception when the daemon answers. The two actions that need
 * a package wait for it.
 */
@Composable
internal fun CrashAlertDialog(
    show: Boolean,
    state: CrashAlertUiState,
    onOpenDetails: () -> Unit,
    onReopen: () -> Unit,
    onMute: () -> Unit,
    onDismiss: () -> Unit,
) {
    if (!show) return

    val group = state.group
    val label = rememberAppLabel(group?.packageName.orEmpty())
    val name = label ?: group?.packageName

    val title = if (name == null) {
        stringResource(R.string.alert_title_unknown)
    } else {
        stringResource(R.string.alert_title, name)
    }

    val message = buildString {
        group?.summaryClass?.let(::shortTypeName)?.let(::append)
        group?.summaryText?.takeIf { it.isNotBlank() }?.let { text ->
            if (isNotEmpty()) append('\n')
            append(text)
        }
        if (isEmpty()) append(stringResource(R.string.alert_message_unknown))
    }

    Dialog(onDismissRequest = onDismiss) {
        MeowCard(
            modifier = Modifier.testTag("crashcatcher.alert"),
            contentPadding = PaddingValues(20.dp),
        ) {
            Text(
                text = title,
                style = MeowTheme.typography.sectionTitle,
                fontWeight = FontWeight.SemiBold,
                color = MeowTheme.colors.onSurface,
            )
            Spacer(Modifier.height(6.dp))
            Text(
                text = message,
                style = MeowTheme.typography.summary,
                color = MeowTheme.colors.onSurfaceVariant,
                // A crash message can be a paragraph; this is an interruption over another
                // app, not a place to read the whole thing. The log is one tap away.
                maxLines = 4,
                overflow = TextOverflow.Ellipsis,
            )

            Spacer(Modifier.height(16.dp))

            Column(
                modifier = Modifier.fillMaxWidth(),
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                CrashCatcherButton(
                    text = stringResource(R.string.alert_open_details),
                    onClick = onOpenDetails,
                    modifier = Modifier
                        .fillMaxWidth()
                        .testTag("crashcatcher.alert.details"),
                )

                // Both need a package, which arrives with the record.
                if (group != null) {
                    CrashCatcherTextButton(
                        text = stringResource(R.string.alert_reopen),
                        onClick = onReopen,
                        modifier = Modifier
                            .fillMaxWidth()
                            .testTag("crashcatcher.alert.reopen"),
                    )
                    CrashCatcherTextButton(
                        text = stringResource(R.string.alert_mute),
                        onClick = onMute,
                        modifier = Modifier
                            .fillMaxWidth()
                            .testTag("crashcatcher.alert.mute"),
                    )
                }

                CrashCatcherTextButton(
                    text = stringResource(R.string.alert_close),
                    onClick = onDismiss,
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(top = 2.dp)
                        .testTag("crashcatcher.alert.close"),
                )
            }
        }
    }
}

/** Loads the one record an alert is about. */
internal class CrashAlertViewModel(
    private val crashes: io.github.lingqiqi5211.crashcatcher.domain.repository.CrashRepository,
) {
    private val state =
        kotlinx.coroutines.flow.MutableStateFlow(CrashAlertUiState())
    val uiState: kotlinx.coroutines.flow.StateFlow<CrashAlertUiState> = state

    suspend fun load(id: RecordId) {
        crashes.getRecord(id).onSuccess { detail ->
            state.value = CrashAlertUiState(group = detail.group)
        }
    }
}
