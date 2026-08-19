package io.github.lingqiqi5211.crashcatcher.domain.model

/**
 * How a piece of remote state is doing.
 *
 * [StaleContent] is the variant that earns this type its keep: when a refresh
 * fails, the screen keeps showing the last good data *and* surfaces the error,
 * rather than blanking out. For a crash viewer that matters — a dropped daemon
 * connection should not make the history the user is reading disappear.
 */
sealed interface LoadState<out T> {
    data object Loading : LoadState<Nothing>

    /** Loaded successfully, and there is genuinely nothing to show. */
    data object Empty : LoadState<Nothing>

    data class Content<T>(val value: T) : LoadState<T>

    /** Last known-good value, plus the error from the refresh that failed. */
    data class StaleContent<T>(val value: T, val error: DomainError) : LoadState<T>

    data class Error(val error: DomainError) : LoadState<Nothing>

    /** A capability this build knows about but the daemon does not provide. */
    data class Unsupported(val reason: String) : LoadState<Nothing>
}

/** The value if there is one, whether or not the latest refresh succeeded. */
val <T> LoadState<T>.valueOrNull: T?
    get() = when (this) {
        is LoadState.Content -> value
        is LoadState.StaleContent -> value
        else -> null
    }

/** The error if the latest attempt failed, even when stale data is still shown. */
val LoadState<*>.errorOrNull: DomainError?
    get() = when (this) {
        is LoadState.Error -> error
        is LoadState.StaleContent -> error
        else -> null
    }

val LoadState<*>.isLoading: Boolean get() = this is LoadState.Loading

/**
 * Keeps whatever value is already on screen when a refresh fails.
 *
 * Callers reduce `(previous, result)` through this instead of overwriting with
 * [LoadState.Error], which is what makes the stale-data behaviour the default
 * rather than something each screen has to remember.
 */
fun <T> LoadState<T>.withError(error: DomainError): LoadState<T> =
    when (val existing = valueOrNull) {
        null -> LoadState.Error(error)
        else -> LoadState.StaleContent(existing, error)
    }

/**
 * A failure described in terms the UI can branch on.
 *
 * [kind] and [code] carry the meaning; [message] is developer detail and is never
 * shown to the user verbatim, so the wording stays free to change and the UI stays
 * translatable.
 */
data class DomainError(
    val kind: DomainErrorKind,
    val message: String,
    val code: DomainErrorCode? = null,
)

enum class DomainErrorKind {
    /** The socket is not there, or went away mid-conversation. */
    ConnectionLost,

    /** The daemon answered, and said no. */
    DaemonRejected,

    /** Frames did not parse — the two sides disagree about the protocol. */
    ProtocolError,

    /** Manager and daemon protocol versions differ. */
    VersionMismatch,

    /** The daemon is up but a dependency it needs is not. */
    Unavailable,

    Unknown,
}

/** Mirrors the daemon's own error vocabulary, for the cases the UI treats specially. */
enum class DomainErrorCode {
    Unauthorized,
    NotFound,
    InvalidRequest,
    PayloadTooLarge,

    /**
     * The page cursor no longer applies.
     *
     * Its own code because the correct response is "start the query over", not
     * "retry the same request" — retrying unchanged would fail identically.
     */
    CursorInvalidated,

    Internal,
}
