package io.github.lingqiqi5211.crashcatcher.ui.theme

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb

/**
 * Brand seed colour, used below Android 12 and whenever the stored key colour is
 * the "follow the system" sentinel but system Monet is unavailable.
 *
 * A warm ember amber: the tool is a crash and ANR viewer, so the palette should
 * read as diagnostics rather than alarm. A pure red seed would make every surface
 * look like an error state and leave no headroom for the genuine error role, while
 * amber keeps a clear distance from `error` and still derives a legible primary in
 * both light and dark after TonalSpot expansion.
 */
internal val BrandSeedEmber: Color = Color(0xFFE2703A)

/**
 * Preset seed colours offered as a quick picker.
 *
 * Stored as ARGB Ints, which is both what DataStore holds and what the appearance
 * setter takes, so no conversion happens between the picker and storage.
 *
 * Order, hues and count follow the Material Design 500-shade row — red, pink,
 * purple, deep purple, indigo, blue, cyan, teal, green, yellow, amber, orange,
 * brown, blue grey — with the brand ember appended so the app's own colour is
 * reachable in one tap after the user has wandered off it.
 */
internal val keyColorOptions: List<Int> = listOf(
    Color(0xFFF44336).toArgb(),
    Color(0xFFE91E63).toArgb(),
    Color(0xFF9C27B0).toArgb(),
    Color(0xFF673AB7).toArgb(),
    Color(0xFF3F51B5).toArgb(),
    Color(0xFF2196F3).toArgb(),
    Color(0xFF00BCD4).toArgb(),
    Color(0xFF009688).toArgb(),
    Color(0xFF4CAF50).toArgb(),
    Color(0xFFFFEB3B).toArgb(),
    Color(0xFFFFC107).toArgb(),
    Color(0xFFFF9800).toArgb(),
    Color(0xFF795548).toArgb(),
    Color(0xFF607D8B).toArgb(),
    BrandSeedEmber.toArgb(),
)
