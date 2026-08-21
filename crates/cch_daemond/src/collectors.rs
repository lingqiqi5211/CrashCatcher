#[cfg(target_os = "android")]
use std::collections::HashSet;
use std::{
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use cch_anrfile::parse_anr;
use cch_dropbox::{DropboxEntry, DropboxKind, parse_path as parse_dropbox};
#[cfg(any(target_os = "android", test))]
use cch_logd::ActivityEvent;
#[cfg(target_os = "android")]
use cch_logd::{AndroidLogReader, LogBuffer, LoggerEntry};
use cch_logd::{CrashBufferReport, parse_crash_buffer};
use cch_merge::{CrashFragment, CrashMerger, EvidenceQuality, FragmentPayload, MergedCrash};
use cch_model::{CrashKind, SourceMask};
use cch_tombstone::{TombstoneFormat, TombstoneReport, parse_proto, parse_text};
use cch_watcher::{
    DiscoveredSource, IngestedRegistry, InotifyWatcher, WatchKind, WatchRoot, startup_scan,
};
use cch_wire::CollectorSource;
use tracing::warn;

use crate::{DaemonCore, now_ms};

const WATCH_POLL_INTERVAL: Duration = Duration::from_millis(250);
const MERGE_POLL_INTERVAL: Duration = Duration::from_millis(100);
#[cfg(target_os = "android")]
const LOG_READER_RETRY_INTERVAL: Duration = Duration::from_millis(250);
#[cfg(target_os = "android")]
const LOG_HISTORY_GRACE_MS: i64 = 10_000;
#[cfg(target_os = "android")]
const EVENTS_LOG_MIGRATION_KEY: &str = "logd:events:durable-source-keys:v1";
#[cfg(target_os = "android")]
const CRASH_LOG_MIGRATION_KEY: &str = "logd:crash:durable-source-keys:v1";
const MAX_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;
type PendingSourceConfirmation = (Option<CollectorSource>, String);

pub struct CollectorRuntime {
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl CollectorRuntime {
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Release);
        for handle in self.threads.drain(..) {
            let _ = handle.join();
        }
    }
}

