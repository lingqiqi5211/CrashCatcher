package io.github.lingqiqi5211.crashcatcher.ui.home

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.github.lingqiqi5211.crashcatcher.data.daemon.CollectorHealth
import io.github.lingqiqi5211.crashcatcher.data.daemon.ModuleStatus
import io.github.lingqiqi5211.crashcatcher.data.daemon.Stats
import io.github.lingqiqi5211.crashcatcher.data.device.readDeviceInfo
import io.github.lingqiqi5211.crashcatcher.domain.model.DeviceInfo
import io.github.lingqiqi5211.crashcatcher.data.daemon.toReconnectOutcome
import io.github.lingqiqi5211.crashcatcher.domain.model.LoadState
import io.github.lingqiqi5211.crashcatcher.domain.model.ReconnectOutcome
import io.github.lingqiqi5211.crashcatcher.domain.model.valueOrNull
import io.github.lingqiqi5211.crashcatcher.domain.repository.CrashRepository
import io.github.lingqiqi5211.crashcatcher.domain.repository.ModuleStatusRepository
import io.github.lingqiqi5211.crashcatcher.domain.repository.StatsRepository
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch

/**
 * How the module is doing, in one word.
 *
 * [Degraded] is the state that matters and the one the tool being replaced has no way
 * to express: the daemon is up and answering, yet a collector cannot do its job — it
 * has been switched off underneath us, or it reported an error. That is how a module
 * ends up looking active while silently recording nothing.
 *
 * Deliberately *not* "some collector has never produced a row". Most sources are only
 * ever exercised by a specific kind of failure: the tombstone reader sees nothing until
 * a native crash happens, the ANR reader nothing until an ANR does. On a device that
 * has not crashed — the good outcome — every source is legitimately empty, and the old
 * rule reported that as a fault. It also read `everReceived`, which is per-daemon-run
 * state, so a reboot alone was enough to make a device with months of history claim
 * that nothing was being collected.
 */
internal enum class RuntimeStatus { Checking, Running, Degraded, Unreachable }

internal data class HomeUiState(
    val moduleStatus: LoadState<ModuleStatus> = LoadState.Loading,
    val stats: LoadState<Stats> = LoadState.Loading,
    /**
     * Read once at construction rather than loaded.
     *
     * It cannot change while the process lives, and it is what makes this screen
     * useful when the daemon is unreachable — which is exactly when someone comes
     * looking for the about information.
     */
    val deviceInfo: DeviceInfo = readDeviceInfo(),
) {
    val collectors: List<CollectorHealth>
        get() = moduleStatus.valueOrNull?.collectors.orEmpty()

    val runtimeStatus: RuntimeStatus
        get() = when (val status = moduleStatus.valueOrNull) {
            null -> if (moduleStatus is LoadState.Loading) {
                RuntimeStatus.Checking
            } else {
                RuntimeStatus.Unreachable
            }

            else -> if (status.collectors.any { it.isImpaired }) {
                RuntimeStatus.Degraded
            } else {
                RuntimeStatus.Running
            }
        }

    /**
     * Collectors that cannot currently do their job.
     *
     * This is what "degraded" means, and the only collector state worth alarming about.
     */
    val impairedCollectors: List<CollectorHealth>
        get() = collectors.filter { it.isImpaired }

    /**
     * Collectors that are healthy but have not been exercised yet.
     *
     * Informational only. For most sources this is the normal state of a device that
     * has not had that kind of crash, so it must not colour the overall status.
     */
    val idleCollectors: List<CollectorHealth>
        get() = collectors.filter { !it.isImpaired && !it.everReceived }

    val totalCollectorCount: Int
        get() = collectors.size
}

/**
 * Whether a collector is failing rather than merely quiet.
 *
 * Two signals, both of which mean something is actually wrong: the source has been
 * switched off underneath the daemon (a ROM or a user clearing `dropbox:<tag>`), or the
 * daemon recorded a reason it could not read from it.
 */
internal val CollectorHealth.isImpaired: Boolean
    get() = !enabled || detail != null

internal class HomeViewModel(
    private val moduleStatus: ModuleStatusRepository,
    private val stats: StatsRepository,
    private val crashes: CrashRepository,
) : ViewModel() {

    init {
        viewModelScope.launch {
            // The storage figures and the recorded-crash count on this page both come
            // from the store, so a delete makes them wrong until something re-reads them.
            crashes.dataChanged.collect { refresh() }
        }
    }

    val uiState: StateFlow<HomeUiState> = combine(
        moduleStatus.status,
        stats.stats,
    ) { status, statistics ->
        HomeUiState(moduleStatus = status, stats = statistics)
    }.stateIn(viewModelScope, SharingStarted.Eagerly, HomeUiState())

    fun refresh() {
        viewModelScope.launch { moduleStatus.refresh() }
        viewModelScope.launch { stats.refresh(timeFromMs = null, timeToMs = null) }
    }

    private val reconnects = Channel<ReconnectOutcome>(Channel.BUFFERED)

    /** One event per tap of the disconnected banner; the shell turns it into a message. */
    val reconnectOutcomes: Flow<ReconnectOutcome> = reconnects.receiveAsFlow()

    fun reconnect() {
        viewModelScope.launch { reconnects.send(moduleStatus.reconnect().toReconnectOutcome()) }
    }
}
