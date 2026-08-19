package io.github.lingqiqi5211.crashcatcher.ui.theme

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import io.github.lingqiqi5211.crashcatcher.domain.model.AppearanceSettings
import io.github.lingqiqi5211.crashcatcher.domain.model.ColorMode
import io.github.lingqiqi5211.crashcatcher.domain.model.UiMode
import io.github.lingqiqi5211.meowui.core.MeowUiStyle
import io.github.lingqiqi5211.meowui.theme.MeowAppearance
import io.github.lingqiqi5211.meowui.theme.MeowColorSpec
import io.github.lingqiqi5211.meowui.theme.MeowPaletteStyle
import io.github.lingqiqi5211.meowui.theme.MeowThemeMode

/**
 * Resolved opaque seed colour for the palette generators.
 *
 * [AppearanceSettings.keyColorArgb] is stored as an Int. When the user picks a
 * preset from [keyColorOptions] the leading alpha byte is already `0xFF`; when the
 * value is the sentinel `0` ("use the system dynamic colour"), callers MUST take
 * the dynamic branch instead of reading this, because `Color(0)` is transparent
 * black and no palette generator can seed from it.
 *
 * The alpha byte is forced opaque defensively: a custom colour that somehow lands
 * with `alpha == 0` (a legacy migration, a hand-written test fixture) would
 * otherwise silently produce a black palette.
 */
internal val AppearanceSettings.seedColor: Color
    get() = Color(keyColorArgb or 0xFF000000.toInt())

/**
 * Projection of the app's persisted [AppearanceSettings] onto MeowUI's single
 * controlled appearance state.
 *
 * [AppearanceSettings.colorMode] conflates two axes that MeowUI keeps separate:
 * light/dark following, and whether colours are generated from a key colour at all.
 * The `MONET_*` modes become `miuixMonetEnabled = true`, the plain modes become
 * `false`, and the light/dark axis is projected onto [MeowThemeMode].
 *
 * `keyColorArgb == 0` is the sentinel for "follow the system key colour", which is
 * exactly [MeowAppearance.dynamicColor]. MeowUI falls back to
 * [MeowAppearance.seedColor] below Android 12, so the brand seed is always supplied
 * rather than left at MeowUI's own default.
 *
 * The pure-black overlay is its own persisted flag rather than a colour mode, so it
 * projects straight onto [MeowAppearance.amoledDarkEnabled] and survives being
 * armed while the theme still follows the system.
 */
internal fun AppearanceSettings.toMeowAppearance(): MeowAppearance = MeowAppearance(
    style = uiMode.toMeowUiStyle(),
    themeMode = colorMode.toMeowThemeMode(),
    dynamicColor = keyColorArgb == 0,
    seedColor = if (keyColorArgb == 0) BrandSeedEmber else seedColor,
    paletteStyle = paletteStyleName.toMeowPaletteStyle(),
    colorSpec = colorSpecName.toMeowColorSpec(),
    miuixMonetEnabled = colorMode.isMonet,
    amoledDarkEnabled = amoledDarkEnabled,
    blurEnabled = blurEnabled,
    predictiveBackEnabled = predictiveBackEnabled,
    interfaceScale = pageScale,
)

/**
 * Colour modes offered in the appearance UI.
 *
 * [ColorMode.DARK_AMOLED] is excluded: pure black is a separate switch, so offering
 * it as a mode as well would let the two disagree.
 */
internal val selectableColorModes: List<ColorMode> =
    ColorMode.entries.filterNot(ColorMode::isAmoled)

internal fun UiMode.toMeowUiStyle(): MeowUiStyle = when (this) {
    UiMode.Material -> MeowUiStyle.MaterialExpressive
    UiMode.Miuix -> MeowUiStyle.Miuix
}

internal fun MeowUiStyle.toUiMode(): UiMode = when (this) {
    MeowUiStyle.MaterialExpressive -> UiMode.Material
    MeowUiStyle.Miuix -> UiMode.Miuix
}

private fun ColorMode.toMeowThemeMode(): MeowThemeMode = when {
    isSystem -> MeowThemeMode.System
    isDark -> MeowThemeMode.Dark
    else -> MeowThemeMode.Light
}

/**
 * Resolves the persisted palette style name against MeowUI's enum.
 *
 * The stored name comes from MaterialKolor's `PaletteStyle`, whose entries match
 * [MeowPaletteStyle] by name, so unknown or legacy values fall back to the Material
 * default rather than failing.
 */
internal fun String.toMeowPaletteStyle(): MeowPaletteStyle =
    MeowPaletteStyle.entries.find { it.name == this } ?: MeowPaletteStyle.TonalSpot

/** Reverse of [toMeowPaletteStyle], for writing the preference back. */
internal fun MeowPaletteStyle.toPaletteStyleName(): String = name

/**
 * Resolves the persisted colour spec name against MeowUI's enum.
 *
 * The stored name comes from MaterialKolor's `ColorSpec.SpecVersion`, which uses
 * `SPEC_2021` / `SPEC_2025`, while `Default` is what a fresh install writes and
 * `Spec2021` / `Spec2025` is the spelling MeowUI's own enum uses. All spellings are
 * accepted; anything else falls back to the 2021 spec, which every palette style
 * supports.
 */
internal fun String.toMeowColorSpec(): MeowColorSpec = when (this) {
    "Spec2025", "SPEC_2025" -> MeowColorSpec.Spec2025
    else -> MeowColorSpec.Spec2021
}

internal fun MeowColorSpec.toColorSpecName(): String = when (this) {
    MeowColorSpec.Spec2021 -> "SPEC_2021"
    MeowColorSpec.Spec2025 -> "SPEC_2025"
}

/**
 * Folds a MeowUI appearance change back onto the persisted settings.
 *
 * Only the fields the app stores are projected back. `dynamicColor` and `seedColor`
 * collapse onto the single `keyColorArgb` slot, using the `0` sentinel for "follow
 * the system key colour".
 */
internal fun AppearanceSettings.mergeFrom(appearance: MeowAppearance): AppearanceSettings = copy(
    uiMode = appearance.style.toUiMode(),
    colorMode = resolveColorMode(appearance),
    keyColorArgb = if (appearance.dynamicColor) 0 else appearance.seedColor.toOpaqueArgb(),
    paletteStyleName = appearance.paletteStyle.toPaletteStyleName(),
    colorSpecName = appearance.colorSpec.toColorSpecName(),
    pageScale = appearance.interfaceScale,
    predictiveBackEnabled = appearance.predictiveBackEnabled,
    amoledDarkEnabled = appearance.amoledDarkEnabled,
    blurEnabled = appearance.blurEnabled,
)

/**
 * Recombines MeowUI's separate light/dark and Monet axes into the single
 * [ColorMode].
 *
 * The pure-black overlay is deliberately not folded in here: it has its own
 * persisted flag, so [ColorMode.DARK_AMOLED] is never produced and a mode written
 * by an older build only survives as long as it takes the user to change modes.
 */
private fun resolveColorMode(appearance: MeowAppearance): ColorMode {
    val monet = appearance.miuixMonetEnabled
    return when (appearance.themeMode) {
        MeowThemeMode.System -> if (monet) ColorMode.MONET_SYSTEM else ColorMode.SYSTEM
        MeowThemeMode.Light -> if (monet) ColorMode.MONET_LIGHT else ColorMode.LIGHT
        MeowThemeMode.Dark -> if (monet) ColorMode.MONET_DARK else ColorMode.DARK
    }
}

private fun Color.toOpaqueArgb(): Int = copy(alpha = 1f).toArgb()
