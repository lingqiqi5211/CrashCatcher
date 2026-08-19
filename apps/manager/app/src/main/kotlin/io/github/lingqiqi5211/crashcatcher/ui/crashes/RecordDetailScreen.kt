package io.github.lingqiqi5211.crashcatcher.ui.crashes

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.calculateEndPadding
import androidx.compose.foundation.layout.calculateStartPadding
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
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
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.unit.takeOrElse
import io.github.lingqiqi5211.crashcatcher.R
import io.github.lingqiqi5211.crashcatcher.data.daemon.ExportRedaction
import io.github.lingqiqi5211.crashcatcher.data.daemon.PayloadState
import io.github.lingqiqi5211.crashcatcher.ui.components.CrashCatcherLoadingState
import io.github.lingqiqi5211.crashcatcher.ui.components.WarningCard
import io.github.lingqiqi5211.crashcatcher.ui.util.processSuffix
import io.github.lingqiqi5211.crashcatcher.ui.util.shortTypeName
import io.github.lingqiqi5211.meowui.component.MeowMultiChoiceDialog
import io.github.lingqiqi5211.meowui.component.MeowScaffold
import io.github.lingqiqi5211.meowui.component.MeowSnackbarState
import io.github.lingqiqi5211.meowui.component.MeowTopBarAction
import io.github.lingqiqi5211.meowui.theme.MeowIcons
import io.github.lingqiqi5211.meowui.theme.MeowTheme

/**
 * The crash detail screen.
 *
 * Every line is a `LazyColumn` item, so only what is on screen is laid out. Putting
 * a whole trace in one text node is what makes the tool being replaced hang for
 * seconds on a large record — a few hundred kilobytes of text measured and laid out
 * in a single pass, on the main thread.
 *
 * Framework runs are folded by default, which usually leaves a first screen of about
 * a dozen lines regardless of how deep the trace is.
 *
 * The trace is selectable, and pinching resizes it: a stack trace is something people
 * read closely and quote from, and both gestures are what they already expect of a wall
 * of monospace.
 */
