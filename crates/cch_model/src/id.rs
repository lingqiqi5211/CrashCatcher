use std::fmt;

use serde::{Deserialize, Serialize};

/// Crockford base32 — no `I`, `L`, `O` or `U`, so ids survive being read aloud
/// or retyped out of a bug report.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

const TIMESTAMP_BITS: u32 = 48;
const SEQUENCE_BITS: u32 = 128 - TIMESTAMP_BITS;
const ID_CHARS: usize = 26;

/// A time-sortable record id.
///
/// Layout is 48 bits of millisecond timestamp followed by 80 bits of sequence,
/// rendered as 26 Crockford base32 characters. Lexicographic order therefore
/// matches creation order, which lets `crash_record.id` double as a tiebreaker
/// in keyset pagination without a second index.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RecordId(String);

impl RecordId {
    /// Wraps an id read back from storage or the wire.
    ///
    /// Rejects anything that is not 26 characters from the alphabet, so a
    /// malformed value cannot reach a SQL parameter.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        if value.len() != ID_CHARS {
            return None;
        }
        if !value.bytes().all(|byte| ALPHABET.contains(&byte)) {
            return None;
        }
        Some(Self(value.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }

    /// The integer an id encodes, or None if a character is outside the alphabet.
    ///
    /// The inverse of [`Self::encode`], and only needed so the generator can resume from
    /// the highest id already stored.
    fn decode(&self) -> Option<u128> {
        let mut value = 0u128;
        for byte in self.0.bytes() {
            let digit = ALPHABET.iter().position(|candidate| *candidate == byte)?;
            value = (value << 5) | digit as u128;
        }
        Some(value)
    }

    fn encode(value: u128) -> Self {
        let mut buffer = [b'0'; ID_CHARS];
        let mut remaining = value;
        for slot in buffer.iter_mut().rev() {
            *slot = ALPHABET[(remaining & 0x1f) as usize];
            remaining >>= 5;
        }
        // Every byte comes from ALPHABET, which is ASCII.
        Self(String::from_utf8_lossy(&buffer).into_owned())
    }
}

impl fmt::Display for RecordId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Hands out strictly increasing [`RecordId`]s.
///
/// Holds a monotonic guard: if the wall clock jumps backwards (an NTP
/// correction, or a device whose clock is set after boot) the generator keeps
/// the last timestamp it used rather than emitting ids that sort before existing
/// rows. Ordering is the property the pagination depends on; the exact
/// millisecond is already stored separately in `happened_at_ms`.
#[derive(Debug, Default)]
pub struct RecordIdGenerator {
    last_ms: u64,
    sequence: u128,
}

impl RecordIdGenerator {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            last_ms: 0,
            sequence: 0,
        }
    }

    /// Resumes after an existing id, so a restart cannot re-emit one.
    ///
    /// Without this the generator restarts at zero every time the store is opened, and
    /// ids are derived from the *crash's* timestamp rather than the current time — so
    /// re-reading a tombstone after a restart produced byte-identical ids and the insert
    /// failed on the primary key. The crash was then silently dropped: the only trace was
    /// a `UNIQUE constraint failed` line in the daemon log.
    ///
    /// Ids that do not decode are ignored rather than rejected; one unreadable row must
    /// not stop the store from opening, and the worst case is the same collision this
    /// exists to avoid.
    pub fn resume_after(&mut self, id: &RecordId) {
        let Some(value) = id.decode() else { return };
        let timestamp = (value >> SEQUENCE_BITS) as u64;
        let sequence = value & ((1u128 << SEQUENCE_BITS) - 1);
        if timestamp > self.last_ms || (timestamp == self.last_ms && sequence >= self.sequence) {
            self.last_ms = timestamp;
            self.sequence = sequence;
        }
    }

    /// Produces the next id for the given wall-clock millisecond.
    pub fn next(&mut self, now_ms: u64) -> RecordId {
        let timestamp = now_ms.max(self.last_ms) & ((1u64 << TIMESTAMP_BITS) - 1);
        if timestamp == self.last_ms {
            self.sequence = self.sequence.wrapping_add(1) & ((1u128 << SEQUENCE_BITS) - 1);
        } else {
            self.last_ms = timestamp;
            self.sequence = 0;
        }
        RecordId::encode((u128::from(timestamp) << SEQUENCE_BITS) | self.sequence)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_have_the_documented_shape() {
        let id = RecordIdGenerator::new().next(1_755_440_000_123);
        assert_eq!(id.as_str().len(), ID_CHARS);
        assert!(id.as_str().bytes().all(|byte| ALPHABET.contains(&byte)));
    }

    #[test]
    fn resuming_never_reissues_an_existing_id() {
        // The bug this guards: ids come from the crash's timestamp, not from "now", so a
        // generator that restarted at zero handed out the same id again the moment the
        // same crash was re-read after a daemon restart. The insert then failed on the
        // primary key and the crash was dropped.
        let crash_ms = 1_755_440_000_123;

        let first = RecordIdGenerator::new().next(crash_ms);

        let mut resumed = RecordIdGenerator::new();
        resumed.resume_after(&first);
        let second = resumed.next(crash_ms);

        assert_ne!(first, second, "a restart must not reissue a stored id");
        assert!(second > first, "ids must keep increasing across restarts");
    }

    #[test]
    fn resuming_ignores_an_id_it_cannot_decode() {
        // A single unreadable row must not stop the store from opening.
        let mut generator = RecordIdGenerator::new();
        generator.resume_after(&RecordId("!".repeat(26)));
        assert_eq!(generator.next(7), RecordIdGenerator::new().next(7));
    }

    #[test]
    fn ids_sort_by_creation_order_within_one_millisecond() {
        let mut generator = RecordIdGenerator::new();
        let first = generator.next(1_000);
        let second = generator.next(1_000);
        let third = generator.next(1_000);
        assert!(first < second);
        assert!(second < third);
    }

    #[test]
    fn ids_sort_by_creation_order_across_milliseconds() {
        let mut generator = RecordIdGenerator::new();
        let earlier = generator.next(1_000);
        let later = generator.next(1_001);
        assert!(earlier < later);
    }

    #[test]
    fn a_backwards_clock_still_yields_increasing_ids() {
        let mut generator = RecordIdGenerator::new();
        let before = generator.next(5_000);
        let after_jump = generator.next(1_000);
        assert!(
            before < after_jump,
            "{before} should sort before {after_jump}"
        );
    }

    #[test]
    fn parse_rejects_malformed_ids() {
        let valid = RecordIdGenerator::new().next(42);
        assert_eq!(RecordId::parse(valid.as_str()), Some(valid));
        assert_eq!(RecordId::parse(""), None);
        assert_eq!(RecordId::parse("TOO-SHORT"), None);
        // `I`, `L`, `O` and `U` are excluded from Crockford base32.
        assert_eq!(RecordId::parse("IIIIIIIIIIIIIIIIIIIIIIIIII"), None);
        assert_eq!(RecordId::parse("'; DROP TABLE crash_record--"), None);
    }
}
