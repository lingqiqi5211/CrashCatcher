package io.github.lingqiqi5211.crashcatcher.data.daemon

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

/**
 * Data types mirroring `cch_model` and `cch_wire::dto`.
 *
 * Field names come from [DaemonJson]'s snake-case strategy; enum and discriminator
 * values are pinned with [SerialName] because those are protocol text, not
 * derived from a Kotlin identifier.
 */

/** Time-sortable record id. Opaque: 26 Crockford base32 characters. */
@JvmInline
@Serializable
value class RecordId(val value: String) {
    override fun toString(): String = value
}

@Serializable
enum class CrashKind {
    @SerialName("java_exception")
    JavaException,

    @SerialName("anr")
    Anr,

    @SerialName("native_crash")
    NativeCrash,

    @SerialName("wtf")
    Wtf,
}

@Serializable
enum class PayloadState {
    @SerialName("present")
    Present,

    /** Stored but cut off at the per-record byte cap. */
    @SerialName("truncated")
    Truncated,

    /** Reclaimed to stay under the total byte quota; the row survives. */
    @SerialName("evicted")
    Evicted,

    /**
     * There never was one — a crash seen only in the events buffer.
     *
     * Kept apart from [Evicted] because the screens say different things: one blames the
     * retention limits for deleting a stack, and telling a user that about a record that never
     * had one sends them to change a setting that had nothing to do with it.
     */
    @SerialName("absent")
    Absent;

    val isReadable: Boolean get() = this == Present || this == Truncated
}

@Serializable
enum class PayloadCodec {
    @SerialName("raw")
    Raw,

    @SerialName("zstd")
    Zstd,
}

/**
 * Which collectors saw an occurrence, as a bitmask.
 *
 * One native crash legitimately arrives on four paths, so this is a set rather than
 * a single origin.
 */
@JvmInline
@Serializable
value class SourceMask(val bits: Int) {
    operator fun contains(other: SourceMask): Boolean = (bits and other.bits) == other.bits

    val isEmpty: Boolean get() = bits == 0

    companion object {
        val Events = SourceMask(1 shl 0)
        val CrashBuffer = SourceMask(1 shl 1)
        val Dropbox = SourceMask(1 shl 2)
        val Tombstone = SourceMask(1 shl 3)
        val AnrFile = SourceMask(1 shl 4)
    }
}

@Serializable
enum class NotifyMode {
    @SerialName("dialog")
    Dialog,

    @SerialName("notification")
    Notification,

    @SerialName("toast")
    Toast,

    @SerialName("nothing")
    Nothing,
}

@Serializable
enum class MuteScope {
    @SerialName("none")
    None,

    @SerialName("until_unlock")
    UntilUnlock,

    @SerialName("until_restart")
    UntilRestart,
}

@Serializable
enum class SortKey {
    @SerialName("last_seen_desc")
    LastSeenDesc,

    @SerialName("first_seen_desc")
    FirstSeenDesc,

    @SerialName("occurrence_desc")
    OccurrenceDesc,

    @SerialName("package_asc")
    PackageAsc,
}

@Serializable
enum class ExportFormat {
    @SerialName("text")
    Text,

    @SerialName("json")
    Json,
}

@Serializable
enum class CollectorSource {
    @SerialName("events")
    Events,

    @SerialName("crash_buffer")
    CrashBuffer,

    @SerialName("dropbox")
    Dropbox,

    @SerialName("tombstone")
    Tombstone,

    @SerialName("anr_file")
    AnrFile,
}

@Serializable
enum class WireErrorCode {
    @SerialName("invalid_request")
    InvalidRequest,

    @SerialName("malformed_frame")
    MalformedFrame,

    @SerialName("unauthorized")
    Unauthorized,

    @SerialName("not_found")
    NotFound,

    @SerialName("payload_too_large")
    PayloadTooLarge,

    @SerialName("unavailable")
    Unavailable,

    @SerialName("version_mismatch")
    VersionMismatch,

    /** The page cursor was issued for a different sort order; restart the query. */
    @SerialName("cursor_invalidated")
    CursorInvalidated,

    @SerialName("internal")
    Internal;

    /** Whether retrying the identical request could plausibly succeed. */
    val isTransient: Boolean get() = this == Unavailable || this == Internal
}

@Serializable
data class WireError(
    val code: WireErrorCode,
    /** Developer-facing detail. Never shown verbatim; the UI picks its own string. */
    val message: String = "",
) {
    override fun toString(): String = "$code: $message"
}

/**
 * An opaque page position.
 *
 * A single string on the wire precisely so it cannot be constructed by hand: echo
 * it back unchanged, or start the query over.
 */
@JvmInline
@Serializable
value class Cursor(val value: String)

@Serializable
data class Page<T>(
    val items: List<T> = emptyList(),
    /** Absent when the result set is exhausted. */
    val nextCursor: Cursor? = null,
)

