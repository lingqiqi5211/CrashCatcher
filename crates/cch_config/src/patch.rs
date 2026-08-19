use serde::{Deserialize, Serialize};

use crate::{AppConfig, GlobalConfig, MuteScope, NotifyMode, RetentionPolicy};

/// A partial update to [`RetentionPolicy`]. Absent fields keep their value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RetentionPatch {
    pub retention_days: Option<u32>,
    pub max_records_per_group: Option<u32>,
    pub max_records_total: Option<u32>,
    pub max_payload_bytes_total: Option<u64>,
    pub max_payload_bytes_per_record: Option<u64>,
}

impl RetentionPatch {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.retention_days.is_none()
            && self.max_records_per_group.is_none()
            && self.max_records_total.is_none()
            && self.max_payload_bytes_total.is_none()
            && self.max_payload_bytes_per_record.is_none()
    }

    #[must_use]
    pub fn apply(&self, base: RetentionPolicy) -> RetentionPolicy {
        RetentionPolicy {
            retention_days: self.retention_days.unwrap_or(base.retention_days),
            max_records_per_group: self
                .max_records_per_group
                .unwrap_or(base.max_records_per_group),
            max_records_total: self.max_records_total.unwrap_or(base.max_records_total),
            max_payload_bytes_total: self
                .max_payload_bytes_total
                .unwrap_or(base.max_payload_bytes_total),
            max_payload_bytes_per_record: self
                .max_payload_bytes_per_record
                .unwrap_or(base.max_payload_bytes_per_record),
        }
        .clamped()
    }
}

/// A partial update to [`GlobalConfig`].
///
/// Partial rather than whole-document so two settings screens editing different
/// rows cannot clobber each other's change on save.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GlobalConfigPatch {
    pub enabled: Option<bool>,
    pub capture_java: Option<bool>,
    pub capture_anr: Option<bool>,
    pub capture_native: Option<bool>,
    pub capture_wtf: Option<bool>,
    pub capture_self_handled: Option<bool>,
    pub notify_mode: Option<NotifyMode>,
    pub only_foreground: Option<bool>,
    pub foreground_unknown_notifies: Option<bool>,
    pub only_main_process: Option<bool>,
    pub include_system_apps: Option<bool>,
    pub takeover_system_dialog: Option<bool>,
    pub retention: RetentionPatch,
}

impl GlobalConfigPatch {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.enabled.is_none()
            && self.capture_java.is_none()
            && self.capture_anr.is_none()
            && self.capture_native.is_none()
            && self.capture_wtf.is_none()
            && self.capture_self_handled.is_none()
            && self.notify_mode.is_none()
            && self.only_foreground.is_none()
            && self.foreground_unknown_notifies.is_none()
            && self.only_main_process.is_none()
            && self.include_system_apps.is_none()
            && self.takeover_system_dialog.is_none()
            && self.retention.is_empty()
    }

    #[must_use]
    pub fn apply(&self, base: &GlobalConfig) -> GlobalConfig {
        GlobalConfig {
            enabled: self.enabled.unwrap_or(base.enabled),
            capture_java: self.capture_java.unwrap_or(base.capture_java),
            capture_anr: self.capture_anr.unwrap_or(base.capture_anr),
            capture_native: self.capture_native.unwrap_or(base.capture_native),
            capture_wtf: self.capture_wtf.unwrap_or(base.capture_wtf),
            capture_self_handled: self
                .capture_self_handled
                .unwrap_or(base.capture_self_handled),
            notify_mode: self.notify_mode.unwrap_or(base.notify_mode),
            only_foreground: self.only_foreground.unwrap_or(base.only_foreground),
            foreground_unknown_notifies: self
                .foreground_unknown_notifies
                .unwrap_or(base.foreground_unknown_notifies),
            only_main_process: self.only_main_process.unwrap_or(base.only_main_process),
            include_system_apps: self.include_system_apps.unwrap_or(base.include_system_apps),
            takeover_system_dialog: self
                .takeover_system_dialog
                .unwrap_or(base.takeover_system_dialog),
            retention: self.retention.apply(base.retention),
        }
    }
}

/// A partial update to one app's overrides.
///
/// `notify_mode` is doubly optional on purpose: the outer `None` means "leave it
/// alone", the inner `None` means "go back to following the global setting".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfigPatch {
    /// `skip_serializing_if` is load-bearing, not tidiness: without it the outer
    /// `None` ("leave it alone") would be written as `null` and read back as
    /// `Some(None)` ("clear the override"), so simply forwarding an untouched patch
    /// would wipe the app's setting.
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub notify_mode: Option<Option<NotifyMode>>,
    pub ignore: Option<bool>,
    pub mute: Option<MuteScope>,
}