@Composable
internal fun RecordDetailScreen(
    state: RecordDetailUiState,
    actions: RecordDetailActions,
    modifier: Modifier = Modifier,
    snackbarState: MeowSnackbarState? = null,
) {
    val group = state.detail?.group
    // The heading is the exception's own name, not its package path: a display-size
    // title wraps at the character, so `java.lang.IllegalStateException` came out as
    // "java.lang.IllegalStateExc / eption". The prefix and the process live in the
    // subtitle, which is set at body size and has room for them.
    val title = group?.summaryClass?.let(::shortTypeName)
        ?: group?.packageName?.let(::shortTypeName)
        ?: stringResource(R.string.loading)
    val subtitle = remember(group) {
        val process = group?.let { processSuffix(it.packageName, it.processName) }
        listOfNotNull(group?.packageName, process).joinToString(separator = " · ")
    }

    // Which hand-off is waiting on the field chooser, if any.
    var handoff by remember { mutableStateOf<Handoff?>(null) }
    // Survives the dialog and the screen: someone who withheld their package name once is
    // unlikely to want it back on the next report they send.
    var included by rememberSaveable { mutableStateOf(ExportField.entries.toSet()) }

    ExportFieldDialog(
        handoff = handoff,
        included = included,
        onConfirmed = { fields ->
            included = fields
            val redaction = redactionOf(fields)
            when (handoff) {
                Handoff.Copy -> actions.onCopy(redaction)
                Handoff.Share -> actions.onShare(redaction)
                null -> Unit
            }
            handoff = null
        },
        onDismiss = { handoff = null },
    )

    MeowScaffold(
        title = title,
        modifier = modifier,
        subtitle = subtitle,
        onBackClick = actions.onBack,
        snackbarState = snackbarState,
        actionItems = listOf(
            MeowTopBarAction.Icon(
                icon = if (state.wrapLines) MeowIcons.WrapLines else MeowIcons.NoWrapLines,
                contentDescription = stringResource(R.string.detail_wrap_lines),
                modifier = Modifier.testTag("crashcatcher.detail.wrap"),
                onClick = actions.onToggleWrap,
            ),
            MeowTopBarAction.Icon(
                icon = MeowIcons.Copy,
                contentDescription = stringResource(R.string.detail_copy),
                modifier = Modifier.testTag("crashcatcher.detail.copy"),
                onClick = { handoff = Handoff.Copy },
            ),
            MeowTopBarAction.Icon(
                icon = MeowIcons.Share,
                contentDescription = stringResource(R.string.detail_share),
                modifier = Modifier.testTag("crashcatcher.detail.share"),
                onClick = { handoff = Handoff.Share },
            ),
            MeowTopBarAction.Icon(
                icon = MeowIcons.Delete,
                contentDescription = stringResource(R.string.detail_delete),
                modifier = Modifier.testTag("crashcatcher.detail.delete"),
                onClick = actions.onDelete,
            ),
        ),
    ) { scaffoldPadding ->
        if (state.isLoading) {
            CrashCatcherLoadingState(
                testTag = "crashcatcher.detail.loading",
                modifier = Modifier.fillMaxSize(),
            )
            return@MeowScaffold
        }

        val layoutDirection = LocalLayoutDirection.current
        val items = remember(state.payload, state.foldFrameworkFrames) { state.items }
        // Hoisted out of the wrap/pan branches below: toggling wrap must not scroll the
        // reader back to the top of a trace they had paged into.
        val listState = rememberLazyListState()
        val panState = rememberScrollState()

        var zoom by rememberSaveable { mutableStateOf(1f) }
        val traceStyle = traceTextStyle(zoom)

        // The heading stays; only the trace moves.
        //
        // Deliberately unlike the list pages, which hand their scroll to the top bar and
        // let it collapse. Here the heading names the exception being read — the one piece
        // of context that stops a screen of frames being anonymous — so it is worth its
        // height for the whole time the trace is open. That makes the top inset *layout*
        // padding: the lines scroll in the space below the title instead of passing under
        // it, which is what previously let them collide with it.
        val topInset = scaffoldPadding.calculateTopPadding()

        BoxWithConstraints(
            modifier = Modifier
                .fillMaxSize()
                .padding(
                    top = topInset,
                    start = scaffoldPadding.calculateStartPadding(layoutDirection),
                    end = scaffoldPadding.calculateEndPadding(layoutDirection),
                )
                .pinchToZoom { factor -> zoom = (zoom * factor).coerceIn(MIN_ZOOM, MAX_ZOOM) },
        ) {
            val viewportWidth = maxWidth
            // Panning is one scroll container around the whole list, not one per line.
            // A `horizontalScroll` node derives `maxValue` from the width of its own
            // content, so hundreds of them sharing a single ScrollState each overwrote
            // it with their own line's overflow — and the shortest visible line, which
            // fits and therefore reports zero, clamped the whole trace to no panning at
            // all. That is the "can't pan in no-wrap mode" bug.
            val contentWidth = maxOf(viewportWidth, traceWidth(items, state.expandedFolds, traceStyle))

            SelectionContainer {
                LazyColumn(
                    state = listState,
                    modifier = Modifier
                        .fillMaxSize()
                        .then(
                            if (state.wrapLines) {
                                Modifier
                            } else {
                                Modifier.horizontalScroll(panState).width(contentWidth)
                            },
                        )
                        .testTag("crashcatcher.detail.trace"),
                    // Bottom only: the top is layout padding above, and the sides are
                    // consumed there too. The last line still scrolls clear of the
                    // navigation bar rather than ending under it.
                    contentPadding = PaddingValues(
                        bottom = scaffoldPadding.calculateBottomPadding(),
                    ),
                ) {
                    item(key = "payload-state") {
                        // Held to the viewport: the list is as wide as the widest frame
                        // when panning, and a notice stretched to that width would run
                        // off the screen it is meant to be read on.
                        PayloadNotice(state = state, width = viewportWidth)
                    }

                    items(items, key = { item -> item.stableKey() }) { item ->
                        when (item) {
                            is StackItem.Line -> TraceLine(
                                line = item.line,
                                wrap = state.wrapLines,
                                style = traceStyle,
                            )

                            is StackItem.FoldedFrames ->
                                if (item.firstIndex in state.expandedFolds) {
                                    Column {
                                        item.lines.forEach { line ->
                                            TraceLine(
                                                line = line,
                                                wrap = state.wrapLines,
                                                style = traceStyle,
                                            )
                                        }
                                    }
                                } else {
                                    FoldExpander(
                                        count = item.lines.size,
                                        width = viewportWidth,
                                        onClick = { actions.onExpandFold(item.firstIndex) },
                                    )
                                }
                        }
                    }

                    if (!state.payloadComplete) {
                        item(key = "streaming") {
                            // The trace is still arriving; say so rather than letting the
                            // list look finished at whatever byte the stream happens to
                            // be on.
                            Text(
                                text = stringResource(R.string.loading),
                                style = MeowTheme.typography.summary,
                                color = MeowTheme.colors.onSurfaceVariant,
                                modifier = Modifier
                                    .width(viewportWidth)
                                    .padding(
                                        horizontal = MeowTheme.dimensions.pageHorizontalPadding,
                                        vertical = 12.dp,
                                    ),
                            )
                        }
                    }

                    item(key = "tail-spacer") { Spacer(Modifier.height(24.dp)) }
                }
            }
        }
    }
}

