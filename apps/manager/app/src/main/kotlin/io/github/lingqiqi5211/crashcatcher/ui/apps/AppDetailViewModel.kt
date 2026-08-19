package io.github.lingqiqi5211.crashcatcher.ui.apps

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.github.lingqiqi5211.crashcatcher.data.daemon.AppConfig
import io.github.lingqiqi5211.crashcatcher.data.daemon.AppConfigPatch
import io.github.lingqiqi5211.crashcatcher.data.daemon.CrashFilter
import io.github.lingqiqi5211.crashcatcher.data.daemon.GroupSummary
import io.github.lingqiqi5211.crashcatcher.data.daemon.MuteScope
import io.github.lingqiqi5211.crashcatcher.data.daemon.NotifyMode
import io.github.lingqiqi5211.crashcatcher.data.daemon.NotifyModeChange
import io.github.lingqiqi5211.crashcatcher.data.daemon.SortKey
import io.github.lingqiqi5211.crashcatcher.data.daemon.domainError
import io.github.lingqiqi5211.crashcatcher.domain.model.DomainError
import io.github.lingqiqi5211.crashcatcher.domain.repository.AppInventoryRepository
import io.github.lingqiqi5211.crashcatcher.ui.util.couldBePackageName
import io.github.lingqiqi5211.crashcatcher.domain.repository.ConfigRepository
import io.github.lingqiqi5211.crashcatcher.domain.repository.CrashRepository
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/**
 * How an app's notify mode is set, including the "no override" state.
 *
 * A plain [NotifyMode] cannot express it: following the global setting is a fifth
 * option, not the absence of one, and collapsing the two is what makes a per-app
 * screen silently pin whatever the global value happened to be when it opened.
 */
internal enum class AppNotifyChoice {
    FollowGlobal,
    Dialog,
    Notification,
    Toast,
    Nothing;

    fun toChange(): NotifyModeChange = when (this) {
        FollowGlobal -> NotifyModeChange.FollowGlobal
        Dialog -> NotifyModeChange.SetTo(NotifyMode.Dialog)
        Notification -> NotifyModeChange.SetTo(NotifyMode.Notification)
        Toast -> NotifyModeChange.SetTo(NotifyMode.Toast)
        Nothing -> NotifyModeChange.SetTo(NotifyMode.Nothing)
    }

    companion object {
        fun from(mode: NotifyMode?): AppNotifyChoice = when (mode) {
            null -> FollowGlobal
            NotifyMode.Dialog -> Dialog
            NotifyMode.Notification -> Notification
            NotifyMode.Toast -> Toast
            NotifyMode.Nothing -> Nothing
        }
    }
}

internal data class AppDetailUiState(
    val packageName: String = "",
    val config: AppConfig = AppConfig(),
    val groups: List<GroupSummary> = emptyList(),
    val isLoading: Boolean = true,
    val error: DomainError? = null,
) {
    val notifyChoice: AppNotifyChoice get() = AppNotifyChoice.from(config.notifyMode)

    /**
     * Whether this page is about a platform process rather than an app.
     *
     * The daemon's verdict where a row has arrived, since the route carries only a name; the
     * name's own shape until then, so the page does not open as an app and rearrange itself a
     * moment later.
     */
    val isPlatformProcess: Boolean
        get() = groups.firstOrNull()
            ?.let { !it.packageInstalled }
            ?: !couldBePackageName(packageName)
}

internal class AppDetailViewModel(
    private val config: ConfigRepository,
    private val crashes: CrashRepository,
    private val apps: AppInventoryRepository,
) : ViewModel() {

    private val state = MutableStateFlow(AppDetailUiState())
    val uiState: StateFlow<AppDetailUiState> = state.asStateFlow()

    private var loaded: String? = null

    fun load(packageName: String) {
        if (loaded == packageName) return
        loaded = packageName
        state.value = AppDetailUiState(packageName = packageName)

        viewModelScope.launch {
            config.appConfig(packageName)
                .onSuccess { appConfig -> state.update { it.copy(config = appConfig) } }
                .onFailure { cause -> state.update { it.copy(error = cause.domainError) } }

            crashes.listGroups(
                filter = CrashFilter(
                    packages = listOf(packageName),
                    // The user opened this entry deliberately, so its own crashes show
                    // regardless of the list's filters. Both of them: without the second, a
                    // platform process's page came up with no history at all — the very rows
                    // whose count the user tapped on.
                    includeSystemApps = true,
                    includeSystemProcesses = true,
                ),
                sort = SortKey.LastSeenDesc,
                cursor = null,
                limit = GROUP_LIMIT,
            ).onSuccess { page ->
                state.update { it.copy(groups = page.items, isLoading = false) }
            }.onFailure { cause ->
                state.update { it.copy(isLoading = false, error = cause.domainError) }
            }
        }
    }

    fun onNotifyChoiceChange(choice: AppNotifyChoice) =
        patch(AppConfigPatch(notifyMode = choice.toChange()))

    fun onIgnoreChange(ignore: Boolean) = patch(AppConfigPatch(ignore = ignore))

    fun onMuteChange(scope: MuteScope) {
        val packageName = state.value.packageName.takeIf { it.isNotEmpty() } ?: return
        viewModelScope.launch {
            config.mute(packageName, scope)
                .onSuccess { patch(AppConfigPatch(mute = scope)) }
                .onFailure { cause -> state.update { it.copy(error = cause.domainError) } }
        }
    }

    fun onReopen() {
        val packageName = state.value.packageName.takeIf { it.isNotEmpty() } ?: return
        viewModelScope.launch { apps.reopen(packageName, userId = 0) }
    }

    private fun patch(patch: AppConfigPatch) {
        val packageName = state.value.packageName.takeIf { it.isNotEmpty() } ?: return
        viewModelScope.launch {
            config.updateAppConfig(packageName, patch)
                .onSuccess { updated -> state.update { it.copy(config = updated, error = null) } }
                .onFailure { cause -> state.update { it.copy(error = cause.domainError) } }
        }
    }

    private companion object {
        const val GROUP_LIMIT = 50
    }
}