@Serializable
data class CrashFilter(
    val packages: List<String> = emptyList(),
    val kinds: List<CrashKind> = emptyList(),
    val userIds: List<Int> = emptyList(),
    /** Inclusive lower bound of the half-open range `[from, to)`, in ms. */
    val timeFromMs: Long? = null,
    val timeToMs: Long? = null,
    val includeSystemApps: Boolean = false,
    /**
     * Processes that are not apps — `/vendor/bin/hw/…` and the like.
     *
     * Separate from [includeSystemApps] because they read differently: one is Settings
     * crashing, the other is a HAL, and wanting the first is not wanting to wade through
     * the second.
     */
    val includeSystemProcesses: Boolean = false,
    val onlyMainProcess: Boolean = false,
    /** Only crashes the app swallowed itself. */
    val onlySelfHandled: Boolean = false,
    /** Matches package name, summary class and summary text — not stack contents. */
    val query: String? = null,
)

@Serializable
data class PageRequest(
    val filter: CrashFilter = CrashFilter(),
    val sort: SortKey = SortKey.LastSeenDesc,
    val cursor: Cursor? = null,
    /** `0` means "use the daemon's default"; it clamps the upper bound regardless. */
    val limit: Int = 0,
)

@Serializable
data class GroupSummary(
    val groupId: String,
    val packageName: String,
    val processName: String,
    val userId: Int,
    val kind: CrashKind,
    val isSystemApp: Boolean,
    val isMainProcess: Boolean,
    val selfHandled: Boolean,
    val summaryClass: String? = null,
    val summaryText: String? = null,
    /** Total sightings, including ones whose detail rows retention has removed. */
    val occurrence: Long,
    val firstSeenMs: Long,
    val lastSeenMs: Long,
    val payloadBytes: Long,
    val mutedUntilMs: Long? = null,
    /**
     * Whether [packageName] is an installed package rather than a platform process.
     *
     * A tombstone reports its process, so native binaries arrive as
     * `/vendor/bin/hw/android.hardware.audio.service_64`. None of an app's affordances — icon,
     * label, launch, per-app notification settings — mean anything for one of those.
     */
    val packageInstalled: Boolean = true,
)

@Serializable
data class RecordSummary(
    val id: RecordId,
    val groupId: String,
    val happenedAtMs: Long,
    val pid: Int,
    val sources: SourceMask,
    val appVersionName: String? = null,
    val appVersionCode: Long? = null,
    val isForeground: Boolean? = null,
    val isRepeating: Boolean,
    /** Sibling reports Android's dropbox rate limiter discarded. */
    val droppedCount: Int? = null,
    val payloadBytes: Long,
    val payloadState: PayloadState,
)

@Serializable
data class RecordDetail(
    val record: RecordSummary,
    val group: GroupSummary,
)

@Serializable
data class CollectorHealth(
    val source: CollectorSource,
    val enabled: Boolean,
    /**
     * Whether this collector has ever produced a row.
     *
     * The field the status card leads with: a collector can be enabled, report no
     * error, and still never have seen anything.
     */
    val everReceived: Boolean,
    val lastReceivedMs: Long? = null,
    val detail: String? = null,
)

@Serializable
data class DialogTakeoverStatus(
    val requested: Boolean,
    val effective: Boolean,
    /** `anr_show_background` is on, which overrides suppression. */
    val anrShowBackgroundConflict: Boolean,
    val unsupportedReason: String? = null,
)

@Serializable
data class StorageStatus(
    val groupCount: Long = 0,
    val recordCount: Long = 0,
    val payloadBytes: Long = 0,
    val databaseBytes: Long = 0,
    val evictedPayloadCount: Long = 0,
)

@Serializable
data class ModuleStatus(
    val daemonVersion: String,
    val protocolVersion: Int,
    val uptimeMs: Long,
    val collectors: List<CollectorHealth> = emptyList(),
    /** The privileged bridge is connected, so notifications are immediate. */
    val bridgeConnected: Boolean,
    val dialogTakeover: DialogTakeoverStatus,
    val storage: StorageStatus = StorageStatus(),
    val runtime: RuntimeFacts,
)

/**
 * What the daemon can say about its own health.
 *
 * Read as a whole on the diagnostics page: a collector that never fired, a bridge that never
 * connected and a package index that never completed all present as "it is not recording", and
 * are told apart only by seeing all three together.
 */
@Serializable
data class RuntimeFacts(
    val pid: Int,
    /** The ABI this daemon was built for, not necessarily the device's preferred one. */
    val abi: String,
    val androidSdk: Int,
    /** `enforcing`, `permissive`, or `unknown`. */
    val selinux: String,
    val storeSchemaVersion: Long,
    val debugLogging: Boolean,
    val packageIndex: PackageIndexFacts,
    val bridge: BridgeFacts,
    /** Apps silenced right now — the first thing to check when notifications stopped. */
    val activeMutes: Int,
)

