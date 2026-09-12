package io.github.lingqiqi5211.crashcatcher.ui.shell

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.widthIn
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.movableContentOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import io.github.lingqiqi5211.crashcatcher.ui.theme.LocalCrashCatcherFloatingNavigationBar
import io.github.lingqiqi5211.meowui.component.MeowNavigationBar
import io.github.lingqiqi5211.meowui.component.MeowNavigationBarStyle
import io.github.lingqiqi5211.meowui.component.MeowNavigationItem
import io.github.lingqiqi5211.meowui.component.MeowNavigationRail
import io.github.lingqiqi5211.meowui.component.MeowScaffold
import io.github.lingqiqi5211.meowui.component.MeowSnackbarState
import io.github.lingqiqi5211.meowui.component.MeowTopBarAction
import io.github.lingqiqi5211.meowui.component.MeowWindowHeight
import io.github.lingqiqi5211.meowui.component.MeowWindowWidth
import io.github.lingqiqi5211.meowui.component.rememberMeowNavigationRailState

/** Reading width cap on large screens. */
internal val ContentMaxWidth = 840.dp

/** Test handle for the root bottom bar; items are addressed by index within it. */
internal const val NavigationBarTag = "crashcatcher.nav.bar"

/** Test handle for the side rail, addressed the same way. */
internal const val NavigationRailTag = "crashcatcher.nav.rail"

/**
 * How the four root destinations are offered.
 *
 * A value rather than a branch inside the chrome, so [rootNavigationFor] can be read — and
 * tested — as the one sentence it is, instead of through a rendered tree that only proves
 * `BoxWithConstraints` reports its size.
 */
internal sealed interface RootNavigation {
    /** Across the bottom. [floating] is the capsule, which draws over the content. */
    data class Bar(val floating: Boolean) : RootNavigation

    /** Down the side. [expanded] adds the labels. */
    data class Rail(val expanded: Boolean) : RootNavigation
}

/**
 * Picks the navigation surface for a window of this shape.
 *
 * The capsule comes first and is never overridden: it is a setting, and it costs a wide window
 * nothing, drawing over the content rather than taking a row from it.
 *
 * Failing that, width decides. Not orientation — a tablet upright is 800dp across and was getting
 * the phone's bottom bar, while a phone sideways is that wide too and already had the rail.
 *
 * Labels stack down the side, so they ask for height rather than width.
 */
internal fun rootNavigationFor(
    windowWidth: Dp,
    windowHeight: Dp,
    floatingBar: Boolean,
): RootNavigation {
    if (floatingBar) return RootNavigation.Bar(floating = true)
    if (windowWidth < MeowWindowWidth.Medium) return RootNavigation.Bar(floating = false)
    return RootNavigation.Rail(expanded = windowHeight >= MeowWindowHeight.Medium)
}

/**
 * The chrome around the four root destinations.
 *
 * [MeowScaffold] owns the top bar, bottom bar, side rail and content insets, so this does not
 * assemble a Material and a Miuix variant of each. Top-bar actions are declared as
 * data ([MeowTopBarAction]) and MeowUI renders them in the active style.
 */
@Composable
internal fun RootScaffold(
    destination: Destination,
    actionItems: List<MeowTopBarAction>,
    onDestinationSelected: (Destination) -> Unit,
    snackbarState: MeowSnackbarState? = null,
    content: @Composable (PaddingValues) -> Unit,
) {
    // The rail and the bar are structurally different trees. Emitting the body directly in each
    // branch would make Compose dispose and rebuild the whole destination subtree whenever the
    // window changes shape — losing the pager page, every list's scroll position, and any
    // request already in flight. A movable content block keeps one instance and relocates it.
    val latestContent by rememberUpdatedState(content)
    val body = remember {
        movableContentOf { padding: PaddingValues ->
            CenteredContent(padding) { resolved -> latestContent(resolved) }
        }
    }
    val floatingBar = LocalCrashCatcherFloatingNavigationBar.current

    BoxWithConstraints(Modifier.fillMaxSize()) {
        val navigation = rootNavigationFor(maxWidth, maxHeight, floatingBar)

        MeowScaffold(
            title = stringResource(destination.labelRes),
            actionItems = actionItems,
            snackbarState = snackbarState,
            bottomBar = {
                if (navigation is RootNavigation.Bar) {
                    DestinationBar(
                        current = destination,
                        onDestinationSelected = onDestinationSelected,
                        floating = navigation.floating,
                    )
                }
            },
            navigationRail = (navigation as? RootNavigation.Rail)?.let { rail ->
                {
                    DestinationRail(
                        current = destination,
                        onDestinationSelected = onDestinationSelected,
                        expanded = rail.expanded,
                    )
                }
            },
        ) { padding -> body(padding) }
    }
}

/**
 * Caps the reading width on large screens while leaving the scroll surface, window
 * insets and focus traversal covering the whole viewport.
 */
@Composable
internal fun CenteredContent(
    paddingValues: PaddingValues,
    content: @Composable (PaddingValues) -> Unit,
) {
    Box(
        modifier = Modifier
            .fillMaxSize()
            .testTag("crashcatcher.content.centered"),
        contentAlignment = Alignment.TopCenter,
    ) {
        Box(
            modifier = Modifier
                .widthIn(max = ContentMaxWidth)
                .fillMaxSize()
                .testTag("crashcatcher.content.frame"),
        ) {
            content(paddingValues)
        }
    }
}

/**
 * The root bottom bar.
 *
 * MeowUI supplies both the standard and the floating capsule bar, including the
 * sliding indicator, labels and navigation-bar safe area, so this only picks a style
 * and hands over the items.
 */
@Composable
internal fun DestinationBar(
    current: Destination,
    onDestinationSelected: (Destination) -> Unit,
    floating: Boolean,
) {
    val destinations = Destination.entries
    MeowNavigationBar(
        items = destinations.map { destination ->
            MeowNavigationItem(
                label = stringResource(destination.labelRes),
                icon = destination.icon(selected = destination == current),
            )
        },
        selectedIndex = destinations.indexOf(current),
        // MeowUI's navigation items carry no per-item modifier, so the bar is tagged
        // and tests address items by position within it.
        modifier = Modifier.testTag(NavigationBarTag),
        onItemSelected = { index -> onDestinationSelected(destinations[index]) },
        style = if (floating) {
            MeowNavigationBarStyle.Floating
        } else {
            MeowNavigationBarStyle.Standard
        },
    )
}

/**
 * The side rail that replaces the bottom bar on a wide window.
 *
 * MeowUI resolves the rail per style — Material's `WideNavigationRail`, Miuix's own — so this
 * hands over items rather than keeping one hand-built copy of each.
 */
@Composable
internal fun DestinationRail(
    current: Destination,
    onDestinationSelected: (Destination) -> Unit,
    expanded: Boolean,
) {
    val destinations = Destination.entries
    val state = rememberMeowNavigationRailState(initiallyExpanded = expanded)
    MeowNavigationRail(
        items = destinations.map { destination ->
            MeowNavigationItem(
                label = stringResource(destination.labelRes),
                icon = destination.icon(selected = destination == current),
            )
        },
        selectedIndex = destinations.indexOf(current),
        onItemSelected = { index -> onDestinationSelected(destinations[index]) },
        modifier = Modifier
            .fillMaxHeight()
            .testTag(NavigationRailTag),
        state = state,
    )
}
