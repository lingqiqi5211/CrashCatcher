package io.github.lingqiqi5211.crashcatcher.ui.components

import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.runtime.Composable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.ui.unit.dp

/**
 * How far the shell's bottom bar reaches into a root destination.
 *
 * The shell does not lay its destinations out above the bottom bar. It pads only the
 * top and sides and publishes the bottom inset here, so a destination's scroll
 * container can take it as *content* padding: items then scroll through the strip the
 * bar occupies and the last one still clears it. Padding the destination itself
 * instead left that strip as flat scaffold background, which made the floating capsule
 * look docked on an opaque band rather than hovering over the content.
 *
 * A destination with no scroll container can apply it as ordinary padding.
 *
 * Published through a composition local rather than passed down as a parameter because
 * the value is produced by the shell and consumed several layers down, by whichever
 * scroll container a destination happens to own; threading it through would put a
 * padding parameter on every screen between the two.
 */
internal val LocalCrashCatcherContentBottomPadding = compositionLocalOf { 0.dp }

/**
 * How far the shell's top bar reaches into a root destination, published the same way:
 * a scrollable page takes it as content padding so its content scrolls *under* the
 * (frosted) top bar; a page with pinned chrome applies it as layout padding instead.
 */
internal val LocalCrashCatcherContentTopPadding = compositionLocalOf { 0.dp }

/** The published bottom inset as [PaddingValues], for scroll containers. */
internal val crashCatcherContentBottomPadding: PaddingValues
    @Composable
    get() = PaddingValues(bottom = LocalCrashCatcherContentBottomPadding.current)

/** Both published insets as [PaddingValues], for pages that scroll under both bars. */
internal val crashCatcherContentScaffoldPadding: PaddingValues
    @Composable
    get() = PaddingValues(
        top = LocalCrashCatcherContentTopPadding.current,
        bottom = LocalCrashCatcherContentBottomPadding.current,
    )
