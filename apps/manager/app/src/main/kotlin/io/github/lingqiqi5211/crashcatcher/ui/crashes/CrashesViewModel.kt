package io.github.lingqiqi5211.crashcatcher.ui.crashes

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.github.lingqiqi5211.crashcatcher.data.daemon.CrashFilter
import io.github.lingqiqi5211.crashcatcher.data.daemon.CrashKind
import io.github.lingqiqi5211.crashcatcher.data.daemon.Cursor
import io.github.lingqiqi5211.crashcatcher.data.daemon.GroupSummary
import io.github.lingqiqi5211.crashcatcher.data.daemon.SortKey
import io.github.lingqiqi5211.crashcatcher.data.daemon.domainError
import io.github.lingqiqi5211.crashcatcher.domain.model.DomainError
import io.github.lingqiqi5211.crashcatcher.domain.model.DomainErrorCode
import io.github.lingqiqi5211.crashcatcher.domain.repository.CrashRepository
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** Which crash kinds a tab selects. `null` means "all". */
internal enum class CrashTab(val kind: CrashKind?) {
    All(null),
    Java(CrashKind.JavaException),
    Anr(CrashKind.Anr),
    Native(CrashKind.NativeCrash),
}

internal data class CrashesUiState(
    val groups: List<GroupSummary> = emptyList(),
    val tab: CrashTab = CrashTab.All,
    val query: String = "",
    val searchExpanded: Boolean = false,
    val includeSystemApps: Boolean = false,
    val onlySelfHandled: Boolean = false,
    val isLoading: Boolean = true,
    val isRefreshing: Boolean = false,
    /** More pages remain; the list appends when the user reaches the end. */
    val hasMore: Boolean = false,
    val isAppending: Boolean = false,
    val error: DomainError? = null,
) {
    val isEmpty: Boolean get() = groups.isEmpty() && !isLoading && error == null

    /**
     * Whether anything is narrowing the list.
     *
     * Decides which empty state to show: "nothing has crashed" is good news, while
     * "nothing matches" means the filter is the thing to change. Telling a user their
     * device is crash-free when they have simply typed a typo is worse than useless.
     */
    val isFiltered: Boolean
        get() = query.isNotBlank() || tab != CrashTab.All || onlySelfHandled
}

/**
 * The crash list.
 *
 * Pages are appended, never re-fetched wholesale: the daemon hands back an opaque
 * cursor and this holds it. A filter or tab change starts a fresh query and drops
 * the cursor, because a cursor is only meaningful for the result set that produced
 * it — reusing one across a filter change is exactly what the daemon rejects with
 * `CursorInvalidated`.
 */
internal class CrashesViewModel(
    private val crashes: CrashRepository,
) : ViewModel() {

    private val state = MutableStateFlow(CrashesUiState())
    val uiState: StateFlow<CrashesUiState> = state.asStateFlow()

    private var nextCursor: Cursor? = null
    private var inFlight: Job? = null

    init {
        refresh()
        viewModelScope.launch {
            // A new occurrence while the list is open should show up without the user
            // having to pull to refresh.
            crashes.crashRecorded.collect { refresh(showSpinner = false) }
        }
        viewModelScope.launch {
            // Same for the other direction: something deleted elsewhere — a record on
            // its detail page, or everything from the storage page — must not leave rows
            // here that no longer exist. Silent, because the user did not ask this screen
            // to load anything.
            crashes.dataChanged.collect { refresh(showSpinner = false) }
        }
    }

    fun onTabSelected(tab: CrashTab) {
        if (tab == state.value.tab) return
        state.update { it.copy(tab = tab) }
        refresh()
    }

    fun onQueryChange(query: String) {
        state.update { it.copy(query = query) }
    }

    fun onSearchExpandedChange(expanded: Boolean) {
        state.update { it.copy(searchExpanded = expanded) }
    }

    fun onSearchSubmit() = refresh()

    fun onIncludeSystemAppsChange(include: Boolean) {
        state.update { it.copy(includeSystemApps = include) }
        refresh()
    }

    fun onOnlySelfHandledChange(only: Boolean) {
        state.update { it.copy(onlySelfHandled = only) }
        refresh()
    }

    fun refresh(showSpinner: Boolean = true) {
        inFlight?.cancel()
        nextCursor = null
        state.update {
            it.copy(
                isLoading = showSpinner && it.groups.isEmpty(),
                isRefreshing = showSpinner && it.groups.isNotEmpty(),
                error = null,
            )
        }
        inFlight = viewModelScope.launch {
            load(append = false)
            state.update { it.copy(isRefreshing = false) }
        }
    }

    /**
     * Reloads for the pull gesture, holding the indicator up for a moment.
     *
     * Separate from [refresh] because of where the data comes from: a local socket
     * answers in single-digit milliseconds, so the indicator was cleared before the pull
     * had even finished retracting and the gesture read as ignored. Racing the load
     * against a floor shows the acknowledgement without making the data wait — whichever
     * takes longer decides. Filter changes keep using [refresh], where an artificial
     * floor would only make the app feel slow.
     */
    fun onPullToRefresh() {
        inFlight?.cancel()
        nextCursor = null
        state.update { it.copy(isRefreshing = true, error = null) }
        inFlight = viewModelScope.launch {
            val floor = launch { delay(MIN_REFRESH_VISIBLE_MS) }
            load(append = false)
            floor.join()
            state.update { it.copy(isRefreshing = false) }
        }
    }

    /** Requests the next page. Ignored while one is already in flight. */
    fun loadMore() {
        val current = state.value
        if (!current.hasMore || current.isAppending || nextCursor == null) return
        state.update { it.copy(isAppending = true) }
        inFlight = viewModelScope.launch { load(append = true) }
    }

    private suspend fun load(append: Boolean) {
        val current = state.value
        val filter = CrashFilter(
            kinds = listOfNotNull(current.tab.kind),
            includeSystemApps = current.includeSystemApps,
            onlySelfHandled = current.onlySelfHandled,
            query = current.query.takeIf { it.isNotBlank() },
        )

        crashes.listGroups(
            filter = filter,
            sort = SortKey.LastSeenDesc,
            cursor = nextCursor.takeIf { append },
            limit = PAGE_SIZE,
        ).onSuccess { page ->
            nextCursor = page.nextCursor
            state.update {
                it.copy(
                    groups = if (append) it.groups + page.items else page.items,
                    isLoading = false,
                    isAppending = false,
                    hasMore = page.nextCursor != null,
                    error = null,
                )
            }
        }.onFailure { cause ->
            val error = cause.domainError
            // A stale cursor is not worth showing the user: start the query over.
            if (error.code == DomainErrorCode.CursorInvalidated && append) {
                nextCursor = null
                state.update { it.copy(isAppending = false) }
                refresh(showSpinner = false)
                return
            }
            state.update {
                it.copy(
                    isLoading = false,
                    isAppending = false,
                    error = error,
                )
            }
        }
    }

    private companion object {
        const val PAGE_SIZE = 40

        /** How long the pull indicator stays up at minimum. */
        const val MIN_REFRESH_VISIBLE_MS = 450L
    }
}
