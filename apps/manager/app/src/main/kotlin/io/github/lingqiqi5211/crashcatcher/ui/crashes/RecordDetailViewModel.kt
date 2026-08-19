package io.github.lingqiqi5211.crashcatcher.ui.crashes

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.github.lingqiqi5211.crashcatcher.data.daemon.ExportRedaction
import io.github.lingqiqi5211.crashcatcher.data.daemon.PayloadState
import io.github.lingqiqi5211.crashcatcher.data.daemon.RecordDetail
import io.github.lingqiqi5211.crashcatcher.data.daemon.RecordId
import io.github.lingqiqi5211.crashcatcher.data.daemon.domainError
import io.github.lingqiqi5211.crashcatcher.data.device.readDeviceInfo
import io.github.lingqiqi5211.crashcatcher.domain.model.DeviceInfo
import io.github.lingqiqi5211.crashcatcher.domain.model.DomainError
import io.github.lingqiqi5211.crashcatcher.domain.repository.CrashRepository
import io.github.lingqiqi5211.crashcatcher.ui.util.buildExportText
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

internal data class RecordDetailUiState(
    val detail: RecordDetail? = null,
    /** Text received so far; grows as the payload streams in. */
    val payload: String = "",
    val bytesRead: Long = 0,
    val totalBytes: Long = 0,
    val payloadComplete: Boolean = false,
    val foldFrameworkFrames: Boolean = true,
    /**
     * On by default.
     *
     * Wrapping used to break frames mid-identifier, which is why this started off — but
     * panning turned out to be the worse answer: on a pushed page a right-drag is the
     * predictive back gesture, so the trace could only be panned in one direction. With
     * break opportunities inserted at the separators (see `withBreakOpportunities`) a
     * wrapped frame splits between segments instead, and needs no gesture at all.
     */
    val wrapLines: Boolean = true,
    val expandedFolds: Set<Int> = emptySet(),
    val isLoading: Boolean = true,
    val error: DomainError? = null,
) {
    val payloadState: PayloadState? get() = detail?.record?.payloadState

    val items: List<StackItem>
        get() = buildStackItems(payload, foldFrameworkFrames)
}

/** One-shot outcomes the screen turns into a snackbar or a share sheet. */
internal sealed interface RecordDetailEvent {
    data class Exported(val text: String) : RecordDetailEvent
    data class Copied(val text: String) : RecordDetailEvent
    data object Deleted : RecordDetailEvent
    data class Failed(val error: DomainError) : RecordDetailEvent
}

/**
 * The crash detail screen.
 *
 * The payload arrives as a stream and is appended as it comes, so the first lines
 * paint immediately even for a multi-megabyte ANR dump. Waiting for the whole text
 * before drawing anything is precisely what makes the tool being replaced take
 * seconds to open a record.
 */
internal class RecordDetailViewModel(
    private val crashes: CrashRepository,
    /**
     * Injected with a default rather than read where it is used: an exported report
     * carries the device's identity, and a test that asserts on that text needs to be
     * able to supply its own instead of picking up whatever ran the test.
     */
    private val device: DeviceInfo = readDeviceInfo(),
) : ViewModel() {

    private val state = MutableStateFlow(RecordDetailUiState())
    val uiState: StateFlow<RecordDetailUiState> = state.asStateFlow()

    private val events = MutableSharedFlow<RecordDetailEvent>(extraBufferCapacity = 8)
    val uiEvents = events.asSharedFlow()

    private var loadJob: Job? = null
    private var recordId: RecordId? = null

    /**
     * Whether this page has already been told to close.
     *
     * Deleting a record satisfies both conditions that close this page — the delete's own
     * success, and the "the record I am showing is gone" check below, since a successful
     * delete is exactly what announces `dataChanged`. Emitting twice popped two pages for
     * one deletion, so the group underneath went away too, and a third arrival could empty
     * the back stack outright.
     */
    private var closed = false

    init {
        viewModelScope.launch {
            // This page can outlive the record it shows: clearing everything from the
            // storage page, or deleting the containing group, both remove it while it is
            // still on screen. Re-reading and treating a miss as "deleted" closes the
            // page instead of leaving a detail view of something that no longer exists.
            crashes.dataChanged.collect {
                val id = recordId ?: return@collect
                if (crashes.getRecord(id).isFailure) close()
            }
        }
    }

    fun load(id: RecordId) {
        if (recordId == id && state.value.detail != null) return
        recordId = id
        closed = false
        loadJob?.cancel()
        state.value = RecordDetailUiState()

        loadJob = viewModelScope.launch {
            crashes.getRecord(id)
                .onSuccess { detail ->
                    state.update { it.copy(detail = detail, isLoading = false) }
                }
                .onFailure { cause ->
                    state.update {
                        it.copy(isLoading = false, error = cause.domainError)
                    }
                    return@launch
                }

            // A record whose payload was reclaimed still has metadata worth showing;
            // only the text is gone, and the screen says so rather than spinning.
            if (state.value.payloadState?.isReadable != true) {
                state.update { it.copy(payloadComplete = true) }
                return@launch
            }

            crashes.payloadText(id)
                .catch { cause ->
                    state.update { it.copy(error = cause.domainError, payloadComplete = true) }
                }
                .collect { chunk ->
                    state.update {
                        it.copy(
                            payload = it.payload + chunk.text,
                            bytesRead = chunk.bytesRead,
                            totalBytes = chunk.totalBytes,
                            payloadComplete = chunk.eof,
                        )
                    }
                }
        }
    }

    fun onToggleFolding() {
        state.update { it.copy(foldFrameworkFrames = !it.foldFrameworkFrames) }
    }

    fun onToggleWrap() {
        state.update { it.copy(wrapLines = !it.wrapLines) }
    }

    fun onExpandFold(firstIndex: Int) {
        state.update { it.copy(expandedFolds = it.expandedFolds + firstIndex) }
    }

    fun onCopy(redaction: ExportRedaction) {
        viewModelScope.launch {
            events.tryEmit(RecordDetailEvent.Copied(exportText(redaction)))
        }
    }

    fun onShare(redaction: ExportRedaction) {
        viewModelScope.launch {
            events.tryEmit(RecordDetailEvent.Exported(exportText(redaction)))
        }
    }

    /**
     * The report this record turns into, formatted for somewhere outside the app.
     *
     * Waits for the payload first. The screen paints from a stream, so on a large ANR
     * dump the visible text is whatever has arrived so far — copying that would hand over
     * a trace cut off mid-frame, and nothing about the result would say so.
     */
    private suspend fun exportText(redaction: ExportRedaction): String {
        loadJob?.join()
        val detail = state.value.detail ?: return ""
        return buildExportText(
            detail = detail,
            payload = state.value.payload,
            redaction = redaction,
            device = device,
        )
    }

    fun onDelete() {
        val id = recordId ?: return
        viewModelScope.launch {
            crashes.delete(io.github.lingqiqi5211.crashcatcher.data.daemon.DeleteTarget.ids(listOf(id)))
                .onSuccess { close() }
                .onFailure { events.tryEmit(RecordDetailEvent.Failed(it.domainError)) }
        }
    }

    /** Closes the page once, however many times the record turns out to be gone. */
    private fun close() {
        if (closed) return
        closed = true
        events.tryEmit(RecordDetailEvent.Deleted)
    }
}
