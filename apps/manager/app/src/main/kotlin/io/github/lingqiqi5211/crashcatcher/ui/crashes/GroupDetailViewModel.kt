package io.github.lingqiqi5211.crashcatcher.ui.crashes

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.github.lingqiqi5211.crashcatcher.data.daemon.Cursor
import io.github.lingqiqi5211.crashcatcher.data.daemon.DeleteTarget
import io.github.lingqiqi5211.crashcatcher.data.daemon.GroupSummary
import io.github.lingqiqi5211.crashcatcher.data.daemon.RecordSummary
import io.github.lingqiqi5211.crashcatcher.data.daemon.domainError
import io.github.lingqiqi5211.crashcatcher.domain.model.DomainError
import io.github.lingqiqi5211.crashcatcher.domain.repository.CrashRepository
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

internal data class GroupDetailUiState(
    val group: GroupSummary? = null,
    val records: List<RecordSummary> = emptyList(),
    val isLoading: Boolean = true,
    val isAppending: Boolean = false,
    val hasMore: Boolean = false,
    val error: DomainError? = null,
) {
    /**
     * Occurrences the retention rules have already removed.
     *
     * `occurrence` counts every sighting ever; the detail rows are capped. Showing
     * the difference keeps the two numbers from looking like a bug.
     */
    val prunedOccurrences: Long
        get() = ((group?.occurrence ?: 0) - records.size).coerceAtLeast(0)
}

internal sealed interface GroupDetailEvent {
    data object Deleted : GroupDetailEvent
    data class Failed(val error: DomainError) : GroupDetailEvent
}

/**
 * One crash fingerprint and every occurrence of it that survives.
 *
 * The header comes from `get_group` rather than from the list row that opened this
 * screen: the counts move while the screen is open, and re-reading is cheaper than
 * threading a snapshot through the back stack and then having it go stale.
 */
internal class GroupDetailViewModel(
    private val crashes: CrashRepository,
) : ViewModel() {

    private val state = MutableStateFlow(GroupDetailUiState())
    val uiState: StateFlow<GroupDetailUiState> = state.asStateFlow()

    private val events = MutableSharedFlow<GroupDetailEvent>(extraBufferCapacity = 8)
    val uiEvents = events.asSharedFlow()

    private var groupId: String? = null
    private var nextCursor: Cursor? = null
    private var inFlight: Job? = null

    /**
     * Whether this page has already been told to close.
     *
     * Deleting the group answers both close conditions at once — the delete's own success,
     * and the re-read below finding it gone — and two closes for one deletion pop the page
     * the user came from as well as this one.
     */
    private var closed = false

    init {
        viewModelScope.launch {
            // A record deleted from its own detail page can take this group with it — the
            // daemon removes a group once its last record is gone — and clearing
            // everything certainly does. Re-read: still there means refresh the list of
            // records, gone means close the page.
            crashes.dataChanged.collect {
                val id = groupId ?: return@collect
                if (crashes.getGroup(id).isSuccess) {
                    load(id, force = true)
                } else {
                    close()
                }
            }
        }
    }

    /**
     * Loads a group.
     *
     * [force] re-reads a group already on screen, for the case where the data underneath
     * it changed rather than the user having navigated somewhere new.
     */
    fun load(id: String, force: Boolean = false) {
        if (!force && groupId == id && state.value.group != null) return
        groupId = id
        closed = false
        nextCursor = null
        inFlight?.cancel()
        state.value = GroupDetailUiState()

        inFlight = viewModelScope.launch {
            crashes.getGroup(id)
                .onSuccess { group -> state.update { it.copy(group = group) } }
                .onFailure { cause ->
                    state.update { it.copy(isLoading = false, error = cause.domainError) }
                    return@launch
                }
            loadRecords(append = false)
        }
    }

    fun loadMore() {
        val current = state.value
        if (!current.hasMore || current.isAppending || nextCursor == null) return
        state.update { it.copy(isAppending = true) }
        inFlight = viewModelScope.launch { loadRecords(append = true) }
    }

    fun delete() {
        val id = groupId ?: return
        viewModelScope.launch {
            crashes.delete(DeleteTarget.group(id))
                .onSuccess { close() }
                .onFailure { events.tryEmit(GroupDetailEvent.Failed(it.domainError)) }
        }
    }

    /** Closes the page once, however many times the group turns out to be gone. */
    private fun close() {
        if (closed) return
        closed = true
        events.tryEmit(GroupDetailEvent.Deleted)
    }

    private suspend fun loadRecords(append: Boolean) {
        val id = groupId ?: return
        crashes.listRecords(id, nextCursor.takeIf { append }, PAGE_SIZE)
            .onSuccess { page ->
                nextCursor = page.nextCursor
                state.update {
                    it.copy(
                        records = if (append) it.records + page.items else page.items,
                        isLoading = false,
                        isAppending = false,
                        hasMore = page.nextCursor != null,
                        error = null,
                    )
                }
            }
            .onFailure { cause ->
                state.update {
                    it.copy(isLoading = false, isAppending = false, error = cause.domainError)
                }
            }
    }

    private companion object {
        const val PAGE_SIZE = 40
    }
}
