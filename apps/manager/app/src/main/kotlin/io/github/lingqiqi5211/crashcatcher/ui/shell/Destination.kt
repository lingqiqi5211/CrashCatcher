package io.github.lingqiqi5211.crashcatcher.ui.shell

import androidx.annotation.StringRes
import androidx.compose.runtime.Composable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.ui.graphics.vector.ImageVector
import io.github.lingqiqi5211.crashcatcher.R
import io.github.lingqiqi5211.crashcatcher.data.daemon.RecordId
import io.github.lingqiqi5211.meowui.theme.MeowIcons

/** The four root destinations, in bottom-bar order. */
internal enum class Destination(
    val route: String,
    @param:StringRes val labelRes: Int,
    val testTag: String,
) {
    Home(
        route = "home",
        labelRes = R.string.destination_home,
        testTag = "crashcatcher.nav.home",
    ),
    Crashes(
        route = "crashes",
        labelRes = R.string.destination_crashes,
        testTag = "crashcatcher.nav.crashes",
    ),
    Apps(
        route = "apps",
        labelRes = R.string.destination_apps,
        testTag = "crashcatcher.nav.apps",
    ),
    Settings(
        route = "settings",
        labelRes = R.string.destination_settings,
        testTag = "crashcatcher.nav.settings",
    ),
}

/**
 * The bar icon for a destination.
 *
 * A function rather than an enum field: which glyph this is depends on the interface style,
 * which is a composition value, and an enum constant is built once before any composition
 * exists. Holding a Material vector in the enum is exactly how a Miuix skin ended up with
 * Material icons in its navigation bar.
 */
@Composable
@ReadOnlyComposable
internal fun Destination.icon(selected: Boolean): ImageVector = when (this) {
    Destination.Home -> if (selected) MeowIcons.HomeSelected else MeowIcons.Home
    Destination.Crashes -> if (selected) MeowIcons.CrashesSelected else MeowIcons.Crashes
    Destination.Apps -> if (selected) MeowIcons.AppsSelected else MeowIcons.Apps
    Destination.Settings -> if (selected) MeowIcons.SettingsSelected else MeowIcons.Settings
}

/**
 * Pages pushed on top of the root shell.
 *
 * Plain objects and data classes rather than string routes: the back stack is an
 * ordinary `List`, so the compiler checks what goes on it and a page can carry a
 * typed argument without any parsing.
 */
internal sealed interface Page {
    /** Page zero — the whole four-tab shell. */
    data object Shell : Page

    data class GroupDetail(val groupId: String) : Page

    data class RecordDetail(val id: RecordId) : Page

    data class AppDetail(val packageName: String) : Page

    data object Appearance : Page

    /**
     * Settings sub-pages.
     *
     * The settings tab is a table of contents rather than a wall of controls: it used to
     * put five sections and about a dozen switches at the top level, so finding one
     * meant reading all of them. Each group of related controls now lives behind its own
     * row, which also gives the page room to explain what a group is for.
     */
    data object CaptureSettings : Page

    data object NotifySettings : Page

    data object DialogSettings : Page

    data object StorageSettings : Page

    /** Reachable when the daemon is not: everything on it reads rather than writes. */
    data object Diagnostics : Page

    /** The daemon's log, on its own page: a wall of fixed-width text that pans sideways. */
    data object RuntimeLog : Page

    data object About : Page
}
