//! Correlates observations from logd, DropBox, tombstones and ANR dumps.

#![forbid(unsafe_code)]
use cch_model::{CrashKind, CrashRecord, CrashSummary, Fingerprint, PayloadSource, SourceMask};
use std::path::PathBuf;

pub const DEFAULT_MERGE_WINDOW_MS: i64 = 10_000;
const RICH_SOURCE_SETTLE_WINDOW_MS: i64 = 250;
/// Different collectors describe one occurrence within a narrow timestamp band. The longer
/// merge window is only a timeout for incomplete evidence; using all ten seconds as identity
/// would fold a later crash of the same still-running process into the first one.
const MAX_SOURCE_SKEW_MS: i64 = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EvidenceQuality {
    Text,
    Structured,
    Artifact,
    Protobuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FragmentPayload {
    None,
    Inline(Vec<u8>),
    File(PathBuf),
}
impl FragmentPayload {
    fn into_model(self) -> PayloadSource {
        match self {
            Self::None => PayloadSource::None,
            Self::Inline(v) => PayloadSource::Inline(v),
            Self::File(v) => PayloadSource::File(v),
        }
    }
    const fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

#[derive(Debug, Clone)]
pub struct CrashFragment {
    pub source: SourceMask,
    pub quality: EvidenceQuality,
    pub kind: CrashKind,
    pub package_name: Option<String>,
    pub process_name: String,
    pub user_id: Option<i32>,
    pub pid: i32,
    pub happened_at_ms: i64,
    pub app_version_name: Option<String>,
    pub app_version_code: Option<i64>,
    pub is_system_app: Option<bool>,
    pub is_foreground: Option<bool>,
    pub dropped_count: Option<u32>,
    pub summary_class: Option<String>,
    pub summary_text: Option<String>,
    pub frames: Vec<String>,
    pub payload: FragmentPayload,
}
impl CrashFragment {
    #[must_use]
    pub fn new(
        source: SourceMask,
        quality: EvidenceQuality,
        kind: CrashKind,
        process_name: impl Into<String>,
        pid: i32,
        happened_at_ms: i64,
    ) -> Self {
        Self {
            source,
            quality,
            kind,
            package_name: None,
            process_name: process_name.into(),
            user_id: None,
            pid,
            happened_at_ms,
            app_version_name: None,
            app_version_code: None,
            is_system_app: None,
            is_foreground: None,
            dropped_count: None,
            summary_class: None,
            summary_text: None,
            frames: vec![],
            payload: FragmentPayload::None,
        }
    }
}

#[derive(Debug)]
pub struct CrashMerger {
    window_ms: i64,
    pending: Vec<Pending>,
    recent: Vec<Recent>,
}
impl Default for CrashMerger {
    fn default() -> Self {
        Self::new(DEFAULT_MERGE_WINDOW_MS)
    }
}
impl CrashMerger {
    #[must_use]
    pub const fn new(window_ms: i64) -> Self {
        Self {
            window_ms,
            pending: Vec::new(),
            recent: Vec::new(),
        }
    }
    pub fn ingest(&mut self, fragment: CrashFragment) -> Vec<CrashRecord> {
        let mut completed = self.flush_before(fragment.happened_at_ms);
        let source_skew_ms = self.window_ms.clamp(0, MAX_SOURCE_SKEW_MS);
        let pending_match = self
            .pending
            .iter()
            .enumerate()
            .filter(|(_, pending)| pending.matches(&fragment, source_skew_ms))
            .min_by_key(|(_, pending)| pending.first_ms.abs_diff(fragment.happened_at_ms))
            .map(|(index, _)| index);
        let index = if let Some(index) = pending_match {
            self.pending[index].merge(fragment);
            index
        } else {
            if self
                .recent
                .iter()
                .any(|recent| recent.matches(&fragment, source_skew_ms))
            {
                return completed;
            }
            self.pending.push(Pending::new(fragment));
            self.pending.len() - 1
        };
        if self.pending[index].completion_delay_ms(self.window_ms) == 0 {
            completed.push(self.finish_pending(index));
            completed.sort_by_key(|record| record.happened_at_ms);
        }
        completed
    }
    pub fn flush_before(&mut self, watermark_ms: i64) -> Vec<CrashRecord> {
        self.recent
            .retain(|recent| recent.first_ms.saturating_add(self.window_ms) >= watermark_ms);
        let mut out = vec![];
        let mut i = 0;
        while i < self.pending.len() {
            let completion_delay = self.pending[i].completion_delay_ms(self.window_ms);
            if self.pending[i].last_ms.saturating_add(completion_delay) < watermark_ms {
                out.push(self.finish_pending(i))
            } else {
                i += 1
            }
        }
        out.sort_by_key(|r| r.happened_at_ms);
        out
    }
    pub fn drain(&mut self) -> Vec<CrashRecord> {
        self.recent.clear();
        let mut out: Vec<_> = self.pending.drain(..).map(Pending::finish).collect();
        out.sort_by_key(|r| r.happened_at_ms);
        out
    }
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    fn finish_pending(&mut self, index: usize) -> CrashRecord {
        let pending = self.pending.swap_remove(index);
        self.recent.push(Recent::from_pending(&pending));
        pending.finish()
    }
}

/// Identity of a record completed before the full correlation window elapsed.
///
/// Enough evidence is persisted and notified immediately. A slower companion source may still
/// arrive afterwards; remembering the same key for the original window keeps that late evidence
/// from becoming a second record and a second alert.
#[derive(Debug)]
struct Recent {
    kind: CrashKind,
    process: String,
    user: Option<i32>,
    pid: i32,
    first_ms: i64,
    sources: SourceMask,
}
impl Recent {
    fn from_pending(pending: &Pending) -> Self {
        Self {
            kind: pending.kind.value,
            process: pending.process.clone(),
            user: pending.user.as_ref().map(|user| user.value),
            pid: pending.pid,
            first_ms: pending.first_ms,
            sources: pending.sources,
        }
    }

    fn matches(&self, fragment: &CrashFragment, source_skew_ms: i64) -> bool {
        // Recent completions only absorb a slower companion source. Seeing the same source
        // again means a new occurrence (notably repeated ANRs in the same still-running pid).
        if self.sources.contains(fragment.source) {
            return false;
        }
        occurrence_identity_matches(self.pid, self.user, fragment.pid, fragment.user_id)
            && self.process == fragment.process_name
            && self.kind == fragment.kind
            && self.first_ms.abs_diff(fragment.happened_at_ms)
                <= u64::try_from(source_skew_ms).unwrap_or(u64::MAX)
    }
}

const fn occurrence_identity_matches(
    left_pid: i32,
    left_user: Option<i32>,
    right_pid: i32,
    right_user: Option<i32>,
) -> bool {
    if left_pid != 0 && right_pid != 0 {
        return left_pid == right_pid
            && match (left_user, right_user) {
                (Some(left), Some(right)) => left == right,
                _ => true,
            };
    }
    matches!((left_user, right_user), (Some(left), Some(right)) if left == right)
}

#[derive(Debug)]
struct Ranked<T> {
    quality: EvidenceQuality,
    value: T,
}
impl<T> Ranked<T> {
    fn replace(&mut self, q: EvidenceQuality, v: T) {
        if q >= self.quality {
            self.quality = q;
            self.value = v
        }
    }
}
fn ranked<T>(q: EvidenceQuality, v: Option<T>) -> Option<Ranked<T>> {
    v.map(|value| Ranked { quality: q, value })
}
fn merge_ranked<T>(slot: &mut Option<Ranked<T>>, q: EvidenceQuality, v: Option<T>) {
    if let Some(v) = v {
        match slot {
            Some(s) => s.replace(q, v),
            None => {
                *slot = Some(Ranked {
                    quality: q,
                    value: v,
                })
            }
        }
    }
}

#[derive(Debug)]
struct Pending {
    kind: Ranked<CrashKind>,
    package: Option<Ranked<String>>,
    process: String,
    user: Option<Ranked<i32>>,
    pid: i32,
    first_ms: i64,
    last_ms: i64,
    version_name: Option<Ranked<String>>,
    version_code: Option<Ranked<i64>>,
    system: Option<Ranked<bool>>,
    foreground: Option<Ranked<bool>>,
    dropped: Option<u32>,
    class: Option<Ranked<String>>,
    text: Option<Ranked<String>>,
    frames: Ranked<Vec<String>>,
    payload: Ranked<FragmentPayload>,
    sources: SourceMask,
}
impl Pending {
    fn new(f: CrashFragment) -> Self {
        let q = f.quality;
        Self {
            kind: Ranked {
                quality: q,
                value: f.kind,
            },
            package: ranked(q, f.package_name),
            process: f.process_name,
            user: ranked(q, f.user_id),
            pid: f.pid,
            first_ms: f.happened_at_ms,
            last_ms: f.happened_at_ms,
            version_name: ranked(q, f.app_version_name),
            version_code: ranked(q, f.app_version_code),
            system: ranked(q, f.is_system_app),
            foreground: ranked(q, f.is_foreground),
            dropped: f.dropped_count,
            class: ranked(q, f.summary_class),
            text: ranked(q, f.summary_text),
            frames: Ranked {
                quality: q,
                value: f.frames,
            },
            payload: Ranked {
                quality: q,
                value: f.payload,
            },
            sources: f.source,
        }
    }
    fn matches(&self, f: &CrashFragment, source_skew_ms: i64) -> bool {
        !self.sources.contains(f.source)
            && occurrence_identity_matches(
                self.pid,
                self.user.as_ref().map(|user| user.value),
                f.pid,
                f.user_id,
            )
            && self.process == f.process_name
            && self.kind.value == f.kind
            && self.first_ms.abs_diff(f.happened_at_ms)
                <= u64::try_from(source_skew_ms).unwrap_or(u64::MAX)
    }
    fn merge(&mut self, f: CrashFragment) {
        let q = f.quality;
        if self.pid == 0 && f.pid != 0 {
            self.pid = f.pid;
        }
        self.first_ms = self.first_ms.min(f.happened_at_ms);
        self.last_ms = self.last_ms.max(f.happened_at_ms);
        self.sources = self.sources.union(f.source);
        self.kind.replace(q, f.kind);
        merge_ranked(&mut self.package, q, f.package_name);
        merge_ranked(&mut self.user, q, f.user_id);
        merge_ranked(&mut self.version_name, q, f.app_version_name);
        merge_ranked(&mut self.version_code, q, f.app_version_code);
        merge_ranked(&mut self.system, q, f.is_system_app);
        merge_ranked(&mut self.foreground, q, f.is_foreground);
        self.dropped = match self.dropped.zip(f.dropped_count) {
            Some((a, b)) => Some(a.max(b)),
            None => self.dropped.or(f.dropped_count),
        };
        merge_ranked(&mut self.class, q, f.summary_class);
        merge_ranked(&mut self.text, q, f.summary_text);
        if !f.frames.is_empty() && (self.frames.value.is_empty() || q >= self.frames.quality) {
            self.frames.replace(q, f.frames)
        }
        if !f.payload.is_none() && (self.payload.value.is_none() || q >= self.payload.quality) {
            self.payload.replace(q, f.payload)
        }
    }

    /// Whether waiting longer can still materially improve this record.
    ///
    /// ActivityManager supplies authoritative identity and foreground state, while the paired
    /// log or file supplies the useful payload. Attributed DropBox reports already contain both
    /// identity and a body, so they need only a short settling period for a richer artifact to
    /// arrive. Keeping complete evidence hidden for ten seconds only delays the user-facing alert.
    fn completion_delay_ms(&self, merge_window_ms: i64) -> i64 {
        let has_events = self.sources.contains(SourceMask::EVENTS);
        let has_dropbox = self.sources.contains(SourceMask::DROPBOX);
        let has_user = self.user.is_some();
        match self.kind.value {
            CrashKind::JavaException
                if has_events && self.sources.contains(SourceMask::CRASH_BUFFER) =>
            {
                0
            }
            CrashKind::NativeCrash
                if has_events
                    && self.sources.contains(SourceMask::TOMBSTONE)
                    && self.kind.quality == EvidenceQuality::Protobuf =>
            {
                0
            }
            CrashKind::Anr if has_events && self.sources.contains(SourceMask::ANR_FILE) => 0,
            CrashKind::Wtf if has_dropbox => 0,
            CrashKind::NativeCrash if has_user && self.sources.contains(SourceMask::TOMBSTONE) => {
                RICH_SOURCE_SETTLE_WINDOW_MS
            }
            CrashKind::JavaException | CrashKind::NativeCrash | CrashKind::Anr
                if has_user && has_dropbox =>
            {
                RICH_SOURCE_SETTLE_WINDOW_MS
            }
            _ => merge_window_ms,
        }
    }

    fn finish(self) -> CrashRecord {
        let package = self.package.map(|v| v.value).unwrap_or_else(|| {
            self.process
                .split(':')
                .next()
                .unwrap_or(&self.process)
                .to_owned()
        });
        let class = self.class.map(|v| v.value);
        let primary = class
            .clone()
            .or_else(|| self.text.as_ref().map(|v| v.value.clone()))
            .unwrap_or_else(|| format!("{:?}", self.kind.value));
        let self_handled = self.kind.value == CrashKind::JavaException
            && self.sources.contains(SourceMask::CRASH_BUFFER)
            && !self.sources.contains(SourceMask::EVENTS);
        CrashRecord {
            kind: self.kind.value,
            package_name: package,
            process_name: self.process,
            user_id: self.user.map(|v| v.value).unwrap_or(0),
            pid: self.pid,
            happened_at_ms: self.first_ms,
            app_version_name: self.version_name.map(|v| v.value),
            app_version_code: self.version_code.map(|v| v.value),
            is_system_app: self.system.map(|v| v.value).unwrap_or(false),
            // No collector knows this; the daemon settles it against the package index while
            // enriching, which is also where `is_system_app` gets its real value.
            package_installed: true,
            is_foreground: self.foreground.map(|v| v.value),
            self_handled,
            dropped_count: self.dropped,
            sources: self.sources,
            summary: CrashSummary::new(class, self.text.map(|v| v.value)),
            fingerprint: Fingerprint::from_raw_frames(self.kind.value, primary, &self.frames.value),
            payload: self.payload.value.into_model(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fragment(source: SourceMask, q: EvidenceQuality, at: i64) -> CrashFragment {
        CrashFragment::new(
            source,
            q,
            CrashKind::JavaException,
            "com.example:worker",
            42,
            at,
        )
    }
    #[test]
    fn merges_and_prefers_structured() {
        let mut m = CrashMerger::default();
        let mut e = fragment(SourceMask::EVENTS, EvidenceQuality::Structured, 1000);
        e.user_id = Some(10);
        e.summary_class = Some("java.lang.Error".into());
        let mut c = fragment(SourceMask::CRASH_BUFFER, EvidenceQuality::Text, 1500);
        c.summary_class = Some("guess".into());
        c.frames = vec!["at com.example.Work.run(Work.kt:9)".into()];
        m.ingest(e);
        let completed = m.ingest(c);
        assert_eq!(completed.len(), 1);
        let r = &completed[0];
        assert_eq!(r.summary.class_name.as_deref(), Some("java.lang.Error"));
        assert_eq!(r.user_id, 10);
        assert!(!r.self_handled);
    }
    #[test]
    fn identifies_self_handled() {
        let mut m = CrashMerger::default();
        m.ingest(fragment(SourceMask::CRASH_BUFFER, EvidenceQuality::Text, 1));
        assert!(m.drain().remove(0).self_handled);
    }
    #[test]
    fn outside_window_finishes_previous() {
        let mut m = CrashMerger::default();
        m.ingest(fragment(
            SourceMask::EVENTS,
            EvidenceQuality::Structured,
            1000,
        ));
        let completed = m.ingest(fragment(
            SourceMask::CRASH_BUFFER,
            EvidenceQuality::Text,
            12000,
        ));
        assert_eq!(completed.len(), 1);
        assert_eq!(m.pending_count(), 1);
    }
    #[test]
    fn protobuf_payload_wins() {
        let mut m = CrashMerger::default();
        let mut a = fragment(SourceMask::DROPBOX, EvidenceQuality::Artifact, 1);
        a.kind = CrashKind::NativeCrash;
        a.user_id = Some(0);
        a.payload = FragmentPayload::Inline(b"text".to_vec());
        let mut b = fragment(SourceMask::TOMBSTONE, EvidenceQuality::Protobuf, 2);
        b.kind = CrashKind::NativeCrash;
        b.user_id = Some(0);
        b.payload = FragmentPayload::Inline(b"proto".to_vec());
        m.ingest(a);
        m.ingest(b);
        assert!(matches!(m.drain().remove(0).payload,PayloadSource::Inline(v) if v==b"proto"));
    }

    #[test]
    fn complete_java_evidence_finishes_without_waiting_for_the_window() {
        let mut merger = CrashMerger::default();
        assert!(
            merger
                .ingest(fragment(
                    SourceMask::CRASH_BUFFER,
                    EvidenceQuality::Text,
                    1_000,
                ))
                .is_empty()
        );

        let completed = merger.ingest(fragment(
            SourceMask::EVENTS,
            EvidenceQuality::Structured,
            1_005,
        ));

        assert_eq!(completed.len(), 1);
        assert_eq!(
            completed[0].sources,
            SourceMask::CRASH_BUFFER.union(SourceMask::EVENTS)
        );
        assert_eq!(merger.pending_count(), 0);
    }

    #[test]
    fn a_late_source_does_not_duplicate_an_already_completed_crash() {
        let mut merger = CrashMerger::default();
        merger.ingest(fragment(
            SourceMask::CRASH_BUFFER,
            EvidenceQuality::Text,
            1_000,
        ));
        assert_eq!(
            merger
                .ingest(fragment(
                    SourceMask::EVENTS,
                    EvidenceQuality::Structured,
                    1_005,
                ))
                .len(),
            1
        );

        assert!(
            merger
                .ingest(fragment(
                    SourceMask::DROPBOX,
                    EvidenceQuality::Structured,
                    1_008,
                ))
                .is_empty()
        );
        assert_eq!(merger.pending_count(), 0);
    }

    #[test]
    fn the_recent_key_expires_after_the_original_merge_window() {
        let mut merger = CrashMerger::new(10_000);
        merger.ingest(fragment(
            SourceMask::CRASH_BUFFER,
            EvidenceQuality::Text,
            1_000,
        ));
        assert_eq!(
            merger
                .ingest(fragment(
                    SourceMask::EVENTS,
                    EvidenceQuality::Structured,
                    1_005,
                ))
                .len(),
            1
        );

        assert!(
            merger
                .ingest(fragment(
                    SourceMask::EVENTS,
                    EvidenceQuality::Structured,
                    11_006,
                ))
                .is_empty()
        );
        assert_eq!(merger.pending_count(), 1);
    }

    #[test]
    fn an_unknown_event_pid_joins_the_native_tombstone() {
        let mut merger = CrashMerger::default();
        let mut event = fragment(SourceMask::EVENTS, EvidenceQuality::Structured, 1_000);
        event.kind = CrashKind::NativeCrash;
        event.pid = 0;
        event.user_id = Some(0);
        let mut tombstone = fragment(SourceMask::TOMBSTONE, EvidenceQuality::Protobuf, 1_004);
        tombstone.kind = CrashKind::NativeCrash;
        tombstone.pid = 4242;
        tombstone.user_id = Some(0);

        assert!(merger.ingest(event).is_empty());
        let completed = merger.ingest(tombstone);

        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].pid, 4242);
        assert_eq!(
            completed[0].sources,
            SourceMask::EVENTS.union(SourceMask::TOMBSTONE)
        );
    }

    #[test]
    fn native_dropbox_evidence_does_not_wait_for_a_tombstone() {
        let mut merger = CrashMerger::default();
        let mut event = fragment(SourceMask::EVENTS, EvidenceQuality::Structured, 1_000);
        event.kind = CrashKind::NativeCrash;
        event.pid = 0;
        event.user_id = Some(0);
        let mut dropbox = fragment(SourceMask::DROPBOX, EvidenceQuality::Text, 1_007);
        dropbox.kind = CrashKind::NativeCrash;
        dropbox.pid = 4242;
        dropbox.user_id = Some(0);

        assert!(merger.ingest(event).is_empty());
        assert!(merger.ingest(dropbox).is_empty());
        let completed = merger.flush_before(1_007 + RICH_SOURCE_SETTLE_WINDOW_MS + 1);

        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].pid, 4242);
        assert_eq!(
            completed[0].sources,
            SourceMask::EVENTS.union(SourceMask::DROPBOX)
        );
    }

    #[test]
    fn a_dropbox_report_finishes_after_a_short_settle() {
        for kind in [
            CrashKind::JavaException,
            CrashKind::NativeCrash,
            CrashKind::Anr,
        ] {
            let mut merger = CrashMerger::default();
            let mut dropbox = fragment(SourceMask::DROPBOX, EvidenceQuality::Structured, 1_000);
            dropbox.kind = kind;
            dropbox.user_id = Some(0);

            assert!(merger.ingest(dropbox).is_empty(), "{kind:?}");
            let completed = merger.flush_before(1_000 + RICH_SOURCE_SETTLE_WINDOW_MS + 1);

            assert_eq!(completed.len(), 1, "{kind:?}");
            assert_eq!(completed[0].sources, SourceMask::DROPBOX);
        }
    }

    #[test]
    fn native_settle_prefers_a_protobuf_tombstone() {
        let mut merger = CrashMerger::default();
        let mut event = fragment(SourceMask::EVENTS, EvidenceQuality::Structured, 1_000);
        event.kind = CrashKind::NativeCrash;
        event.pid = 0;
        event.user_id = Some(0);
        let mut dropbox = fragment(SourceMask::DROPBOX, EvidenceQuality::Structured, 1_004);
        dropbox.kind = CrashKind::NativeCrash;
        dropbox.user_id = Some(0);
        dropbox.payload = FragmentPayload::Inline(b"dropbox".to_vec());
        let mut tombstone = fragment(SourceMask::TOMBSTONE, EvidenceQuality::Protobuf, 1_006);
        tombstone.kind = CrashKind::NativeCrash;
        tombstone.user_id = Some(0);
        tombstone.payload = FragmentPayload::Inline(b"protobuf".to_vec());

        assert!(merger.ingest(event).is_empty());
        assert!(merger.ingest(dropbox).is_empty());
        let completed = merger.ingest(tombstone);

        assert_eq!(completed.len(), 1);
        assert!(matches!(
            &completed[0].payload,
            PayloadSource::Inline(payload) if payload == b"protobuf"
        ));
    }

    #[test]
    fn a_standalone_tombstone_uses_the_short_settle_window() {
        let mut merger = CrashMerger::default();
        let mut tombstone = fragment(SourceMask::TOMBSTONE, EvidenceQuality::Protobuf, 1_000);
        tombstone.kind = CrashKind::NativeCrash;
        tombstone.user_id = Some(0);

        assert!(merger.ingest(tombstone).is_empty());
        assert!(
            merger
                .flush_before(1_000 + RICH_SOURCE_SETTLE_WINDOW_MS)
                .is_empty()
        );
        assert_eq!(
            merger
                .flush_before(1_000 + RICH_SOURCE_SETTLE_WINDOW_MS + 1)
                .len(),
            1
        );
    }

    #[test]
    fn an_anr_event_and_file_finish_without_waiting_for_dropbox() {
        let mut merger = CrashMerger::default();
        let mut event = fragment(SourceMask::EVENTS, EvidenceQuality::Structured, 1_000);
        event.kind = CrashKind::Anr;
        let mut file = fragment(SourceMask::ANR_FILE, EvidenceQuality::Text, 1_004);
        file.kind = CrashKind::Anr;

        assert!(merger.ingest(event).is_empty());
        let completed = merger.ingest(file);

        assert_eq!(completed.len(), 1);
        assert_eq!(
            completed[0].sources,
            SourceMask::EVENTS.union(SourceMask::ANR_FILE)
        );
    }

    #[test]
    fn an_anr_file_beats_dropbox_without_losing_the_event_reason() {
        let mut merger = CrashMerger::default();
        let mut event = fragment(SourceMask::EVENTS, EvidenceQuality::Structured, 1_000);
        event.kind = CrashKind::Anr;
        event.summary_text = Some("Input dispatching timed out".to_owned());
        let mut dropbox = fragment(SourceMask::DROPBOX, EvidenceQuality::Structured, 1_004);
        dropbox.kind = CrashKind::Anr;
        dropbox.payload = FragmentPayload::Inline(b"dropbox".to_vec());
        let mut file = fragment(SourceMask::ANR_FILE, EvidenceQuality::Artifact, 1_006);
        file.kind = CrashKind::Anr;
        file.payload = FragmentPayload::Inline(b"full anr".to_vec());

        assert!(merger.ingest(event).is_empty());
        assert!(merger.ingest(dropbox).is_empty());
        let completed = merger.ingest(file);

        assert_eq!(completed.len(), 1);
        assert_eq!(
            completed[0].summary.text.as_deref(),
            Some("Input dispatching timed out")
        );
        assert!(matches!(
            &completed[0].payload,
            PayloadSource::Inline(payload) if payload == b"full anr"
        ));
    }

    #[test]
    fn unknown_pids_do_not_join_or_suppress_a_different_user() {
        let mut merger = CrashMerger::default();
        let mut owner_event = fragment(SourceMask::EVENTS, EvidenceQuality::Structured, 1_000);
        owner_event.kind = CrashKind::NativeCrash;
        owner_event.pid = 0;
        owner_event.user_id = Some(0);
        let mut work_event = owner_event.clone();
        work_event.happened_at_ms = 1_001;
        work_event.user_id = Some(10);

        assert!(merger.ingest(owner_event).is_empty());
        assert!(merger.ingest(work_event).is_empty());
        assert_eq!(merger.pending_count(), 2);

        let mut work_dropbox = fragment(SourceMask::DROPBOX, EvidenceQuality::Structured, 1_004);
        work_dropbox.kind = CrashKind::NativeCrash;
        work_dropbox.pid = 10_042;
        work_dropbox.user_id = Some(10);
        assert!(merger.ingest(work_dropbox).is_empty());
        let work_completed = merger.flush_before(1_004 + RICH_SOURCE_SETTLE_WINDOW_MS + 1);
        assert_eq!(work_completed.len(), 1);
        assert_eq!(work_completed[0].user_id, 10);
        assert_eq!(work_completed[0].pid, 10_042);

        let mut owner_dropbox = fragment(SourceMask::DROPBOX, EvidenceQuality::Structured, 1_006);
        owner_dropbox.kind = CrashKind::NativeCrash;
        owner_dropbox.user_id = Some(0);
        assert!(merger.ingest(owner_dropbox).is_empty());
        let owner_completed = merger.flush_before(1_006 + RICH_SOURCE_SETTLE_WINDOW_MS + 1);
        assert_eq!(owner_completed.len(), 1);
        assert_eq!(owner_completed[0].user_id, 0);
        assert_eq!(merger.pending_count(), 0);
    }

    #[test]
    fn consecutive_wtf_dropbox_entries_are_distinct_occurrences() {
        let mut merger = CrashMerger::default();
        let mut first = fragment(SourceMask::DROPBOX, EvidenceQuality::Structured, 1_000);
        first.kind = CrashKind::Wtf;
        let mut second = first.clone();
        second.happened_at_ms = 1_100;

        assert_eq!(merger.ingest(first).len(), 1);
        assert_eq!(merger.ingest(second).len(), 1);
        assert_eq!(merger.pending_count(), 0);
    }

    #[test]
    fn consecutive_anr_dropbox_entries_are_distinct_occurrences() {
        let mut merger = CrashMerger::default();
        let mut first = fragment(SourceMask::DROPBOX, EvidenceQuality::Structured, 1_000);
        first.kind = CrashKind::Anr;
        first.user_id = Some(0);
        let mut second = first.clone();
        second.happened_at_ms = 1_500;

        assert!(merger.ingest(first).is_empty());
        assert_eq!(
            merger
                .flush_before(1_000 + RICH_SOURCE_SETTLE_WINDOW_MS + 1)
                .len(),
            1
        );
        assert!(merger.ingest(second).is_empty());
        assert_eq!(merger.pending_count(), 1);
        assert_eq!(
            merger
                .flush_before(1_500 + RICH_SOURCE_SETTLE_WINDOW_MS + 1)
                .len(),
            1
        );
    }

    #[test]
    fn same_source_fragments_never_share_one_pending_occurrence() {
        let mut merger = CrashMerger::default();
        let mut first = fragment(SourceMask::EVENTS, EvidenceQuality::Structured, 1_000);
        first.user_id = Some(0);
        let mut second = first.clone();
        second.happened_at_ms = 1_500;

        assert!(merger.ingest(first).is_empty());
        assert!(merger.ingest(second).is_empty());
        assert_eq!(merger.pending_count(), 2);
        assert_eq!(merger.flush_before(11_501).len(), 2);
    }

    #[test]
    fn a_different_source_outside_the_skew_is_a_new_occurrence() {
        let mut merger = CrashMerger::default();
        let mut event = fragment(SourceMask::EVENTS, EvidenceQuality::Structured, 1_000);
        event.kind = CrashKind::Anr;
        event.user_id = Some(0);
        let mut file = fragment(SourceMask::ANR_FILE, EvidenceQuality::Artifact, 1_004);
        file.kind = CrashKind::Anr;
        file.user_id = Some(0);
        assert!(merger.ingest(event).is_empty());
        assert_eq!(merger.ingest(file).len(), 1);

        let mut next = fragment(
            SourceMask::DROPBOX,
            EvidenceQuality::Structured,
            1_004 + MAX_SOURCE_SKEW_MS + 1,
        );
        next.kind = CrashKind::Anr;
        next.user_id = Some(0);
        let next_ms = next.happened_at_ms;
        assert!(merger.ingest(next).is_empty());
        assert_eq!(merger.pending_count(), 1);
        assert_eq!(
            merger
                .flush_before(next_ms + RICH_SOURCE_SETTLE_WINDOW_MS + 1)
                .len(),
            1
        );
    }

    #[test]
    fn a_companion_source_inside_the_skew_is_suppressed_after_completion() {
        let mut merger = CrashMerger::default();
        let mut event = fragment(SourceMask::EVENTS, EvidenceQuality::Structured, 1_000);
        event.kind = CrashKind::Anr;
        event.user_id = Some(0);
        let mut file = fragment(SourceMask::ANR_FILE, EvidenceQuality::Artifact, 1_004);
        file.kind = CrashKind::Anr;
        file.user_id = Some(0);
        assert!(merger.ingest(event).is_empty());
        assert_eq!(merger.ingest(file).len(), 1);

        let mut dropbox = fragment(SourceMask::DROPBOX, EvidenceQuality::Structured, 1_500);
        dropbox.kind = CrashKind::Anr;
        dropbox.user_id = Some(0);
        assert!(merger.ingest(dropbox).is_empty());
        assert_eq!(merger.pending_count(), 0);
    }

    #[test]
    fn a_new_pending_occurrence_wins_over_an_older_recent_one() {
        let mut merger = CrashMerger::default();
        let mut first_event = fragment(SourceMask::EVENTS, EvidenceQuality::Structured, 1_000);
        first_event.kind = CrashKind::Anr;
        first_event.user_id = Some(0);
        let mut first_file = fragment(SourceMask::ANR_FILE, EvidenceQuality::Artifact, 1_004);
        first_file.kind = CrashKind::Anr;
        first_file.user_id = Some(0);
        assert!(merger.ingest(first_event).is_empty());
        assert_eq!(merger.ingest(first_file).len(), 1);

        let mut next_event = fragment(SourceMask::EVENTS, EvidenceQuality::Structured, 1_500);
        next_event.kind = CrashKind::Anr;
        next_event.user_id = Some(0);
        let mut next_dropbox = fragment(SourceMask::DROPBOX, EvidenceQuality::Structured, 1_505);
        next_dropbox.kind = CrashKind::Anr;
        next_dropbox.user_id = Some(0);

        assert!(merger.ingest(next_event).is_empty());
        assert!(merger.ingest(next_dropbox).is_empty());
        assert_eq!(merger.pending_count(), 1);
        assert_eq!(
            merger
                .flush_before(1_505 + RICH_SOURCE_SETTLE_WINDOW_MS + 1)
                .len(),
            1
        );
    }

    #[test]
    fn an_unknown_user_cannot_use_a_zero_pid_as_a_cross_user_wildcard() {
        let mut merger = CrashMerger::default();
        let mut event = fragment(SourceMask::EVENTS, EvidenceQuality::Structured, 1_000);
        event.kind = CrashKind::NativeCrash;
        event.pid = 0;
        event.user_id = Some(10);
        let mut tombstone = fragment(SourceMask::TOMBSTONE, EvidenceQuality::Protobuf, 1_004);
        tombstone.kind = CrashKind::NativeCrash;
        tombstone.user_id = None;

        assert!(merger.ingest(event).is_empty());
        assert!(merger.ingest(tombstone).is_empty());
        assert_eq!(merger.pending_count(), 2);
    }
}
