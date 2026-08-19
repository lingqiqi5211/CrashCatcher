//! Correlates observations from logd, DropBox, tombstones and ANR dumps.

#![forbid(unsafe_code)]
use cch_model::{CrashKind, CrashRecord, CrashSummary, Fingerprint, PayloadSource, SourceMask};
use std::path::PathBuf;

pub const DEFAULT_MERGE_WINDOW_MS: i64 = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EvidenceQuality {
    Text,
    Structured,
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
        }
    }
    pub fn ingest(&mut self, fragment: CrashFragment) -> Vec<CrashRecord> {
        let completed = self.flush_before(fragment.happened_at_ms);
        if let Some(p) = self
            .pending
            .iter_mut()
            .find(|p| p.matches(&fragment, self.window_ms))
        {
            p.merge(fragment)
        } else {
            self.pending.push(Pending::new(fragment))
        }
        completed
    }
    pub fn flush_before(&mut self, watermark_ms: i64) -> Vec<CrashRecord> {
        let mut out = vec![];
        let mut i = 0;
        while i < self.pending.len() {
            if self.pending[i].last_ms.saturating_add(self.window_ms) < watermark_ms {
                out.push(self.pending.swap_remove(i).finish())
            } else {
                i += 1
            }
        }
        out.sort_by_key(|r| r.happened_at_ms);
        out
    }
    pub fn drain(&mut self) -> Vec<CrashRecord> {
        let mut out: Vec<_> = self.pending.drain(..).map(Pending::finish).collect();
        out.sort_by_key(|r| r.happened_at_ms);
        out
    }
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
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
    fn matches(&self, f: &CrashFragment, window: i64) -> bool {
        self.pid == f.pid
            && self.process == f.process_name
            && self.kind.value == f.kind
            && self.first_ms.abs_diff(f.happened_at_ms)
                <= u64::try_from(window.max(0)).unwrap_or(u64::MAX)
    }
    fn merge(&mut self, f: CrashFragment) {
        let q = f.quality;
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
        m.ingest(c);
        let r = m.drain().remove(0);
        assert_eq!(r.summary.class_name.as_deref(), Some("java.lang.Error"));
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
        let mut a = fragment(SourceMask::TOMBSTONE, EvidenceQuality::Text, 1);
        a.kind = CrashKind::NativeCrash;
        a.payload = FragmentPayload::Inline(b"text".to_vec());
        let mut b = fragment(SourceMask::TOMBSTONE, EvidenceQuality::Protobuf, 2);
        b.kind = CrashKind::NativeCrash;
        b.payload = FragmentPayload::Inline(b"proto".to_vec());
        m.ingest(a);
        m.ingest(b);
        assert!(matches!(m.drain().remove(0).payload,PayloadSource::Inline(v) if v==b"proto"));
    }
}
