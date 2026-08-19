package io.github.lingqiqi5211.crashcatcher.ui.components

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import io.github.lingqiqi5211.meowui.component.MeowTip
import io.github.lingqiqi5211.meowui.component.MeowTipStyle

/**
 * An inline notice: a daemon that cannot be reached, a collector the kernel refused,
 * a payload that was evicted before it could be read.
 *
 * Backed by [MeowTip], so the container, icon and action styling are MeowUI's per
 * style. The app keeps its own name and `title`/`body` parameter pair because the
 * notice is used across every screen and reads better than `message`/`title` at the
 * call site.
 *
 * [icon] defaults to null so MeowUI picks the icon that matches [style]; passing a
 * fixed icon here would make every notice look alike regardless of severity. [style]
 * defaults to [MeowTipStyle.Warning] — the common case — so only genuine failures
 * and purely informational notices have to state one.
 *
 * MeowUI's tip has no disabled-action state, so [actionEnabled] hides the action
 * instead of showing a dead button: an action that cannot run is better omitted than
 * offered, and every call site already knows whether the daemon is writable at all.
 */
@Composable
internal fun WarningCard(
    title: String,
    body: String,
    modifier: Modifier = Modifier,
    icon: ImageVector? = null,
    style: MeowTipStyle = MeowTipStyle.Warning,
    actionLabel: String? = null,
    actionModifier: Modifier = Modifier,
    actionEnabled: Boolean = true,
    onAction: (() -> Unit)? = null,
) {
    val action = onAction?.takeIf { actionEnabled }
    MeowTip(
        message = body,
        modifier = modifier,
        title = title,
        style = style,
        icon = icon,
        actionText = actionLabel?.takeIf { action != null },
        onAction = action,
        actionModifier = actionModifier,
    )
}
