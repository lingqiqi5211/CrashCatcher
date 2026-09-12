package io.github.lingqiqi5211.crashcatcher.ui.shell

import androidx.compose.material3.Text
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.lingqiqi5211.crashcatcher.test.TestManagerTheme
import io.github.lingqiqi5211.crashcatcher.ui.theme.LocalCrashCatcherFloatingNavigationBar
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * That the chrome draws the surface [rootNavigationFor] picks, against a real window.
 *
 * Which surface belongs to which shape is arithmetic and lives in [RootNavigationTest]; this is
 * the half that needs a tree. The window sizes are Robolectric qualifiers rather than a sized
 * `Box`: the chrome reads the constraints it is handed, and a `Box` larger than the emulated
 * screen lays its bar out past the viewport, where it exists but is not displayed.
 *
 * Pinned to SDK 29 so the theme avoids the system-Monet branch and the tokens stay deterministic.
 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [29])
@GraphicsMode(GraphicsMode.Mode.LEGACY)
class RootChromeTest {
    @get:Rule
    val compose = createComposeRule()

    private fun showChrome(capsule: Boolean = false) {
        compose.setContent {
            TestManagerTheme {
                CompositionLocalProvider(
                    LocalCrashCatcherFloatingNavigationBar provides capsule,
                ) {
                    RootScaffold(
                        destination = Destination.Home,
                        actionItems = emptyList(),
                        onDestinationSelected = {},
                    ) { Text("body") }
                }
            }
        }
    }

    @Test
    @Config(qualifiers = "w400dp-h880dp")
    fun aPhoneUprightKeepsTheBottomBar() {
        showChrome()

        compose.onNodeWithTag(NavigationBarTag).assertIsDisplayed()
        compose.onNodeWithTag(NavigationRailTag).assertDoesNotExist()
    }

    /** The case that used to need a tablet-shaped window and an orientation to get right. */
    @Test
    @Config(qualifiers = "w777dp-h1164dp")
    fun aTabletUprightGetsTheRail() {
        showChrome()

        compose.onNodeWithTag(NavigationRailTag).assertIsDisplayed()
        compose.onNodeWithTag(NavigationBarTag).assertDoesNotExist()
    }

    @Test
    @Config(qualifiers = "w1164dp-h777dp")
    fun aTabletOnItsSideGetsTheRail() {
        showChrome()

        compose.onNodeWithTag(NavigationRailTag).assertIsDisplayed()
        compose.onNodeWithTag(NavigationBarTag).assertDoesNotExist()
    }

    /** The setting wins at tablet size too, which is the size it is most likely to have meant. */
    @Test
    @Config(qualifiers = "w1164dp-h777dp")
    fun theCapsuleIsNotTradedForARail() {
        showChrome(capsule = true)

        compose.onNodeWithTag(NavigationBarTag).assertIsDisplayed()
        compose.onNodeWithTag(NavigationRailTag).assertDoesNotExist()
    }
}
