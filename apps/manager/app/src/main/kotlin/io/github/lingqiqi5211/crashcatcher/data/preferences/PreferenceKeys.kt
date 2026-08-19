package io.github.lingqiqi5211.crashcatcher.data.preferences

import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.floatPreferencesKey
import androidx.datastore.preferences.core.intPreferencesKey
import androidx.datastore.preferences.core.stringPreferencesKey

/**
 * DataStore keys for the persisted appearance.
 *
 * The names are the storage contract: renaming one silently resets that preference
 * for every existing install, so a change of meaning must use a new key and read the
 * old one as a fallback (see how [PreferenceKeys.AMOLED_DARK] falls back to
 * [PreferenceKeys.COLOR_MODE] in `AppearancePreferencesRepository`).
 */
internal object PreferenceKeys {
    val UI_MODE = stringPreferencesKey("ui_mode")
    val COLOR_MODE = intPreferencesKey("color_mode")
    val KEY_COLOR = intPreferencesKey("key_color")
    val COLOR_STYLE = stringPreferencesKey("color_style")
    val COLOR_SPEC = stringPreferencesKey("color_spec")
    val PAGE_SCALE = floatPreferencesKey("page_scale")
    val FLOATING_NAVIGATION_BAR = booleanPreferencesKey("floating_navigation_bar")
    val PREDICTIVE_BACK = booleanPreferencesKey("predictive_back")
    val AMOLED_DARK = booleanPreferencesKey("amoled_dark")
    val BLUR = booleanPreferencesKey("blur_enabled")
}

internal const val APPEARANCE_DATA_STORE_NAME: String = "crashcatcher_appearance"
