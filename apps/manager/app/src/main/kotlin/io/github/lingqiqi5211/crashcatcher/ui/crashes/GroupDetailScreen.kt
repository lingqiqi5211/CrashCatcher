package io.github.lingqiqi5211.crashcatcher.ui.crashes

import androidx.compose.foundation.layout.Arrangement
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
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.lingqiqi5211.crashcatcher.R
import io.github.lingqiqi5211.crashcatcher.data.daemon.PayloadState
import io.github.lingqiqi5211.crashcatcher.data.daemon.RecordSummary
import io.github.lingqiqi5211.crashcatcher.data.daemon.SourceMask
import io.github.lingqiqi5211.crashcatcher.ui.components.CrashCatcherLoadingState
import io.github.lingqiqi5211.crashcatcher.ui.components.StatusTag
import io.github.lingqiqi5211.crashcatcher.ui.components.StatusTagTone
import io.github.lingqiqi5211.crashcatcher.ui.components.TonalCard
import io.github.lingqiqi5211.crashcatcher.ui.components.rememberAppLabel
import io.github.lingqiqi5211.crashcatcher.ui.util.formatTimestamp
import io.github.lingqiqi5211.crashcatcher.ui.util.processSuffix
import io.github.lingqiqi5211.crashcatcher.ui.util.shortTypeName
import io.github.lingqiqi5211.meowui.component.MeowCard
import io.github.lingqiqi5211.meowui.component.MeowScaffold
import io.github.lingqiqi5211.meowui.component.meowScaffoldScroll
import io.github.lingqiqi5211.meowui.component.MeowTopBarAction
import io.github.lingqiqi5211.meowui.theme.MeowIcons
import io.github.lingqiqi5211.meowui.theme.MeowTheme

/**
 * One crash fingerprint, and every occurrence of it that still has a row.
 *
 * The header separates "how many times this happened" from "how many are still
 * stored" — retention prunes detail rows but never the count, and a screen that
 * showed only the surviving rows would quietly understate a recurring crash.
 */
@Composable
internal fun GroupDetailScreen(
    state: GroupDetailUiState,
    actions: GroupDetailActions,
    modifier: Modifier = Modifier,
) {
    val group = state.group

    // Same reasoning as the record page: the exception's own name is the heading, the
    // package and the process go in the subtitle where they fit on one line.
    val label = rememberAppLabel(group?.packageName.orEmpty())
    val subtitle = remember(group, label) {
        val process = group?.let { processSuffix(it.packageName, it.processName) }
        listOfNotNull(label ?: group?.packageName, process).joinToString(separator = " · ")
    }

    MeowScaffold(
        title = group?.summaryClass?.let(::shortTypeName)
            ?: group?.packageName?.let(::shortTypeName)
            ?: stringResource(R.string.loading),
        modifier = modifier,
        subtitle = subtitle,
        onBackClick = actions.onBack,
        actionItems = listOf(
            MeowTopBarAction.Icon(
                icon = MeowIcons.Delete,
                contentDescription = stringResource(R.string.detail_delete),
                modifier = Modifier.testTag("crashcatcher.group.delete"),
                onClick = actions.onDelete,
            ),
        ),
    ) { scaffoldPadding ->
        if (state.isLoading) {
            CrashCatcherLoadingState(
                testTag = "crashcatcher.group.loading",
                modifier = Modifier.fillMaxSize(),
            )
            return@MeowScaffold
        }

        val listState = rememberLazyListState()
        val atEnd by remember {
            derivedStateOf {
                val last = listState.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0
                last >= listState.layoutInfo.totalItemsCount - PREFETCH_DISTANCE
            }
        }
        LaunchedEffect(listState) {
            snapshotFlow { atEnd }.collect { if (it) actions.onLoadMore() }
        }

        LazyColumn(
            state = listState,
            modifier = Modifier
                .fillMaxSize()
                // Without this the top bar never collapses, so the rows scroll up into a
                // full-height title that is still standing there and collide with it. The
                // library containers (MeowPreferenceScreen, MeowPullToRefresh) attach the
                // connection themselves; a hand-built list has to ask.
                .meowScaffoldScroll()
                .testTag("crashcatcher.group.records"),
            contentPadding = scaffoldPadding,
            // The rows are separate cards, so they need a gap; without it consecutive
            // cards touch and the rounded corners meet, which reads as one ragged block
            // rather than a list. Same spacing the crash list uses.
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            if (group != null) {
                item(key = "header") {
                    HeaderCard(state = state)
                }
            }

            if (state.records.isNotEmpty()) {
                // A heading rather than a wider gap alone. The card above and the rows
                // below are both cards, so with only whitespace between them the eye
                // reads one ragged stack of six; a label says where the summary ends and
                // the occurrence list starts, and pays for its own height.
                item(key = "records-heading") {
                    SectionHeading(text = stringResource(R.string.group_records))
                }
            }

            items(state.records, key = { it.id.value }) { record ->
                RecordRow(
                    record = record,
                    onClick = { actions.onOpenRecord(record) },
                    modifier = Modifier.padding(
                        horizontal = MeowTheme.dimensions.pageHorizontalPadding,
                    ),
                )
            }

            item(key = "tail-spacer") { Spacer(Modifier.height(24.dp)) }
        }
    }
}

