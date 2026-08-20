package io.github.lingqiqi5211.crashcatcher.ui.settings

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import io.github.lingqiqi5211.crashcatcher.R
import io.github.lingqiqi5211.crashcatcher.ui.components.CrashCatcherErrorState
import io.github.lingqiqi5211.crashcatcher.ui.components.CrashCatcherLoadingState
import io.github.lingqiqi5211.crashcatcher.ui.components.MAX_ZOOM
import io.github.lingqiqi5211.crashcatcher.ui.components.MIN_ZOOM
import io.github.lingqiqi5211.crashcatcher.ui.components.monospaceContentWidth
import io.github.lingqiqi5211.crashcatcher.ui.components.monospaceTextStyle
import io.github.lingqiqi5211.crashcatcher.ui.components.pinchToZoom
import io.github.lingqiqi5211.crashcatcher.ui.home.formatBytes
import io.github.lingqiqi5211.crashcatcher.ui.util.errorDescription
import io.github.lingqiqi5211.crashcatcher.ui.util.errorTitle
import io.github.lingqiqi5211.crashcatcher.ui.util.isRetryable
import io.github.lingqiqi5211.meowui.component.MeowMenuItem
import io.github.lingqiqi5211.meowui.component.MeowScaffold
import io.github.lingqiqi5211.meowui.component.MeowTopBarAction
import io.github.lingqiqi5211.meowui.component.meowScaffoldScroll
import io.github.lingqiqi5211.meowui.theme.MeowIcons
import io.github.lingqiqi5211.meowui.theme.MeowTheme

/**
 * The daemon's log.
 *
 * Its own page rather than a block in the diagnostics list: a preference list lays out
 * label-and-value rows, and this needs both scroll axes.
 *
 * Built like the stack page, and for the same reasons. One line per list item, so a four-megabyte
 * file is not measured in a single pass on the main thread. One `horizontalScroll` around the
 * whole list rather than one per line, because each such node writes `maxValue` from its own
 * content and the shortest line would clamp the rest to nothing.
 */
@Composable
internal fun RuntimeLogPage(
    log: RuntimeLogUiState,
    actions: RuntimeLogActions,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    MeowScaffold(
        title = stringResource(R.string.diagnostics_section_log),
        modifier = modifier,
        subtitle = when {
            log.name.isEmpty() -> ""
            log.truncated -> stringResource(
                R.string.diagnostics_log_subtitle_truncated,
                log.name,
                formatBytes(log.totalBytes),
            )
            else -> stringResource(
                R.string.diagnostics_log_subtitle,
                log.name,
                formatBytes(log.totalBytes),
            )
        },
        onBackClick = onBack,
        // No copy or share here. Rotation means there are up to eighteen of these files, and
        // sending one of them as text is the wrong unit — the diagnostics page packs the lot.
        actionItems = listOf(
            MeowTopBarAction.Menu(
                icon = MeowIcons.Filter,
                contentDescription = stringResource(R.string.diagnostics_choose_log),
                modifier = Modifier.testTag("crashcatcher.log.choose"),
                items = log.files.map { file ->
                    MeowMenuItem(
                        text = "${file.name} · ${formatBytes(file.bytes)}",
                        selected = file.name == log.name,
                        onClick = { actions.onSelect(file.name) },
                    )
                },
            ),
            MeowTopBarAction.Icon(
                icon = MeowIcons.Refresh,
                contentDescription = stringResource(R.string.diagnostics_refresh_log),
                modifier = Modifier.testTag("crashcatcher.log.refresh"),
                onClick = actions.onRefresh,
            ),
        ),
    ) { scaffoldPadding ->
        if (log.isLoading && log.text.isEmpty()) {
            CrashCatcherLoadingState(
                testTag = "crashcatcher.log.loading",
                modifier = Modifier.fillMaxSize(),
            )
            return@MeowScaffold
        }
        log.error?.let { error ->
            CrashCatcherErrorState(
                testTag = "crashcatcher.log.error",
                title = errorTitle(error),
                description = errorDescription(error),
                onRetry = actions.onRefresh.takeIf { error.isRetryable() },
                modifier = Modifier.fillMaxSize(),
            )
            return@MeowScaffold
        }

        val empty = stringResource(R.string.diagnostics_log_empty)
        val lines = remember(log.text, empty) {
            log.text.trimEnd().ifBlank { empty }.lines()
        }
        // Keyed on the file: switching files starts at the top of the new one, while a refresh
        // of the same file keeps the reader where they were.
        val listState = remember(log.name) { LazyListState() }
        val panState = rememberScrollState()
        var zoom by rememberSaveable { mutableStateOf(1f) }
        val style = monospaceTextStyle(zoom)

        BoxWithConstraints(
            modifier = Modifier
                .fillMaxSize()
                // Above the list, not on it: the pinch has to be seen before the scroll node
                // claims the pointers.
                .pinchToZoom { factor -> zoom = (zoom * factor).coerceIn(MIN_ZOOM, MAX_ZOOM) },
        ) {
            val contentWidth = maxOf(
                maxWidth,
                monospaceContentWidth(lines, style, gutter = HORIZONTAL_PADDING * 2),
            )

            SelectionContainer {
                LazyColumn(
                    state = listState,
                    modifier = Modifier
                        .fillMaxSize()
                        // Before the scrollables, so the top bar gets the gesture first and
                        // collapses. After them it is a child of the scroll node, which is
                        // never consulted — the title then stays at full height forever.
                        .meowScaffoldScroll()
                        .horizontalScroll(panState)
                        .width(contentWidth)
                        .testTag("crashcatcher.log.text"),
                    contentPadding = PaddingValues(
                        top = scaffoldPadding.calculateTopPadding(),
                        bottom = scaffoldPadding.calculateBottomPadding() + 8.dp,
                    ),
                ) {
                    items(lines) { line ->
                        Text(
                            text = line,
                            style = style,
                            color = MeowTheme.colors.onSurfaceVariant,
                            softWrap = false,
                            modifier = Modifier.padding(horizontal = HORIZONTAL_PADDING),
                        )
                    }
                }
            }
        }
    }
}

private val HORIZONTAL_PADDING = 16.dp

internal data class RuntimeLogActions(
    val onRefresh: () -> Unit,
    val onSelect: (String) -> Unit,
)
