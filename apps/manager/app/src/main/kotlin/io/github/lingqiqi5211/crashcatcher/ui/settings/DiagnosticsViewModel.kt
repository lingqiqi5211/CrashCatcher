package io.github.lingqiqi5211.crashcatcher.ui.settings

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
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
    val text: String = "",
    /** Something was cut from the front; what is shown is the end. */
    val truncated: Boolean = false,
    val totalBytes: Long = 0,
    val isLoading: Boolean = true,
    val error: DomainError? = null,
) {
    val isEmpty: Boolean get() = text.isBlank() && !isLoading && error == null
}

/**
 * The daemon's own log, fetched on demand.
 *
 * Its own view model rather than more state on the settings one: this is the only screen whose
 * content is worth hundreds of kilobytes, and it must not be attached to something the settings
 * tab polls to draw itself.
 */
internal class DiagnosticsViewModel(
    private val config: ConfigRepository,
) : ViewModel() {

    private val state = MutableStateFlow(RuntimeLogUiState())
    val uiState: StateFlow<RuntimeLogUiState> = state.asStateFlow()

    private var inFlight: Job? = null

    fun refresh() {
        inFlight?.cancel()
        state.update { it.copy(isLoading = true, error = null) }
        inFlight = viewModelScope.launch {
            config.runtimeLog()
                .onSuccess { log ->
                    state.value = RuntimeLogUiState(
                        text = log.text,
                        truncated = log.truncated,
                        totalBytes = log.totalBytes,
                        isLoading = false,
                    )
                }
                .onFailure { cause ->
                    state.update {
                        it.copy(isLoading = false, error = cause.domainError)
                    }
                }
        }
    }
}