@Composable
private fun HeaderCard(state: GroupDetailUiState) {
    val group = state.group ?: return

    TonalCard(
        // Horizontal only: the list's own arrangement supplies the vertical gap now, and
        // padding here as well would double it under the header.
        modifier = Modifier
            .padding(horizontal = MeowTheme.dimensions.pageHorizontalPadding)
            .testTag("crashcatcher.group.header"),
    ) {
        group.summaryText?.takeIf { it.isNotBlank() }?.let { message ->
            Text(
                text = message,
                style = MeowTheme.typography.title,
                color = MeowTheme.colors.onSurface,
            )
            Spacer(Modifier.height(8.dp))
        }

        InfoRow(stringResource(R.string.group_package), group.packageName)
        InfoRow(stringResource(R.string.group_occurrence), group.occurrence.toString())
        InfoRow(stringResource(R.string.group_first_seen), formatTimestamp(group.firstSeenMs))
        InfoRow(stringResource(R.string.group_last_seen), formatTimestamp(group.lastSeenMs))
        if (group.userId != 0) {
            InfoRow(stringResource(R.string.group_user), group.userId.toString())
        }

        if (state.prunedOccurrences > 0) {
            Spacer(Modifier.height(8.dp))
            // Says why the list is shorter than the count, instead of leaving the two
            // numbers looking inconsistent.
            Text(
                text = stringResource(R.string.group_pruned, state.prunedOccurrences),
                style = MeowTheme.typography.summary,
                color = MeowTheme.colors.onSurfaceVariant,
            )
        }

        if (group.selfHandled) {
            Spacer(Modifier.height(8.dp))
            StatusTag(
                text = stringResource(R.string.crashes_self_handled),
                tone = StatusTagTone.Info,
            )
        }
    }
}

/**
 * A label dividing one run of cards from the next.
 *
 * Matches the accent-coloured group titles the settings pages already use, so a heading
 * inside a list looks like the same kind of thing there as it does here. The extra top
 * padding is the gap: it belongs to the heading rather than to the card above, which is
 * what keeps the summary from looking like the first row of the list.
 */
@Composable
private fun SectionHeading(text: String) {
    Text(
        text = text,
        style = MeowTheme.typography.value,
        color = MeowTheme.colors.primary,
        modifier = Modifier.padding(
            start = MeowTheme.dimensions.pageHorizontalPadding,
            end = MeowTheme.dimensions.pageHorizontalPadding,
            top = 12.dp,
            bottom = 2.dp,
        ),
    )
}

@Composable
private fun RecordRow(
    record: RecordSummary,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    MeowCard(
        modifier = modifier.testTag("crashcatcher.group.record.${record.id.value}"),
        onClick = onClick,
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = formatTimestamp(record.happenedAtMs),
                style = MeowTheme.typography.title,
                color = MeowTheme.colors.onSurface,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f, fill = false),
            )
            if (record.payloadState == PayloadState.Evicted) {
                StatusTag(
                    text = stringResource(R.string.record_payload_gone),
                    tone = StatusTagTone.Neutral,
                )
            } else if (record.isRepeating) {
                StatusTag(
                    text = stringResource(R.string.record_repeating),
                    tone = StatusTagTone.Warning,
                )
            }
        }

        Spacer(Modifier.height(2.dp))

        Text(
            text = buildString {
                append("pid ${record.pid}")
                record.appVersionName?.let { append(" · $it") }
                append(" · ${sourceLabel(record.sources)}")
                record.droppedCount?.takeIf { it > 0 }?.let { append(" · -$it") }
            },
            style = MeowTheme.typography.summary,
            color = MeowTheme.colors.onSurfaceVariant,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

/**
 * One label/value fact.
 *
 * 3.dp of vertical padding made four of these read as a single block of text with no
 * spacing at all, and `SpaceBetween` let a long package name run right up against its
 * own label with no gap. The value now takes the remaining width and is end-aligned, so
 * a long one wraps inside its own column instead of colliding.
 */
@Composable
private fun InfoRow(label: String, value: String) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 7.dp),
        horizontalArrangement = Arrangement.spacedBy(16.dp),
        verticalAlignment = Alignment.Top,
    ) {
        Text(
            text = label,
            style = MeowTheme.typography.title,
            color = MeowTheme.colors.onSurface,
        )
        Text(
            text = value,
            style = MeowTheme.typography.value,
            color = MeowTheme.colors.onSurfaceVariant,
            textAlign = TextAlign.End,
            modifier = Modifier.weight(1f),
        )
    }
}

/**
 * Which collectors saw this occurrence, as short labels.
 *
 * Worth showing: one native crash legitimately arrives on several paths, and seeing
 * which ones fired is how a user can tell "the tombstone watcher is working" from
 * "only the event log noticed".
 */
private fun sourceLabel(sources: SourceMask): String = buildList {
    if (SourceMask.Events in sources) add("evt")
    if (SourceMask.CrashBuffer in sources) add("log")
    if (SourceMask.Dropbox in sources) add("dbx")
    if (SourceMask.Tombstone in sources) add("tomb")
    if (SourceMask.AnrFile in sources) add("anr")
}.joinToString("+").ifEmpty { "—" }

internal data class GroupDetailActions(
    val onBack: () -> Unit,
    val onDelete: () -> Unit,
    val onLoadMore: () -> Unit,
    val onOpenRecord: (RecordSummary) -> Unit,
)

private const val PREFETCH_DISTANCE = 5
