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
use cch_logd::{CrashBufferReport, parse_crash_buffer};
use cch_merge::{CrashFragment, CrashMerger, EvidenceQuality, FragmentPayload};
use cch_model::{CrashKind, SourceMask};
use cch_tombstone::{TombstoneFormat, TombstoneReport, parse_proto, parse_text};
use cch_watcher::{
    DiscoveredSource, IngestedRegistry, InotifyWatcher, WatchKind, WatchRoot, startup_scan,
};
use cch_wire::CollectorSource;
use tracing::warn;

use crate::{DaemonCore, now_ms};

const WATCH_POLL_INTERVAL: Duration = Duration::from_millis(250);
const MERGE_POLL_INTERVAL: Duration = Duration::from_millis(500);
const MAX_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;

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
            while !stop.load(Ordering::Acquire) {
                match receiver.recv_timeout(MERGE_POLL_INTERVAL) {
                    Ok(fragment) => {
                        let source = collector_from_mask(fragment.source);
                        core.mark_collector_received(source, fragment.happened_at_ms);
                        for record in merger.ingest(fragment) {
                            if let Err(error) = core.ingest(record) {
                                core.mark_collector_error(source, error.to_string());
                            }
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        for record in merger.flush_before(now_ms()) {
                            if let Err(error) = core.ingest(record) {
                                warn!(%error, "failed to persist merged crash");
                            }
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
            for record in merger.drain() {
                if let Err(error) = core.ingest(record) {
                    warn!(%error, "failed to persist crash while stopping");
                }
            }
        })
        .unwrap_or_else(|error| {
            warn!(%error, "failed to start ingest thread");
            thread::spawn(|| {})
        })
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
        Ok(Some(fragment)) => {
            if sender.send(fragment).is_ok()
                && let Err(error) = core.mark_source_ingested(&source.identity.key)
            {
                core.mark_collector_error(health_source, error.to_string());
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
            if let Ok(report) = parse_anr(entry.body.as_bytes()) {
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
            EvidenceQuality::Text
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
        EvidenceQuality::Text,
        CrashKind::Anr,
        report.process_name.clone(),
        report.pid,
        modified_ms(source),
    );
    fragment.package_name = Some(report.package_name().to_owned());
    fragment.summary_class = Some("ANR".to_owned());
    fragment.summary_text = Some("Application not responding".to_owned());
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
fn spawn_events_loop(
    core: Arc<DaemonCore>,
    stop: Arc<AtomicBool>,
    sender: Sender<CrashFragment>,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("ct-log-events".to_owned())
        .spawn(move || {
            use cch_logd::{
                AndroidLogReader, LogBuffer, parse_activity_event, parse_event_payload,
                parse_screen_event,
            };
            let mut reader = match AndroidLogReader::open(LogBuffer::Events) {
                Ok(reader) => reader,
                Err(error) => {
                    core.mark_collector_error(CollectorSource::Events, error.to_string());
                    return;
                }
            };
            while !stop.load(Ordering::Acquire) {
                match reader.read_entry() {
                    Ok(entry) => {
                        let at_ms = i64::from(entry.seconds) * 1_000
                            + i64::from(entry.nanoseconds / 1_000_000);
                        let Ok(record) = parse_event_payload(entry.payload) else {
                            continue;
                        };
                        // This buffer is also the only place the daemon can see the screen
                        // being unlocked, which is what `MuteScope::UntilUnlock` expires on.
                        if let Some(screen) = parse_screen_event(&record) {
                            match core.clear_unlock_mutes() {
                                Ok(cleared) if cleared > 0 => {
                                    tracing::info!(
                                        ?screen,
                                        cleared,
                                        "released the until-unlock mutes"
                                    );
                                }
                                Ok(_) => {}
                                Err(error) => {
                                    warn!(%error, "could not clear the until-unlock mutes");
                                }
                            }
                            continue;
                        }
                        if let Ok(event) = parse_activity_event(record) {
                            let fragment = fragment_from_activity(event, at_ms);
                            if sender.send(fragment).is_err() {
                                break;
                            }
                        }
                    }
                    Err(error) => {
                        core.mark_collector_error(CollectorSource::Events, error.to_string());
                        thread::sleep(Duration::from_millis(250));
                    }
                }
            }
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
            use cch_logd::{AndroidLogReader, LogBuffer, TextLogEntry};
            let mut reader = match AndroidLogReader::open(LogBuffer::Crash) {
                Ok(reader) => reader,
                Err(error) => {
                    core.mark_collector_error(CollectorSource::CrashBuffer, error.to_string());
                    return;
                }
            };
            while !stop.load(Ordering::Acquire) {
                match reader.read_entry() {
                    Ok(entry) => {
                        let Ok(text) = TextLogEntry::parse(entry.payload) else {
                            continue;
                        };
                        if text.tag != "AndroidRuntime"
                            || !text.message.contains("FATAL EXCEPTION:")
                        {
                            continue;
                        }
                        let Ok(report) = parse_crash_buffer(text.message) else {
                            continue;
                        };
                        let at_ms = i64::from(entry.seconds) * 1_000
                            + i64::from(entry.nanoseconds / 1_000_000);
                        if sender.send(fragment_from_crash(report, at_ms)).is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        core.mark_collector_error(CollectorSource::CrashBuffer, error.to_string());
                        thread::sleep(Duration::from_millis(250));
                    }
                }
            }
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
            let mut fragment = CrashFragment::new(
                SourceMask::EVENTS,
                EvidenceQuality::Structured,
                kind_of_am_crash(&event.exception_class),
                event.process_name.clone(),
                event.pid,
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
        assert_eq!(
            am_crash("Native crash", "Aborted").kind,
            CrashKind::NativeCrash
        );
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
}
