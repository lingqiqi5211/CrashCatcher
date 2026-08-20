package io.github.lingqiqi5211.crashcatcher.ui.apps

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.lingqiqi5211.crashcatcher.R
import io.github.lingqiqi5211.crashcatcher.data.daemon.AppEntry
import io.github.lingqiqi5211.crashcatcher.data.daemon.MuteScope
import io.github.lingqiqi5211.crashcatcher.data.daemon.NotifyMode
import io.github.lingqiqi5211.crashcatcher.ui.components.CrashCatcherEmptyState
import io.github.lingqiqi5211.crashcatcher.ui.components.ListRowPadding
import io.github.lingqiqi5211.crashcatcher.ui.components.CrashCatcherErrorState
import io.github.lingqiqi5211.crashcatcher.ui.components.CrashCatcherLoadingState
import io.github.lingqiqi5211.crashcatcher.ui.components.LocalCrashCatcherContentTopPadding
import io.github.lingqiqi5211.crashcatcher.ui.components.StatusTag
import io.github.lingqiqi5211.crashcatcher.ui.components.StatusTagTone
import io.github.lingqiqi5211.crashcatcher.ui.components.crashCatcherContentBottomPadding
import io.github.lingqiqi5211.crashcatcher.ui.components.AppIcon
import io.github.lingqiqi5211.crashcatcher.ui.components.rememberAppLabel
import io.github.lingqiqi5211.crashcatcher.ui.crashes.ChromeSpacing
import io.github.lingqiqi5211.crashcatcher.ui.crashes.refreshTexts
import io.github.lingqiqi5211.crashcatcher.ui.util.errorDescription
import io.github.lingqiqi5211.crashcatcher.ui.util.errorTitle
import io.github.lingqiqi5211.crashcatcher.ui.util.formatTimestampCompact
import io.github.lingqiqi5211.crashcatcher.ui.util.processDisplayName
import io.github.lingqiqi5211.crashcatcher.ui.util.isRetryable
import io.github.lingqiqi5211.meowui.component.MeowCard
import io.github.lingqiqi5211.meowui.component.MeowPullToRefresh
import io.github.lingqiqi5211.meowui.component.MeowSearchBar
import io.github.lingqiqi5211.meowui.component.meowScaffoldScroll
import io.github.lingqiqi5211.meowui.theme.MeowTheme

/**
 * Apps with crash history, and the way into their per-app settings.
 *
 * Same shape as the crash list: the scope toggle lives in the top bar's menu, the
 * search bar is the only pinned chrome, and the feed owns the rest of the page.
 */
@Composable
internal fun AppsScreen(
    state: AppsUiState,
    actions: AppsActions,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier
            .fillMaxSize()
            .testTag("crashcatcher.apps.scroll")
            .meowScaffoldScroll()
            .padding(
                start = MeowTheme.dimensions.pageHorizontalPadding,
                top = LocalCrashCatcherContentTopPadding.current + ChromeSpacing,
                end = MeowTheme.dimensions.pageHorizontalPadding,
            ),
        verticalArrangement = Arrangement.spacedBy(ChromeSpacing),
    ) {
        MeowSearchBar(
            query = state.query,
            onQueryChange = actions.onQueryChange,
            expanded = state.searchExpanded,
            onExpandedChange = actions.onSearchExpandedChange,
            modifier = Modifier.testTag("crashcatcher.apps.search"),
            placeholder = stringResource(R.string.apps_search_placeholder),
            cancelText = stringResource(R.string.crashes_search_cancel),
        ) {
            if (state.query.isNotBlank()) {
                AppFeed(state, actions)
            }
        }

        if (state.searchExpanded) return@Column

        // Only the feed is wrapped, so the indicator comes down from the top of the
        // list — under the pinned search bar — rather than over the bar itself. Both
        // padding parameters are zero for the same reason: there is nothing above this
        // box to offset past.
        MeowPullToRefresh(
            isRefreshing = state.isRefreshing,
            onRefresh = actions.onPullToRefresh,
            modifier = Modifier.weight(1f),
            contentPadding = PaddingValues(0.dp),
            scaffoldPadding = PaddingValues(0.dp),
            refreshTexts = refreshTexts(),
        ) {
            AppFeed(state, actions)
        }
    }
}

@Composable
private fun AppFeed(state: AppsUiState, actions: AppsActions) {
    val error = state.error

    when {
        state.isLoading -> CrashCatcherLoadingState(
            testTag = "crashcatcher.apps.loading",
            modifier = Modifier.fillMaxSize(),
        )

        error != null && state.apps.isEmpty() -> CrashCatcherErrorState(
            testTag = "crashcatcher.apps.error",
            title = errorTitle(error),
            description = errorDescription(error),
            onRetry = actions.onRefresh.takeIf { error.isRetryable() },
            modifier = Modifier.fillMaxSize(),
        )

        state.apps.isEmpty() -> CrashCatcherEmptyState(
            testTag = "crashcatcher.apps.empty",
            modifier = Modifier.fillMaxSize(),
            title = stringResource(
                if (state.query.isNotBlank()) {
                    R.string.apps_filtered_empty_title
                } else {
                    R.string.apps_empty_title
                },
            ),
            description = stringResource(
                if (state.query.isNotBlank()) {
                    R.string.apps_filtered_empty_description
                } else {
                    R.string.apps_empty_description
                },
            ),
        )

        else -> LazyColumn(
            modifier = Modifier
                .fillMaxSize()
                .testTag("crashcatcher.apps.list"),
            contentPadding = crashCatcherContentBottomPadding,
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            items(state.apps, key = { "${it.packageName}:${it.userId}" }) { app ->
                AppRow(app = app, onClick = { actions.onOpenApp(app) })
            }
            item(key = "tail-spacer") { Spacer(Modifier.height(8.dp)) }
        }
    }
}

