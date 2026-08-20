use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Current on-disk schema version.
///
/// Bumped only when a migration is needed; additive fields ride on serde
/// defaults instead.
pub const CONFIG_SCHEMA_VERSION: u32 = 1;

/// How the user is told about a crash.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotifyMode {
    /// Full-screen detail activity, launched by the privileged bridge.
    Dialog,
    #[default]
    Notification,
    Toast,
    /// Record silently.
    Nothing,
}

/// How long an app's crashes stay muted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MuteScope {
    #[default]
    None,
    /// Cleared on `ACTION_USER_PRESENT`.
    UntilUnlock,
    /// Cleared on boot; held in memory only.
    UntilRestart,
}

/// The four storage ceilings, plus the per-record cap.
///
/// Evaluated in field order by the store: days, then per-group count, then total
/// count, then total bytes. Only the byte quota degrades gracefully — it drops
/// payloads and keeps the metadata rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RetentionPolicy {
    pub retention_days: u32,
    pub max_records_per_group: u32,
    pub max_records_total: u32,
    pub max_payload_bytes_total: u64,
    pub max_payload_bytes_per_record: u64,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            retention_days: 30,
            max_records_per_group: 20,
            max_records_total: 2_000,
            max_payload_bytes_total: 256 * 1024 * 1024,
            max_payload_bytes_per_record: 2 * 1024 * 1024,
        }
    }
}

impl RetentionPolicy {
    /// Smallest values that still leave the tool useful.
    pub const MIN_RETENTION_DAYS: u32 = 1;
    pub const MIN_RECORDS_PER_GROUP: u32 = 1;
    pub const MIN_RECORDS_TOTAL: u32 = 50;
    pub const MIN_PAYLOAD_BYTES_TOTAL: u64 = 8 * 1024 * 1024;
    pub const MIN_PAYLOAD_BYTES_PER_RECORD: u64 = 64 * 1024;

    /// Largest values the UI offers, so a slider cannot ask for something absurd.
    pub const MAX_RETENTION_DAYS: u32 = 365;
    pub const MAX_RECORDS_PER_GROUP: u32 = 500;
    pub const MAX_RECORDS_TOTAL: u32 = 100_000;
    pub const MAX_PAYLOAD_BYTES_TOTAL: u64 = 8 * 1024 * 1024 * 1024;
    pub const MAX_PAYLOAD_BYTES_PER_RECORD: u64 = 64 * 1024 * 1024;

    /// Pulls every field into its supported range.
    ///
    /// Clamping rather than rejecting: a config file hand-edited to something
    /// silly should still boot the daemon with sane behaviour, and the UI never
    /// needs a validation error path for its own sliders.
    #[must_use]
    pub fn clamped(self) -> Self {
        Self {
            retention_days: self
                .retention_days
                .clamp(Self::MIN_RETENTION_DAYS, Self::MAX_RETENTION_DAYS),
            max_records_per_group: self
                .max_records_per_group
                .clamp(Self::MIN_RECORDS_PER_GROUP, Self::MAX_RECORDS_PER_GROUP),
            max_records_total: self
                .max_records_total
                .clamp(Self::MIN_RECORDS_TOTAL, Self::MAX_RECORDS_TOTAL),
            max_payload_bytes_total: self
                .max_payload_bytes_total
                .clamp(Self::MIN_PAYLOAD_BYTES_TOTAL, Self::MAX_PAYLOAD_BYTES_TOTAL),
            max_payload_bytes_per_record: self.max_payload_bytes_per_record.clamp(
                Self::MIN_PAYLOAD_BYTES_PER_RECORD,
                Self::MAX_PAYLOAD_BYTES_PER_RECORD,
            ),
        }
    }
}

/// Settings that apply unless an app overrides them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GlobalConfig {
    /// Master switch. When off the daemon still serves history but records nothing.
    pub enabled: bool,
    pub capture_java: bool,
    pub capture_anr: bool,
    pub capture_native: bool,
    pub capture_wtf: bool,
    /// Record crashes an app handled itself. On by default — this is the class of
    /// event no comparable tool surfaces.
    pub capture_self_handled: bool,
    pub notify_mode: NotifyMode,
    pub only_foreground: bool,
    /// Decides `only_foreground` when the foreground state could not be
    /// established. Defaults to notifying: a missed crash is worse than a
    /// slightly noisy one, and pure-events records legitimately lack the flag.
    pub foreground_unknown_notifies: bool,
    pub only_main_process: bool,
    pub include_system_apps: bool,
    /// Ask the daemon to set `Settings.Global.hide_error_dialogs`.
    ///
    /// Off by default: it also changes behaviour — a crash proceeds as if the user
    /// had pressed "force quit", and an ANR kills the app outright instead of
    /// offering "wait".
    pub takeover_system_dialog: bool,
    /// Log at debug level instead of info.
    ///
    /// Off by default and meant to be turned on only while reproducing something: the daemon
    /// writes to a file nothing rotates, and the interesting paths — every screen event, every
    /// package lookup — are exactly the ones that fire constantly.
    pub debug_logging: bool,
    pub retention: RetentionPolicy,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            capture_java: true,
            capture_anr: true,
            capture_native: true,
            capture_wtf: false,
            capture_self_handled: true,
            notify_mode: NotifyMode::default(),
            only_foreground: false,
            foreground_unknown_notifies: true,
            only_main_process: false,
            include_system_apps: false,
            takeover_system_dialog: false,
            debug_logging: false,
            retention: RetentionPolicy::default(),
        }
    }
}

