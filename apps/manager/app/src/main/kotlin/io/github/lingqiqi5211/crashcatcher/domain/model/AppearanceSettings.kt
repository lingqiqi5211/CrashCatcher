package io.github.lingqiqi5211.crashcatcher.domain.model

/**
 * How the app resolves light/dark and whether colours are generated at all.
 *
 * The two axes are conflated into one persisted enum because that is what the
 * settings UI offers as a single choice. MeowUI keeps them separate, so
 * [io.github.lingqiqi5211.crashcatcher.ui.theme.toMeowAppearance] splits them again.
 *
 * The integer [value] is the stored form: DataStore holds an Int, and an ordinal
 * would silently change meaning if the entries were ever reordered.
 */
enum class ColorMode(val value: Int) {
    SYSTEM(0),
    LIGHT(1),
    DARK(2),
    MONET_SYSTEM(3),
    MONET_LIGHT(4),
    MONET_DARK(5),

    /**
     * Kept only so preferences written before pure black became its own switch
     * still resolve; never produced by the current UI.
     */
    DARK_AMOLED(6),
    ;

    val isSystem: Boolean get() = value == 0 || value == 3
    val isDark: Boolean get() = value == 2 || value == 5 || value == 6
    val isAmoled: Boolean get() = value == 6
    val isMonet: Boolean get() = value >= 3

    companion object {
        fun fromValue(value: Int): ColorMode = entries.find { it.value == value } ?: SYSTEM
    }
}

/**
 * Which of MeowUI's two native looks the app wears.
 *
 * Stored as a string rather than an ordinal for the same reason as [ColorMode]:
 * the persisted form must not depend on declaration order.
 */
enum class UiMode(val value: String) {
    Miuix("miuix"),
    Material("material"),
    ;

    companion object {
        fun fromValue(value: String): UiMode = entries.find { it.value == value } ?: Miuix
    }
}

/**
 * The app's persisted appearance.
 *
 * This is the whole of what the crash viewer stores about how it looks; every
 * field is projected onto MeowUI's `MeowAppearance` by
 * [io.github.lingqiqi5211.crashcatcher.ui.theme.toMeowAppearance], except the two
 * that MeowUI does not model ([floatingNavigationBar]) or deliberately leaves to
 * the navigation layer ([predictiveBackEnabled]).
 */
data class AppearanceSettings(
    val colorMode: ColorMode,
    /**
     * The seed colour as opaque ARGB, or `0` meaning "follow the system dynamic
     * colour".
     *
     * `0` is a sentinel rather than a real colour: `Color(0)` is transparent
     * black, which no palette generator can seed from, so callers must take the
     * dynamic branch before reading it as a colour.
     */
    val keyColorArgb: Int,
    /** MaterialKolor / MeowUI palette style name, e.g. `TonalSpot`. */
    val paletteStyleName: String,
    /** Material colour specification name, e.g. `SPEC_2021` or `SPEC_2025`. */
    val colorSpecName: String,
    val uiMode: UiMode = UiMode.Miuix,
    /** Interface scale, 0.8–1.1; the system font scale is left untouched. */
    val pageScale: Float = 1f,
    /** Capsule bottom bar instead of the style's standard one. */
    val floatingNavigationBar: Boolean = false,
    val predictiveBackEnabled: Boolean = true,
    /**
     * Pure-black dark backgrounds.
     *
     * Stored on its own axis rather than folded into [colorMode]: the overlay is
     * a property of the dark palette, so the user must be able to arm it while
     * the theme still follows the system and have it take effect when the system
     * turns dark.
     */
    val amoledDarkEnabled: Boolean = false,
    /** Backdrop blur behind floating chrome; needs RuntimeShader (Android 13+). */
    val blurEnabled: Boolean = true,
)
