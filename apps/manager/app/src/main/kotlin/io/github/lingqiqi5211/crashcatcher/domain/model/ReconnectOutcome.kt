package io.github.lingqiqi5211.crashcatcher.domain.model

/**
 * What came of one user-initiated reconnect.
 *
 * Delivered as a one-shot event rather than as state, because two failed attempts in a
 * row are two things the user did: a state flow would collapse them into the same
 * unchanged value, leaving the second press indistinguishable from no press at all.
 *
 * [error] is null exactly when the reconnect succeeded, and carries the reason the
 * daemon could not be reached otherwise, so the message can name the cause instead of
 * saying only that something went wrong.
 */
internal data class ReconnectOutcome(val error: DomainError?) {
    val connected: Boolean get() = error == null
}
