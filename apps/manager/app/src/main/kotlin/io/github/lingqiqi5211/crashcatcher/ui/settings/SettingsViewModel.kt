package io.github.lingqiqi5211.crashcatcher.ui.settings

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.github.lingqiqi5211.crashcatcher.data.daemon.DialogTakeoverStatus
import io.github.lingqiqi5211.crashcatcher.data.daemon.GlobalConfig
import io.github.lingqiqi5211.crashcatcher.data.daemon.GlobalConfigPatch
import io.github.lingqiqi5211.crashcatcher.data.daemon.NotifyMode
import io.github.lingqiqi5211.crashcatcher.data.daemon.RetentionPatch
import io.github.lingqiqi5211.crashcatcher.data.daemon.domainError
import io.github.lingqiqi5211.crashcatcher.data.daemon.toReconnectOutcome
import io.github.lingqiqi5211.crashcatcher.domain.model.DomainError
import io.github.lingqiqi5211.crashcatcher.domain.model.LoadState
import io.github.lingqiqi5211.crashcatcher.domain.model.ReconnectOutcome
import io.github.lingqiqi5211.crashcatcher.domain.model.valueOrNull
import io.github.lingqiqi5211.crashcatcher.data.daemon.DeleteTarget
import io.github.lingqiqi5211.crashcatcher.data.daemon.ModuleStatus
import io.github.lingqiqi5211.crashcatcher.domain.repository.ConfigRepository
import io.github.lingqiqi5211.crashcatcher.domain.repository.CrashRepository
import io.github.lingqiqi5211.crashcatcher.domain.repository.ModuleStatusRepository
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch

internal data class SettingsUiState(
    val config: LoadState<GlobalConfig> = LoadState.Loading,
    /**
     * Carried here so the storage and about pages can show live figures without owning
     * a second subscription to the same repository.
     */
    val moduleStatusState: LoadState<ModuleStatus> = LoadState.Loading,
    val dialogTakeover: DialogTakeoverStatus? = null,
    /** Set when the daemon clamped a value the user asked for. */
    val lastAdjustment: String? = null,
    val error: DomainError? = null,
) {
    val value: GlobalConfig? get() = config.valueOrNull

    /** The status where one was read, for pages that treat "unreachable" as a finding. */
    val moduleStatus: ModuleStatus? get() = moduleStatusState.valueOrNull
}

internal class SettingsViewModel(
    private val config: ConfigRepository,
    private val moduleStatus: ModuleStatusRepository,
    private val crashes: CrashRepository,
) : ViewModel() {

    private val local = MutableStateFlow(SettingsUiState())

    private val reconnects = Channel<ReconnectOutcome>(Channel.BUFFERED)

    /** One event per press of the reconnect row; the shell turns it into a message. */
    val reconnectOutcomes: Flow<ReconnectOutcome> = reconnects.receiveAsFlow()

    val uiState: StateFlow<SettingsUiState> = combine(
        config.globalConfig,
        moduleStatus.status,
        local,
    ) { globalConfig, status, extra ->
        extra.copy(
            config = globalConfig,
            moduleStatusState = status,
            dialogTakeover = extra.dialogTakeover ?: status.valueOrNull?.dialogTakeover,
        )
    }.stateIn(viewModelScope, SharingStarted.Eagerly, SettingsUiState())

    init {
        refresh()
    }

    fun refresh() {
        viewModelScope.launch { config.refreshGlobalConfig() }
        viewModelScope.launch { moduleStatus.refresh() }
    }

    fun onCaptureJavaChange(enabled: Boolean) = patch(GlobalConfigPatch(captureJava = enabled))

    fun onCaptureAnrChange(enabled: Boolean) = patch(GlobalConfigPatch(captureAnr = enabled))

    fun onCaptureNativeChange(enabled: Boolean) = patch(GlobalConfigPatch(captureNative = enabled))

    fun onCaptureSelfHandledChange(enabled: Boolean) =
        patch(GlobalConfigPatch(captureSelfHandled = enabled))

    fun onIncludeSystemAppsChange(enabled: Boolean) =
        patch(GlobalConfigPatch(includeSystemApps = enabled))

    fun onDebugLoggingChange(enabled: Boolean) =
        patch(GlobalConfigPatch(debugLogging = enabled))

    fun onOnlyMainProcessChange(enabled: Boolean) =
        patch(GlobalConfigPatch(onlyMainProcess = enabled))

    fun onOnlyForegroundChange(enabled: Boolean) =
        patch(GlobalConfigPatch(onlyForeground = enabled))

    fun onNotifyModeChange(mode: NotifyMode) = patch(GlobalConfigPatch(notifyMode = mode))

    fun onRetentionDaysChange(days: Int) =
        patch(GlobalConfigPatch(retention = RetentionPatch(retentionDays = days)))

    fun onMaxRecordsTotalChange(records: Int) =
        patch(GlobalConfigPatch(retention = RetentionPatch(maxRecordsTotal = records)))

    /**
     * Turns the crash-dialog takeover on or off.
     *
     * Separate from the ordinary config patches because it changes system-wide
     * behaviour and the daemon reports back what actually took effect — including
     * whether `anr_show_background` is overriding the suppression, which the UI has
     * to surface or the setting looks broken.
     */
    fun onDialogTakeoverChange(enabled: Boolean) {
        viewModelScope.launch {
            config.setDialogTakeover(enabled)
                .onSuccess { outcome ->
                    local.value = local.value.copy(
                        dialogTakeover = DialogTakeoverStatus(
                            requested = enabled,
                            effective = outcome.effective,
                            anrShowBackgroundConflict = outcome.anrShowBackgroundConflict,
                            unsupportedReason = outcome.unsupportedReason,
                        ),
                        error = null,
                    )
                    // The outcome above only lives in this screen's local state. Without
                    // re-reading the config, leaving the page and coming back would show
                    // the stale stored value again — the setting would look like it had
                    // reverted itself.
                    config.refreshGlobalConfig()
                }
                .onFailure { local.value = local.value.copy(error = it.domainError) }
        }
    }

    fun onReconnect() {
        viewModelScope.launch { reconnects.send(moduleStatus.reconnect().toReconnectOutcome()) }
    }

    /**
     * Clears every stored record.
     *
     * The repository announces the deletion, so the lists, the per-app counts, the
     * overview's figures and any open detail page all react on their own — this does not
     * have to know who is watching. Refreshing the module status here as well is for the
     * page the user is looking at: the storage numbers on it come from that status, and a
     * cleared store that still reads "2 records" would look like the button did nothing.
     */
    fun onDeleteAll() {
        viewModelScope.launch {
            crashes.delete(DeleteTarget.all())
                .onSuccess {
                    local.value = local.value.copy(error = null)
                    moduleStatus.refresh()
                }
                .onFailure { local.value = local.value.copy(error = it.domainError) }
        }
    }

    fun onAdjustmentShown() {
        local.value = local.value.copy(lastAdjustment = null)
    }

    private fun patch(patch: GlobalConfigPatch) {
        viewModelScope.launch {
            config.updateGlobalConfig(patch)
                .onSuccess { update ->
                    // Say so when the daemon clamped the request: a slider silently
                    // disagreeing with the stored value is worse than a short notice.
                    local.value = local.value.copy(
                        lastAdjustment = if (update.adjusted) ADJUSTED else null,
                        error = null,
                    )
                }
                .onFailure { local.value = local.value.copy(error = it.domainError) }
        }
    }

    private companion object {
        const val ADJUSTED = "adjusted"
    }
}
