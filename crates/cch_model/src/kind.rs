use serde::{Deserialize, Serialize};

/// What kind of failure a record describes.
///
/// The discriminants are persisted in SQLite and sent on the wire, so they are
/// fixed: append new variants, never renumber.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrashKind {
    JavaException,
    Anr,
    NativeCrash,
    Wtf,
}

impl CrashKind {
    pub const ALL: [Self; 4] = [Self::JavaException, Self::Anr, Self::NativeCrash, Self::Wtf];

    #[must_use]
    pub const fn as_i64(self) -> i64 {
        match self {
            Self::JavaException => 0,
            Self::Anr => 1,
            Self::NativeCrash => 2,
            Self::Wtf => 3,
        }
    }

    #[must_use]
    pub const fn from_i64(value: i64) -> Option<Self> {
        match value {
            0 => Some(Self::JavaException),
            1 => Some(Self::Anr),
            2 => Some(Self::NativeCrash),
            3 => Some(Self::Wtf),
            _ => None,
        }
    }
}

/// Which collectors observed a given occurrence.
///
/// One native crash legitimately shows up on four different paths (the
/// `tombstone_NN` text, its `.pb` sibling, the `SYSTEM_TOMBSTONE_*` dropbox
/// entry, and `data_app_native_crash` via `NativeCrashListener`), so the merge
/// step ORs these together rather than picking a winner.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SourceMask(u32);

impl SourceMask {
    /// logd events buffer — `am_crash` (30039) / `am_anr` (30008). The trigger.
    pub const EVENTS: Self = Self(1 << 0);
    /// logd crash buffer — `AndroidRuntime: FATAL EXCEPTION`. Carries the stack.
    pub const CRASH_BUFFER: Self = Self(1 << 1);
    /// `/data/system/dropbox/<tag>@<ms>.txt.gz`. Structured headers.
    pub const DROPBOX: Self = Self(1 << 2);
    /// `/data/tombstones/tombstone_NN[.pb]`.
    pub const TOMBSTONE: Self = Self(1 << 3);
    /// `/data/anr/anr_<timestamp>`.
    pub const ANR_FILE: Self = Self(1 << 4);

    const KNOWN: u32 = 0b1_1111;

    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Drops bits this build does not know about, so a record written by a newer
    /// daemon does not resurrect as a nonsense mask after a downgrade.
    #[must_use]
    pub const fn from_bits_truncate(bits: u32) -> Self {
        Self(bits & Self::KNOWN)
    }

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// Whether a record has a readable payload, and if not, why.
///
/// `Evicted` is the graceful-degradation state from the retention rules: the metadata row
/// survives so history, counts and statistics stay intact even though the full stack is gone.
/// `Absent` says the opposite thing — nothing was ever stored — and the two must not be
/// conflated, or a record that never had a stack claims retention deleted one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayloadState {
    Present,
    /// Stored, but cut off at the per-record byte cap.
    Truncated,
    /// Reclaimed to stay under the total byte quota.
    Evicted,
    /// There never was one.
    ///
    /// Distinct from [`Self::Evicted`], which says storage reclaimed it. A crash seen only in
    /// the events buffer — `am_crash` with no matching stack in the crash buffer, usually
    /// because log rotation reached it first — has no payload to begin with, and calling that
    /// "reclaimed to stay under the quota" tells the reader their retention settings threw
    /// away something that was never there.
    Absent,
}

impl PayloadState {
    #[must_use]
    pub const fn as_i64(self) -> i64 {
        match self {
            Self::Present => 0,
            Self::Truncated => 1,
            Self::Evicted => 2,
            Self::Absent => 3,
        }
    }

    #[must_use]
    pub const fn from_i64(value: i64) -> Option<Self> {
        match value {
            0 => Some(Self::Present),
            1 => Some(Self::Truncated),
            2 => Some(Self::Evicted),
            3 => Some(Self::Absent),
            _ => None,
        }
    }

    #[must_use]
    pub const fn is_readable(self) -> bool {
        matches!(self, Self::Present | Self::Truncated)
    }
}

/// How a payload file is encoded on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayloadCodec {
    Raw,
    Zstd,
}

impl PayloadCodec {
    #[must_use]
    pub const fn as_i64(self) -> i64 {
        match self {
            Self::Raw => 0,
            Self::Zstd => 1,
        }
    }

    #[must_use]
    pub const fn from_i64(value: i64) -> Option<Self> {
        match value {
            0 => Some(Self::Raw),
            1 => Some(Self::Zstd),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crash_kind_discriminants_round_trip() {
        for kind in CrashKind::ALL {
            assert_eq!(CrashKind::from_i64(kind.as_i64()), Some(kind));
        }
        assert_eq!(CrashKind::from_i64(4), None);
        assert_eq!(CrashKind::from_i64(-1), None);
    }

    #[test]
    fn source_mask_unions_and_tests_bits() {
        let mask = SourceMask::EVENTS.union(SourceMask::DROPBOX);
        assert!(mask.contains(SourceMask::EVENTS));
        assert!(mask.contains(SourceMask::DROPBOX));
        assert!(!mask.contains(SourceMask::TOMBSTONE));
        assert!(!mask.is_empty());
        assert!(SourceMask::empty().is_empty());
    }

    #[test]
    fn source_mask_drops_unknown_bits() {
        // A newer daemon may have written a bit this build has no meaning for.
        let mask = SourceMask::from_bits_truncate(0xFFFF_FFFF);
        assert_eq!(mask.bits(), SourceMask::KNOWN);
    }

    #[test]
    fn payload_state_readability() {
        assert!(PayloadState::Present.is_readable());
        assert!(PayloadState::Truncated.is_readable());
        assert!(!PayloadState::Evicted.is_readable());
        assert!(!PayloadState::Absent.is_readable());
        for state in [
            PayloadState::Present,
            PayloadState::Truncated,
            PayloadState::Evicted,
            PayloadState::Absent,
        ] {
            assert_eq!(PayloadState::from_i64(state.as_i64()), Some(state));
        }
        assert_eq!(PayloadState::from_i64(4), None);
    }

    #[test]
    fn payload_codec_discriminants_round_trip() {
        for codec in [PayloadCodec::Raw, PayloadCodec::Zstd] {
            assert_eq!(PayloadCodec::from_i64(codec.as_i64()), Some(codec));
        }
        assert_eq!(PayloadCodec::from_i64(2), None);
    }
}
