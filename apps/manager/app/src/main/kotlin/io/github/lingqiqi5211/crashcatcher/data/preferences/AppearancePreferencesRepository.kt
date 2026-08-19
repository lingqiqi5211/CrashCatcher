package io.github.lingqiqi5211.crashcatcher.data.preferences

import android.content.Context
import android.util.Log
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.emptyPreferences
import androidx.datastore.preferences.preferencesDataStore
import io.github.lingqiqi5211.crashcatcher.domain.model.AppearanceSettings
import io.github.lingqiqi5211.crashcatcher.domain.model.ColorMode
import io.github.lingqiqi5211.crashcatcher.domain.model.UiMode
import java.io.IOException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.runBlocking

private val Context.appearanceDataStore: DataStore<Preferences> by preferencesDataStore(
    name = APPEARANCE_DATA_STORE_NAME,
)

private const val LOG_TAG = "AppearancePrefs"
private const val DEFAULT_PALETTE_STYLE = "TonalSpot"
private const val DEFAULT_COLOR_SPEC = "Default"

/**
 * Interface scale bounds.
 *
 * These mirror `MeowAppearanceDefaults.MinInterfaceScale`/`MaxInterfaceScale`, which
 * is the range MeowUI's appearance slider offers and the range `MeowTheme` clamps to.
 * Storing a value outside it would be silently narrowed at render time, so it is
 * clamped at write time instead — and on read too, so a file written by a build with
 * different bounds cannot render at a scale the slider cannot express.
 */
private const val MIN_PAGE_SCALE = 0.8f
private const val MAX_PAGE_SCALE = 1.1f

/** The appearance a fresh install starts from: system light/dark, system key colour. */
internal fun defaultAppearanceSettings(): AppearanceSettings = AppearanceSettings(
    colorMode = ColorMode.SYSTEM,
    keyColorArgb = 0,
    paletteStyleName = DEFAULT_PALETTE_STYLE,
    colorSpecName = DEFAULT_COLOR_SPEC,
)

/**
 * DataStore-backed persistence for [AppearanceSettings].
 *
 * The current value is exposed as a [StateFlow] rather than a plain flow because the
 * theme needs a value synchronously for its first composition, and each writer takes
 * exactly one field: the settings UI edits one row at a time, and a whole-object
 * writer would let a stale copy held by one screen overwrite a field another screen
 * just changed.
 *
 * [dataStoreOverride] exists for tests, which supply a store rooted in a temporary
 * directory instead of the app's data dir.
 */
