package io.github.lingqiqi5211.crashcatcher.data.daemon

/**
 * Transport-level failures, kept apart from the daemon's own [WireErrorCode].
 *
 * The distinction matters to the UI: a [WireErrorCode] means the daemon understood
 * the request and declined it, while these mean the conversation itself broke and
 * the connection has to be re-established.
 */
sealed class DaemonException(message: String) : Exception(message) {

    class ConnectionClosed(message: String) : DaemonException(message)

    class MalformedFrame(message: String) : DaemonException("malformed frame: $message")

    class PayloadTooLarge(val bytes: Long) :
        DaemonException("frame body of $bytes bytes exceeds the limit")

    class Timeout(message: String) : DaemonException("timed out: $message")

    /** The daemon answered a `seq` we never sent, or answered twice. */
    class ProtocolViolation(message: String) : DaemonException("protocol violation: $message")

    /** The daemon rejected the request; [error] carries its reason. */
    class Rejected(val error: WireError) : DaemonException(error.toString())
}
