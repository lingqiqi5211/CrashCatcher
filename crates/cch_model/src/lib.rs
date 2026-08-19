//! Domain model for crash records — the contract between the collectors and the store.
//!
//! Deliberately free of any Android or SQLite dependency so the whole crate is
//! testable on the host. The collectors (logd / dropbox / tombstone / ANR-file
//! readers) produce [`CrashRecord`]s; `cch_store` consumes them.
//!
//! The split that makes the UI fast lives here: a record carries a small,
//! indexable [`CrashSummary`] plus a [`Fingerprint`], and separately a bulky
//! [`PayloadSource`] holding the human-readable stack text. Only the former
//! reaches SQLite.

#![forbid(unsafe_code)]

mod fingerprint;
mod id;
mod kind;
mod record;

pub use fingerprint::{
    is_framework_frame, normalize_java_frame, normalize_native_frame, Fingerprint, GroupKey,
    GROUP_ID_LEN,
};
pub use id::{RecordId, RecordIdGenerator};
pub use kind::{CrashKind, PayloadCodec, PayloadState, SourceMask};
pub use record::{
    truncate_on_char_boundary, CrashRecord, CrashSummary, PayloadSource, SUMMARY_TEXT_MAX_BYTES,
};

/// Number of leading stack frames that take part in a fingerprint.
///
/// Four is enough to separate distinct crash sites while still collapsing the
/// same bug reached through different callers, which is what a user scanning a
/// grouped list wants.
pub const FINGERPRINT_FRAME_COUNT: usize = 4;
