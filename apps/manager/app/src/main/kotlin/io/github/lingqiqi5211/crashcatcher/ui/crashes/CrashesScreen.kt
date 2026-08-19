package io.github.lingqiqi5211.crashcatcher.ui.crashes

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
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.lingqiqi5211.crashcatcher.R
import io.github.lingqiqi5211.crashcatcher.data.daemon.CrashKind
import io.github.lingqiqi5211.crashcatcher.data.daemon.GroupSummary
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
import io.github.lingqiqi5211.crashcatcher.ui.util.errorDescription
import io.github.lingqiqi5211.crashcatcher.ui.util.processDisplayName
import io.github.lingqiqi5211.crashcatcher.ui.util.processSuffix
import io.github.lingqiqi5211.crashcatcher.ui.util.shortTypeName
import io.github.lingqiqi5211.crashcatcher.ui.util.errorTitle
import io.github.lingqiqi5211.crashcatcher.ui.util.formatTimestampCompact
import io.github.lingqiqi5211.crashcatcher.ui.util.isRetryable
import io.github.lingqiqi5211.meowui.component.MeowCard
import io.github.lingqiqi5211.meowui.component.MeowSearchBar
import io.github.lingqiqi5211.meowui.component.MeowPullToRefresh
import io.github.lingqiqi5211.meowui.component.meowScaffoldScroll
import io.github.lingqiqi5211.meowui.theme.MeowTheme

/** Gap between the pinned search bar and the feed below it. */
internal val ChromeSpacing = 8.dp

/**
 * The crash list.
 *
 * Rows come from a single indexed query on the daemon side and carry no stack text,
 * so this stays responsive at any history size; the full trace is fetched only when a
 * row is opened.
 *
 * The kind filter and the scope toggles live in the top bar's menu rather than on the
 * page. A row of filter chips above a list eats a fifth of a phone screen to show
 * controls that are used once and then ignored — the list is what the screen is for.
 *
 * Layout follows the shell's inset contract: the search bar is pinned chrome and takes
 * the published top inset as *layout* padding, while the feed reaches the window's
 * bottom edge and pads its own content instead, so rows scroll through the strip the
 * bottom bar occupies rather than stopping above it.
 */
@Composable
internal fun CrashesScreen(
    state: CrashesUiState,
    actions: CrashesActions,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier
            .fillMaxSize()
            .testTag("crashcatcher.crashes.scroll")
            // Feed the list's nested scroll to the top bar so scrolling collapses it
            // the way it does on the library's own containers.
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
            modifier = Modifier.testTag("crashcatcher.crashes.search"),
            placeholder = stringResource(R.string.crashes_search_placeholder),
            cancelText = stringResource(R.string.crashes_search_cancel),
            onSearch = { actions.onSearchSubmit() },
        ) {
            // Nothing until something is typed: the full list is not a search result.
            if (state.query.isNotBlank()) {
                CrashFeed(state, actions)
            }
        }

        // The expanded search panel covers the page; drawing the feed behind it
        // would cost layout for something nobody can see.
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
            CrashFeed(state, actions)
        }
    }
}

/**
 * The four states Miuix's pull indicator labels.
 *
 * MeowUI leaves these to the caller: miuix ships English strings and the library has no
 * localisation of its own, so an unlocalised app would show "Pull to refresh" under a
 * Chinese page.
 */
@Composable
internal fun refreshTexts(): List<String> = listOf(
    stringResource(R.string.refresh_pull),
    stringResource(R.string.refresh_release),
    stringResource(R.string.refresh_refreshing),
    stringResource(R.string.refresh_done),
)