impl Drop for CollectorRuntime {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

#[must_use]
pub fn start_collectors(core: Arc<DaemonCore>) -> CollectorRuntime {
    let stop = Arc::new(AtomicBool::new(false));
    let (sender, receiver) = mpsc::channel();
    let threads = vec![
        spawn_ingest_loop(Arc::clone(&core), Arc::clone(&stop), receiver),
        spawn_artifact_loop(Arc::clone(&core), Arc::clone(&stop), sender.clone()),
    ];

    #[cfg(target_os = "android")]
    let threads = {
        let mut threads = threads;
        threads.push(spawn_events_loop(
            Arc::clone(&core),
            Arc::clone(&stop),
            sender.clone(),
        ));
        threads.push(spawn_crash_loop(core, Arc::clone(&stop), sender));
        threads
    };

    CollectorRuntime { stop, threads }
}

fn spawn_ingest_loop(
    core: Arc<DaemonCore>,
    stop: Arc<AtomicBool>,
    receiver: Receiver<CrashFragment>,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("ct-ingest".to_owned())
        .spawn(move || {
            let mut merger = CrashMerger::default();
            let mut pending_confirmations = Vec::new();
            while !stop.load(Ordering::Acquire) {
                match receiver.recv_timeout(MERGE_POLL_INTERVAL) {
                    Ok(fragment) => {
                        let source = collector_from_mask(fragment.source);
                        core.mark_collector_received(source, fragment.happened_at_ms);
                        let completed = merger.ingest(fragment);
                        persist_completed(
                            &core,
                            &mut merger,
                            Some(source),
                            completed,
                            &mut pending_confirmations,
                        );
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        let completed = merger.flush_before(now_ms());
                        persist_completed(
                            &core,
                            &mut merger,
                            None,
                            completed,
                            &mut pending_confirmations,
                        );
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
            let completed = merger.drain();
            persist_completed(
                &core,
                &mut merger,
                None,
                completed,
                &mut pending_confirmations,
            );
        })
        .unwrap_or_else(|error| {
            warn!(%error, "failed to start ingest thread");
            thread::spawn(|| {})
        })
}

fn persist_completed(
    core: &DaemonCore,
    merger: &mut CrashMerger,
    source: Option<CollectorSource>,
    completed: Vec<MergedCrash>,
    pending_confirmations: &mut Vec<PendingSourceConfirmation>,
) {
    for merged in completed {
        let completion_id = merged.completion_id;
        match core.ingest_with_sources(merged.record, &merged.source_keys) {
            Ok(_) => {
                merger.confirm_persisted(completion_id);
            }
            Err(error) => {
                merger.reject_completion(completion_id);
                if let Some(source) = source {
                    core.mark_collector_error(source, error.to_string());
                }
                warn!(%error, "failed to persist merged crash");
            }
        }
    }
    pending_confirmations.extend(
        merger
            .take_confirmed_suppressed_source_keys()
            .into_iter()
            .map(|source_key| (source, source_key)),
    );
    *pending_confirmations = mark_source_keys(core, std::mem::take(pending_confirmations));
}

fn mark_source_keys(
    core: &DaemonCore,
    source_keys: Vec<PendingSourceConfirmation>,
) -> Vec<PendingSourceConfirmation> {
    let mut failed = Vec::new();
    for (source, key) in source_keys {
        if let Err(error) = core.mark_source_ingested(&key) {
            if let Some(source) = source {
                core.mark_collector_error(source, error.to_string());
            }
            warn!(%error, source_key = %key, "failed to confirm ingested source");
            failed.push((source, key));
        }
    }
    failed
}

fn spawn_artifact_loop(
    core: Arc<DaemonCore>,
    stop: Arc<AtomicBool>,
    sender: Sender<CrashFragment>,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("ct-artifacts".to_owned())
        .spawn(move || {
            let roots = android_watch_roots();
            let registry = CoreRegistry(Arc::clone(&core));
            match startup_scan(&roots, &registry) {
                Ok(sources) => {
                    for source in sources {
                        ingest_source(&core, &sender, source);
                    }
                }
                Err(error) => warn!(%error, "artifact startup scan failed"),
            }

            let mut watcher = match InotifyWatcher::new(&roots) {
                Ok(watcher) => watcher,
                Err(error) => {
                    for source in [
                        CollectorSource::Dropbox,
                        CollectorSource::Tombstone,
                        CollectorSource::AnrFile,
                    ] {
                        core.mark_collector_error(source, error.to_string());
                    }
                    return;
                }
            };
            while !stop.load(Ordering::Acquire) {
                match watcher.poll() {
                    Ok(sources) => {
                        for source in sources {
                            ingest_source(&core, &sender, source);
                        }
                    }
                    Err(error) => warn!(%error, "artifact watcher poll failed"),
                }
                thread::sleep(WATCH_POLL_INTERVAL);
            }
        })
        .unwrap_or_else(|error| {
            warn!(%error, "failed to start artifact thread");
            thread::spawn(|| {})
        })
}

fn ingest_source(core: &DaemonCore, sender: &Sender<CrashFragment>, source: DiscoveredSource) {
    let health_source = collector_from_watch(source.kind);
    if matches!(core.was_source_ingested(&source.identity.key), Ok(true)) {
        return;
    }
    match fragment_from_source(&source) {
        Ok(Some(mut fragment)) => {
            fragment.source_key = Some(source.identity.key.clone());
            if sender.send(fragment).is_err() {
                core.mark_collector_error(health_source, "ingest queue disconnected");
            }
        }
        // Read fine, just not a crash this tool records. Marked ingested so the same
        // file is not re-examined on every scan, and *not* an error: `/data/system/dropbox`
        // is mostly unrelated tags (strictmode, netstats, boot events), so treating one
        // as a fault painted the whole DropBox collector as broken the first time the
        // watcher saw any of them — and it stayed that way.
        Ok(None) => {
            let _ = core.mark_source_ingested(&source.identity.key);
        }
        Err(error) => core.mark_collector_error(health_source, error),
    }
}

/// Turns a discovered file into a fragment.
///
/// Three outcomes, not two. `Ok(None)` means the file was read successfully and is simply
/// not a crash this tool records — the common case in a dropbox directory — and must stay
/// distinct from `Err`, which means the source could not be read and the collector really
/// is impaired.
fn fragment_from_source(source: &DiscoveredSource) -> Result<Option<CrashFragment>, String> {
    match source.kind {
        WatchKind::Dropbox => {
            dropbox_fragment(parse_dropbox(&source.preferred_path).map_err(|e| e.to_string())?)
        }
        WatchKind::Tombstone => tombstone_fragment(source).map(Some),
        WatchKind::Anr => anr_fragment(source).map(Some),
    }
}

fn dropbox_fragment(entry: DropboxEntry) -> Result<Option<CrashFragment>, String> {
    let kind = match entry.kind {
        DropboxKind::JavaCrash | DropboxKind::StrictMode => CrashKind::JavaException,
        DropboxKind::Anr | DropboxKind::LowMemory | DropboxKind::Watchdog => CrashKind::Anr,
        DropboxKind::NativeCrash | DropboxKind::NativeRecoverableCrash => CrashKind::NativeCrash,
        DropboxKind::Wtf => CrashKind::Wtf,
        // Every other tag in the directory: boot events, strictmode reports, network
        // stats. Nothing is wrong — they are simply not ours.
        DropboxKind::Unknown => return Ok(None),
    };
    // An entry with neither a process nor a package cannot be attributed to an app, so
    // there is nothing useful to record. Still not a collector fault.
    let Some(process_name) = entry
        .process_name
        .clone()
        .or_else(|| entry.package_name.clone())
    else {
        return Ok(None);
    };
    let mut fragment = CrashFragment::new(
        SourceMask::DROPBOX,
        EvidenceQuality::Structured,
        kind,
        process_name,
        entry.pid.unwrap_or(0),
        entry.file_name.happened_at_ms,
    );
    fragment.package_name = entry.package_name;
    fragment.user_id = entry.uid.map(android_user_id);
    fragment.is_foreground = entry.foreground;
    fragment.dropped_count = entry.dropped_count;
    fragment.payload = FragmentPayload::Inline(entry.body.as_bytes().to_vec());
    fragment.summary_text = first_non_empty_line(&entry.body);

    match kind {
        CrashKind::JavaException => {
            if let Ok(report) = parse_crash_buffer(&entry.body) {
                apply_java_report(&mut fragment, &report);
            } else {
                fragment.frames = java_like_frames(&entry.body);
                fragment.summary_class = exception_class(&entry.body);
            }
        }
        CrashKind::Anr => {
            fragment.summary_class = Some("ANR".to_owned());
            // The first body line is usually an ANR dump header, not the reason. Keep the
            // ActivityManager reason when it exists and let the payload carry the dump.
            fragment.summary_text = None;
            if let Ok(report) = parse_anr(entry.body.as_bytes()) {
                fragment.process_name = report.process_name.clone();
                fragment.package_name = Some(report.package_name().to_owned());
                fragment.pid = report.pid;
                fragment.frames = report
                    .main_thread()
                    .and_then(|thread| thread.top_frame())
                    .map(|frame| vec![frame.to_owned()])
                    .unwrap_or_default();
            }
        }
        CrashKind::NativeCrash => {
            if let Ok(report) = parse_text(entry.body.as_bytes()) {
                apply_native_report(&mut fragment, &report);
            }
        }
        CrashKind::Wtf => {
            fragment.frames = java_like_frames(&entry.body);
        }
    }
    Ok(Some(fragment))
}

fn tombstone_fragment(source: &DiscoveredSource) -> Result<CrashFragment, String> {
    let bytes = read_limited(&source.preferred_path)?;
    let report = if source
        .preferred_path
        .extension()
        .is_some_and(|extension| extension == "pb")
    {
        parse_proto(&bytes).map_err(|error| error.to_string())?
    } else {
        parse_text(&bytes).map_err(|error| error.to_string())?
    };
    let mut fragment = CrashFragment::new(
        SourceMask::TOMBSTONE,
        if report.format == TombstoneFormat::Protobuf {
            EvidenceQuality::Protobuf
        } else {
            EvidenceQuality::Artifact
        },
        CrashKind::NativeCrash,
        report.process_name.clone(),
        report.pid,
        modified_ms(source),
    );
    fragment.package_name = Some(report.package_name().to_owned());
    fragment.user_id = report.uid.map(android_user_id);
    apply_native_report(&mut fragment, &report);
    fragment.payload = if report.format == TombstoneFormat::Text {
        FragmentPayload::File(source.preferred_path.clone())
    } else {
        FragmentPayload::Inline(render_tombstone(&report).into_bytes())
    };
    Ok(fragment)
}

fn anr_fragment(source: &DiscoveredSource) -> Result<CrashFragment, String> {
    let bytes = read_limited(&source.preferred_path)?;
    let report = parse_anr(&bytes).map_err(|error| error.to_string())?;
    let mut fragment = CrashFragment::new(
        SourceMask::ANR_FILE,
        EvidenceQuality::Artifact,
        CrashKind::Anr,
        report.process_name.clone(),
        report.pid,
        modified_ms(source),
    );
    fragment.package_name = Some(report.package_name().to_owned());
    fragment.source_instance = Some(source.path.to_string_lossy().into_owned());
    fragment.summary_class = Some("ANR".to_owned());
    fragment.frames = report
        .main_thread()
        .and_then(|thread| thread.top_frame())
        .map(|frame| vec![frame.to_owned()])
        .unwrap_or_default();
    fragment.payload = FragmentPayload::File(source.preferred_path.clone());
    Ok(fragment)
}

fn apply_java_report(fragment: &mut CrashFragment, report: &CrashBufferReport) {
    fragment.process_name = report.process_name.clone();
    fragment.package_name = Some(package_from_process(&report.process_name));
    fragment.pid = report.pid;
    fragment.summary_class = Some(report.exception_class.clone());
    fragment.summary_text =
        (!report.exception_message.is_empty()).then(|| report.exception_message.clone());
    fragment.frames = report
        .frames
        .iter()
        .map(|frame| frame.normalized())
        .collect();
    fragment.payload = FragmentPayload::Inline(report.raw.as_bytes().to_vec());
}

fn apply_native_report(fragment: &mut CrashFragment, report: &TombstoneReport) {
    // OEM DropBox process/pid headers are not consistently populated. The tombstone body is the
    // native crash authority for those fields; the DropBox uid remains authoritative when it is
    // present, with the body uid as a fallback. A wrong header pid otherwise prevents Events,
    // DropBox and Tombstone from joining.
    fragment.process_name = report.process_name.clone();
    fragment.package_name = Some(report.package_name().to_owned());
    fragment.pid = report.pid;
    if fragment.user_id.is_none() {
        fragment.user_id = report.uid.map(android_user_id);
    }
    fragment.summary_class = report.signal_name.clone().or_else(|| {
        report
            .signal_number
            .map(|number| format!("signal {number}"))
    });
    fragment.summary_text = report
        .cause
        .clone()
        .or_else(|| report.abort_message.clone())
        .or_else(|| report.signal_code_name.clone());
    fragment.frames = report
        .frames
        .iter()
        .map(|frame| frame.normalized())
        .collect();
}

fn render_tombstone(report: &TombstoneReport) -> String {
    let mut lines = vec![format!(
        "pid: {}, tid: {}, name: {} >>> {} <<<",
        report.pid,
        report.tid,
        report.thread_name.as_deref().unwrap_or("unknown"),
        report.process_name
    )];
    if let Some(uid) = report.uid {
        lines.push(format!("uid: {uid}"));
    }
    lines.push(format!(
        "signal {} ({}) code {} ({})",
        report.signal_number.unwrap_or(0),
        report.signal_name.as_deref().unwrap_or("unknown"),
        report.signal_code.unwrap_or(0),
        report.signal_code_name.as_deref().unwrap_or("unknown")
    ));
    if let Some(message) = &report.abort_message {
        lines.push(format!("Abort message: {message}"));
    }
    for (index, frame) in report.frames.iter().enumerate() {
        lines.push(format!("#{index:02} {}", frame.normalized()));
    }
    lines.join("\n")
}

fn read_limited(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::metadata(path).map_err(|error| format!("{}: {error}", path.display()))?;
    if metadata.len() > MAX_ARTIFACT_BYTES {
        return Err(format!("{} exceeds artifact size limit", path.display()));
    }
    fs::read(path).map_err(|error| format!("{}: {error}", path.display()))
}

fn modified_ms(source: &DiscoveredSource) -> i64 {
    i64::try_from(source.identity.modified_ns / 1_000_000).unwrap_or(i64::MAX)
}

fn first_non_empty_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToOwned::to_owned)
}

fn exception_class(text: &str) -> Option<String> {
    text.lines().map(str::trim).find_map(|line| {
        let class = line.split_once(':').map_or(line, |(class, _)| class);
        (class.contains('.') && !class.starts_with("at ")).then(|| class.to_owned())
    })
}

fn java_like_frames(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| line.starts_with("at "))
        .map(ToOwned::to_owned)
        .collect()
}

fn package_from_process(process: &str) -> String {
    process.split(':').next().unwrap_or(process).to_owned()
}

fn android_user_id(uid: i32) -> i32 {
    uid.max(0) / 100_000
}

fn android_watch_roots() -> Vec<WatchRoot> {
    vec![
        WatchRoot::new(WatchKind::Dropbox, "/data/system/dropbox"),
        WatchRoot::new(WatchKind::Tombstone, "/data/tombstones"),
        WatchRoot::new(WatchKind::Anr, "/data/anr"),
    ]
}

fn collector_from_watch(kind: WatchKind) -> CollectorSource {
    match kind {
        WatchKind::Dropbox => CollectorSource::Dropbox,
        WatchKind::Tombstone => CollectorSource::Tombstone,
        WatchKind::Anr => CollectorSource::AnrFile,
    }
}

fn collector_from_mask(mask: SourceMask) -> CollectorSource {
    if mask.contains(SourceMask::EVENTS) {
        CollectorSource::Events
    } else if mask.contains(SourceMask::CRASH_BUFFER) {
        CollectorSource::CrashBuffer
    } else if mask.contains(SourceMask::DROPBOX) {
        CollectorSource::Dropbox
    } else if mask.contains(SourceMask::TOMBSTONE) {
        CollectorSource::Tombstone
    } else {
        CollectorSource::AnrFile
    }
}

struct CoreRegistry(Arc<DaemonCore>);

impl IngestedRegistry for CoreRegistry {
    fn contains(&self, key: &str) -> Result<bool, String> {
        self.0
            .was_source_ingested(key)
            .map_err(|error| error.to_string())
    }
}

#[cfg(target_os = "android")]
fn run_log_reader(
    core: &DaemonCore,
    stop: &AtomicBool,
    source: CollectorSource,
    buffer: LogBuffer,
    mut handle_entry: impl for<'entry> FnMut(LoggerEntry<'entry>) -> bool,
) {
    while !stop.load(Ordering::Acquire) {
        let mut reader = match AndroidLogReader::open(buffer) {
            Ok(reader) => {
                core.clear_collector_error(source);
                reader
            }
            Err(error) => {
                core.mark_collector_error(source, error.to_string());
                thread::sleep(LOG_READER_RETRY_INTERVAL);
                continue;
            }
        };
        loop {
            if stop.load(Ordering::Acquire) {
                return;
            }
            match reader.read_entry() {
                Ok(entry) => {
                    core.clear_collector_error(source);
                    if !handle_entry(entry) {
                        return;
                    }
                }
                Err(error) => {
                    let reconnect = error.requires_reconnect();
                    core.mark_collector_error(source, error.to_string());
                    if reconnect {
                        break;
                    }
                    thread::sleep(LOG_READER_RETRY_INTERVAL);
                }
            }
        }
        thread::sleep(LOG_READER_RETRY_INTERVAL);
    }
}