impl GlobalConfig {
    /// Normalizes anything a hand-edited file could have got wrong.
    #[must_use]
    pub fn normalized(mut self) -> Self {
        self.retention = self.retention.clamped();
        self
    }
}

/// Per-app overrides. Every field falls back to [`GlobalConfig`] when unset.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// `None` follows the global mode.
    pub notify_mode: Option<NotifyMode>,
    /// Do not record this app at all.
    pub ignore: bool,
    pub mute: MuteScope,
}

impl AppConfig {
    /// True when this override carries no information and can be dropped.
    ///
    /// Keeps the config file from accumulating an entry per app the user merely
    /// visited in the picker.
    #[must_use]
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

/// The whole persisted document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConfigDocument {
    pub schema_version: u32,
    pub global: GlobalConfig,
    /// Keyed by package name. `BTreeMap` so the file has a stable, diffable order.
    pub apps: BTreeMap<String, AppConfig>,
}

impl Default for ConfigDocument {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            global: GlobalConfig::default(),
            apps: BTreeMap::new(),
        }
    }
}

impl ConfigDocument {
    /// Clamps values and drops no-op app entries.
    #[must_use]
    pub fn normalized(mut self) -> Self {
        self.schema_version = CONFIG_SCHEMA_VERSION;
        self.global = self.global.normalized();
        self.apps.retain(|_, config| !config.is_default());
        self
    }

    /// The override for `package`, or the default when there is none.
    #[must_use]
    pub fn app(&self, package: &str) -> AppConfig {
        self.apps.get(package).cloned().unwrap_or_default()
    }

    /// The notify mode actually in force for `package`.
    #[must_use]
    pub fn effective_notify_mode(&self, package: &str) -> NotifyMode {
        let app = self.app(package);
        if app.ignore {
            return NotifyMode::Nothing;
        }
        app.notify_mode.unwrap_or(self.global.notify_mode)
    }

    /// Whether an occurrence should be recorded at all.
    #[must_use]
    pub fn should_record(
        &self,
        package: &str,
        is_system_app: bool,
        is_main_process: bool,
        self_handled: bool,
    ) -> bool {
        if !self.global.enabled || self.app(package).ignore {
            return false;
        }
        if is_system_app && !self.global.include_system_apps {
            return false;
        }
        if self.global.only_main_process && !is_main_process {
            return false;
        }
        if self_handled && !self.global.capture_self_handled {
            return false;
        }
        true
    }