@Composable
private fun CrashFeed(state: CrashesUiState, actions: CrashesActions) {
    val error = state.error

    when {
        state.isLoading -> CrashCatcherLoadingState(
            testTag = "crashcatcher.crashes.loading",
            modifier = Modifier.fillMaxSize(),
        )

        // A failure with nothing to fall back on is the whole screen's state, not a
        // banner floating above an empty page.
        error != null && state.groups.isEmpty() -> CrashCatcherErrorState(
            testTag = "crashcatcher.crashes.error",
            title = errorTitle(error),
            description = errorDescription(error),
            onRetry = actions.onRefresh.takeIf { error.isRetryable() },
            modifier = Modifier.fillMaxSize(),
        )

        state.groups.isEmpty() -> CrashCatcherEmptyState(
            testTag = "crashcatcher.crashes.empty",
            modifier = Modifier.fillMaxSize(),
            title = stringResource(
                if (state.isFiltered) {
                    R.string.crashes_filtered_empty_title
                } else {
                    R.string.crashes_empty_title
                },
            ),
            description = stringResource(
                if (state.isFiltered) {
                    R.string.crashes_filtered_empty_description
                } else {
                    R.string.crashes_empty_description
                },
            ),
        )

        else -> CrashList(
            groups = state.groups,
            onOpen = actions.onOpenGroup,
            onReachedEnd = actions.onLoadMore,
        )
    }
}

@Composable
private fun CrashList(
    groups: List<GroupSummary>,
    onOpen: (GroupSummary) -> Unit,
    onReachedEnd: () -> Unit,
) {
    val listState = rememberLazyListState()

    // Append when the tail comes into view rather than behind a "load more" button:
    // paging is an implementation detail the user should never have to operate.
    val atEnd by remember {
        derivedStateOf {
            val last = listState.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0
            last >= listState.layoutInfo.totalItemsCount - PREFETCH_DISTANCE
        }
    }
    LaunchedEffect(listState) {
        snapshotFlow { atEnd }.collect { if (it) onReachedEnd() }
    }

    LazyColumn(
        state = listState,
        modifier = Modifier
            .fillMaxSize()
            .testTag("crashcatcher.crashes.list"),
        contentPadding = crashCatcherContentBottomPadding,
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        items(groups, key = { it.groupId }) { group ->
            CrashRow(group = group, onClick = { onOpen(group) })
        }
        item(key = "tail-spacer") { Spacer(Modifier.height(8.dp)) }
    }
}

/**
 * One crash group.
 *
 * Three lines with a fixed job each: what broke, what it said, and where and when.
 * The kind tag sits on the title line because it is what the eye scans for when
 * looking for "the ANRs"; the count only appears when it is more than one, so a list
 * of one-off crashes is not covered in badges saying "1".
 */
