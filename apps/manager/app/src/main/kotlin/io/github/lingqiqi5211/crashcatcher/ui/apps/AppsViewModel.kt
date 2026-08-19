package io.github.lingqiqi5211.crashcatcher.ui.apps

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.github.lingqiqi5211.crashcatcher.data.daemon.AppEntry
import io.github.lingqiqi5211.crashcatcher.data.daemon.domainError
import io.github.lingqiqi5211.crashcatcher.domain.model.DomainError
import io.github.lingqiqi5211.crashcatcher.domain.repository.AppInventoryRepository
import io.github.lingqiqi5211.crashcatcher.domain.repository.CrashRepository
import kotlinx.coroutines.FlowPreview
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.debounce
import kotlinx.coroutines.flow.drop
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

internal data class AppsUiState(
    val apps: List<AppEntry> = emptyList(),
    val query: String = "",
    val searchExpanded: Boolean = false,
    val includeSystemApps: Boolean = false,
    val isLoading: Boolean = true,
    val isRefreshing: Boolean = false,
    val error: DomainError? = null,
) {
    val isEmpty: Boolean get() = apps.isEmpty() && !isLoading && error == null
}

/**
 * Apps that have crashed, most recently first.
 *
 * The daemon does the filtering and the ordering — it has the crash counts indexed,
 * and shipping the whole installed-app list here just to sort it in the UI would be
 * the same mistake the storage design exists to avoid.
 */
internal class AppsViewModel(
    private val apps: AppInventoryRepository,
    private val crashes: CrashRepository,
) : ViewModel() {

    private val state = MutableStateFlow(AppsUiState())
    val uiState: StateFlow<AppsUiState> = state.asStateFlow()

    private var inFlight: Job? = null

    @OptIn(FlowPreview::class)
    private val queryFlow = MutableStateFlow("")

    init {
        refresh()
        viewModelScope.launch {
            // Debounced so typing does not put one request per keystroke on the socket;
            // `drop(1)` skips the initial value, which `refresh()` above already covers.
            queryFlow.drop(1).debounce(SEARCH_DEBOUNCE_MS).map { it.trim() }.collect { refresh() }
        }
        viewModelScope.launch {
            // Per-app counts come from the crash store, so a delete anywhere makes this
            // list wrong — including making an app that now has no crashes at all still
            // appear in it.
            crashes.dataChanged.collect { load() }
        }
    }

    fun onQueryChange(query: String) {
        state.update { it.copy(query = query) }
        queryFlow.value = query
    }

    fun onSearchExpandedChange(expanded: Boolean) {
        state.update { it.copy(searchExpanded = expanded) }
    }

    fun onIncludeSystemAppsChange(include: Boolean) {
        state.update { it.copy(includeSystemApps = include) }
        refresh()
    }

    fun refresh() {
        inFlight?.cancel()
        state.update {
            it.copy(
                isLoading = it.apps.isEmpty(),
                isRefreshing = it.apps.isNotEmpty(),
                error = null,
            )
        }
        inFlight = viewModelScope.launch {
            load()
            state.update { it.copy(isRefreshing = false) }
        }
    }

    /**
     * Reloads for the pull gesture, holding the indicator up for a moment.
     *
     * See `CrashesViewModel.onPullToRefresh`: the daemon answers faster than the pull
     * animation retracts, so without a floor the gesture looks like it did nothing.
     */
    fun onPullToRefresh() {
        inFlight?.cancel()
        state.update { it.copy(isRefreshing = true, error = null) }
        inFlight = viewModelScope.launch {
            val floor = launch { delay(MIN_REFRESH_VISIBLE_MS) }
            load()
            floor.join()
            state.update { it.copy(isRefreshing = false) }
        }
    }

    private suspend fun load() {
        val current = state.value
        apps.listApps(
            includeSystemApps = current.includeSystemApps,
            query = current.query.takeIf { it.isNotBlank() },
            limit = PAGE_LIMIT,
        ).onSuccess { entries ->
            state.update {
                it.copy(apps = entries, isLoading = false, error = null)
            }
        }.onFailure { cause ->
            state.update {
                it.copy(isLoading = false, error = cause.domainError)
            }
        }
    }

    private companion object {
        const val PAGE_LIMIT = 200
        const val SEARCH_DEBOUNCE_MS = 250L

        /** How long the pull indicator stays up at minimum. */
        const val MIN_REFRESH_VISIBLE_MS = 450L
    }
}