@Serializable
data class PackageIndexFacts(
    val packageCount: Int,
    /**
     * Whether the system-app flags came from PackageManager.
     *
     * False means the index predates it answering, and while it holds every app looks
     * third-party — which is what makes "record system apps" appear to do nothing.
     */
    val systemFlagsKnown: Boolean,
)

/**
 * One readable log file.
 *
 * [name] is what a read request takes back, `old/` prefixed for the previous boot's copies. Not
 * a path: the daemon resolves only the shapes it produced.
 */
@Serializable
data class RuntimeLogFile(
    val name: String,
    val bytes: Long,
    val modifiedMs: Long,
)

@Serializable
data class BridgeFacts(
    val connected: Boolean,
    /** From the bridge's own hello, so a mismatch with the daemon means a stale dex. */
    val version: String? = null,
    val androidSdk: Int? = null,
)

@Serializable
data class RetentionPolicy(
    val retentionDays: Int,
    val maxRecordsPerGroup: Int,
    val maxRecordsTotal: Int,
    val maxPayloadBytesTotal: Long,
    val maxPayloadBytesPerRecord: Long,
)

@Serializable
data class GlobalConfig(
    val enabled: Boolean,
    val captureJava: Boolean,
    val captureAnr: Boolean,
    val captureNative: Boolean,
    val captureWtf: Boolean,
    val captureSelfHandled: Boolean,
    val notifyMode: NotifyMode,
    val onlyForeground: Boolean,
    /** What `onlyForeground` does when the foreground state is unknown. */
    val foregroundUnknownNotifies: Boolean,
    val onlyMainProcess: Boolean,
    val includeSystemApps: Boolean,
    val takeoverSystemDialog: Boolean,
    /** Log at debug level. Meant to be on only while reproducing something. */
    val debugLogging: Boolean = false,
    val retention: RetentionPolicy,
)

@Serializable
data class AppConfig(
    /** `null` follows the global mode. */
    val notifyMode: NotifyMode? = null,
    val ignore: Boolean = false,
    val mute: MuteScope = MuteScope.None,
)

@Serializable
data class AppEntry(
    val packageName: String,
    val label: String? = null,
    val userId: Int,
    val isSystemApp: Boolean,
    val groupCount: Long,
    val occurrence: Long,
    val lastSeenMs: Long? = null,
    val config: AppConfig = AppConfig(),
    /** False when this is a platform process rather than an app; see [GroupSummary]. */
    val packageInstalled: Boolean = true,
)

@Serializable
data class KindCount(val kind: CrashKind, val count: Long)

@Serializable
data class PackageCount(val packageName: String, val label: String? = null, val count: Long)

@Serializable
data class ExceptionCount(val className: String, val count: Long)

@Serializable
data class TrendBucket(val fromMs: Long, val count: Long)

@Serializable
data class Stats(
    val total: Long,
    val byKind: List<KindCount> = emptyList(),
    val topPackages: List<PackageCount> = emptyList(),
    val topExceptions: List<ExceptionCount> = emptyList(),
    val trend: List<TrendBucket> = emptyList(),
    val crashedAppCount: Long = 0,
    val installedAppCount: Long = 0,
)

@Serializable
data class PayloadOpened(
    val totalBytes: Long,
    val state: PayloadState,
    val codecOnDisk: PayloadCodec,
    /**
     * A read-only descriptor arrived with this frame over `SCM_RIGHTS`.
     *
     * When true, stream that instead of calling `read_payload`: it skips framing,
     * JSON escaping and the frame size limit entirely.
     */
    val fdAttached: Boolean,
    val handle: Long? = null,
)

@Serializable
data class PayloadChunk(
    val offset: Long,
    val text: String,
    /** Pass as `offset` on the next call; may be short of `offset + len`. */
    val nextOffset: Long,
    val eof: Boolean,
)

@Serializable
data class ExportRedaction(
    val hideDeviceBrand: Boolean = false,
    val hideDeviceModel: Boolean = false,
    val hideBuildDisplayId: Boolean = false,
    val hidePackageName: Boolean = false,
) {
    val hidesAnything: Boolean
        get() = hideDeviceBrand || hideDeviceModel || hideBuildDisplayId || hidePackageName
}

@Serializable
data class GlobalConfigResult(
    val config: GlobalConfig,
    /** Clamping changed something the client asked for. */
    val adjusted: Boolean = false,
)

@Serializable
data class AppConfigResult(val packageName: String, val config: AppConfig)

@Serializable
data class DialogTakeoverResult(val status: DialogTakeoverStatus)

@Serializable
data class MuteResult(
    val packageName: String,
    val scope: MuteScope,
    val mutedUntilMs: Long? = null,
)
