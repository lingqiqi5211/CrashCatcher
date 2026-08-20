package io.github.lingqiqi5211.crashcatcher.ui.settings

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.github.lingqiqi5211.crashcatcher.data.daemon.RuntimeLogFile
import io.github.lingqiqi5211.crashcatcher.data.daemon.domainError
import io.github.lingqiqi5211.crashcatcher.domain.model.DomainError
import io.github.lingqiqi5211.crashcatcher.domain.repository.ConfigRepository
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

internal data class RuntimeLogUiState(
    /** Which file is shown, matching one of [files]. */
    val name: String = "",
    val text: String = "",
    /** Something was cut from the front; what is shown is the end. */
    val truncated: Boolean = false,
    val totalBytes: Long = 0,
    /** Everything the daemon has, newest first. */
    val files: List<RuntimeLogFile> = emptyList(),
    val isLoading: Boolean = true,
    val error: DomainError? = null,
) {
    /** Every file together — what the logs cost on disk, not what one of them weighs. */
    val allBytes: Long get() = files.sumOf { it.bytes }
}

/**
 * The daemon's logs, fetched on demand.
 *
 * Its own view model, not more state on the settings one: this is the only screen whose content
 * runs to hundreds of kilobytes, and the settings tab polls itself to draw.
 */
internal class DiagnosticsViewModel(
    private val config: ConfigRepository,
) : ViewModel() {

    private val state = MutableStateFlow(RuntimeLogUiState())
    val uiState: StateFlow<RuntimeLogUiState> = state.asStateFlow()

    private var inFlight: Job? = null

    /** Reloads the current file, or the newest one on first open. */
    fun refresh() {
        load(state.value.name.takeIf { it.isNotEmpty() })
    }

    fun select(name: String) {
        load(name)
    }

    private fun load(name: String?) {
        inFlight?.cancel()
        state.update { it.copy(isLoading = true, error = null) }
        inFlight = viewModelScope.launch {
            config.runtimeLog(name)
                .onSuccess { log ->
                    state.value = RuntimeLogUiState(
                        name = log.name,
                        text = log.text,
                        truncated = log.truncated,
                        totalBytes = log.totalBytes,
                        files = log.files,
                        isLoading = false,
                    )
                }
                .onFailure { cause ->
                    state.update { it.copy(isLoading = false, error = cause.domainError) }
                }
        }
    }

    /**
     * Every file's contents, for the diagnostics page's archive.
     *
     * Fetched one at a time because that is what the protocol offers, and there are at most a
     * couple of dozen of them. Files that fail are skipped: an archive missing one entry is
     * still worth sending.
     */
    suspend fun readAll(): Map<String, String> {
        val listing = state.value.files.ifEmpty {
            config.runtimeLog().getOrNull()?.files.orEmpty()
        }
        val entries = LinkedHashMap<String, String>()
        for (file in listing) {
            config.runtimeLog(file.name).onSuccess { log ->
                entries[file.name] = log.text
            }
        }
        return entries
    }
}
