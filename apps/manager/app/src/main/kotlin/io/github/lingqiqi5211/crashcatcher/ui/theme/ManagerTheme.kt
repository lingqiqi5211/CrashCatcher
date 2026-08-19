package io.github.lingqiqi5211.crashcatcher.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.movableContentOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.graphics.Color
import com.materialkolor.PaletteStyle
import com.materialkolor.dynamiccolor.ColorSpec
import com.materialkolor.rememberDynamicColorScheme
import io.github.lingqiqi5211.crashcatcher.domain.model.AppearanceSettings
import io.github.lingqiqi5211.meowui.core.MeowUiStyle
import io.github.lingqiqi5211.meowui.theme.MeowTheme

/**
 * Whether the app should render its dark palette for [appearance].
 *
 * Kept as a standalone helper because the activity needs the resolved value before
 * any MeowUI composition exists, to pick the splash/window background. MeowUI
 * performs the same resolution internally for the theme itself, so the two must
 * stay in agreement.
 */
@Composable
@ReadOnlyComposable
internal fun resolveDarkMode(appearance: AppearanceSettings): Boolean =
    appearance.colorMode.isDark ||
        (appearance.colorMode.isSystem && isSystemInDarkTheme())

/**
 * Installs MeowUI as the app's UI layer.
 *
 * All styling, system-bar appearance and interface scaling come from [MeowTheme];
 * the app only projects its persisted settings onto
 * [io.github.lingqiqi5211.meowui.theme.MeowAppearance].
 *
 * MeowUI's Miuix branch deliberately installs only `MiuixTheme`, so plain
 * Material 3 primitives used for the app's own custom surfaces (rows and tags the
 * library has no component for) would otherwise fall back to the Material baseline
 * purple. [MiuixMaterialBridge] supplies a Material colour scheme derived from the
 * active Miuix palette so those primitives stay in the same palette as MeowUI's own
 * components.
 */
@Composable
internal fun ManagerTheme(
    appearance: AppearanceSettings,
    content: @Composable () -> Unit,
) {
    val meowAppearance = remember(appearance) { appearance.toMeowAppearance() }
    // Only the Miuix branch needs the Material bridge, but emitting `content()`
    // directly in one branch and wrapped in the other makes the two structurally
    // different call sites: switching interface style would dispose and rebuild the
    // entire app subtree, losing the pager page, scroll offsets and back stack. A
    // movable content block keeps one instance and relocates it between branches.
    val latestContent by rememberUpdatedState(content)
    val body = remember { movableContentOf { latestContent() } }
    MeowTheme(appearance = meowAppearance) {
        CompositionLocalProvider(
            LocalCrashCatcherFloatingNavigationBar provides appearance.floatingNavigationBar,
            LocalCrashCatcherPredictiveBack provides appearance.predictiveBackEnabled,
        ) {
            when (MeowTheme.style) {
                MeowUiStyle.MaterialExpressive -> body()
                MeowUiStyle.Miuix -> MiuixMaterialBridge(appearance = appearance) { body() }
            }
        }
    }
}

/**
 * Re-derives a Material 3 colour scheme from the Miuix palette MeowUI resolved.
 *
 * This is a safety net for Material primitives with no MeowUI equivalent — `Text` and
 * `Icon` are used app-wide in both styles and take their default colour from
 * `LocalContentColor`, which Material 3 defaults to black outside a `Surface`: unreadable
 * on a dark Miuix background. The scheme starts from MaterialKolor with the same seed,
 * palette style and colour spec MeowUI was given, then the roles MeowUI exposes are
 * overlaid so the bridged colours track the live Miuix palette rather than drifting.
 *
 * It is explicitly *not* a colour source for this app's own surfaces. Screens read
 * `MeowTheme.colors`, which maps every role — tonal containers included — onto the active
 * style's own palette; going through here instead handed a Miuix skin Material's tonal
 * containers, which are visibly more tinted than anything around them.
 */
@Composable
private fun MiuixMaterialBridge(
    appearance: AppearanceSettings,
    content: @Composable () -> Unit,
) {
    val meowColors = MeowTheme.colors
    val darkTheme = resolveDarkMode(appearance)
    val paletteStyle = remember(appearance.paletteStyleName) {
        runCatching { PaletteStyle.valueOf(appearance.paletteStyleName) }
            .getOrDefault(PaletteStyle.TonalSpot)
    }
    val colorSpec = remember(appearance.colorSpecName) {
        runCatching { ColorSpec.SpecVersion.valueOf(appearance.colorSpecName) }
            .getOrDefault(ColorSpec.SpecVersion.Default)
    }
    val baseScheme = rememberDynamicColorScheme(
        seedColor = meowColors.primary.takeIf { it != Color.Unspecified } ?: BrandSeedEmber,
        isDark = darkTheme,
        style = paletteStyle,
        specVersion = colorSpec,
    )
    val bridgedScheme = remember(baseScheme, meowColors) {
        baseScheme.copy(
            primary = meowColors.primary,
            onPrimary = meowColors.onPrimary,
            background = meowColors.background,
            onBackground = meowColors.onBackground,
            surface = meowColors.surface,
            onSurface = meowColors.onSurface,
            surfaceVariant = meowColors.surfaceVariant,
            surfaceContainer = meowColors.surfaceVariant,
            surfaceContainerLow = meowColors.surfaceVariant,
            onSurfaceVariant = meowColors.onSurfaceVariant,
            outline = meowColors.outline,
            outlineVariant = meowColors.divider,
            error = meowColors.error,
            onError = meowColors.onError,
        )
    }

    MaterialTheme(colorScheme = bridgedScheme) {
        CompositionLocalProvider(
            LocalContentColor provides meowColors.onBackground,
            content = content,
        )
    }
}