/**
 * One app.
 *
 * Two lines: which app, and how much it has crashed and when. The override tag only
 * appears when the app actually has one — a badge on every row saying "following the
 * global setting" tells the reader nothing they did not already assume.
 */
@Composable
private fun AppRow(app: AppEntry, onClick: () -> Unit) {
    MeowCard(
        modifier = Modifier.testTag("crashcatcher.apps.row.${app.packageName}.${app.userId}"),
        contentPadding = ListRowPadding,
        onClick = onClick,
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            // The daemon's label comes from the privileged bridge and can be absent; the
            // local PackageManager lookup covers that, so a row is never a package name
            // printed twice with nothing else to tell the two lines apart.
            //
            // A platform process has no label to find either way, and its binary's name is
            // what identifies it; the full path stays on the line below.
            val label = if (app.packageInstalled) {
                app.label ?: rememberAppLabel(app.packageName)
            } else {
                processDisplayName(app.packageName)
            }

            AppIcon(
                packageName = app.packageName,
                label = label,
                size = 36.dp,
                isProcess = !app.packageInstalled,
            )

            // Two lines, not three: name and badges, then the package with the time beside
            // it. The count is a tag rather than loose text — it is the same kind of thing
            // as the override badge next to it, and as plain grey text on the title line it
            // read as part of the app's name.
            Column(modifier = Modifier.weight(1f)) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        text = label ?: app.packageName,
                        style = MeowTheme.typography.title,
                        fontWeight = FontWeight.Medium,
                        color = MeowTheme.colors.onSurface,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f),
                    )
                    // Ahead of the per-app override badge, because origin changes what the
                    // rest of the row means — a platform process has no app to notify about
                    // or launch at all. The two are mutually exclusive: a process that is not
                    // an installed package cannot be a system *app*.
                    originLabel(app)?.let { origin ->
                        StatusTag(text = origin, tone = StatusTagTone.Neutral)
                    }
                    if (app.userId != 0) {
                        StatusTag(
                            text = stringResource(R.string.app_user, app.userId),
                            tone = StatusTagTone.Neutral,
                        )
                    }
                    appOverrideLabel(app)?.let { override ->
                        StatusTag(text = override, tone = StatusTagTone.Neutral)
                    }
                    StatusTag(
                        text = stringResource(R.string.crashes_occurrence, app.occurrence),
                        tone = StatusTagTone.Neutral,
                    )
                }

                Spacer(Modifier.height(3.dp))

                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    // Dropped when it is already the heading: the same string twice reads
                    // as a rendering bug rather than as extra detail. The row stays for the
                    // timestamp, which then simply moves left.
                    if (label != null) {
                        Text(
                            text = app.packageName,
                            style = MeowTheme.typography.summary,
                            color = MeowTheme.colors.onSurfaceVariant,
                            maxLines = 1,
                            overflow = TextOverflow.MiddleEllipsis,
                            modifier = Modifier.weight(1f),
                        )
                    } else {
                        Spacer(Modifier.weight(1f))
                    }

                    app.lastSeenMs?.let { lastSeen ->
                        Text(
                            text = formatTimestampCompact(lastSeen),
                            style = MeowTheme.typography.summary,
                            color = MeowTheme.colors.onSurfaceVariant,
                            maxLines = 1,
                        )
                    }
                }
            }
        }
    }
}

/** Where this row's crashes came from, or null for an ordinary user-installed app. */
@Composable
private fun originLabel(app: AppEntry): String? = when {
    !app.packageInstalled -> stringResource(R.string.crashes_system_process)
    app.isSystemApp -> stringResource(R.string.crashes_system_app)
    else -> null
}

@Composable
private fun appOverrideLabel(app: AppEntry): String? = when {
    app.config.ignore -> stringResource(R.string.app_override_ignored)
    app.config.mute != MuteScope.None -> stringResource(R.string.app_override_muted)
    app.config.notifyMode == NotifyMode.Nothing -> stringResource(R.string.notify_mode_nothing)
    app.config.notifyMode != null -> stringResource(R.string.app_override_custom)
    else -> null
}

internal data class AppsActions(
    val onQueryChange: (String) -> Unit,
    val onSearchExpandedChange: (Boolean) -> Unit,
    val onIncludeSystemAppsChange: (Boolean) -> Unit,
    /** Retry after a failure; no artificial delay. */
    val onRefresh: () -> Unit,
    /** The pull gesture, which keeps its indicator up long enough to be seen. */
    val onPullToRefresh: () -> Unit,
    val onOpenApp: (AppEntry) -> Unit,
)
