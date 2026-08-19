package io.github.lingqiqi5211.crashcatcher.test

import androidx.compose.runtime.Composable
import io.github.lingqiqi5211.crashcatcher.domain.model.AppearanceSettings
import io.github.lingqiqi5211.crashcatcher.domain.model.ColorMode
import io.github.lingqiqi5211.crashcatcher.domain.model.UiMode
import io.github.lingqiqi5211.crashcatcher.ui.theme.ManagerTheme

/**
 * The appearance a component test composes under.
 *
 * Defaults deliberately differ from the shipped defaults: [ColorMode.LIGHT] rather than
 * `SYSTEM` so the palette does not depend on the Robolectric host's dark-mode
 * qualifier, and [UiMode.Material] rather than `Miuix` so a test that says nothing about
 * style exercises the branch most components have the most layout logic in. A test that
 * cares about either axis states it.
 */
internal fun defaultTestAppearance(
    colorMode: ColorMode = ColorMode.LIGHT,
    keyColorArgb: Int = 0,
    paletteStyleName: String = "TonalSpot",
    colorSpecName: String = "Default",
    uiMode: UiMode = UiMode.Material,
): AppearanceSettings = AppearanceSettings(
    colorMode = colorMode,
    keyColorArgb = keyColorArgb,
    paletteStyleName = paletteStyleName,
    colorSpecName = colorSpecName,
    uiMode = uiMode,
)

/**
 * Composes [content] under the real app theme.
 *
 * Tests go through the production [ManagerTheme] rather than installing MeowUI directly,
 * so anything a component reads — MeowUI tokens, and under Miuix the Material roles
 * `MiuixMaterialBridge` supplies — is present exactly as it is at runtime. A component
 * that only looked right under a hand-built test theme would be a component that is
 * broken in the app.
 */
@Composable
internal fun TestManagerTheme(
    appearance: AppearanceSettings = defaultTestAppearance(),
    content: @Composable () -> Unit,
) {
    ManagerTheme(appearance = appearance, content = content)
}