#[cfg(target_os = "android")]
fn log_source_key(buffer: &str, entry: &LoggerEntry<'_>, discriminator: &str) -> String {
    format!(
        "logd:{buffer}:{}:{}:{}:{}:{discriminator}",
        entry.seconds, entry.nanoseconds, entry.pid, entry.tid,
    )
}

#[cfg(target_os = "android")]
fn confirm_processed_log_source(core: &DaemonCore, source: CollectorSource, key: &str) {
    if let Err(error) = core.mark_source_ingested(key) {
        core.mark_collector_error(source, error.to_string());
    }
}

#[cfg(target_os = "android")]
struct LogReplayMigration {
    cutoff_ms: i64,
    marker: &'static str,
    active: bool,
}

#[cfg(target_os = "android")]
impl LogReplayMigration {
    fn new(core: &DaemonCore, marker: &'static str) -> Self {
        Self {
            cutoff_ms: now_ms().saturating_sub(LOG_HISTORY_GRACE_MS),
            marker,
            active: !matches!(core.was_source_ingested(marker), Ok(true)),
        }
    }

    fn is_historical(
        &mut self,
        core: &DaemonCore,
        source: CollectorSource,
        happened_at_ms: i64,
    ) -> bool {
        if !self.active {
            return false;
        }
        if happened_at_ms < self.cutoff_ms {
            return true;
        }
        self.active = false;
        // This marker means the one-time migration reached live entries. Historical crash keys
        // below are deliberately confirmed as ignored, not misreported as stored records, so a
        // later daemon restart cannot import the pre-upgrade log buffer as fresh crashes.
        confirm_processed_log_source(core, source, self.marker);
        false
    }
}