internal class AppearancePreferencesRepository(
    applicationContext: Context,
    scope: CoroutineScope,
    dataStoreOverride: DataStore<Preferences>? = null,
) {

    private val dataStore: DataStore<Preferences> =
        dataStoreOverride ?: applicationContext.applicationContext.appearanceDataStore

    private val appearanceFlow = dataStore.data
        .catch { error ->
            // A corrupt or unreadable file must not take the UI down with it: the
            // defaults are a usable appearance, and the next write repairs the file.
            if (error is IOException) {
                Log.w(LOG_TAG, "Failed to read appearance preferences; emitting defaults", error)
                emit(emptyPreferences())
            } else {
                throw error
            }
        }
        .map { preferences -> preferences.toAppearanceSettings() }

    /**
     * The stored appearance, read once synchronously so the theme's first frame
     * already wears it.
     *
     * Starting from the defaults instead painted the dynamic palette for a frame
     * before the real colours landed. The preferences file is tiny, so the blocking
     * read costs single-digit milliseconds on the cold path.
     */
    val appearance: StateFlow<AppearanceSettings> = appearanceFlow.stateIn(
        scope = scope,
        started = SharingStarted.Eagerly,
        initialValue = runBlocking { appearanceFlow.first() },
    )

    /**
     * Writes every appearance field in one edit.
     *
     * The appearance page hands back a whole settings object, and applying it field
     * by field would mean one `DataStore` transaction per field — several file
     * rewrites for one slider drag, and a window where the persisted state is a mix
     * of old and new values that the theme would briefly render.
     */
    suspend fun update(settings: AppearanceSettings) {
        dataStore.edit { preferences ->
            preferences[PreferenceKeys.COLOR_MODE] = settings.colorMode.value
            preferences[PreferenceKeys.UI_MODE] = settings.uiMode.value
            preferences[PreferenceKeys.KEY_COLOR] = settings.keyColorArgb
            preferences[PreferenceKeys.COLOR_STYLE] = settings.paletteStyleName
            preferences[PreferenceKeys.COLOR_SPEC] = settings.colorSpecName
            preferences[PreferenceKeys.PAGE_SCALE] =
                settings.pageScale.coerceIn(MIN_PAGE_SCALE, MAX_PAGE_SCALE)
            preferences[PreferenceKeys.FLOATING_NAVIGATION_BAR] = settings.floatingNavigationBar
            preferences[PreferenceKeys.PREDICTIVE_BACK] = settings.predictiveBackEnabled
            preferences[PreferenceKeys.AMOLED_DARK] = settings.amoledDarkEnabled
            preferences[PreferenceKeys.BLUR] = settings.blurEnabled
        }
    }

    suspend fun setColorMode(colorMode: ColorMode) {
        dataStore.edit { preferences ->
            preferences[PreferenceKeys.COLOR_MODE] = colorMode.value
        }
    }

    suspend fun setUiMode(uiMode: UiMode) {
        dataStore.edit { preferences ->
            preferences[PreferenceKeys.UI_MODE] = uiMode.value
        }
    }

    suspend fun setKeyColor(keyColorArgb: Int) {
        dataStore.edit { preferences ->
            preferences[PreferenceKeys.KEY_COLOR] = keyColorArgb
        }
    }

    suspend fun setPaletteStyleName(name: String) {
        dataStore.edit { preferences ->
            preferences[PreferenceKeys.COLOR_STYLE] = name
        }
    }

    suspend fun setColorSpecName(name: String) {
        dataStore.edit { preferences ->
            preferences[PreferenceKeys.COLOR_SPEC] = name
        }
    }

    suspend fun setPageScale(scale: Float) {
        dataStore.edit { preferences ->
            preferences[PreferenceKeys.PAGE_SCALE] = scale.coerceIn(MIN_PAGE_SCALE, MAX_PAGE_SCALE)
        }
    }

    suspend fun setFloatingNavigationBar(enabled: Boolean) {
        dataStore.edit { preferences ->
            preferences[PreferenceKeys.FLOATING_NAVIGATION_BAR] = enabled
        }
    }

    suspend fun setPredictiveBackEnabled(enabled: Boolean) {
        dataStore.edit { preferences ->
            preferences[PreferenceKeys.PREDICTIVE_BACK] = enabled
        }
    }

    suspend fun setAmoledDarkEnabled(enabled: Boolean) {
        dataStore.edit { preferences ->
            preferences[PreferenceKeys.AMOLED_DARK] = enabled
        }
    }

    suspend fun setBlurEnabled(enabled: Boolean) {
        dataStore.edit { preferences ->
            preferences[PreferenceKeys.BLUR] = enabled
        }
    }
}

private fun Preferences.toAppearanceSettings(): AppearanceSettings = AppearanceSettings(
    colorMode = ColorMode.fromValue(this[PreferenceKeys.COLOR_MODE] ?: ColorMode.SYSTEM.value),
    keyColorArgb = this[PreferenceKeys.KEY_COLOR] ?: 0,
    paletteStyleName = this[PreferenceKeys.COLOR_STYLE] ?: DEFAULT_PALETTE_STYLE,
    colorSpecName = this[PreferenceKeys.COLOR_SPEC] ?: DEFAULT_COLOR_SPEC,
    uiMode = UiMode.fromValue(this[PreferenceKeys.UI_MODE] ?: UiMode.Miuix.value),
    pageScale = (this[PreferenceKeys.PAGE_SCALE] ?: 1f).coerceIn(MIN_PAGE_SCALE, MAX_PAGE_SCALE),
    floatingNavigationBar = this[PreferenceKeys.FLOATING_NAVIGATION_BAR] ?: false,
    predictiveBackEnabled = this[PreferenceKeys.PREDICTIVE_BACK] ?: true,
    // Builds before AMOLED became its own axis stored it as a colour mode, so a
    // stored DARK_AMOLED still arms the overlay while the dedicated key is absent.
    amoledDarkEnabled = this[PreferenceKeys.AMOLED_DARK]
        ?: (this[PreferenceKeys.COLOR_MODE] == ColorMode.DARK_AMOLED.value),
    blurEnabled = this[PreferenceKeys.BLUR] ?: true,
)
