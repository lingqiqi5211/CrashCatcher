package io.github.lingqiqi5211.crashcatcher.ui.components

import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.unit.takeOrElse
import io.github.lingqiqi5211.meowui.theme.MeowTheme

/**
 * Fixed-width text that can be pinched, shared by the stack page and the runtime log.
 *
 * Both show machine output where columns have to line up, and both are read on a phone at a
 * size chosen for prose.
 */

/** The summary style, monospaced and scaled by [zoom]. */
@Composable
internal fun monospaceTextStyle(zoom: Float): TextStyle {
    val summary = MeowTheme.typography.summary
    return remember(summary, zoom) {
        val size = summary.fontSize.takeOrElse { DEFAULT_MONOSPACE_SIZE } * zoom
        summary.copy(
            fontFamily = FontFamily.Monospace,
            fontSize = size,
            lineHeight = size * MONOSPACE_LINE_HEIGHT,
        )
    }
}

/**
 * Reports the pinch factor of a two-finger gesture.
 *
 * Claimed on the initial pass. Scrolling and the selection handles both react on the main pass,
 * so a pinch that waited its turn would have scrolled and started selecting text before being
 * recognised. Single-pointer events are left untouched.
 */
internal fun Modifier.pinchToZoom(onZoom: (Float) -> Unit): Modifier = pointerInput(Unit) {
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

/**
 * Width [lines] need at [style] without wrapping, plus [gutter].
 *
 * Panning is one scroll container around the whole block, so it needs a width up front. Not every
 * line is measured: a tombstone runs to tens of thousands, and the width is measured again on
 * every pinch. The candidates are picked by estimated columns first, and only those go through
 * the measurer.
 */
@Composable
internal fun monospaceContentWidth(lines: List<String>, style: TextStyle, gutter: Dp): Dp {
    val measurer = rememberTextMeasurer()
    val density = LocalDensity.current
    return remember(lines, style, measurer, density, gutter) {
        val pixels = widestLines(lines, WIDTH_CANDIDATES).maxOfOrNull { line ->
            measurer.measure(text = line, style = style, softWrap = false).size.width
        } ?: 0
        with(density) { pixels.toDp() } + gutter
    }
}

/**
 * The [count] longest lines, by estimated columns.
 *
 * One pass, with the estimate computed once per line. Sorting instead would re-run it on every
 * comparison.
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

/** Clamped so a pinch cannot leave the text unreadably small or a single word per screen. */
internal const val MIN_ZOOM = 0.7f
internal const val MAX_ZOOM = 2.5f

private val DEFAULT_MONOSPACE_SIZE = 13.sp
private const val MONOSPACE_LINE_HEIGHT = 1.4f

/**
 * How many of the longest lines are measured precisely.
 *
 * The column estimate treats every wide character as exactly two, which is close but not exact
 * for proportional fallbacks, so the top few are measured rather than the single winner.
 */
private const val WIDTH_CANDIDATES = 8

/** Past this, a character is assumed to print double-width — CJK, and close enough elsewhere. */
private const val WIDE_CHARACTER_START = 0x1100
