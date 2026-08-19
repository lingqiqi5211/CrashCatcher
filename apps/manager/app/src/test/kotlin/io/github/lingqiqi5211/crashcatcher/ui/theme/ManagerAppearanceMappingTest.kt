package io.github.lingqiqi5211.crashcatcher.ui.theme

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import io.github.lingqiqi5211.crashcatcher.domain.model.AppearanceSettings
import io.github.lingqiqi5211.crashcatcher.domain.model.ColorMode
import io.github.lingqiqi5211.crashcatcher.domain.model.UiMode
import io.github.lingqiqi5211.meowui.core.MeowUiStyle
import io.github.lingqiqi5211.meowui.theme.MeowColorSpec
import io.github.lingqiqi5211.meowui.theme.MeowPaletteStyle
import io.github.lingqiqi5211.meowui.theme.MeowThemeMode
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The projection between the app's persisted appearance and MeowUI's controlled
 * appearance state.
 *
 * This is where the two models disagree and therefore where mistakes hide:
 * [ColorMode] conflates the light/dark axis with whether colours are generated at all,
 * while MeowUI keeps `themeMode` and `miuixMonetEnabled` separate; and
 * `keyColorArgb == 0` is this app's sentinel for MeowUI's `dynamicColor`.
 *
 * Pure Kotlin — no Robolectric — because the mapping touches no Android APIs and a
 * mapping bug should fail in milliseconds.
 */
class ManagerAppearanceMappingTest {

    private fun settings(
        colorMode: ColorMode = ColorMode.SYSTEM,
        keyColorArgb: Int = 0,
        paletteStyleName: String = "TonalSpot",
        colorSpecName: String = "Default",
        uiMode: UiMode = UiMode.Miuix,
        pageScale: Float = 1f,
        floatingNavigationBar: Boolean = false,
        predictiveBackEnabled: Boolean = true,
        amoledDarkEnabled: Boolean = false,
        blurEnabled: Boolean = true,
    ) = AppearanceSettings(
        colorMode = colorMode,
        keyColorArgb = keyColorArgb,
        paletteStyleName = paletteStyleName,
        colorSpecName = colorSpecName,
        uiMode = uiMode,
        pageScale = pageScale,
        floatingNavigationBar = floatingNavigationBar,
        predictiveBackEnabled = predictiveBackEnabled,
        amoledDarkEnabled = amoledDarkEnabled,
        blurEnabled = blurEnabled,
    )

    @Test
    fun plainColorModesDisableMonetAndProjectTheLightDarkAxis() {
        assertEquals(
            MeowThemeMode.System,
            settings(colorMode = ColorMode.SYSTEM).toMeowAppearance().themeMode,
        )
        assertEquals(
            MeowThemeMode.Light,
            settings(colorMode = ColorMode.LIGHT).toMeowAppearance().themeMode,
        )
        assertEquals(
            MeowThemeMode.Dark,
            settings(colorMode = ColorMode.DARK).toMeowAppearance().themeMode,
        )
        listOf(ColorMode.SYSTEM, ColorMode.LIGHT, ColorMode.DARK).forEach { mode ->
            assertFalse(
                "$mode must not enable Monet",
                settings(colorMode = mode).toMeowAppearance().miuixMonetEnabled,
            )
        }
    }

    @Test
    fun monetColorModesEnableMonetAndProjectTheLightDarkAxis() {
        assertEquals(
            MeowThemeMode.System,
            settings(colorMode = ColorMode.MONET_SYSTEM).toMeowAppearance().themeMode,
        )
        assertEquals(
            MeowThemeMode.Light,
            settings(colorMode = ColorMode.MONET_LIGHT).toMeowAppearance().themeMode,
        )
        assertEquals(
            MeowThemeMode.Dark,
            settings(colorMode = ColorMode.MONET_DARK).toMeowAppearance().themeMode,
        )
        listOf(
            ColorMode.MONET_SYSTEM,
            ColorMode.MONET_LIGHT,
            ColorMode.MONET_DARK,
        ).forEach { mode ->
            assertTrue(
                "$mode must enable Monet",
                settings(colorMode = mode).toMeowAppearance().miuixMonetEnabled,
            )
        }
    }

    @Test
    fun theOverlayFlagProjectsIndependentlyOfTheThemeMode() {
        listOf(ColorMode.SYSTEM, ColorMode.LIGHT, ColorMode.DARK).forEach { mode ->
            val projected = settings(colorMode = mode, amoledDarkEnabled = true)
                .toMeowAppearance()

            assertTrue(
                "$mode must still carry the pure-black overlay to MeowUI",
                projected.amoledDarkEnabled,
            )
        }
        assertFalse(
            "AMOLED is its own switch, so it must not also be offered as a colour mode",
            selectableColorModes.any(ColorMode::isAmoled),
        )
    }

    @Test
    fun plainDarkDoesNotEnableThePureBlackOverlay() {
        listOf(ColorMode.DARK, ColorMode.MONET_DARK).forEach { mode ->
            assertFalse(
                "$mode must not turn on the pure-black overlay",
                settings(colorMode = mode).toMeowAppearance().amoledDarkEnabled,
            )
        }
    }

