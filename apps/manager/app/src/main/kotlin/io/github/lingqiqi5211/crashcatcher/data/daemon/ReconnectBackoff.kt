package io.github.lingqiqi5211.crashcatcher.data.daemon

import kotlin.random.Random
import kotlin.time.Duration
import kotlin.time.Duration.Companion.milliseconds
import kotlin.time.Duration.Companion.seconds

/**
 * Delay schedule for reconnecting to the daemon.
 *
 * Exponential with jitter and a ceiling. The daemon being absent is a normal state —
 * the module may not be installed, or may be restarting — so the client has to keep
 * trying without turning into a busy loop against a socket that is not there.
 */
interface ReconnectBackoff {
    /** Delay before the next attempt, advancing the schedule. */
    fun nextDelay(): Duration

    /** Called after a successful connection. */
    fun reset()
}

class DefaultReconnectBackoff(
    private val initial: Duration = 250.milliseconds,
    private val max: Duration = 30.seconds,
    private val factor: Double = 2.0,
    /**
     * Fraction of the delay that is randomised.
     *
     * Without it, every manager process on a device that just rebooted would retry
     * in lockstep.
     */
    private val jitterRatio: Double = 0.2,
    private val random: Random = Random.Default,
) : ReconnectBackoff {

    private var attempt = 0

    override fun nextDelay(): Duration {
        val exponent = attempt.coerceAtMost(MAX_EXPONENT)
        attempt = (attempt + 1).coerceAtMost(MAX_EXPONENT)

        val scaled = initial.inWholeMilliseconds * factor.pow(exponent)
        val capped = scaled.coerceAtMost(max.inWholeMilliseconds.toDouble())
        val jitterSpan = capped * jitterRatio
        val jittered = capped - jitterSpan / 2 + random.nextDouble() * jitterSpan

        return jittered.coerceAtLeast(1.0).toLong().milliseconds
    }

    override fun reset() {
        attempt = 0
    }

    private fun Double.pow(exponent: Int): Double {
        var result = 1.0
        repeat(exponent) { result *= this }
        return result
    }

    private companion object {
        /** Past this the delay is already at [max]; bounded so it cannot overflow. */
        const val MAX_EXPONENT = 16
    }
}
