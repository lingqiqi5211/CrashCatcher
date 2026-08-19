use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{CrashKind, Fingerprint, GroupKey, SourceMask};

/// Cap on the indexed summary text.
///
/// The summary lives in SQLite and is read on every list query, so it stays
/// small; the full message is in the payload.
pub const SUMMARY_TEXT_MAX_BYTES: usize = 256;

/// The small, indexable part of a crash — everything the list screen renders.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrashSummary {
    /// Exception class, signal name, or ANR reason.
    pub class_name: Option<String>,
    /// First line of the message, truncated to [`SUMMARY_TEXT_MAX_BYTES`].
    pub text: Option<String>,
}

impl CrashSummary {
    /// Builds a summary, truncating `text` on a UTF-8 character boundary.
    #[must_use]
    pub fn new(class_name: Option<String>, text: Option<String>) -> Self {
        Self {
            class_name,
            text: text.map(|value| truncate_on_char_boundary(&value, SUMMARY_TEXT_MAX_BYTES)),
        }
    }
}

/// Truncates to at most `max_bytes`, never splitting a character.
#[must_use]
pub fn truncate_on_char_boundary(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

/// Where the bulky human-readable text for a record comes from.
///
/// `File` lets a collector hand over a path (a tombstone, an ANR dump) so the
/// store can stream and compress it without the whole thing passing through
/// memory twice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PayloadSource {
    Inline(Vec<u8>),
    File(PathBuf),
    None,
}

impl PayloadSource {
    #[must_use]
    pub const fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

/// One observed occurrence of a crash, as produced by the collectors.
///
/// The store derives both the group row and the record row from this, and writes
/// `payload` out to its own file. Nothing here is Android-specific, which is what
/// keeps the store testable on the host.
#[derive(Debug, Clone)]
pub struct CrashRecord {
    pub kind: CrashKind,
    pub package_name: String,
    /// May differ from `package_name` for `:remote`-style subprocesses.
    pub process_name: String,
    /// `uid / 100000` — separates work profiles and cloned apps.
    pub user_id: i32,
    pub pid: i32,
    pub happened_at_ms: i64,
    pub app_version_name: Option<String>,
    pub app_version_code: Option<i64>,
    pub is_system_app: bool,
    /// `None` when no collector could establish it; see the design note on
    /// foreground determination.
    pub is_foreground: Option<bool>,
    /// The app installed its own `UncaughtExceptionHandler` and swallowed this:
    /// the crash buffer has a `FATAL EXCEPTION` but the events buffer has no
    /// matching `am_crash`.
    pub self_handled: bool,
    /// `Dropped-Count:` from a dropbox entry — how many siblings the Android 13+
    /// rate limiter discarded.
    pub dropped_count: Option<u32>,
    pub sources: SourceMask,
    pub summary: CrashSummary,
    pub fingerprint: Fingerprint,
    pub payload: PayloadSource,
}

impl CrashRecord {
    #[must_use]
    pub fn is_main_process(&self) -> bool {
        self.package_name == self.process_name
    }

    #[must_use]
    pub fn group_key(&self) -> GroupKey<'_> {
        GroupKey {
            package_name: &self.package_name,
            process_name: &self.process_name,
            user_id: self.user_id,
            fingerprint: &self.fingerprint,
        }
    }

    #[must_use]
    pub fn group_id(&self) -> String {
        self.group_key().group_id()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_text_is_truncated_to_the_cap() {
        let long = "x".repeat(SUMMARY_TEXT_MAX_BYTES + 100);
        let summary = CrashSummary::new(None, Some(long));
        assert_eq!(
            summary.text.map(|text| text.len()),
            Some(SUMMARY_TEXT_MAX_BYTES)
        );
    }

    #[test]
    fn truncation_never_splits_a_multibyte_character() {
        // Each `の` is 3 bytes, so a 10-byte cap lands mid-character.
        let value = "のののの";
        let truncated = truncate_on_char_boundary(value, 10);
        assert_eq!(truncated, "のののの"[..9]);
        assert!(truncated.is_char_boundary(truncated.len()));
    }

    #[test]
    fn short_text_is_left_alone() {
        let summary =
            CrashSummary::new(Some("java.lang.Error".to_owned()), Some("boom".to_owned()));
        assert_eq!(summary.text.as_deref(), Some("boom"));
    }

    #[test]
    fn main_process_is_derived_from_the_names() {
        let mut record = record_fixture();
        assert!(record.is_main_process());
        record.process_name = "com.example.app:remote".to_owned();
        assert!(!record.is_main_process());
    }

    #[test]
    fn subprocess_gets_its_own_group() {
        let main = record_fixture();
        let mut remote = record_fixture();
        remote.process_name = "com.example.app:remote".to_owned();
        assert_ne!(main.group_id(), remote.group_id());
    }

    fn record_fixture() -> CrashRecord {
        CrashRecord {
            kind: CrashKind::JavaException,
            package_name: "com.example.app".to_owned(),
            process_name: "com.example.app".to_owned(),
            user_id: 0,
            pid: 12_874,
            happened_at_ms: 1_755_440_000_123,
            app_version_name: Some("1.4.2".to_owned()),
            app_version_code: Some(10_402),
            is_system_app: false,
            is_foreground: Some(true),
            self_handled: false,
            dropped_count: None,
            sources: SourceMask::EVENTS.union(SourceMask::CRASH_BUFFER),
            summary: CrashSummary::new(
                Some("java.lang.IllegalStateException".to_owned()),
                Some("Fragment already added".to_owned()),
            ),
            fingerprint: Fingerprint::from_raw_frames(
                CrashKind::JavaException,
                "java.lang.IllegalStateException",
                &["at com.example.app.MainActivity.onCreate(MainActivity.kt:37)".to_owned()],
            ),
            payload: PayloadSource::None,
        }
    }
}
