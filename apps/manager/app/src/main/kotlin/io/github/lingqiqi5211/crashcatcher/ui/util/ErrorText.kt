package io.github.lingqiqi5211.crashcatcher.ui.util

import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
import io.github.lingqiqi5211.crashcatcher.R
import io.github.lingqiqi5211.crashcatcher.domain.model.DomainError
import io.github.lingqiqi5211.crashcatcher.domain.model.DomainErrorCode
import io.github.lingqiqi5211.crashcatcher.domain.model.DomainErrorKind

/**
 * Turns a [DomainError] into something a person can read.
 *
 * Branches on `code` and `kind`, never on `message`. The message is developer detail
 * written in English by the daemon — putting it on screen means showing a user
 * "waiting for daemon at @crash_catcher_manager_listener" and
 * expecting them to know what to do with it.
 */
@Composable
internal fun errorTitle(error: DomainError): String = stringResource(
    when {
        error.code == DomainErrorCode.Unauthorized -> R.string.error_title_unauthorized
        error.kind == DomainErrorKind.ConnectionLost -> R.string.error_title_unreachable
        error.kind == DomainErrorKind.VersionMismatch -> R.string.error_title_version
        error.kind == DomainErrorKind.Unavailable -> R.string.error_title_unavailable
        error.kind == DomainErrorKind.ProtocolError -> R.string.error_title_protocol
        else -> R.string.error_title_generic
    },
)

@Composable
internal fun errorDescription(error: DomainError): String = stringResource(
    when {
        error.code == DomainErrorCode.Unauthorized -> R.string.error_body_unauthorized
        error.kind == DomainErrorKind.ConnectionLost -> R.string.error_body_unreachable
        error.kind == DomainErrorKind.VersionMismatch -> R.string.error_body_version
        error.kind == DomainErrorKind.Unavailable -> R.string.error_body_unavailable
        error.kind == DomainErrorKind.ProtocolError -> R.string.error_body_protocol
        else -> R.string.error_body_generic
    },
)

/** Whether retrying the same request unchanged could plausibly succeed. */
internal fun DomainError.isRetryable(): Boolean = when {
    code == DomainErrorCode.Unauthorized -> false
    kind == DomainErrorKind.VersionMismatch -> false
    else -> true
}
