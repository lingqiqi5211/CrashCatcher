package io.github.lingqiqi5211.crashcatcher.ui.crashes

/**
 * Package prefixes treated as framework noise.
 *
 * The **list** mirrors `cch_model::FRAMEWORK_PREFIXES`, so a frame can never be
 * "interesting enough to group by" on the daemon side and "noise" here. The
 * predicates around it differ on purpose: the daemon is handed frames and only has
 * to judge their package, while this sees every line of a trace — headers,
 * `Caused by:`, blank lines and native backtraces included — and has to decide
 * first whether the line is a frame at all.
 */
private val FRAMEWORK_PREFIXES = listOf(
    "android.",
    "androidx.",
    "com.android.internal.",
    "com.android.server.",
    "dalvik.",
    "java.",
    "javax.",
    "kotlin.",
    "kotlinx.coroutines.",
    "libcore.",
    "sun.",
    "art_",
)

/**
 * True when a stack line belongs to the platform rather than to app code.
 *
 * Only lines that are actually *frames* can qualify. An exception header reads
 * `java.lang.IllegalStateException: Fragment already added` — matching it on the
 * `java.` prefix would fold away the single most important line of the trace, and
 * the same goes for `Caused by:` lines. So a line has to carry the `at ` marker
 * before its package is even considered.
 *
 * Native backtrace lines (`#00 pc … /lib/libfoo.so`) are never folded: they are
 * library paths, not Java packages, and this list would say nothing useful about them.
 */
internal fun isFrameworkFrame(line: String): Boolean {
    val trimmed = line.trim()
    if (!trimmed.startsWith(FRAME_MARKER)) return false
    val frame = trimmed.removePrefix(FRAME_MARKER).trimStart()
    return FRAMEWORK_PREFIXES.any { frame.startsWith(it) }
}

private const val FRAME_MARKER = "at "

/** One rendered line of a stack trace. */
internal data class StackLine(
    val index: Int,
    val text: String,
    val isFramework: Boolean,
)

/**
 * A zero-width space, which text layout may break a line at but which draws nothing.
 */
private const val BREAK_OPPORTUNITY = '​'

/**
 * Marks the places a stack line may be wrapped.
 *
 * A Java frame has no spaces in it, so ordinary wrapping breaks wherever the width runs
 * out — `MainActivi` / `ty.kt:46`, mid-identifier. Horizontal panning was the first
 * answer to that and is not a good one: a right-drag on a pushed page is the predictive
 * back gesture, so the trace could only ever be panned one way.
 *
 * Inserting zero-width spaces after the separators that already divide a frame into
 * meaningful parts — package dots, inner-class `$`, the path slashes of a native frame —
 * lets the line wrap between segments instead of inside them, with nothing added to what
 * is drawn or to what [io.github.lingqiqi5211.crashcatcher.ui.crashes.RecordDetailUiState]
 * hands to the clipboard.
 *
 * Not inserted after `(`: `(MainActivity.kt:46)` is one token to a reader looking for
 * where the frame lives, and breaking into it costs more than the width it saves.
 */
internal fun withBreakOpportunities(line: String): String = buildString(line.length * 2) {
    for (character in line) {
        append(character)
        if (character in BREAKABLE_AFTER) append(BREAK_OPPORTUNITY)
    }
}

private const val BREAKABLE_AFTER = ".$/"

/**
 * A run of consecutive framework lines, collapsed behind one expander.
 *
 * Runs rather than per-line hiding: a trace is typically a handful of app frames
 * separated by long platform stretches, and collapsing each stretch as a unit keeps
 * the surviving app frames in their original order and context.
 */
internal sealed interface StackItem {
    data class Line(val line: StackLine) : StackItem

    data class FoldedFrames(val lines: List<StackLine>) : StackItem {
        val firstIndex: Int get() = lines.first().index
    }
}

/**
 * Splits payload text into lines and folds framework runs.
 *
 * Runs shorter than [minimumFoldSize] are left alone: replacing two lines with an
 * expander that says "show 2 frames" costs the reader a tap and saves nothing.
 */
internal fun buildStackItems(
    text: String,
    foldFrameworkFrames: Boolean,
    minimumFoldSize: Int = 3,
): List<StackItem> {
    val lines = text.lineSequence()
        .mapIndexed { index, line -> StackLine(index, line, isFrameworkFrame(line)) }
        .toList()

    if (!foldFrameworkFrames) return lines.map(StackItem::Line)

    val items = mutableListOf<StackItem>()
    var run = mutableListOf<StackLine>()

    fun flush() {
        when {
            run.isEmpty() -> Unit
            run.size >= minimumFoldSize -> items += StackItem.FoldedFrames(run.toList())
            else -> run.forEach { items += StackItem.Line(it) }
        }
        run = mutableListOf()
    }

    for (line in lines) {
        // A blank line inside a run is part of the run; a blank line outside one is
        // just spacing. Treating blanks as neutral keeps a trace's paragraph breaks
        // from splitting one platform stretch into several expanders.
        if (line.isFramework || (run.isNotEmpty() && line.text.isBlank())) {
            run += line
        } else {
            flush()
            items += StackItem.Line(line)
        }
    }
    flush()

    return items
}