/** Where a report is headed, and therefore which title the chooser carries. */
private enum class Handoff { Copy, Share }

/**
 * The identifying fields a report may withhold.
 *
 * The same four the tool being replaced offers, and the same four the wire's
 * [ExportRedaction] already carries: a crash report is routinely pasted into a public
 * issue, and these are the lines that say which device it is and what the reporter has
 * installed. Everything else in the report describes the crash, so there is nothing to
 * gain by hiding it.
 */
private enum class ExportField(val labelRes: Int) {
    DeviceBrand(R.string.export_field_device_brand),
    DeviceModel(R.string.export_field_device_model),
    BuildDisplay(R.string.export_field_build_display),
    PackageName(R.string.export_field_package_name),
}

/** Included fields, as the wire's "hide this" flags. */
private fun redactionOf(included: Set<ExportField>) = ExportRedaction(
    hideDeviceBrand = ExportField.DeviceBrand !in included,
    hideDeviceModel = ExportField.DeviceModel !in included,
    hideBuildDisplayId = ExportField.BuildDisplay !in included,
    hidePackageName = ExportField.PackageName !in included,
)

/**
 * Asks what to include, every time, before anything leaves the app.
 *
 * Deliberately not a settings toggle applied silently: the answer depends on where this
 * particular report is going — an issue on a public tracker and a message to the app's own
 * developer do not want the same fields — and the moment of sharing is when the person
 * knows which it is.
 */
@Composable
private fun ExportFieldDialog(
    handoff: Handoff?,
    included: Set<ExportField>,
    onConfirmed: (Set<ExportField>) -> Unit,
    onDismiss: () -> Unit,
) {
    // Resolved here rather than in the label lambda: the dialog calls that lambda from a
    // plain function, where `stringResource` is not available.
    val labels = ExportField.entries.associateWith { stringResource(it.labelRes) }
    val title = stringResource(
        when (handoff) {
            Handoff.Share -> R.string.detail_fields_share_title
            Handoff.Copy, null -> R.string.detail_fields_copy_title
        },
    )

    MeowMultiChoiceDialog(
        show = handoff != null,
        title = title,
        selected = included,
        options = ExportField.entries,
        onConfirmed = onConfirmed,
        onDismissRequest = onDismiss,
        optionLabel = { field -> labels.getValue(field) },
        cancelText = stringResource(R.string.action_cancel),
        confirmText = stringResource(R.string.action_confirm),
    )
}

/**
 * The style every trace line shares, scaled by the pinch gesture.
 *
 * One style object for the whole list, so the width measurement below and what is
 * actually drawn cannot disagree — a pan sized against a different font than the text
 * either clips the longest line or pans into empty space.
 */
@Composable
private fun traceTextStyle(zoom: Float): TextStyle {
    val summary = MeowTheme.typography.summary
    return remember(summary, zoom) {
        val size = summary.fontSize.takeOrElse { DEFAULT_TRACE_FONT_SIZE } * zoom
        summary.copy(
            fontFamily = FontFamily.Monospace,
            fontSize = size,
            lineHeight = size * TRACE_LINE_HEIGHT,
        )
    }
}

/**
 * How wide the trace has to be laid out for nothing to be clipped when panning.
 *
 * Measured rather than estimated, because the answer decides whether the last character
 * of the longest frame is reachable. Only a handful of candidate lines are measured: the
 * font is monospace, so counting columns ranks lines almost perfectly, and the exact
 * measurement then settles the ranking's blind spot — a message line of CJK text, whose
 * glyphs are twice as wide as the frames around it.
 */
