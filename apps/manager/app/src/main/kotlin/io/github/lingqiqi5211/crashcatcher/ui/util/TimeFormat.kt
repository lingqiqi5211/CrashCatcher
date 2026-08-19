package io.github.lingqiqi5211.crashcatcher.ui.util

import java.text.SimpleDateFormat
import java.util.Calendar
import java.util.Date
import java.util.Locale

/**
 * Formats a wall-clock instant for display.
 *
 * Absolute rather than relative ("3 hours ago"): the whole point of a crash log is
 * correlating a failure with something else that happened, and a relative label
 * makes that harder the moment the reader looks away from the screen.
 */
internal fun formatTimestamp(epochMs: Long, locale: Locale = Locale.getDefault()): String =
    SimpleDateFormat(TIMESTAMP_PATTERN, locale).format(Date(epochMs))

/** Same instant, without the date — for rows already grouped under one day. */
internal fun formatTimeOfDay(epochMs: Long, locale: Locale = Locale.getDefault()): String =
    SimpleDateFormat(TIME_PATTERN, locale).format(Date(epochMs))

/**
 * The shortest form that is still unambiguous, for list rows.
 *
 * `2026-08-18 15:34:43` is nineteen characters of mostly-redundant context in a row
 * that also has to fit a name, a package and a count — it squeezed the package down to
 * `io.gith…`. Dropping what the reader can infer keeps the row readable:
 *
 * - today: the time alone
 * - this year: month, day and time
 * - earlier: the date, since the time no longer helps place it
 *
 * Seconds are kept wherever a time is shown at all. Crashes arrive in bursts — a
 * retry loop can produce several inside one minute — and `15:34` for four different
 * records makes an ordered list look like it is repeating itself.
 *
 * Still absolute rather than "3 hours ago", for the reason [formatTimestamp] gives:
 * detail pages need an instant that can be correlated with something else.
 */
internal fun formatTimestampCompact(
    epochMs: Long,
    nowMs: Long = System.currentTimeMillis(),
    locale: Locale = Locale.getDefault(),
): String {
    val moment = Calendar.getInstance(locale).apply { timeInMillis = epochMs }
    val now = Calendar.getInstance(locale).apply { timeInMillis = nowMs }

    val sameYear = moment.get(Calendar.YEAR) == now.get(Calendar.YEAR)
    val sameDay = sameYear &&
        moment.get(Calendar.DAY_OF_YEAR) == now.get(Calendar.DAY_OF_YEAR)

    val pattern = when {
        sameDay -> SHORT_TIME_PATTERN
        sameYear -> SHORT_DATE_TIME_PATTERN
        else -> DATE_PATTERN
    }
    return SimpleDateFormat(pattern, locale).format(Date(epochMs))
}

private const val TIMESTAMP_PATTERN = "yyyy-MM-dd HH:mm:ss"
private const val TIME_PATTERN = "HH:mm:ss"
private const val SHORT_TIME_PATTERN = "HH:mm:ss"
private const val SHORT_DATE_TIME_PATTERN = "MM-dd HH:mm:ss"
private const val DATE_PATTERN = "yyyy-MM-dd"
