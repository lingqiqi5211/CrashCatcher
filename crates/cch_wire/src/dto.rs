use cch_config::{AppConfig, GlobalConfig, MuteScope};
use cch_model::{CrashKind, PayloadCodec, PayloadState, RecordId, SourceMask};
use serde::{Deserialize, Serialize};

/// One row of the crash list.
///
/// Everything here comes from a single-table query against `crash_group` — no
/// join, no payload read. That is what keeps opening the list instant no matter
/// how much history exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupSummary {
    pub group_id: String,
    pub package_name: String,
    pub process_name: String,
    pub user_id: i32,
    pub kind: CrashKind,
    pub is_system_app: bool,
    pub is_main_process: bool,
    pub self_handled: bool,
    pub summary_class: Option<String>,
    pub summary_text: Option<String>,
    /// Total times this crash was seen, including occurrences whose detail rows
    /// retention has since removed.
    pub occurrence: u64,
    pub first_seen_ms: i64,
    pub last_seen_ms: i64,
    pub payload_bytes: u64,
    pub muted_until_ms: Option<i64>,
    /// Whether `package_name` is an installed package rather than a platform process.
    ///
    /// False for the native binaries a tombstone reports by path — the manager labels those
    /// separately, since none of an app's affordances (icon, label, launch, per-app settings)
    /// mean anything for `/vendor/bin/hw/android.hardware.audio.service_64`.
    #[serde(default = "default_true")]
    pub package_installed: bool,
}

const fn default_true() -> bool {
    true
}

/// One occurrence inside a group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordSummary {
    pub id: RecordId,
    pub group_id: String,
    pub happened_at_ms: i64,
    pub pid: i32,
    pub sources: SourceMask,
    pub app_version_name: Option<String>,
    pub app_version_code: Option<i64>,
    pub is_foreground: Option<bool>,
    pub is_repeating: bool,
    /// How many sibling reports Android's dropbox rate limiter discarded.
    pub dropped_count: Option<u32>,
    pub payload_bytes: u64,
    pub payload_state: PayloadState,
}

/// A record plus its group, for the detail screen's header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordDetail {
    pub record: RecordSummary,
    pub group: GroupSummary,
}

/// Where a collector gets its data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollectorSource {
    Events,
    CrashBuffer,
    Dropbox,
    Tombstone,
    AnrFile,
}

impl CollectorSource {
    pub const ALL: [Self; 5] = [
        Self::Events,
        Self::CrashBuffer,
        Self::Dropbox,
        Self::Tombstone,
        Self::AnrFile,
    ];
}

/// Per-collector liveness.
///
/// `ever_received` is the field that matters. A collector can be enabled, hold no
/// error, and still never have produced a row — that is exactly the failure the
/// reference implementation hides behind a green "activated" badge, so the UI
/// surfaces it directly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectorHealth {
    pub source: CollectorSource,
    pub enabled: bool,
    pub ever_received: bool,
    pub last_received_ms: Option<i64>,
    /// Why this collector is degraded, when it is.
    pub detail: Option<String>,
}

/// State of the `hide_error_dialogs` takeover.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DialogTakeoverStatus {
    /// What the config asks for.
    pub requested: bool,
    /// What the system setting actually reads back as.
    pub effective: bool,
    /// `Settings.Secure.anr_show_background` is on, which overrides suppression.
    pub anr_show_background_conflict: bool,
    /// Set when this Android version cannot support the takeover at all.
    pub unsupported_reason: Option<String>,
}

/// Storage occupancy, for the home screen and the settings page.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageStatus {
    pub group_count: u64,
    pub record_count: u64,
    pub payload_bytes: u64,
    pub database_bytes: u64,
    /// Records whose payload was reclaimed to stay under quota.
    pub evicted_payload_count: u64,
}

/// Everything the home screen's status card needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleStatus {
    pub daemon_version: String,
    pub protocol_version: u32,
    pub uptime_ms: i64,
    pub collectors: Vec<CollectorHealth>,
    /// The privileged Java bridge is connected, so notifications are immediate.
    pub bridge_connected: bool,
    pub dialog_takeover: DialogTakeoverStatus,
    pub storage: StorageStatus,
    pub runtime: RuntimeFacts,
}

