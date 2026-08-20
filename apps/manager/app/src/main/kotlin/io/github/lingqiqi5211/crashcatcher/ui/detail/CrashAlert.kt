package io.github.lingqiqi5211.crashcatcher.ui.detail

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import io.github.lingqiqi5211.crashcatcher.R
import io.github.lingqiqi5211.crashcatcher.data.daemon.GroupSummary
import io.github.lingqiqi5211.crashcatcher.data.daemon.RecordId
import io.github.lingqiqi5211.crashcatcher.ui.components.AppIcon
import io.github.lingqiqi5211.crashcatcher.ui.components.CrashCatcherButton
import io.github.lingqiqi5211.crashcatcher.ui.components.CrashCatcherTextButton
import io.github.lingqiqi5211.crashcatcher.ui.components.rememberAppLabel
import io.github.lingqiqi5211.crashcatcher.ui.theme.isMiuixStyle
import io.github.lingqiqi5211.crashcatcher.ui.util.shortTypeName
import io.github.lingqiqi5211.meowui.component.MeowCard
import io.github.lingqiqi5211.meowui.theme.MeowIcons
import io.github.lingqiqi5211.meowui.theme.MeowTheme

/** What the alert knows about the crash it is announcing. */
internal data class CrashAlertUiState(
    val packageName: String? = null,
    val group: GroupSummary? = null,
)

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
    val packageName = group?.packageName ?: state.packageName
    val label = rememberAppLabel(packageName.orEmpty())
    val name = label ?: packageName

    val title = if (name == null) {
        stringResource(R.string.alert_title_unknown)
    } else {
        stringResource(R.string.alert_title, name)
    }

    val exceptionClass = group?.summaryClass?.let(::shortTypeName)
    val summaryText = group?.summaryText?.takeIf { it.isNotBlank() }
    val unknownMessage = stringResource(R.string.alert_message_unknown)
    val message = buildString {
        exceptionClass?.let(::append)
        summaryText?.let { text ->
            if (isNotEmpty()) append('\n')
            append(text)
        }
        if (isEmpty()) append(unknownMessage)
    }
    val miuix = isMiuixStyle()

    Dialog(onDismissRequest = onDismiss) {
        MeowCard(
            modifier = Modifier.testTag("crashcatcher.alert"),
            contentPadding = PaddingValues(if (miuix) 20.dp else 24.dp),
        ) {
            if (miuix) {
                CompactAlertSummary(title = title, message = message)
            } else {
                ExpressiveAlertSummary(
                    packageName = packageName,
                    label = name,
                    title = title,
                    exceptionClass = exceptionClass,
                    summaryText = summaryText,
                    unknownMessage = unknownMessage,
                )
            }

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

/** Keeps the established Miuix dialog compact while MD3E gets its own stronger hierarchy. */
@Composable
private fun CompactAlertSummary(title: String, message: String) {
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
        maxLines = 4,
        overflow = TextOverflow.Ellipsis,
    )
}

/**
 * MD3E's alert hierarchy: identify the app first, then isolate the actual failure in an
 * error-toned block. The old `sectionTitle + grey paragraph` treatment read like an ordinary
 * settings card, especially over another app, so the interruption had no visual anchor.
 */
@Composable
private fun ExpressiveAlertSummary(
    packageName: String?,
    label: String?,
    title: String,
    exceptionClass: String?,
    summaryText: String?,
    unknownMessage: String,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (packageName == null) {
            Box(
                modifier = Modifier
                    .size(48.dp)
                    .background(MeowTheme.colors.errorContainer, CircleShape),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    imageVector = MeowIcons.Error,
                    contentDescription = null,
                    tint = MeowTheme.colors.onErrorContainer,
                    modifier = Modifier.size(24.dp),
                )
            }
        } else {
            AppIcon(
                packageName = packageName,
                label = label,
                size = 48.dp,
            )
        }
        Spacer(Modifier.width(14.dp))
        Text(
            text = title,
            modifier = Modifier.weight(1f),
            style = MeowTheme.typography.pageTitle.copy(
                fontSize = 22.sp,
                lineHeight = 27.sp,
            ),
            fontWeight = FontWeight.SemiBold,
            color = MeowTheme.colors.onSurface,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
        )
    }

    Spacer(Modifier.height(18.dp))

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .background(MeowTheme.colors.errorContainer, MeowTheme.shapes.item)
            .padding(14.dp)
            .testTag("crashcatcher.alert.summary"),
        verticalAlignment = Alignment.Top,
    ) {
        Icon(
            imageVector = MeowIcons.Error,
            contentDescription = null,
            tint = MeowTheme.colors.onErrorContainer,
            modifier = Modifier.size(22.dp),
        )
        Spacer(Modifier.width(12.dp))
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = exceptionClass ?: summaryText ?: unknownMessage,
                style = MeowTheme.typography.title,
                fontWeight = FontWeight.SemiBold,
                color = MeowTheme.colors.onErrorContainer,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            if (exceptionClass != null && summaryText != null) {
                Spacer(Modifier.height(4.dp))
                Text(
                    text = summaryText,
                    style = MeowTheme.typography.summary,
                    color = MeowTheme.colors.onErrorContainer,
                    maxLines = 3,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
}

/** Loads the one record an alert is about. */
internal class CrashAlertViewModel(
    private val crashes: io.github.lingqiqi5211.crashcatcher.domain.repository.CrashRepository,
    packageName: String? = null,
) {
    private val state =
        kotlinx.coroutines.flow.MutableStateFlow(CrashAlertUiState(packageName = packageName))
    val uiState: kotlinx.coroutines.flow.StateFlow<CrashAlertUiState> = state

    suspend fun load(id: RecordId) {
        crashes.getRecord(id).onSuccess { detail ->
            state.value = CrashAlertUiState(
                packageName = detail.group.packageName,
                group = detail.group,
            )
        }
    }
}
