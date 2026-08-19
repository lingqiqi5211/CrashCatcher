package io.github.lingqiqi5211.crashcatcher.ui.settings

import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import io.github.lingqiqi5211.crashcatcher.R
import io.github.lingqiqi5211.crashcatcher.domain.model.AppearanceSettings
import io.github.lingqiqi5211.crashcatcher.ui.components.SettingsSection
import io.github.lingqiqi5211.crashcatcher.ui.components.SettingsSwitchRow
import io.github.lingqiqi5211.crashcatcher.ui.theme.mergeFrom
import io.github.lingqiqi5211.crashcatcher.ui.theme.toMeowAppearance
import io.github.lingqiqi5211.meowui.component.MeowAppearanceLabels
import io.github.lingqiqi5211.meowui.component.MeowAppearancePage

/**
 * The appearance page.
 *
 * Delegates the whole surface to MeowUI: theme colour, light/dark, palette style,
 * colour standard, interface style, predictive back and scale are all things the
 * library already models, and re-implementing the page here would mean two
 * definitions of the same settings drifting apart.
 *
 * Only the labels are ours — the library's defaults are English.
 */
@Composable
internal fun AppearanceScreen(
    settings: AppearanceSettings,
    onSettingsChange: (AppearanceSettings) -> Unit,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val meowAppearance = remember(settings) { settings.toMeowAppearance() }

    MeowAppearancePage(
        appearance = meowAppearance,
        // The page hands back a whole MeowAppearance; merging it into our own model
        // keeps the fields MeowUI does not model — the floating bottom bar — from
        // being reset on every edit.
        onAppearanceChange = { changed -> onSettingsChange(settings.mergeFrom(changed)) },
        modifier = modifier,
        onBackClick = onBack,
        labels = appearanceLabels(),
    ) {
        // Two switches the library's page does not offer: the floating bar is this
        // app's own layout choice, and blur is a value MeowUI reads but leaves the
        // host to expose.
        SettingsSection(
            title = stringResource(R.string.appearance_section_chrome),
            testTag = "crashcatcher.appearance.chrome",
        ) {
            SettingsSwitchRow(
                title = stringResource(R.string.appearance_floating_bar),
                description = stringResource(R.string.appearance_floating_bar_summary),
                checked = settings.floatingNavigationBar,
                onCheckedChange = { enabled ->
                    onSettingsChange(settings.copy(floatingNavigationBar = enabled))
                },
                modifier = Modifier.testTag("crashcatcher.appearance.floatingbar"),
            )
            SettingsSwitchRow(
                title = stringResource(R.string.appearance_blur),
                description = stringResource(R.string.appearance_blur_summary),
                checked = settings.blurEnabled,
                onCheckedChange = { enabled ->
                    onSettingsChange(settings.copy(blurEnabled = enabled))
                },
                modifier = Modifier.testTag("crashcatcher.appearance.blur"),
            )
        }
    }
}

@Composable
private fun appearanceLabels() = MeowAppearanceLabels(
    title = stringResource(R.string.settings_section_appearance),
    themeColor = stringResource(R.string.appearance_theme_color),
    themeMode = stringResource(R.string.appearance_theme_mode),
    systemMode = stringResource(R.string.appearance_mode_system),
    lightMode = stringResource(R.string.appearance_mode_light),
    darkMode = stringResource(R.string.appearance_mode_dark),
    amoledDark = stringResource(R.string.appearance_amoled),
    amoledDarkSummary = stringResource(R.string.appearance_amoled_summary),
    colorSettings = stringResource(R.string.appearance_colors),
    paletteStyle = stringResource(R.string.appearance_palette_style),
    colorSpec = stringResource(R.string.appearance_color_spec),
    miuixMonet = stringResource(R.string.appearance_monet),
    miuixMonetSummary = stringResource(R.string.appearance_monet_summary),
    interfaceSettings = stringResource(R.string.appearance_interface),
    interfaceStyle = stringResource(R.string.appearance_interface_style),
    predictiveBack = stringResource(R.string.appearance_predictive_back),
    predictiveBackSummary = stringResource(R.string.appearance_predictive_back_summary),
    interfaceScale = stringResource(R.string.appearance_scale),
    interfaceScaleSummary = stringResource(R.string.appearance_scale_summary),
    customColor = stringResource(R.string.appearance_custom_color),
    dialogConfirm = stringResource(R.string.action_confirm),
    dialogCancel = stringResource(R.string.action_cancel),
)