impl AppConfigPatch {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.notify_mode.is_none() && self.ignore.is_none() && self.mute.is_none()
    }

    #[must_use]
    pub fn apply(&self, base: &AppConfig) -> AppConfig {
        AppConfig {
            notify_mode: match self.notify_mode {
                Some(value) => value,
                None => base.notify_mode,
            },
            ignore: self.ignore.unwrap_or(base.ignore),
            mute: self.mute.unwrap_or(base.mute),
        }
    }
}

/// Lets `"notify_mode": null` mean "clear it" while an absent key means
/// "leave it alone".
mod double_option {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S, T>(value: &Option<Option<T>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: Serialize,
    {
        match value {
            Some(inner) => inner.serialize(serializer),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de>,
    {
        Option::<T>::deserialize(deserializer).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_patch_changes_nothing() {
        let base = GlobalConfig::default();
        assert!(GlobalConfigPatch::default().is_empty());
        assert_eq!(GlobalConfigPatch::default().apply(&base), base);
    }

    #[test]
    fn a_patch_touches_only_the_fields_it_names() {
        let base = GlobalConfig::default();
        let patch = GlobalConfigPatch {
            takeover_system_dialog: Some(true),
            ..GlobalConfigPatch::default()
        };
        let updated = patch.apply(&base);
        assert!(updated.takeover_system_dialog);
        assert_eq!(updated.enabled, base.enabled);
        assert_eq!(updated.notify_mode, base.notify_mode);
        assert_eq!(updated.retention, base.retention);
    }

    #[test]
    fn patched_retention_is_clamped() {
        let patch = GlobalConfigPatch {
            retention: RetentionPatch {
                retention_days: Some(0),
                ..RetentionPatch::default()
            },
            ..GlobalConfigPatch::default()
        };
        let updated = patch.apply(&GlobalConfig::default());
        assert_eq!(
            updated.retention.retention_days,
            RetentionPolicy::MIN_RETENTION_DAYS
        );
    }

    #[test]
    fn app_patch_distinguishes_leave_alone_from_follow_global() {
        let base = AppConfig {
            notify_mode: Some(NotifyMode::Dialog),
            ..AppConfig::default()
        };

        // Absent key: keep the override.
        let untouched: AppConfigPatch = serde_json::from_str("{}").expect("empty patch");
        assert_eq!(untouched.apply(&base).notify_mode, Some(NotifyMode::Dialog));

        // Explicit null: go back to following the global setting.
        let cleared: AppConfigPatch =
            serde_json::from_str(r#"{"notify_mode":null}"#).expect("null patch");
        assert_eq!(cleared.apply(&base).notify_mode, None);

        // A value: replace the override.
        let replaced: AppConfigPatch =
            serde_json::from_str(r#"{"notify_mode":"toast"}"#).expect("value patch");
        assert_eq!(replaced.apply(&base).notify_mode, Some(NotifyMode::Toast));
    }

    #[test]
    fn the_three_notify_mode_states_survive_serialization() {
        // The deserialize direction is only half the contract: an untouched patch
        // that round-trips into "clear the override" would silently wipe settings.
        let cases = [
            (None, r#"{"ignore":null,"mute":null}"#),
            (
                Some(None),
                r#"{"notify_mode":null,"ignore":null,"mute":null}"#,
            ),
            (
                Some(Some(NotifyMode::Toast)),
                r#"{"notify_mode":"toast","ignore":null,"mute":null}"#,
            ),
        ];

        for (notify_mode, expected_json) in cases {
            let patch = AppConfigPatch {
                notify_mode,
                ..AppConfigPatch::default()
            };
            let json = serde_json::to_string(&patch).expect("serializes");
            assert_eq!(json, expected_json, "for {notify_mode:?}");

            let parsed: AppConfigPatch = serde_json::from_str(&json).expect("deserializes");
            assert_eq!(
                parsed.notify_mode, notify_mode,
                "round trip for {notify_mode:?}"
            );
        }
    }

    #[test]
    fn forwarding_an_untouched_patch_preserves_an_existing_override() {
        let base = AppConfig {
            notify_mode: Some(NotifyMode::Dialog),
            ..AppConfig::default()
        };
        let untouched = AppConfigPatch::default();

        // Serialize, ship over the wire, apply on the other side.
        let json = serde_json::to_string(&untouched).expect("serializes");
        let received: AppConfigPatch = serde_json::from_str(&json).expect("deserializes");

        assert_eq!(received.apply(&base).notify_mode, Some(NotifyMode::Dialog));
    }

    #[test]
    fn app_patch_emptiness_reflects_the_double_option() {
        let cleared: AppConfigPatch =
            serde_json::from_str(r#"{"notify_mode":null}"#).expect("null patch");
        assert!(
            !cleared.is_empty(),
            "clearing the override is a real change"
        );
        assert!(AppConfigPatch::default().is_empty());
    }
}
