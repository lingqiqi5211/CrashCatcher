package io.github.lingqiqi5211.crashcatcher.ui.components

import androidx.compose.foundation.layout.Column
import androidx.compose.material3.Text
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.lingqiqi5211.crashcatcher.domain.model.UiMode
import io.github.lingqiqi5211.crashcatcher.test.TestManagerTheme
import io.github.lingqiqi5211.crashcatcher.test.defaultTestAppearance
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * Structural coverage for [TonalCard]:
 *  - renders body content inside the card surface and exposes the test tag attached
 *    to the card, so screens can select their own status cards,
 *  - invokes `onClick` when the clickable variant is tapped — the tag must land on
 *    the node that carries the click, not on a wrapper above it,
 *  - renders in both styles, because the card delegates to a MeowUI component whose
 *    two branches build different node trees.
 *
 * Pinned to SDK 29 so the theme avoids the system-Monet branch (gated on SDK >= S)
 * and the resulting tokens stay deterministic across Robolectric runs.
 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [29])
@GraphicsMode(GraphicsMode.Mode.LEGACY)
class TonalCardTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun rendersBodyContent() {
        compose.setContent {
            TestManagerTheme {
                TonalCard(modifier = Modifier.testTag("crashcatcher.card.status")) {
                    Text("body content")
                }
            }
        }
        compose.waitForIdle()

        compose.onNodeWithTag("crashcatcher.card.status").assertIsDisplayed()
    }

    @Test
    fun clickableVariantInvokesOnClick() {
        var clicks = 0
        compose.setContent {
            TestManagerTheme {
                TonalCard(
                    modifier = Modifier.testTag("crashcatcher.card.clickable"),
                    onClick = { clicks += 1 },
                ) {
                    Text("tap me")
                }
            }
        }
        compose.waitForIdle()

        compose.onNodeWithTag("crashcatcher.card.clickable").performClick()

        compose.runOnIdle {
            assertEquals(1, clicks)
        }
    }

    @Test
    fun rendersInBothInterfaceStyles() {
        compose.setContent {
            Column {
                TestManagerTheme(appearance = defaultTestAppearance(uiMode = UiMode.Miuix)) {
                    TonalCard(modifier = Modifier.testTag("crashcatcher.card.miuix")) {
                        Text("miuix")
                    }
                }
                TestManagerTheme(appearance = defaultTestAppearance(uiMode = UiMode.Material)) {
                    TonalCard(modifier = Modifier.testTag("crashcatcher.card.material")) {
                        Text("material")
                    }
                }
            }
        }
        compose.waitForIdle()

        compose.onNodeWithTag("crashcatcher.card.miuix").assertIsDisplayed()
        compose.onNodeWithTag("crashcatcher.card.material").assertIsDisplayed()
    }

    /**
     * A grouped card leaves room below every item but the last, which is how adjacent
     * cards read as one group. If the gap were applied to the last item too, a list
     * would end with a stray strip of scaffold background.
     */
    @Test
    fun groupedCardsReserveSpacingForEveryItemButTheLast() {
        compose.setContent {
            TestManagerTheme {
                Column {
                    TonalCard(
                        modifier = Modifier.testTag("crashcatcher.card.group.first"),
                        index = 0,
                        count = 2,
                    ) {
                        Text("first")
                    }
                    TonalCard(
                        modifier = Modifier.testTag("crashcatcher.card.group.last"),
                        index = 1,
                        count = 2,
                    ) {
                        Text("last")
                    }
                }
            }
        }
        compose.waitForIdle()

        val first = compose.onNodeWithTag("crashcatcher.card.group.first")
            .getUnclippedBoundsInRoot()
        val last = compose.onNodeWithTag("crashcatcher.card.group.last")
            .getUnclippedBoundsInRoot()

        assertTrue(
            "a non-final grouped card must be taller than the final one by its group gap",
            (first.bottom - first.top) > (last.bottom - last.top),
        )
    }
}
