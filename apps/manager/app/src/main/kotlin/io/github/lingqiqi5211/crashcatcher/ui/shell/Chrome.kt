package io.github.lingqiqi5211.crashcatcher.ui.shell

import android.content.res.Configuration
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.displayCutout
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.systemBars
import androidx.compose.foundation.layout.union
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.material3.Icon
import androidx.compose.material3.NavigationRail
import androidx.compose.material3.NavigationRailItem
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.movableContentOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import io.github.lingqiqi5211.crashcatcher.ui.theme.LocalCrashCatcherFloatingNavigationBar
import io.github.lingqiqi5211.crashcatcher.ui.theme.isMiuixStyle
import io.github.lingqiqi5211.meowui.component.MeowNavigationBar
import io.github.lingqiqi5211.meowui.component.MeowNavigationBarStyle
import io.github.lingqiqi5211.meowui.component.MeowNavigationItem
import io.github.lingqiqi5211.meowui.component.MeowScaffold
import io.github.lingqiqi5211.meowui.component.MeowSnackbarState
import io.github.lingqiqi5211.meowui.component.MeowTopBarAction
import top.yukonga.miuix.kmp.basic.NavigationRail as MiuixNavigationRail
import top.yukonga.miuix.kmp.basic.NavigationRailItem as MiuixNavigationRailItem

/** Reading width cap on large screens. */
internal val ContentMaxWidth = 840.dp

/** Test handle for the root bottom bar; items are addressed by index within it. */
internal const val NavigationBarTag = "crashcatcher.nav.bar"

/**
 * The chrome around the four root destinations.
 *
 * [MeowScaffold] owns the top bar, bottom bar and content insets, so this does not
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
    // Orientation and bar style pick between structurally different trees (rail +
    // scaffold, or scaffold + bottom bar). Emitting the body directly in each branch
    // would make Compose dispose and rebuild the whole destination subtree on every
    // rotation — losing the pager page, every list's scroll position, and any request
    // already in flight. A movable content block keeps one instance and relocates it.
    val latestContent by rememberUpdatedState(content)
    val body = remember {
        movableContentOf { padding: PaddingValues ->
            CenteredContent(padding) { resolved -> latestContent(resolved) }
        }
    }

    val landscape =
        LocalConfiguration.current.orientation == Configuration.ORIENTATION_LANDSCAPE
    val floatingNavigationBar = LocalCrashCatcherFloatingNavigationBar.current
    // The floating capsule already reads as an overlay rather than a docked bar, so
    // it stays at the bottom in landscape instead of collapsing into a rail.
    val useNavigationRail = landscape && !floatingNavigationBar

    if (useNavigationRail) {
        val startInsets = systemBarInsets.only(WindowInsetsSides.Start)
        Row(Modifier.fillMaxSize()) {
            DestinationRail(destination, onDestinationSelected)
            MeowScaffold(
                title = stringResource(destination.labelRes),
                modifier = Modifier
                    .weight(1f)
                    .consumeWindowInsets(startInsets),
                actionItems = actionItems,
                snackbarState = snackbarState,
            ) { padding -> body(padding) }
        }
        return
    }

    MeowScaffold(
        title = stringResource(destination.labelRes),
        actionItems = actionItems,
        snackbarState = snackbarState,
        bottomBar = {
            DestinationBar(
                current = destination,
                onDestinationSelected = onDestinationSelected,
                floating = floatingNavigationBar,
            )
        },
    ) { padding -> body(padding) }
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
 * The landscape navigation rail.
 *
 * MeowUI has no rail component, so this stays an app surface and uses each design
 * system's own rail rather than a recoloured copy of the other.
 */
@Composable
internal fun DestinationRail(
    current: Destination,
    onDestinationSelected: (Destination) -> Unit,
) {
    val railInsets = systemBarInsets.only(WindowInsetsSides.Start + WindowInsetsSides.Vertical)

    if (isMiuixStyle()) {
        MiuixNavigationRail(
            modifier = Modifier
                .fillMaxHeight()
                .windowInsetsPadding(railInsets),
        ) {
            Spacer(Modifier.weight(1f))
            Destination.entries.forEach { destination ->
                MiuixNavigationRailItem(
                    modifier = Modifier
                        .padding(vertical = 4.dp)
                        .testTag(destination.testTag),
                    selected = destination == current,
                    onClick = { onDestinationSelected(destination) },
                    icon = destination.icon(selected = true),
                    label = stringResource(destination.labelRes),
                )
            }
            Spacer(Modifier.weight(1f))
        }
        return
    }

    NavigationRail(
        modifier = Modifier.fillMaxHeight(),
        windowInsets = railInsets,
    ) {
        Spacer(Modifier.weight(1f))
        Destination.entries.forEach { destination ->
            val selected = destination == current
            NavigationRailItem(
                selected = selected,
                onClick = { onDestinationSelected(destination) },
                icon = {
                    Icon(
                        imageVector = destination.icon(selected = selected),
                        contentDescription = null,
                    )
                },
                label = { Text(text = stringResource(destination.labelRes)) },
                modifier = Modifier.testTag(destination.testTag),
            )
        }
        Spacer(Modifier.weight(1f))
    }
}

private val systemBarInsets: WindowInsets
    @Composable get() = WindowInsets.systemBars.union(WindowInsets.displayCutout)