    @Test
    fun theOverlayCanBeArmedWithoutSelectingDark() {
        val stored = settings(colorMode = ColorMode.SYSTEM)

        // The whole point of the separate flag: the user arms pure black while the theme
        // still follows the system, and it takes effect when the system turns dark.
        // Folding MeowUI's state back must not silently drop it.
        val merged = stored.mergeFrom(stored.toMeowAppearance().copy(amoledDarkEnabled = true))

        assertEquals(ColorMode.SYSTEM, merged.colorMode)
        assertTrue("the overlay must survive under a system theme", merged.amoledDarkEnabled)
    }

    @Test
    fun theOverlayFlagSurvivesAnAppearanceRoundTrip() {
        val stored = settings(colorMode = ColorMode.DARK, amoledDarkEnabled = true)

        val merged = stored.mergeFrom(stored.toMeowAppearance())

        assertEquals(
            "folding MeowUI's state back must not downgrade the theme mode",
            ColorMode.DARK,
            merged.colorMode,
        )
        assertTrue("folding MeowUI's state back must not drop AMOLED", merged.amoledDarkEnabled)
    }

    @Test
    fun theKeyColorSentinelMapsOntoDynamicColor() {
        val dynamic = settings(keyColorArgb = 0).toMeowAppearance()
        assertTrue(dynamic.dynamicColor)
        assertEquals(
            "a seed must still be supplied for pre-Android-12 fallback",
            BrandSeedEmber,
            dynamic.seedColor,
        )

        val seeded = settings(keyColorArgb = Color(0xFF2196F3).toArgb()).toMeowAppearance()
        assertFalse(seeded.dynamicColor)
        assertEquals(Color(0xFF2196F3), seeded.seedColor)
    }

    @Test
    fun dynamicColorFoldsBackOntoTheSentinel() {
        val stored = settings(keyColorArgb = Color(0xFF2196F3).toArgb())

        val merged = stored.mergeFrom(stored.toMeowAppearance().copy(dynamicColor = true))

        assertEquals(0, merged.keyColorArgb)
    }

    @Test
    fun aChosenSeedColorFoldsBackAsOpaqueArgb() {
        val stored = settings()

        val merged = stored.mergeFrom(
            stored.toMeowAppearance().copy(
                dynamicColor = false,
                seedColor = Color(0x804CAF50),
            ),
        )

        assertEquals(
            "the stored key colour must stay opaque so the palette can seed from it",
            0xFF,
            merged.keyColorArgb ushr 24 and 0xFF,
        )
    }

    @Test
    fun everyPresetKeyColorIsAUsableOpaqueSeed() {
        keyColorOptions.forEach { argb ->
            assertEquals(
                "a preset must be opaque, or the palette seeds from transparent black",
                0xFF,
                argb ushr 24 and 0xFF,
            )
            assertTrue(
                "0 is the follow-the-system sentinel and must not appear as a preset",
                argb != 0,
            )
        }
    }

    @Test
    fun interfaceStyleMapsBothWays() {
        assertEquals(
            MeowUiStyle.Miuix,
            settings(uiMode = UiMode.Miuix).toMeowAppearance().style,
        )
        assertEquals(
            MeowUiStyle.MaterialExpressive,
            settings(uiMode = UiMode.Material).toMeowAppearance().style,
        )
        assertEquals(UiMode.Miuix, MeowUiStyle.Miuix.toUiMode())
        assertEquals(UiMode.Material, MeowUiStyle.MaterialExpressive.toUiMode())
    }

    @Test
    fun paletteStyleAndColorSpecNamesResolveAndFallBack() {
        assertEquals(MeowPaletteStyle.Vibrant, "Vibrant".toMeowPaletteStyle())
        assertEquals(
            "an unknown stored name must fall back rather than fail",
            MeowPaletteStyle.TonalSpot,
            "NotAStyle".toMeowPaletteStyle(),
        )

        assertEquals(MeowColorSpec.Spec2025, "SPEC_2025".toMeowColorSpec())
        assertEquals(
            "the MeowUI spelling must resolve too",
            MeowColorSpec.Spec2025,
            "Spec2025".toMeowColorSpec(),
        )
        assertEquals(
            "the value a fresh install stores must resolve to the safe spec",
            MeowColorSpec.Spec2021,
            "Default".toMeowColorSpec(),
        )
    }

    @Test
    fun colorSpecNamesRoundTripThroughStorage() {
        MeowColorSpec.entries.forEach { spec ->
            assertEquals(spec, spec.toColorSpecName().toMeowColorSpec())
        }
    }

    @Test
    fun paletteStyleNamesRoundTripThroughStorage() {
        MeowPaletteStyle.entries.forEach { style ->
            assertEquals(style, style.toPaletteStyleName().toMeowPaletteStyle())
        }
    }

    @Test
    fun appOnlyFieldsSurviveTheRoundTrip() {
        val stored = settings(
            pageScale = 0.9f,
            floatingNavigationBar = true,
            predictiveBackEnabled = false,
            blurEnabled = false,
        )

        val projected = stored.toMeowAppearance()
        assertEquals(0.9f, projected.interfaceScale)
        assertFalse(projected.predictiveBackEnabled)
        assertFalse(projected.blurEnabled)

        val merged = stored.mergeFrom(projected)
        assertEquals(0.9f, merged.pageScale)
        assertFalse(merged.predictiveBackEnabled)
        assertFalse(merged.blurEnabled)
        assertTrue(
            "the bottom-bar style is app-only and must not be dropped",
            merged.floatingNavigationBar,
        )
    }
}