/// What the daemon can say about its own health, for someone working out why something is not
/// working.
///
/// Gathered into one answer rather than spread over several requests because the useful reading
/// is the whole chain at one instant: a collector that never fired, a bridge that never
/// connected and a package index that never completed produce the same symptom — "it is not
/// recording" — and are told apart only by looking at all three at once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeFacts {
    pub pid: u32,
    /// The ABI this daemon was built for, which is not necessarily the one the device prefers.
    pub abi: String,
    /// The SDK level the daemon read, which is what its platform-specific paths key off.
    pub android_sdk: u32,
    /// `enforcing`, `permissive`, or `unknown` where it could not be read. Descriptor passing
    /// and several collector paths behave differently under enforcing, so it belongs in a
    /// report about why something did not arrive.
    pub selinux: String,
    pub store_schema_version: i64,
    /// Whether the daemon is currently logging at debug level.
    pub debug_logging: bool,
    pub package_index: PackageIndexFacts,
    pub bridge: BridgeFacts,
    /// Apps silenced right now, which is the first thing to check when notifications stopped.
    pub active_mutes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageIndexFacts {
    pub package_count: u32,
    /// Whether the system-app flags came from PackageManager.
    ///
    /// False means the index was built before it was answering — the normal state for the first
    /// seconds after boot — and while it holds, every app looks third-party and the
    /// "record system apps" setting appears to do nothing.
    pub system_flags_known: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeFacts {
    pub connected: bool,
    /// From the bridge's own hello, so a mismatch with the daemon means a stale dex.
    pub version: Option<String>,
    /// The SDK the bridge sees, which is a different process's view of the same device.
    pub android_sdk: Option<u32>,
}

/// An installed app, with how much it has crashed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppEntry {
    pub package_name: String,
    pub label: Option<String>,
    pub user_id: i32,
    pub is_system_app: bool,
    pub group_count: u64,
    pub occurrence: u64,
    pub last_seen_ms: Option<i64>,
    /// The override in force, if any.
    pub config: AppConfig,
    /// False when this is a platform process rather than an app; see
    /// [`GroupSummary::package_installed`].
    #[serde(default = "default_true")]
    pub package_installed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KindCount {
    pub kind: CrashKind,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageCount {
    pub package_name: String,
    pub label: Option<String>,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExceptionCount {
    pub class_name: String,
    pub count: u64,
}

/// One bucket of the trend chart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrendBucket {
    /// Start of the bucket, in ms.
    pub from_ms: i64,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stats {
    pub total: u64,
    pub by_kind: Vec<KindCount>,
    pub top_packages: Vec<PackageCount>,
    pub top_exceptions: Vec<ExceptionCount>,
    pub trend: Vec<TrendBucket>,
    /// Installed apps that have crashed at least once, over total installed —
    /// the reference implementation's headline statistic.
    pub crashed_app_count: u64,
    pub installed_app_count: u64,
}

/// Result of `open_payload`.
///
/// When `fd_attached` is true the frame arrived with a read-only descriptor over
/// `SCM_RIGHTS` and the client should stream that instead of issuing
/// `read_payload`. The chunked path exists only for hosts where fd passing is
/// unavailable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayloadOpened {
    pub total_bytes: u64,
    pub state: PayloadState,
    pub codec_on_disk: PayloadCodec,
    pub fd_attached: bool,
    /// Handle for `read_payload`, present when `fd_attached` is false.
    pub handle: Option<u64>,
}

/// Largest chunk the fallback read path will return.
///
/// Well under the frame limit so JSON escaping of the text cannot push a reply
/// over it.
pub const MAX_PAYLOAD_CHUNK_BYTES: u32 = 256 * 1024;

/// A chunk from the fallback read path.
///
/// Payloads are UTF-8 text (a stack trace, a rendered tombstone, an ANR dump), so
/// chunks carry a string rather than bytes — no base64 inflation, and the detail
/// screen can render what it receives directly.
///
/// The daemon moves the boundary back to the nearest character boundary and
/// reports where it actually stopped, so a multi-byte character is never split
/// across two chunks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayloadChunk {
    pub offset: u64,
    pub text: String,
    /// Offset to pass as `offset` on the next call.
    pub next_offset: u64,
    pub eof: bool,
}

/// What an export should look like.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Text,
    Json,
}

/// Which identifying details to strip before sharing.
///
/// Defaults keep everything, so an accidental omission cannot silently leak more
/// than the user chose — the UI opts *out* field by field.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportRedaction {
    pub hide_device_brand: bool,
    pub hide_device_model: bool,
    pub hide_build_display_id: bool,
    pub hide_package_name: bool,
}

impl ExportRedaction {
    #[must_use]
    pub const fn hides_anything(&self) -> bool {
        self.hide_device_brand
            || self.hide_device_model
            || self.hide_build_display_id
            || self.hide_package_name
    }
}

/// What a delete request targets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "target", rename_all = "snake_case")]
pub enum DeleteTarget {
    Ids {
        ids: Vec<RecordId>,
    },
    Group {
        group_id: String,
    },
    /// Everything. Named explicitly so it can never be the result of an empty list.
    All,
}

/// Result of a config write: what was actually stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalConfigResult {
    pub config: GlobalConfig,
    /// True when clamping changed a value the client asked for, so the UI can
    /// show the corrected number rather than silently disagreeing with itself.
    pub adjusted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfigResult {
    pub package_name: String,
    pub config: AppConfig,
}

/// Result of asking for the dialog takeover.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DialogTakeoverResult {
    pub status: DialogTakeoverStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MuteResult {
    pub package_name: String,
    pub scope: MuteScope,
    pub muted_until_ms: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delete_target_all_is_explicit_on_the_wire() {
        assert_eq!(
            serde_json::to_string(&DeleteTarget::All).expect("serializes"),
            r#"{"target":"all"}"#
        );
        // An empty id list must not be confusable with "everything".
        let empty = DeleteTarget::Ids { ids: Vec::new() };
        assert_ne!(
            serde_json::to_string(&empty).expect("serializes"),
            r#"{"target":"all"}"#
        );
    }

    #[test]
    fn redaction_defaults_to_hiding_nothing() {
        let redaction = ExportRedaction::default();
        assert!(!redaction.hides_anything());
        let parsed: ExportRedaction = serde_json::from_str("{}").expect("empty object");
        assert_eq!(parsed, redaction);
    }

    #[test]
    fn every_collector_source_has_a_distincch_wire_name() {
        let mut names: Vec<String> = CollectorSource::ALL
            .iter()
            .map(|source| serde_json::to_string(source).expect("serializes"))
            .collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), CollectorSource::ALL.len());
    }
}