@Composable
private fun traceWidth(
    items: List<StackItem>,
    expandedFolds: Set<Int>,
    style: TextStyle,
): Dp {
    val measurer = rememberTextMeasurer()
    val density = LocalDensity.current
    val gutter = MeowTheme.dimensions.pageHorizontalPadding * 2 + FRAME_RAIL_GUTTER
    return remember(items, expandedFolds, style, measurer, density, gutter) {
        val widest = widestLines(visibleLines(items, expandedFolds), WIDTH_CANDIDATES)
        val pixels = widest.maxOfOrNull { line ->
            measurer.measure(text = line, style = style, softWrap = false).size.width
        } ?: 0
        with(density) { pixels.toDp() } + gutter
    }
}

/** The lines actually on screen — a collapsed run contributes nothing to the width. */
private fun visibleLines(items: List<StackItem>, expandedFolds: Set<Int>): List<String> =
    buildList {
        for (item in items) {
            when (item) {
                is StackItem.Line -> add(item.line.text.trimEnd())
                is StackItem.FoldedFrames -> if (item.firstIndex in expandedFolds) {
                    item.lines.forEach { add(it.text.trimEnd()) }
                }
            }
        }
    }

/**
 * The [count] longest lines, by estimated columns.
 *
 * One pass, with the estimate computed once per line. Sorting instead would re-run it on
 * every comparison, and a tombstone runs to tens of thousands of lines — enough for the
 * difference to be felt on each pinch, since the width is measured again whenever the
 * font size changes.
 */
private fun widestLines(lines: List<String>, count: Int): List<String> {
    val top = ArrayList<Pair<String, Int>>(count + 1)
    for (line in lines) {
        val columns = displayColumns(line)
        if (top.size == count && columns <= top[top.size - 1].second) continue
        val at = top.indexOfFirst { columns > it.second }.takeIf { it >= 0 } ?: top.size
        top.add(at, line to columns)
        if (top.size > count) top.removeAt(top.size - 1)
    }
    return top.map { it.first }
}

/** Rough printed width of a line, counting anything past the ASCII range as double. */
private fun displayColumns(text: String): Int {
    var columns = 0
    for (character in text) {
        columns += if (character.code >= WIDE_CHARACTER_START) 2 else 1
    }
    return columns
}

/**
 * Two-finger resize, without taking the one-finger gestures away.
 *
 * Claimed on the initial pass: the list's scroll and the selection handles both react on
 * the main pass, so a pinch that waited its turn would have scrolled the trace and
 * started selecting text before it was recognised. Single-pointer events are left
 * untouched and reach them as usual.
 */
private fun Modifier.pinchToZoom(onZoom: (Float) -> Unit): Modifier = pointerInput(Unit) {
    awaitEachGesture {
        awaitFirstDown(requireUnconsumed = false, pass = PointerEventPass.Initial)
        do {
            val event = awaitPointerEvent(PointerEventPass.Initial)
            val pressed = event.changes.filter { it.pressed }
            if (pressed.size >= 2) {
                val spread = (pressed[0].position - pressed[1].position).getDistance()
                val before =
                    (pressed[0].previousPosition - pressed[1].previousPosition).getDistance()
                if (before > 0f && spread > 0f) onZoom(spread / before)
                pressed.forEach { it.consume() }
            }
        } while (event.changes.any { it.pressed })
    }
}

@Composable
private fun PayloadNotice(state: RecordDetailUiState, width: Dp) {
    val noticeModifier = Modifier
        .width(width)
        .padding(
            horizontal = MeowTheme.dimensions.pageHorizontalPadding,
            vertical = 8.dp,
        )

    when (state.payloadState) {
        PayloadState.Truncated -> WarningCard(
            title = stringResource(R.string.detail_payload_truncated),
            body = stringResource(R.string.detail_payload_truncated_body),
            modifier = noticeModifier.testTag("crashcatcher.detail.truncated"),
        )

        PayloadState.Evicted -> WarningCard(
            title = stringResource(R.string.detail_payload_evicted),
            body = stringResource(R.string.detail_payload_evicted_body),
            modifier = noticeModifier.testTag("crashcatcher.detail.evicted"),
        )

        // Says which collector saw it and why that leaves no stack, rather than implying the
        // retention limits took one away.
        PayloadState.Absent -> WarningCard(
            title = stringResource(R.string.detail_payload_absent),
            body = stringResource(R.string.detail_payload_absent_body),
            modifier = noticeModifier.testTag("crashcatcher.detail.absent"),
        )

        else -> Unit
    }
}