#[cfg(target_os = "android")]
fn log_source_seen(
    core: &DaemonCore,
    source: CollectorSource,
    seen: &mut HashSet<String>,
    source_key: &str,
) -> bool {
    if !seen.insert(source_key.to_owned()) {
        return true;
    }
    match core.was_source_ingested(source_key) {
        Ok(ingested) => ingested,
        Err(error) => {
            core.mark_collector_error(source, error.to_string());
            false
        }
    }
}

#[cfg(target_os = "android")]
fn spawn_events_loop(
    core: Arc<DaemonCore>,
    stop: Arc<AtomicBool>,
    sender: Sender<CrashFragment>,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("ct-log-events".to_owned())
        .spawn(move || {
            use cch_logd::{parse_activity_event, parse_event_payload, parse_screen_event};
            let mut replay = LogReplayMigration::new(&core, EVENTS_LOG_MIGRATION_KEY);
            let mut seen = HashSet::new();
            run_log_reader(
                &core,
                &stop,
                CollectorSource::Events,
                LogBuffer::Events,
                |entry| {
                    let at_ms =
                        i64::from(entry.seconds) * 1_000 + i64::from(entry.nanoseconds / 1_000_000);
                    let historical = replay.is_historical(&core, CollectorSource::Events, at_ms);
                    let Ok(record) = parse_event_payload(entry.payload) else {
                        return true;
                    };
                    // This buffer is also the only place the daemon can see the screen
                    // being unlocked, which is what `MuteScope::UntilUnlock` expires on.
                    if let Some(screen) = parse_screen_event(&record) {
                        let source_key = log_source_key("events", &entry, &record.tag.to_string());
                        if log_source_seen(&core, CollectorSource::Events, &mut seen, &source_key) {
                            return true;
                        }
                        if historical {
                            confirm_processed_log_source(
                                &core,
                                CollectorSource::Events,
                                &source_key,
                            );
                            return true;
                        }
                        match core.clear_unlock_mutes() {
                            Ok(cleared) if cleared > 0 => {
                                tracing::info!(?screen, cleared, "released the until-unlock mutes");
                                confirm_processed_log_source(
                                    &core,
                                    CollectorSource::Events,
                                    &source_key,
                                );
                            }
                            Ok(_) => {
                                confirm_processed_log_source(
                                    &core,
                                    CollectorSource::Events,
                                    &source_key,
                                );
                            }
                            Err(error) => {
                                warn!(%error, "could not clear the until-unlock mutes");
                            }
                        }
                        return true;
                    }
                    let event_tag = record.tag;
                    if let Ok(event) = parse_activity_event(record) {
                        let source_key = log_source_key("events", &entry, &event_tag.to_string());
                        if log_source_seen(&core, CollectorSource::Events, &mut seen, &source_key) {
                            return true;
                        }
                        if historical {
                            confirm_processed_log_source(
                                &core,
                                CollectorSource::Events,
                                &source_key,
                            );
                            return true;
                        }
                        let mut fragment = fragment_from_activity(event, at_ms);
                        fragment.source_key = Some(source_key);
                        if sender.send(fragment).is_err() {
                            return false;
                        }
                    }
                    true
                },
            );
        })
        .unwrap_or_else(|error| {
            warn!(%error, "failed to start events reader");
            thread::spawn(|| {})
        })
}

