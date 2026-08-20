package io.github.lingqiqi5211.crashcatcher.ui.settings

import io.github.lingqiqi5211.crashcatcher.data.daemon.CollectorHealth
import io.github.lingqiqi5211.crashcatcher.data.daemon.CollectorSource
import io.github.lingqiqi5211.crashcatcher.data.daemon.DaemonConstants
import io.github.lingqiqi5211.crashcatcher.data.daemon.ModuleStatus
import io.github.lingqiqi5211.crashcatcher.domain.model.DeviceInfo

/**
 * The whole chain, as plain text that can be pasted into a bug report.
 *
 * Assembled in one place rather than read off the screen because the useful thing is the
 * *combination*: "no crashes recorded" is a collector that never fired, a bridge that never
 * connected, a package index that never completed, or a filter hiding them, and the four look
 * identical from outside. Anything unreachable is still printed, with what is known — a report
 * that omits the broken half is a report about the working half.
 *
 * Deliberately English keys with the values verbatim: this is meant to be pasted somewhere, and
 * a translated key makes an issue harder to read for whoever receives it.
 */
internal fun buildDiagnosticsReport(
    status: ModuleStatus?,
    device: DeviceInfo,
    connected: Boolean,
): String = buildString {
    appendLine("# CrashCatcher diagnostics")
    appendLine()

    appendLine("## Manager")
    appendLine("version: ${device.managerVersionName} (${device.managerVersionCode})")
    appendLine("protocol: ${DaemonConstants.PROTOCOL_VERSION}")
    appendLine("connected: $connected")
    appendLine()

    appendLine("## Daemon")
    if (status == null) {
        // The most common reason to be reading this page at all, so it says what is known
        // rather than nothing: the manager's own protocol number above is half of a mismatch.
        appendLine("unreachable — nothing below could be read")
        appendLine()
    } else {
        appendLine("version: ${status.daemonVersion}")
        appendLine("protocol: ${status.protocolVersion}")
        appendLine("uptime_ms: ${status.uptimeMs}")
        appendLine("pid: ${status.runtime.pid}")
        appendLine("abi: ${status.runtime.abi}")
        appendLine("android_sdk: ${status.runtime.androidSdk}")
        appendLine("selinux: ${status.runtime.selinux}")
        appendLine("store_schema: ${status.runtime.storeSchemaVersion}")
        appendLine("debug_logging: ${status.runtime.debugLogging}")
        appendLine("active_mutes: ${status.runtime.activeMutes}")
        appendLine()

        appendLine("## Bridge")
        appendLine("connected: ${status.runtime.bridge.connected}")
        appendLine("version: ${status.runtime.bridge.version ?: "-"}")
        appendLine("android_sdk: ${status.runtime.bridge.androidSdk ?: "-"}")
        appendLine()

        appendLine("## Package index")
        appendLine("packages: ${status.runtime.packageIndex.packageCount}")
        appendLine("system_flags_known: ${status.runtime.packageIndex.systemFlagsKnown}")
        appendLine()

        appendLine("## Collectors")
        for (collector in status.collectors.sortedBy { it.source.name }) {
            appendLine(collectorLine(collector))
        }
        appendLine()

        appendLine("## Dialog takeover")
        appendLine("requested: ${status.dialogTakeover.requested}")
        appendLine("effective: ${status.dialogTakeover.effective}")
        appendLine("anr_show_background_conflict: ${status.dialogTakeover.anrShowBackgroundConflict}")
        status.dialogTakeover.unsupportedReason?.let { appendLine("unsupported: $it") }
        appendLine()

        appendLine("## Storage")
        appendLine("groups: ${status.storage.groupCount}")
        appendLine("records: ${status.storage.recordCount}")
        appendLine("payload_bytes: ${status.storage.payloadBytes}")
        appendLine("database_bytes: ${status.storage.databaseBytes}")
        appendLine("evicted_payloads: ${status.storage.evictedPayloadCount}")
        appendLine()
    }

    appendLine("## Device")
    appendLine("brand: ${device.brand}")
    appendLine("manufacturer: ${device.manufacturer}")
    appendLine("model: ${device.model}")
    appendLine("android: ${device.androidRelease} (API ${device.androidApiLevel})")
    appendLine("build: ${device.buildDisplayId}")
    appendLine("fingerprint: ${device.fingerprint}")
    // Against the daemon's `abi` above: a module flashed for the wrong architecture is a real
    // failure mode, and these two lines together are what make it visible.
    appendLine("supported_abis: ${device.supportedAbis.joinToString()}")
}

private fun collectorLine(collector: CollectorHealth): String = buildString {
    append(collector.source.reportName)
    append(": enabled=${collector.enabled}")
    append(" received=${collector.everReceived}")
    collector.lastReceivedMs?.let { append(" last_ms=$it") }
    collector.detail?.let { append(" detail=$it") }
}

/** The wire name, so a report and the protocol agree on what to call a source. */
private val CollectorSource.reportName: String
    get() = when (this) {
        CollectorSource.Events -> "events"
        CollectorSource.CrashBuffer -> "crash_buffer"
        CollectorSource.Dropbox -> "dropbox"
        CollectorSource.Tombstone -> "tombstone"
        CollectorSource.AnrFile -> "anr_file"
    }