/**
 * One line of a trace.
 *
 * Three kinds of line are told apart, because a wall of identical monospace is what
 * made this unreadable: the exception header (and any `Caused by:`) carries the actual
 * failure and is emphasised, app frames are the ones worth reading and stay at full
 * contrast, and platform frames recede.
 *
 * Frames are indented and drawn against a faint rail so the trace reads as one block
 * rather than a list of unrelated rows.
 */
@Composable
private fun TraceLine(line: StackLine, wrap: Boolean, style: TextStyle) {
    val isFrame = line.text.trimStart().startsWith("at ")
    val isHeader = !isFrame && line.text.isNotBlank()

    val color = when {
        isHeader -> MeowTheme.colors.onSurface
        line.isFramework -> MeowTheme.colors.onSurfaceVariant
        else -> MeowTheme.colors.onSurface
    }

    Row(
        modifier = Modifier
            .fillMaxWidth()
            // The row is exactly as tall as its text, which is what lets the rail below
            // match the line it belongs to at any zoom level.
            .height(IntrinsicSize.Min)
            .padding(horizontal = MeowTheme.dimensions.pageHorizontalPadding),
    ) {
        if (isFrame) {
            // A rail rather than an indent alone: it marks where the frames start and
            // survives horizontal panning, which a leading space does not.
            Box(
                modifier = Modifier
                    .padding(end = 10.dp)
                    .width(2.dp)
                    .fillMaxHeight()
                    .background(MeowTheme.colors.onSurfaceVariant.copy(alpha = 0.25f)),
            )
        }

        val text = remember(line.text, wrap) {
            val trimmed = line.text.trimEnd()
            // Break opportunities only matter when wrapping; panning wants the line
            // measured exactly as written.
            if (wrap) withBreakOpportunities(trimmed) else trimmed
        }

        Text(
            text = text,
            style = style,
            color = color,
            fontWeight = if (isHeader) FontWeight.Medium else FontWeight.Normal,
            maxLines = if (wrap) Int.MAX_VALUE else 1,
            overflow = TextOverflow.Clip,
            softWrap = wrap,
            modifier = Modifier
                .weight(1f)
                .padding(vertical = 2.dp),
        )
    }
}

@Composable
private fun FoldExpander(count: Int, width: Dp, onClick: () -> Unit) {
    Text(
        text = stringResource(R.string.detail_expand_framework_frames, count),
        style = MeowTheme.typography.summary,
        color = MeowTheme.colors.primary,
        modifier = Modifier
            .width(width)
            .clickable(onClick = onClick)
            .padding(
                horizontal = MeowTheme.dimensions.pageHorizontalPadding,
                vertical = 8.dp,
            )
            .testTag("crashcatcher.detail.expand"),
    )
}

private fun StackItem.stableKey(): String = when (this) {
    is StackItem.Line -> "line-${line.index}"
    is StackItem.FoldedFrames -> "fold-$firstIndex"
}

/** Used only if the active style leaves `summary` without a size of its own. */
private val DEFAULT_TRACE_FONT_SIZE = 13.sp

private const val TRACE_LINE_HEIGHT = 1.4f

private const val MIN_ZOOM = 0.7f
private const val MAX_ZOOM = 2.5f

/** How many of the longest lines are measured exactly. */
private const val WIDTH_CANDIDATES = 5

/** Where the column estimate starts assuming double-width glyphs. */
private const val WIDE_CHARACTER_START = 0x1100

/** The rail and its gap, which sit to the left of every frame. */
private val FRAME_RAIL_GUTTER = 12.dp

internal data class RecordDetailActions(
    val onBack: () -> Unit,
    val onCopy: (ExportRedaction) -> Unit,
    val onShare: (ExportRedaction) -> Unit,
    val onDelete: () -> Unit,
    val onExpandFold: (Int) -> Unit,
    val onToggleWrap: () -> Unit,
)