    /// Whether the user should be told, given the recorded foreground state.
    #[must_use]
    pub fn should_notify(&self, package: &str, is_foreground: Option<bool>) -> bool {
        if self.effective_notify_mode(package) == NotifyMode::Nothing {
            return false;
        }
        if !self.global.only_foreground {
            return true;
        }
        match is_foreground {
            Some(foreground) => foreground,
            None => self.global.foreground_unknown_notifies,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_conservative_where_it_matters() {
        let global = GlobalConfig::default();
        // Changing the device's crash-dialog behaviour is opt-in.
        assert!(!global.takeover_system_dialog);
        // Platform noise stays out of the list until asked for.
        assert!(!global.include_system_apps);
        // But the capability nothing else has is on.
        assert!(global.capture_self_handled);
    }

    #[test]
    fn retention_is_clamped_in_both_directions() {
        let absurd = RetentionPolicy {
            retention_days: 0,
            max_records_per_group: 0,
            max_records_total: 1,
            max_payload_bytes_total: 1,
            max_payload_bytes_per_record: 1,
        }
        .clamped();
        assert_eq!(absurd.retention_days, RetentionPolicy::MIN_RETENTION_DAYS);
        assert_eq!(absurd.max_records_total, RetentionPolicy::MIN_RECORDS_TOTAL);
        assert_eq!(
            absurd.max_payload_bytes_total,
            RetentionPolicy::MIN_PAYLOAD_BYTES_TOTAL
        );

        let huge = RetentionPolicy {
            retention_days: u32::MAX,
            max_records_per_group: u32::MAX,
            max_records_total: u32::MAX,
            max_payload_bytes_total: u64::MAX,
            max_payload_bytes_per_record: u64::MAX,
        }
        .clamped();
        assert_eq!(huge.retention_days, RetentionPolicy::MAX_RETENTION_DAYS);
        assert_eq!(
            huge.max_payload_bytes_per_record,
            RetentionPolicy::MAX_PAYLOAD_BYTES_PER_RECORD
        );
    }

    #[test]
    fn defaults_survive_clamping_unchanged() {
        let default = RetentionPolicy::default();
        assert_eq!(default.clamped(), default);
    }

    #[test]
    fn ignore_beats_every_other_notify_setting() {
        let mut document = ConfigDocument::default();
        document.apps.insert(
            "com.example.app".to_owned(),
            AppConfig {
                notify_mode: Some(NotifyMode::Dialog),
                ignore: true,
                mute: MuteScope::None,
            },
        );
        assert_eq!(
            document.effective_notify_mode("com.example.app"),
            NotifyMode::Nothing
        );
        assert!(!document.should_record("com.example.app", false, true, false));
    }

    #[test]
    fn per_app_mode_overrides_the_global_one() {
        let mut document = ConfigDocument::default();
        document.global.notify_mode = NotifyMode::Toast;
        document.apps.insert(
            "com.example.app".to_owned(),
            AppConfig {
                notify_mode: Some(NotifyMode::Dialog),
                ..AppConfig::default()
            },
        );
        assert_eq!(
            document.effective_notify_mode("com.example.app"),
            NotifyMode::Dialog
        );
        assert_eq!(
            document.effective_notify_mode("com.other.app"),
            NotifyMode::Toast
        );
    }

    #[test]
    fn unknown_foreground_state_follows_its_own_setting() {
        let mut document = ConfigDocument::default();
        document.global.only_foreground = true;

        document.global.foreground_unknown_notifies = true;
        assert!(document.should_notify("com.example.app", None));
        assert!(document.should_notify("com.example.app", Some(true)));
        assert!(!document.should_notify("com.example.app", Some(false)));

        document.global.foreground_unknown_notifies = false;
        assert!(!document.should_notify("com.example.app", None));
    }

    #[test]
    fn recording_respects_master_switch_and_scope_filters() {
        let mut document = ConfigDocument::default();
        assert!(document.should_record("com.example.app", false, true, false));

        // System app hidden by default.
        assert!(!document.should_record("com.android.systemui", true, true, false));
        document.global.include_system_apps = true;
        assert!(document.should_record("com.android.systemui", true, true, false));

        document.global.only_main_process = true;
        assert!(!document.should_record("com.example.app", false, false, false));

        document.global.capture_self_handled = false;
        assert!(!document.should_record("com.example.app", false, true, true));

        document.global.enabled = false;
        assert!(!document.should_record("com.example.app", false, true, false));
    }

    #[test]
    fn normalizing_drops_app_entries_that_say_nothing() {
        let mut document = ConfigDocument::default();
        document
            .apps
            .insert("com.noop.app".to_owned(), AppConfig::default());
        document.apps.insert(
            "com.real.app".to_owned(),
            AppConfig {
                ignore: true,
                ..AppConfig::default()
            },
        );
        let normalized = document.normalized();
        assert!(!normalized.apps.contains_key("com.noop.app"));
        assert!(normalized.apps.contains_key("com.real.app"));
    }

    #[test]
    fn an_empty_document_deserializes_to_defaults() {
        let document: ConfigDocument = serde_json::from_str("{}").expect("empty object is valid");
        assert_eq!(document, ConfigDocument::default());
    }

    #[test]
    fn a_partial_document_keeps_defaults_for_absent_fields() {
        let document: ConfigDocument =
            serde_json::from_str(r#"{"global":{"takeover_system_dialog":true}}"#)
                .expect("partial document is valid");
        assert!(document.global.takeover_system_dialog);
        assert!(document.global.enabled);
        assert_eq!(document.global.retention, RetentionPolicy::default());
    }

    #[test]
    fn fields_added_by_a_newer_daemon_are_ignored() {
        let document: ConfigDocument =
            serde_json::from_str(r#"{"global":{"invented_later":42},"invented_at_top":true}"#)
                .expect("unknown fields must not stop the daemon booting");
        assert_eq!(document.global, GlobalConfig::default());
    }
}