#[cfg(target_os = "android")]
fn spawn_crash_loop(
    core: Arc<DaemonCore>,
    stop: Arc<AtomicBool>,
    sender: Sender<CrashFragment>,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("ct-log-crash".to_owned())
        .spawn(move || {
            use cch_logd::TextLogEntry;
            let mut replay = LogReplayMigration::new(&core, CRASH_LOG_MIGRATION_KEY);
            let mut seen = HashSet::new();
            run_log_reader(
                &core,
                &stop,
                CollectorSource::CrashBuffer,
                LogBuffer::Crash,
                |entry| {
                    let at_ms =
                        i64::from(entry.seconds) * 1_000 + i64::from(entry.nanoseconds / 1_000_000);
                    let historical =
                        replay.is_historical(&core, CollectorSource::CrashBuffer, at_ms);
                    let Ok(text) = TextLogEntry::parse(entry.payload) else {
                        return true;
                    };
                    if text.tag != "AndroidRuntime" || !text.message.contains("FATAL EXCEPTION:") {
                        return true;
                    }
                    let Ok(report) = parse_crash_buffer(text.message) else {
                        return true;
                    };
                    let source_key = log_source_key("crash", &entry, "fatal");
                    if log_source_seen(&core, CollectorSource::CrashBuffer, &mut seen, &source_key)
                    {
                        return true;
                    }
                    if historical {
                        confirm_processed_log_source(
                            &core,
                            CollectorSource::CrashBuffer,
                            &source_key,
                        );
                        return true;
                    }
                    let mut fragment = fragment_from_crash(report, at_ms);
                    fragment.source_key = Some(source_key);
                    if sender.send(fragment).is_err() {
                        return false;
                    }
                    true
                },
            );
        })
        .unwrap_or_else(|error| {
            warn!(%error, "failed to start crash reader");
            thread::spawn(|| {})
        })
}

