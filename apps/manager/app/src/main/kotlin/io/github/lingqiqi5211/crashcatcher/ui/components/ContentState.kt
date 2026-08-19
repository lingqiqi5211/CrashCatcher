package io.github.lingqiqi5211.crashcatcher.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.progressSemantics
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import io.github.lingqiqi5211.crashcatcher.R
import io.github.lingqiqi5211.meowui.theme.MeowIcons
import io.github.lingqiqi5211.meowui.theme.MeowTheme
import androidx.compose.ui.BiasAlignment

/**
 * Where a full-page state sits vertically, and how much room it leaves below itself.
 *
 * Two corrections over a plain centre, both of which showed up as "the empty page sits
 * too low":
 *
 * - The box these fill runs to the bottom of the window, *under* the navigation bar,
 *   because the shell publishes that inset for scroll containers to consume rather than
 *   applying it as layout padding. Centring in the raw box therefore parks the block
 *   half a navigation bar too low. [stateInsetPadding] takes it back.
 * - What is left after the page's pinned chrome is the space *below the search bar*, not
 *   the page. Centring in that leftover box still reads low, because the eye judges
 *   against the whole screen; the bias lifts the block to roughly the screen's optical
 *   centre.
 */
private val StateVerticalAlignment = BiasAlignment.Vertical(-0.2f)

private val stateInsetPadding: Modifier
    @Composable
    get() = Modifier.padding(bottom = LocalCrashCatcherContentBottomPadding.current)

/**
 * A centred busy indicator for a content region that is still loading.
 *
 * The progress semantics live on the wrapper and the indicator itself is cleared, so
 * assistive technology announces one busy region with a readable description instead
 * of an unlabelled spinner followed by a second, silent progress node.
 *
 * [testTag] is required rather than optional: a spinner has no text to find it by, so
 * without a tag a test can only assert that *some* progress node exists somewhere.
 */
@Composable
internal fun CrashCatcherLoadingState(
    testTag: String,
    modifier: Modifier = Modifier,
    description: String = stringResource(R.string.loading),
) {
    Box(
        modifier = modifier
            .fillMaxWidth()
            .then(stateInsetPadding)
            .testTag(testTag)
            .progressSemantics()
            .semantics { contentDescription = description },
        contentAlignment = BiasAlignment(horizontalBias = 0f, verticalBias = -0.2f),
    ) {
        CrashCatcherCircularProgressIndicator(
            modifier = Modifier
                .size(40.dp)
                .clearAndSetSemantics {},
        )
    }
}

/**
 * The empty state for a list that loaded successfully but has nothing to show.
 *
 * For this app that is the good outcome — no crashes recorded — so the wording stays
 * the caller's business and the default is neutral rather than apologetic.
 *
 * MeowUI has no empty-state component, so the layout is this app's; both the typography
 * and the tonal badge come from MeowUI tokens, which means the badge is painted from the
 * active style's own palette instead of Material's containers under a Miuix skin.
 */
@Composable
internal fun CrashCatcherEmptyState(
    testTag: String,
    modifier: Modifier = Modifier,
    title: String = stringResource(R.string.content_empty_title),
    description: String = stringResource(R.string.content_empty_description),
    icon: ImageVector = MeowIcons.Empty,
) {
    Column(
        modifier = modifier
            .fillMaxWidth()
            .then(stateInsetPadding)
            .padding(horizontal = 24.dp, vertical = 20.dp)
            .testTag(testTag),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(12.dp, StateVerticalAlignment),
    ) {
        Box(
            modifier = Modifier
                .size(72.dp)
                .background(MeowTheme.colors.secondaryContainer, CircleShape),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                imageVector = icon,
                // Decorative: the title below already says what the state is, and a
                // second announcement would read the empty region out twice.
                contentDescription = null,
                modifier = Modifier.size(32.dp),
                tint = MeowTheme.colors.onSecondaryContainer,
            )
        }
        Text(
            text = title,
            color = MeowTheme.colors.onSurface,
            style = MeowTheme.typography.sectionTitle,
            fontWeight = FontWeight.SemiBold,
            textAlign = TextAlign.Center,
        )
        Text(
            text = description,
            color = MeowTheme.colors.onSurfaceVariant,
            style = MeowTheme.typography.summary,
            textAlign = TextAlign.Center,
        )
    }
}

/**
 * The state for a list that could not load.
 *
 * A full-page state rather than a banner above an empty page: when there is nothing
 * to show, a banner plus a screen of blank space reads as two problems instead of
 * one, and buries the retry the user actually needs. A banner is right only when
 * there *is* data underneath it and the failure is a stale refresh.
 *
 * Retry is the whole block, not a button inside it. A filled button is the heaviest
 * thing this app draws, and on an otherwise empty page it dominates a surface whose
 * entire message is "there is nothing here" — while every pixel around it, which the
 * user is already pointing at, does nothing. Making the state itself the target keeps
 * the action and drops the chrome, and matches how the overview's banner already
 * behaves.
 */
@Composable
internal fun CrashCatcherErrorState(
    testTag: String,
    title: String,
    description: String,
    onRetry: (() -> Unit)?,
    modifier: Modifier = Modifier,
    icon: ImageVector = MeowIcons.Offline,
) {
    val retryLabel = stringResource(R.string.action_retry)

    Column(
        modifier = modifier
            .fillMaxWidth()
            .then(stateInsetPadding)
            .then(
                if (onRetry == null) {
                    Modifier
                } else {
                    // No ripple: the target is a whole page of mostly empty space, and a
                    // ripple spreading across all of it reads as the page itself
                    // flashing rather than as a control responding.
                    Modifier
                        .clickable(
                            interactionSource = null,
                            indication = null,
                            onClickLabel = retryLabel,
                            onClick = onRetry,
                        )
                        .testTag("$testTag.retry")
                },
            )
            .padding(horizontal = 24.dp, vertical = 20.dp)
            .testTag(testTag),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(12.dp, StateVerticalAlignment),
    ) {
        Box(
            modifier = Modifier
                .size(72.dp)
                .background(MeowTheme.colors.errorContainer, CircleShape),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                imageVector = icon,
                contentDescription = null,
                modifier = Modifier.size(32.dp),
                tint = MeowTheme.colors.onErrorContainer,
            )
        }
        Text(
            text = title,
            color = MeowTheme.colors.onSurface,
            style = MeowTheme.typography.sectionTitle,
            fontWeight = FontWeight.SemiBold,
            textAlign = TextAlign.Center,
        )
        Text(
            text = description,
            color = MeowTheme.colors.onSurfaceVariant,
            style = MeowTheme.typography.summary,
            textAlign = TextAlign.Center,
        )
        if (onRetry != null) {
            // Says the block is tappable, since nothing about a centred illustration
            // otherwise suggests it. Drawn in the accent colour so it reads as the one
            // actionable line rather than a third sentence of explanation.
            Text(
                text = retryLabel,
                color = MeowTheme.colors.primary,
                style = MeowTheme.typography.summary,
                fontWeight = FontWeight.Medium,
                textAlign = TextAlign.Center,
            )
        }
    }
}
