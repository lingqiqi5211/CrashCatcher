package io.github.lingqiqi5211.crashcatcher.ui.components

import androidx.compose.foundation.layout.Column
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.lingqiqi5211.crashcatcher.domain.model.UiMode
import io.github.lingqiqi5211.crashcatcher.test.TestManagerTheme
import io.github.lingqiqi5211.crashcatcher.test.defaultTestAppearance
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * Structural coverage for the loading and empty states:
 *  - the loading region carries indeterminate progress semantics *and* a description
 *    on one node, which is what lets assistive technology announce a single busy
 *    region instead of an unlabelled spinner,
 *  - the spinner's own semantics are cleared, so the region is not announced twice,
 *  - both states render in both interface styles, since the spinner is a
 *    style-dispatched primitive.
 *
 * Pinned to SDK 29 for deterministic theme tokens.
 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [29])
@GraphicsMode(GraphicsMode.Mode.LEGACY)
class ContentStateTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun loadingStateMergesDescriptionWithProgressSemantics() {
        compose.setContent {
            TestManagerTheme {
                CrashCatcherLoadingState(
                    testTag = "crashcatcher.state.loading",
                    description = "loading crashes",
                )
            }
        }
        compose.waitForIdle()

        compose.onNodeWithTag("crashcatcher.state.loading")
            .assert(
                SemanticsMatcher.expectValue(
                    SemanticsProperties.ProgressBarRangeInfo,
                    ProgressBarRangeInfo.Indeterminate,
                ),
            )
            .assert(
                SemanticsMatcher.expectValue(
                    SemanticsProperties.ContentDescription,
                    listOf("loading crashes"),
                ),
            )
    }

    /**
     * The indicator inside the busy region has `clearAndSetSemantics {}`, so the whole
     * region owns exactly one progress node. Two would make screen readers announce the
     * same wait twice, once without a label.
     */
    @Test
    fun theBusyRegionOwnsASingleProgressNode() {
        compose.setContent {
            TestManagerTheme {
                CrashCatcherLoadingState(testTag = "crashcatcher.state.loading.single")
            }
        }
        compose.waitForIdle()

        val progressNodes = compose
            .onAllNodes(
                SemanticsMatcher.expectValue(
                    SemanticsProperties.ProgressBarRangeInfo,
                    ProgressBarRangeInfo.Indeterminate,
                ),
                useUnmergedTree = true,
            )
            .fetchSemanticsNodes()

        assertEquals(
            "the busy region must expose exactly one progress node",
            1,
            progressNodes.size,
        )
    }

    @Test
    fun loadingStateRendersInBothInterfaceStyles() {
        compose.setContent {
            Column {
                TestManagerTheme(appearance = defaultTestAppearance(uiMode = UiMode.Miuix)) {
                    CrashCatcherLoadingState(testTag = "crashcatcher.state.loading.miuix")
                }
                TestManagerTheme(appearance = defaultTestAppearance(uiMode = UiMode.Material)) {
                    CrashCatcherLoadingState(testTag = "crashcatcher.state.loading.material")
                }
            }
        }
        compose.waitForIdle()

        compose.onNodeWithTag("crashcatcher.state.loading.miuix").assertIsDisplayed()
        compose.onNodeWithTag("crashcatcher.state.loading.material").assertIsDisplayed()
    }

    @Test
    fun emptyStateRendersTitleAndDescription() {
        compose.setContent {
            TestManagerTheme {
                CrashCatcherEmptyState(
                    testTag = "crashcatcher.state.empty",
                    title = "no crashes",
                    description = "capture is running",
                )
            }
        }
        compose.waitForIdle()

        compose.onNodeWithTag("crashcatcher.state.empty").assertIsDisplayed()
    }

    @Test
    fun emptyStateFallsBackToItsOwnCopy() {
        compose.setContent {
            TestManagerTheme {
                CrashCatcherEmptyState(testTag = "crashcatcher.state.empty.default")
            }
        }
        compose.waitForIdle()

        // No text assertion: the copy is localized, so the test only proves the state
        // composes without a caller-supplied title or description.
        compose.onNodeWithTag("crashcatcher.state.empty.default").assertIsDisplayed()
    }

    @Test
    fun emptyStateRendersInBothInterfaceStyles() {
        compose.setContent {
            Column {
                TestManagerTheme(appearance = defaultTestAppearance(uiMode = UiMode.Miuix)) {
                    CrashCatcherEmptyState(testTag = "crashcatcher.state.empty.miuix")
                }
                TestManagerTheme(appearance = defaultTestAppearance(uiMode = UiMode.Material)) {
                    CrashCatcherEmptyState(testTag = "crashcatcher.state.empty.material")
                }
            }
        }
        compose.waitForIdle()

        compose.onNodeWithTag("crashcatcher.state.empty.miuix").assertIsDisplayed()
        compose.onNodeWithTag("crashcatcher.state.empty.material").assertIsDisplayed()
    }
}
