package io.github.lingqiqi5211.crashcatcher.ui.shell

import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * Which navigation surface a window of a given shape gets.
 *
 * A plain function rather than a rendered tree: the decision is arithmetic on two measurements,
 * and testing it through Compose would only prove that `BoxWithConstraints` reports its size.
 *
 * The shapes are the real ones. 400×880 is a phone upright, 880×400 the same phone on its side,
 * and 777×1164 / 1164×777 a Xiaomi Pad 7 in its two orientations.
 */
class RootNavigationTest {
    private fun navigation(
        width: Dp,
        height: Dp,
        floatingBar: Boolean = false,
    ): RootNavigation = rootNavigationFor(width, height, floatingBar)

    @Test
    fun aPhoneUprightGetsTheBottomBar() {
        assertEquals(RootNavigation.Bar(floating = false), navigation(400.dp, 880.dp))
    }

    /**
     * The case the old rule got right by accident: it asked about orientation, and landscape
     * happened to mean wide. Collapsed, because four labels stack down a side that is 400dp tall.
     */
    @Test
    fun aPhoneOnItsSideGetsACollapsedRail() {
        assertEquals(RootNavigation.Rail(expanded = false), navigation(880.dp, 400.dp))
    }

    /** The case it got wrong: upright, so not landscape, but far too wide for a bottom bar. */
    @Test
    fun aTabletGetsTheRailInBothOrientations() {
        assertEquals(RootNavigation.Rail(expanded = true), navigation(777.dp, 1164.dp))
        assertEquals(RootNavigation.Rail(expanded = true), navigation(1164.dp, 777.dp))
    }

    /**
     * The capsule is a setting, and a window shape is not an argument against it. It draws over
     * the content rather than taking a row from it, so a wide window loses nothing by keeping it.
     *
     * Every shape, so that no size silently answers a question the user already answered.
     */
    @Test
    fun theCapsuleIsKeptAtEverySize() {
        for ((width, height) in listOf(
            400.dp to 880.dp,
            880.dp to 400.dp,
            777.dp to 1164.dp,
            1164.dp to 777.dp,
        )) {
            assertEquals(
                "$width x $height",
                RootNavigation.Bar(floating = true),
                navigation(width, height, floatingBar = true),
            )
        }
    }
}
