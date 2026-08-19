package io.github.lingqiqi5211.crashcatcher.ui.util

/**
 * The last segment of a dotted name, for use as a heading.
 *
 * A large title is set at display size and wraps at the character, so a
 * fully-qualified name lands as `java.lang.IllegalStateExc` / `eption` — two lines of
 * mostly `java.lang.` and a word cut in half. The part that identifies the crash is
 * always the last segment, and the prefix is still shown as the subtitle, so nothing
 * is lost by leading with `IllegalStateException`.
 *
 * Names with no dot, and trailing-dot oddities, come back unchanged rather than empty.
 */
internal fun shortTypeName(qualified: String): String {
    val trimmed = qualified.trim()
    val lastDot = trimmed.lastIndexOf('.')
    if (lastDot < 0 || lastDot == trimmed.lastIndex) return trimmed
    return trimmed.substring(lastDot + 1)
}

/**
 * How a process is best identified in one line.
 *
 * Android names a secondary process `com.example:remote`, and that suffix is the whole
 * point when an app crashes in a background process rather than its main one: two
 * crashes with the same package and the same exception are different bugs if they came
 * from different processes. Only the suffix is shown, because the package is already
 * the heading's neighbour — but the colon is kept, since a badge reading `tag` says
 * nothing while `:tag` is recognisably a process name.
 *
 * Returns null when there is nothing to add — a main-process crash, where the process
 * name equals the package name, or where the daemon could not resolve one.
 */
internal fun processSuffix(packageName: String, processName: String?): String? {
    val process = processName?.trim()?.takeIf { it.isNotEmpty() } ?: return null
    if (process == packageName) return null
    val suffix = process.removePrefix("$packageName:")
    // A process outside this package's namespace is shown whole; there is no suffix to
    // take, and truncating it would misattribute the crash.
    return if (suffix == process) process else ":$suffix"
}