/// What an `am_crash` event is actually reporting.
///
/// Not every one is a Java exception. `NativeCrashListener` builds a `CrashInfo` for a native
/// crash too and hands it to the same activity-manager path, with this exact class name and
/// `strsignal()` as the message — so a segfault arrives here as `Native crash` / `Aborted`.
///
/// Filing those as Java was visible twice over. The row said "Java" next to a title reading
/// "Native crash", and because [`cch_merge`] only merges fragments that agree on the kind, the
/// tombstone for the same crash could not join it: one crash, listed twice, once mislabelled.
#[cfg(any(target_os = "android", test))]
const AM_CRASH_NATIVE_CLASS: &str = "Native crash";

#[cfg(any(target_os = "android", test))]
fn kind_of_am_crash(exception_class: &str) -> CrashKind {
    if exception_class == AM_CRASH_NATIVE_CLASS {
        CrashKind::NativeCrash
    } else {
        CrashKind::JavaException
    }
}

#[cfg(any(target_os = "android", test))]
fn fragment_from_activity(event: ActivityEvent, happened_at_ms: i64) -> CrashFragment {
    match event {
        ActivityEvent::Crash(event) => {
            let kind = kind_of_am_crash(&event.exception_class);
            // NativeCrashListener reports through system_server's ActivityManager path on
            // some ROMs, so this field is system_server's pid rather than the process that
            // produced the tombstone. Mark it unknown and let the tombstone supply it; keeping
            // the framework pid prevents the two sources from ever joining.
            let pid = if kind == CrashKind::NativeCrash {
                0
            } else {
                event.pid
            };
            let mut fragment = CrashFragment::new(
                SourceMask::EVENTS,
                EvidenceQuality::Structured,
                kind,
                event.process_name.clone(),
                pid,
                happened_at_ms,
            );
            fragment.package_name = Some(package_from_process(&event.process_name));
            fragment.user_id = Some(event.user_id);
            fragment.summary_class = Some(event.exception_class);
            fragment.summary_text = (!event.message.is_empty()).then_some(event.message);
            if !event.file.is_empty() {
                fragment.frames = vec![format!("{}:{}", event.file, event.line)];
            }
            fragment
        }
        ActivityEvent::Anr(event) => {
            let mut fragment = CrashFragment::new(
                SourceMask::EVENTS,
                EvidenceQuality::Structured,
                CrashKind::Anr,
                event.process_name.clone(),
                event.pid,
                happened_at_ms,
            );
            fragment.package_name = Some(package_from_process(&event.process_name));
            fragment.user_id = Some(event.user_id);
            fragment.summary_class = Some("ANR".to_owned());
            fragment.summary_text = Some(event.reason);
            fragment
        }
        ActivityEvent::Wtf(event) => {
            let mut fragment = CrashFragment::new(
                SourceMask::EVENTS,
                EvidenceQuality::Structured,
                CrashKind::Wtf,
                event.process_name.clone(),
                event.pid,
                happened_at_ms,
            );
            fragment.package_name = Some(package_from_process(&event.process_name));
            fragment.user_id = Some(event.user_id);
            fragment.payload = FragmentPayload::Inline(
                format!(
                    "WTF report\nProcess: {}\nPID: {}\nFlags: {}\nTag: {}\nMessage: {}\n",
                    event.process_name, event.pid, event.flags, event.tag, event.message,
                )
                .into_bytes(),
            );
            fragment.summary_class = (!event.tag.is_empty()).then_some(event.tag);
            fragment.summary_text = (!event.message.is_empty()).then_some(event.message);
            fragment
        }
    }
}

