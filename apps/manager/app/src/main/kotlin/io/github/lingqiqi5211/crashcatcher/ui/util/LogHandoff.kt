package io.github.lingqiqi5211.crashcatcher.ui.util

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.core.content.FileProvider
import java.io.File
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream

/**
 * Handing a crash log to somewhere outside the app.
 *
 * The two ways out differ on purpose. Copying is for a fragment someone is about to paste
 * into a message they are writing, so it goes to the clipboard as text. Sharing is for the
 * whole report going to someone else, so it goes as a file — hundreds of lines pasted into
 * a chat are unreadable, and a payload that is not a few lines but a few megabytes (an ANR
 * dump carries every thread in the process) does not fit in a binder transaction anyway.
 */

/** Puts [text] on the clipboard. False if the system refused it. */
internal fun copyLog(context: Context, label: String, text: String): Boolean {
    val clipboard = context.getSystemService(ClipboardManager::class.java) ?: return false
    // A clip travels to the system through a binder transaction like anything else, so a
    // large enough trace throws rather than being truncated. The caller reports it; a
    // silently empty clipboard would be worse.
    return runCatching { clipboard.setPrimaryClip(ClipData.newPlainText(label, text)) }.isSuccess
}

/**
 * Offers [text] to another app as a `.txt` file. False if nothing could receive it.
 *
 * Always a file, never `EXTRA_TEXT`. A stack trace is hundreds of lines: pasted into a
 * chat it becomes a wall nobody can scroll past and every client wraps differently, while
 * a file arrives named, foldable, and byte-for-byte what the log said. It also sidesteps
 * the size question entirely — a tombstone or an ANR dump overflows a binder transaction,
 * and `EXTRA_TEXT` does not truncate at that point, it throws.
 */
internal fun shareLog(context: Context, subject: String, text: String): Boolean {
    val uri = writeShareFile(context, subject, text) ?: return false

    val send = Intent(Intent.ACTION_SEND).apply {
        type = MIME_TYPE
        putExtra(Intent.EXTRA_SUBJECT, subject)
        putExtra(Intent.EXTRA_STREAM, uri)
        // Both: the chooser target is not known ahead of time, so the grant travels with
        // the intent, and the clip data is what several receivers actually read the
        // attachment from.
        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        clipData = ClipData.newRawUri(subject, uri)
    }

    return runCatching { context.startActivity(Intent.createChooser(send, null)) }.isSuccess
}

/**
 * Offers several files as one `.zip`. False if nothing could receive it.
 *
 * The diagnostics hand-off is a report plus up to eighteen rotated logs. As separate attachments
 * most targets take only the first; as one concatenated text file the boundaries are lost.
 */
internal fun shareArchive(
    context: Context,
    subject: String,
    entries: Map<String, String>,
): Boolean {
    val uri = writeArchive(context, subject, entries) ?: return false

    val send = Intent(Intent.ACTION_SEND).apply {
        type = ARCHIVE_MIME_TYPE
        putExtra(Intent.EXTRA_SUBJECT, subject)
        putExtra(Intent.EXTRA_STREAM, uri)
        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        clipData = ClipData.newRawUri(subject, uri)
    }

    return runCatching { context.startActivity(Intent.createChooser(send, null)) }.isSuccess
}

private fun writeArchive(
    context: Context,
    subject: String,
    entries: Map<String, String>,
): Uri? = runCatching {
    val directory = File(context.cacheDir, SHARE_DIRECTORY)
    directory.listFiles()?.forEach { it.delete() }
    directory.mkdirs()

    val file = File(directory, shareFileName(subject).removeSuffix(".txt") + ".zip")
    ZipOutputStream(file.outputStream().buffered()).use { zip ->
        for ((name, text) in entries) {
            // Flattened: `old/daemon.log` would otherwise become a directory in the archive,
            // and some viewers show only the top level.
            zip.putNextEntry(ZipEntry(name.replace('/', '-')))
            zip.write(text.toByteArray())
            zip.closeEntry()
        }
    }

    FileProvider.getUriForFile(context, "${context.packageName}$SHARE_AUTHORITY_SUFFIX", file)
}.getOrNull()

/**
 * Writes the log where a [FileProvider] can serve it, and returns its content Uri.
 *
 * Previous hand-offs are cleared first rather than left to accumulate: they are one-shot
 * copies, and by the time another share happens the last receiver has long since read
 * whatever it was going to read.
 */
private fun writeShareFile(context: Context, subject: String, text: String): Uri? = runCatching {
    val directory = File(context.cacheDir, SHARE_DIRECTORY)
    directory.listFiles()?.forEach { it.delete() }
    directory.mkdirs()

    val file = File(directory, shareFileName(subject))
    file.writeText(text)

    FileProvider.getUriForFile(context, "${context.packageName}$SHARE_AUTHORITY_SUFFIX", file)
}.getOrNull()

/** A file name a receiving app can show, derived from what the share is called. */
private fun shareFileName(subject: String): String {
    val safe = subject
        .map { character -> if (character.isLetterOrDigit()) character else '-' }
        .joinToString(separator = "")
        .trim('-')
        .take(MAX_NAME_LENGTH)
        .ifEmpty { "crash" }
    return "$safe.txt"
}

private const val MIME_TYPE = "text/plain"

private const val ARCHIVE_MIME_TYPE = "application/zip"

private const val SHARE_DIRECTORY = "shares"

/** Must match `android:authorities` on the provider in the manifest. */
private const val SHARE_AUTHORITY_SUFFIX = ".shares"

private const val MAX_NAME_LENGTH = 48
