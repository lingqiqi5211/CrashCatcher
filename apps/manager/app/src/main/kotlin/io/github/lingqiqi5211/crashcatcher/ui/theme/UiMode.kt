package io.github.lingqiqi5211.crashcatcher.ui.theme

import androidx.compose.runtime.Composable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.runtime.staticCompositionLocalOf
import io.github.lingqiqi5211.meowui.core.MeowUiStyle
import io.github.lingqiqi5211.meowui.theme.MeowTheme

/**
 * Whether the active MeowUI style is Miuix.
 *
 * The app keeps no UI-mode composition local of its own: the style is owned by
 * [MeowTheme] and read back through `MeowTheme.style`. This helper exists only for
 * the app's own custom surfaces, which still have to pick between a
 * squircle-flavoured and a Material-flavoured layout in the handful of places
 * MeowUI has no component for.
 */
@Composable
@ReadOnlyComposable
internal fun isMiuixStyle(): Boolean = MeowTheme.style == MeowUiStyle.Miuix

/**
 * Whether the navigation bar should use MeowUI's floating (capsule) style.
 *
 * This stays an app-side preference because
 * [io.github.lingqiqi5211.meowui.theme.MeowAppearance] does not model bottom-bar
 * style; the shell reads it to choose a
 * [io.github.lingqiqi5211.meowui.component.MeowNavigationBarStyle].
 */
internal val LocalCrashCatcherFloatingNavigationBar = staticCompositionLocalOf { false }

/**
 * Whether the user wants predictive back previews.
 *
 * `MeowAppearance.predictiveBackEnabled` is a stored preference that MeowUI
 * deliberately does not act on: the navigation layer owns back handling, so it has
 * to read the preference and decide. Back handling reads it from here to choose
 * between a predictive handler and a plain one.
 */
internal val LocalCrashCatcherPredictiveBack = staticCompositionLocalOf { true }
