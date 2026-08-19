package io.github.lingqiqi5211.crashcatcher.ui.components

import androidx.activity.compose.BackHandler
import androidx.activity.compose.PredictiveBackHandler
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.tween
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import io.github.lingqiqi5211.crashcatcher.ui.theme.LocalCrashCatcherPredictiveBack
import kotlin.coroutines.cancellation.CancellationException

/**
 * Back handling for a page that can be dismissed, honouring the user's
 * predictive-back preference.
 *
 * MeowUI stores the preference but leaves the decision to the navigation layer, so
 * every place this app handles back goes through this helper: with previews on it
 * registers the gesture-tracking handler and reports progress, with them off it
 * registers a plain handler and the reported progress stays at `0f`. Pair the returned
 * state with [crashCatcherBackPreview] to render the preview.
 *
 * The progress is returned as a [State] rather than a plain `Float` so a caller can
 * read it inside a `graphicsLayer` block: reading it there keeps the gesture on the
 * draw phase instead of recomposing the whole page on every frame of the drag.
 */
@Composable
internal fun rememberCrashCatcherBackProgress(
    enabled: Boolean,
    onBack: () -> Unit,
): State<Float> {
    val progress = remember { Animatable(0f) }
    val latestOnBack by rememberUpdatedState(onBack)
    if (LocalCrashCatcherPredictiveBack.current) {
        PredictiveBackHandler(enabled = enabled) { events ->
            try {
                events.collect { event -> progress.snapTo(event.progress) }
                latestOnBack()
                progress.animateTo(0f, animationSpec = tween(SettleDurationMillis))
            } catch (error: CancellationException) {
                progress.animateTo(0f, animationSpec = tween(SettleDurationMillis))
                throw error
            }
        }
    } else {
        // Turning the preference off mid-gesture would otherwise leave the page parked
        // at whatever offset the last gesture reached.
        LaunchedEffect(Unit) { progress.snapTo(0f) }
        BackHandler(enabled = enabled) { latestOnBack() }
    }
    return progress.asState()
}

/**
 * The predictive-back preview: the page shrinks slightly, slides away from the gesture
 * edge and fades, revealing what sits behind it.
 *
 * Shared so the root pager, settings sub-pages and the crash detail page all move the
 * same way.
 */
internal fun Modifier.crashCatcherBackPreview(progress: State<Float>): Modifier =
    graphicsLayer {
        val value = progress.value
        if (value == 0f) return@graphicsLayer
        translationX = size.width * PreviewTranslationFraction * value
        scaleX = 1f - PreviewScaleReduction * value
        scaleY = 1f - PreviewScaleReduction * value
        alpha = 1f - PreviewAlphaReduction * value
    }

private const val SettleDurationMillis = 180
private const val PreviewTranslationFraction = 0.08f
private const val PreviewScaleReduction = 0.03f
private const val PreviewAlphaReduction = 0.08f