#[cfg(target_os = "android")]
fn fragment_from_crash(report: CrashBufferReport, happened_at_ms: i64) -> CrashFragment {
    let mut fragment = CrashFragment::new(
        SourceMask::CRASH_BUFFER,
        EvidenceQuality::Text,
        CrashKind::JavaException,
        report.process_name.clone(),
        report.pid,
        happened_at_ms,
    );
    apply_java_report(&mut fragment, &report);
    fragment
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_fragment_preserves_android_user_id() {
        let fragment = fragment_from_activity(
            ActivityEvent::Anr(cch_logd::AmAnrEvent {
                user_id: 10,
                pid: 42,
                process_name: "com.example:remote".to_owned(),
                flags: 0,
                reason: "input timeout".to_owned(),
            }),
            1_000,
        );
        assert_eq!(fragment.package_name.as_deref(), Some("com.example"));
        assert_eq!(fragment.user_id, Some(10));
        assert_eq!(fragment.kind, CrashKind::Anr);
    }

    #[test]
    fn am_wtf_is_a_complete_attributed_fragment() {
        let fragment = fragment_from_activity(
            ActivityEvent::Wtf(cch_logd::AmWtfEvent {
                user_id: 10,
                pid: 42,
                process_name: "com.example:remote".to_owned(),
                flags: 7,
                tag: "ExampleTag".to_owned(),
                message: "terrible failure".to_owned(),
            }),
            1_000,
        );
        assert_eq!(fragment.package_name.as_deref(), Some("com.example"));
        assert_eq!(fragment.user_id, Some(10));
        assert_eq!(fragment.kind, CrashKind::Wtf);
        assert_eq!(fragment.summary_class.as_deref(), Some("ExampleTag"));
        assert_eq!(fragment.summary_text.as_deref(), Some("terrible failure"));
        assert!(
            matches!(fragment.payload, FragmentPayload::Inline(ref payload)
            if String::from_utf8_lossy(payload).contains("Flags: 7")
                && String::from_utf8_lossy(payload).contains("Tag: ExampleTag")
                && String::from_utf8_lossy(payload).contains("Message: terrible failure"))
        );
    }

    fn am_crash(exception_class: &str, message: &str) -> CrashFragment {
        fragment_from_activity(
            ActivityEvent::Crash(cch_logd::AmCrashEvent {
                user_id: 0,
                pid: 42,
                process_name: "com.example:ijkservice".to_owned(),
                flags: 0,
                exception_class: exception_class.to_owned(),
                message: message.to_owned(),
                file: "unknown".to_owned(),
                line: 0,
                recoverable: false,
            }),
            1_000,
        )
    }

    /// The shape `NativeCrashListener` produces. Read as a Java exception it gave a row titled
    /// "Native crash" badged "Java", and kept the tombstone from merging into it.
    #[test]
    fn an_am_crash_can_be_reporting_a_native_crash() {
        let native = am_crash("Native crash", "Aborted");
        assert_eq!(native.kind, CrashKind::NativeCrash);
        assert_eq!(native.pid, 0);
        assert_eq!(
            am_crash("java.lang.NullPointerException", "boom").kind,
            CrashKind::JavaException
        );
    }

    #[test]
    fn protobuf_tombstones_render_human_readable_payloads() {
        let report = TombstoneReport {
            format: TombstoneFormat::Protobuf,
            pid: 1,
            tid: 2,
            uid: Some(10_123),
            process_name: "com.example".to_owned(),
            thread_name: Some("main".to_owned()),
            signal_number: Some(11),
            signal_name: Some("SIGSEGV".to_owned()),
            signal_code: Some(1),
            signal_code_name: Some("SEGV_MAPERR".to_owned()),
            abort_message: None,
            cause: None,
            frames: Vec::new(),
            raw_text: None,
        };
        let rendered = render_tombstone(&report);
        assert!(rendered.contains("SIGSEGV"));
        assert!(rendered.contains("com.example"));
    }

    #[test]
    fn native_body_repairs_an_unreliable_dropbox_identity() {
        let report = parse_text(
            b"pid: 4242, tid: 4243, name: worker  >>> com.example:native <<<\n\
              uid: 1010123\n\
              signal 11 (SIGSEGV), code 1 (SEGV_MAPERR)\n",
        )
        .expect("tombstone body");
        let mut fragment = CrashFragment::new(
            SourceMask::DROPBOX,
            EvidenceQuality::Structured,
            CrashKind::NativeCrash,
            "wrong.process",
            99,
            1_000,
        );

        apply_native_report(&mut fragment, &report);

        assert_eq!(fragment.process_name, "com.example:native");
        assert_eq!(fragment.package_name.as_deref(), Some("com.example"));
        assert_eq!(fragment.pid, 4242);
        assert_eq!(fragment.user_id, Some(10));
    }
}
