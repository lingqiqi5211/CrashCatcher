package io.github.lingqiqi5211.crashcatcher.ui.components

import androidx.compose.foundation.layout.Column
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.lingqiqi5211.crashcatcher.domain.model.UiMode
import io.github.lingqiqi5211.crashcatcher.test.TestManagerTheme
import io.github.lingqiqi5211.crashcatcher.test.defaultTestAppearance
import kotlin.math.abs
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * Structural coverage for [StatusTag]:
 *  - every tone renders and stays selectable through the tag's test tag, so a tone
 *    added later cannot silently fail to resolve its colour pair,
 *  - the two styles agree on the tag's height, because a tag sits inline in a crash
 *    list row and a height change would shift that row's layout when the user
 *    switches interface style.
 *
 * Pinned to SDK 29 for deterministic theme tokens.
 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [29])
@GraphicsMode(GraphicsMode.Mode.LEGACY)
class StatusTagTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun everyToneRenders() {
        compose.setContent {
            TestManagerTheme {
                Column {
                    StatusTagTone.entries.forEach { tone ->
                        StatusTag(
                            text = tone.name,
                            tone = tone,
                            modifier = Modifier.testTag("crashcatcher.tag.${tone.name}"),
                        )
                    }
                }
            }
        }
        compose.waitForIdle()

        StatusTagTone.entries.forEach { tone ->
            compose.onNodeWithTag("crashcatcher.tag.${tone.name}").assertIsDisplayed()
        }
    }

    @Test
    fun everyToneRendersUnderMiuix() {
        compose.setContent {
            TestManagerTheme(appearance = defaultTestAppearance(uiMode = UiMode.Miuix)) {
                Column {
                    StatusTagTone.entries.forEach { tone ->
                        StatusTag(
                            text = tone.name,
                            tone = tone,
                            modifier = Modifier.testTag("crashcatcher.tag.miuix.${tone.name}"),
                        )
                    }
                }
            }
        }
        compose.waitForIdle()

        StatusTagTone.entries.forEach { tone ->
            compose.onNodeWithTag("crashcatcher.tag.miuix.${tone.name}").assertIsDisplayed()
        }
    }

    /**
     * The active style comes from `MeowTheme`, which owns its own composition local, so
     * the two variants are compared by nesting one themed subtree per style rather than
     * by overriding an app-side local mid-tree.
     */
    @Test
    fun bothStylesAgreeOnTagHeight() {
        compose.setContent {
            Column {
                TestManagerTheme(appearance = defaultTestAppearance(uiMode = UiMode.Miuix)) {
                    StatusTag(
                        text = "ANR",
                        tone = StatusTagTone.Neutral,
                        modifier = Modifier.testTag("crashcatcher.tag.height.miuix"),
                    )
                }
                TestManagerTheme(appearance = defaultTestAppearance(uiMode = UiMode.Material)) {
                    StatusTag(
                        text = "ANR",
                        tone = StatusTagTone.Neutral,
                        modifier = Modifier.testTag("crashcatcher.tag.height.material"),
                    )
                }
            }
        }
        compose.waitForIdle()

        val miuix = compose.onNodeWithTag("crashcatcher.tag.height.miuix")
            .getUnclippedBoundsInRoot()
        val material = compose.onNodeWithTag("crashcatcher.tag.height.material")
            .getUnclippedBoundsInRoot()
        val difference = abs(
            ((material.bottom - material.top) - (miuix.bottom - miuix.top)).value,
        )

        assertTrue("Tag height differed by $difference dp", difference <= 1f)
    }
}
