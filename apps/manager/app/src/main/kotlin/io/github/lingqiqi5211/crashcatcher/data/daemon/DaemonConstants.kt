package io.github.lingqiqi5211.crashcatcher.data.daemon

import kotlin.time.Duration.Companion.seconds

/**
 * Wire constants, mirroring `cch_wire::frame`.
 *
 * Any change here has to land on both sides in the same commit; the cross-language
 * vector tests in `WireCompatibilityTest` are what catch it when it does not.
 */
object DaemonConstants {
    /**
     * Abstract-namespace socket the Manager listens on and the daemon connects to.
     *
     * A distinct name from the old daemon listener avoids an updated Manager colliding with a
     * still-running pre-reversal daemon before the module has rebooted.
     */
    const val ABSTRACT_SOCKET_NAME = "crash_catcher_manager_listener"

    const val LENGTH_PREFIX_BYTES = 4

    /**
     * Largest frame body either side will accept.
     *
     * Bulk payloads never travel in a frame — they arrive as a file descriptor — so
     * this only has to hold a page of list rows.
     */
    const val MAX_FRAME_BODY_BYTES = 1024 * 1024

    /**
     * Must equal `cch_wire::PROTOCOL_VERSION`; the handshake refuses the connection otherwise.
     *
     * Bumped whenever either side needs the other updated with it — see the Rust constant for
     * why an additive request still counts, and what each version added.
     */
    const val PROTOCOL_VERSION = 5

    val DEFAULT_REQUEST_TIMEOUT = 5.seconds
}