@Composable
private fun CrashRow(group: GroupSummary, onClick: () -> Unit) {
    // Standalone cards rather than MeowCard's index/count grouping: the list is data
    // driven, and grouped corners would shift under the neighbours whenever a row is
    // added or removed.
    // Every qualifier in one row, in a fixed order, all in the same tone.
    //
    // These used to be scattered — the process badge wedged between the app name and the
    // timestamp, "应用自行处理" alone on a line below in a different colour — which made two
    // rows differ in height and colour for reasons that had nothing to do with what they
    // said. They are all neutral detail and are drawn as such.
    val qualifiers = buildList {
        // Leads the row's qualifiers because it changes what everything else means: there is
        // no app here, so there is no version, no icon and nothing to open.
        if (!group.packageInstalled) add(stringResource(R.string.crashes_system_process))
        // Which *process* died, when it was not the main one. Two crashes sharing a
        // package and an exception are different bugs if they came from different
        // processes, and a background-process crash is invisible to the user in a way a
        // main-process one is not.
        //
        // Skipped for a platform process: its "package" *is* its process path, so the badge
        // would repeat the line above it verbatim.
        if (!group.isMainProcess && group.packageInstalled) {
            processSuffix(group.packageName, group.processName)?.let(::add)
        }
        // Worth calling out: no comparable tool records this class of crash at all.
        if (group.selfHandled) add(stringResource(R.string.crashes_self_handled))
    }

    MeowCard(
        modifier = Modifier.testTag("crashcatcher.crashes.row.${group.groupId}"),
        contentPadding = ListRowPadding,
        onClick = onClick,
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            // The icon is how a list of exception names becomes scannable: the eye finds
            // the app long before it reads `java.lang.IllegalStateException`. Aligned to
            // the top of the text column rather than centred on the whole card, so it sits
            // beside the title on a two-line row and a four-line one alike.
            AppIcon(
                packageName = group.packageName,
                label = null,
                size = 36.dp,
                isProcess = !group.packageInstalled,
            )

            // One column for every line of text, so the message and the meta line start
            // where the title starts instead of running back under the icon.
            Column(modifier = Modifier.weight(1f)) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        // The bare class name, not its package: `java.lang.` is the same
                        // on every row and pushes the part that differs off the end. With no
                        // class — a signal, typically — the process is all there is to lead with.
                        text = group.summaryClass?.let(::shortTypeName)
                            ?: if (group.packageInstalled) {
                                group.packageName
                            } else {
                                processDisplayName(group.processName)
                            },
                        style = MeowTheme.typography.title,
                        fontWeight = FontWeight.Medium,
                        color = MeowTheme.colors.onSurface,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f),
                    )
                    StatusTag(
                        text = stringResource(group.kind.labelRes),
                        tone = group.kind.tone,
                    )
                }

                group.summaryText?.takeIf { it.isNotBlank() }?.let { message ->
                    Spacer(Modifier.height(3.dp))
                    Text(
                        text = message,
                        style = MeowTheme.typography.summary,
                        color = MeowTheme.colors.onSurfaceVariant,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                }

                Spacer(Modifier.height(5.dp))

                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        // The app's name where it is known, since "which app" is what the
                        // reader is scanning for; the package is on the detail page.
                        //
                        // A platform process is not looked up at all: PackageManager has
                        // nothing to say about `/vendor/bin/hw/…`, and the binary's own name
                        // is what identifies it.
                        text = if (group.packageInstalled) {
                            rememberAppLabel(group.packageName) ?: group.packageName
                        } else {
                            processDisplayName(group.processName)
                        },
                        style = MeowTheme.typography.summary,
                        color = MeowTheme.colors.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f),
                    )
                    if (group.occurrence > 1) {
                        Text(
                            text = stringResource(
                                R.string.crashes_occurrence,
                                group.occurrence,
                            ),
                            style = MeowTheme.typography.summary,
                            color = MeowTheme.colors.onSurfaceVariant,
                        )
                    }
                    Text(
                        text = formatTimestampCompact(group.lastSeenMs),
                        style = MeowTheme.typography.summary,
                        color = MeowTheme.colors.onSurfaceVariant,
                    )
                }

                if (qualifiers.isNotEmpty()) {
                    Spacer(Modifier.height(6.dp))
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(6.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        qualifiers.forEach { qualifier ->
                            StatusTag(text = qualifier, tone = StatusTagTone.Neutral)
                        }
                    }
                }
            }
        }
    }
}


/** Callbacks the screen needs, as one bag so the screen stays a pure function. */
internal data class CrashesActions(
    val onTabSelected: (CrashTab) -> Unit,
    val onQueryChange: (String) -> Unit,
    val onSearchExpandedChange: (Boolean) -> Unit,
    val onSearchSubmit: () -> Unit,
    val onIncludeSystemAppsChange: (Boolean) -> Unit,
    val onOnlySelfHandledChange: (Boolean) -> Unit,
    /** Retry after a failure; no artificial delay. */
    val onRefresh: () -> Unit,
    /** The pull gesture, which keeps its indicator up long enough to be seen. */
    val onPullToRefresh: () -> Unit,
    val onLoadMore: () -> Unit,
    val onOpenGroup: (GroupSummary) -> Unit,
)

internal val CrashTab.labelRes: Int
    get() = when (this) {
        CrashTab.All -> R.string.crashes_tab_all
        CrashTab.Java -> R.string.crashes_tab_java
        CrashTab.Anr -> R.string.crashes_tab_anr
        CrashTab.Native -> R.string.crashes_tab_native
    }

internal val CrashKind.labelRes: Int
    get() = when (this) {
        CrashKind.JavaException -> R.string.crashes_tab_java
        CrashKind.Anr -> R.string.crashes_tab_anr
        CrashKind.NativeCrash -> R.string.crashes_tab_native
        CrashKind.Wtf -> R.string.crashes_kind_wtf
    }

/**
 * Neutral for every kind, on purpose.
 *
 * These tags name a *category*, not a severity. Mapping Java and native crashes onto the
 * error tone put an alarm-coloured badge on every row of a list where every row is a
 * crash — so the colour distinguished nothing and only made the page look like it was on
 * fire, which under Miuix's dark red container is exactly how it read. The label already
 * says which kind it is; the tone is left to say something the label cannot.
 */
private val CrashKind.tone: StatusTagTone
    get() = StatusTagTone.Neutral

private const val PREFETCH_DISTANCE = 5
