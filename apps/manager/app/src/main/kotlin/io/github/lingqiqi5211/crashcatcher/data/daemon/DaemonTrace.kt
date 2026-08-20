package io.github.lingqiqi5211.crashcatcher.data.daemon

import android.util.Log
import java.io.File
import java.time.Instant

/** Connection diagnostics that never include request bodies, certificates or signing pins. */
interface DaemonTrace {
    fun event(message: String)

    fun failure(message: String, cause: Throwable)
}

/** Default for tests or callers that deliberately do not persist connection diagnostics. */
internal data object NoopDaemonTrace : DaemonTrace {
    override fun event(message: String) = Unit

    override fun failure(message: String, cause: Throwable) = Unit
}

/** Debug-only logcat mirror; the supplied archive remains the durable source. */
internal class LogcatDaemonTrace(
    private val archive: DaemonTrace = NoopDaemonTrace,
) : DaemonTrace {
    override fun event(message: String) {
        Log.d(TAG, message)
        archive.event(message)
    }

    override fun failure(message: String, cause: Throwable) {
        Log.e(TAG, message, cause)
        archive.failure(message, cause)
    }

    private companion object {
        const val TAG = "CCH.Manager"
    }
}

/**
 * Small rotating log owned by the Manager, so a failed socket can be diagnosed without adb.
 *
 * Writes are best-effort: diagnostics must never become another reason the connection fails.
 * One previous file is retained so a reconnect loop cannot grow app storage without bound.
 */
internal class ManagerTraceStore(
    private val directory: File,
    private val nowMillis: () -> Long = System::currentTimeMillis,
    private val maxBytes: Long = DEFAULT_MAX_BYTES,
) : DaemonTrace {
    private val lock = Any()
    private val current = File(directory, CURRENT_NAME)
    private val previous = File(directory, PREVIOUS_NAME)

    override fun event(message: String) {
        append("DEBUG", message)
    }

    override fun failure(message: String, cause: Throwable) {
        append("ERROR", "$message\n${cause.stackTraceToString()}")
    }

    /** Files to add to the existing diagnostics zip, oldest first. */
    fun readAll(): Map<String, String> = synchronized(lock) {
        buildMap {
            read(previous)?.let { put(PREVIOUS_NAME, it) }
            read(current)?.let { put(CURRENT_NAME, it) }
        }
    }

    private fun append(level: String, message: String) {
        val safeMessage = message.take(MAX_ENTRY_CHARS)
        val line = "${Instant.ofEpochMilli(nowMillis())} $level $safeMessage\n"
        val bytes = line.toByteArray().size
        synchronized(lock) {
            runCatching {
                directory.mkdirs()
                if (current.isFile && current.length() + bytes > maxBytes) {
                    previous.delete()
                    if (!current.renameTo(previous)) {
                        current.copyTo(previous, overwrite = true)
                        current.writeText("")
                    }
                }
                current.appendText(line)
            }
        }
    }

    private fun read(file: File): String? = runCatching {
        file.takeIf(File::isFile)?.useLines { lines ->
            val retained = lines.filterNot { line ->
                line.contains(" DEBUG frame send seq=") ||
                    line.contains(" DEBUG frame received seq=")
            }.toList()
            retained.takeIf { it.isNotEmpty() }?.joinToString("\n", postfix = "\n")
        }
    }.getOrNull()

    private companion object {
        const val CURRENT_NAME = "manager.log"
        const val PREVIOUS_NAME = "manager-previous.log"
        const val DEFAULT_MAX_BYTES = 256L * 1024L
        const val MAX_ENTRY_CHARS = 32 * 1024
    }
}
