//! The daemon's own logs, for the diagnostics page.
//!
//! Three writers end up under `logs/`:
//!
//! - `daemon.log` and its rotated `.1`…`.8`, written by this process. See [`crate::RollingLog`].
//! - `daemon.stderr.log`, the launcher's redirect. Holds panics and failures that happen before
//!   logging exists.
//! - `service.log`, the launcher's own record of exits and restarts. A daemon that dies before
//!   it can log anything leaves a trace here and nowhere else.
//!
//! `logs/old/` holds the same set from the previous boot, moved aside by `service.sh`. The crash
//! being chased is often the one that ended that session.
//!
//! Files are read from the tail: what matters is at the end.

use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use cch_wire::RuntimeLogFile;

use crate::logsink::LOG_FILE_NAME;

/// How much of one file is returned when no size is asked for.
pub const DEFAULT_LOG_BYTES: u64 = 128 * 1024;
/// Ceiling on one request, so the answer still fits in a frame.
pub const MAX_LOG_BYTES: u64 = 512 * 1024;

/// Where the previous boot's logs are kept.
const OLD_DIRECTORY: &str = "old";

/// The launcher's redirect of the daemon's stderr.
const STDERR_FILE_NAME: &str = "daemon.stderr.log";

/// The launcher's own record of starts, exits and restarts.
const SERVICE_FILE_NAME: &str = "service.log";

pub struct RuntimeLog {
    /// Which file this is, matching one of [`Self::files`].
    pub name: String,
    pub text: String,
    /// Whether anything was cut from the front.
    pub truncated: bool,
    /// Size of this file on disk.
    pub total_bytes: u64,
    /// Everything available to read, newest first.
    pub files: Vec<RuntimeLogFile>,
}

/// Reads one log file, and lists the rest.
///
/// The listing travels with the content so the page can offer the other files without a second
/// round trip. An unknown or absent `name` falls back to the first listed file — the daemon's
/// live log, which is what someone opening the page came for.
pub fn read_runtime_log(state_dir: &Path, name: Option<&str>, max_bytes: u64) -> RuntimeLog {
    let logs = state_dir.join("logs");
    let files = list_files(&logs);

    let selected = name
        .and_then(|name| files.iter().find(|file| file.name == name))
        .or_else(|| files.first());

    let Some(selected) = selected else {
        return RuntimeLog {
            name: String::new(),
            text: "(no logs yet)\n".to_owned(),
            truncated: false,
            total_bytes: 0,
            files,
        };
    };

    let budget = max_bytes.clamp(4 * 1024, MAX_LOG_BYTES);
    let tail = read_tail(&resolve(&logs, &selected.name), budget);
    RuntimeLog {
        name: selected.name.clone(),
        text: if tail.text.trim().is_empty() {
            "(empty)\n".to_owned()
        } else {
            tail.text
        },
        truncated: tail.truncated,
        total_bytes: tail.total_bytes,
        files,
    }
}

/// Everything readable under `logs/`, this boot before the last, live file first.
fn list_files(logs: &Path) -> Vec<RuntimeLogFile> {
    let mut files = Vec::new();
    collect(logs, None, &mut files);
    collect(&logs.join(OLD_DIRECTORY), Some(OLD_DIRECTORY), &mut files);

    // By what each file is, not when it was last written. Sorting by mtime put whichever file
    // had just been appended to at the top, so the page opened on `service.log` — three lines
    // from the launcher — whenever the daemon had been quiet for a moment, and the menu
    // reshuffled itself between visits.
    files.sort_by_key(|file| rank(&file.name));
    files
}

/// Sort key: this boot before `old/`, and within a boot the daemon's own log first.
fn rank(name: &str) -> (u8, u8, u32, String) {
    let (boot, file) = match name.split_once('/') {
        Some((OLD_DIRECTORY, file)) => (1, file),
        _ => (0, name),
    };
    let (kind, index) = match file {
        LOG_FILE_NAME => (0, 0),
        STDERR_FILE_NAME => (2, 0),
        SERVICE_FILE_NAME => (3, 0),
        // `daemon.log.1` … `.8`, oldest last.
        _ => match file
            .strip_prefix(LOG_FILE_NAME)
            .and_then(|rest| rest.strip_prefix('.'))
            .and_then(|number| number.parse::<u32>().ok())
        {
            Some(number) => (1, number),
            None => (4, 0),
        },
    };
    (boot, kind, index, file.to_owned())
}

fn collect(directory: &Path, prefix: Option<&str>, out: &mut Vec<RuntimeLogFile>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let Some(file_name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        // `.boot_id` is bookkeeping, not a log.
        if file_name.starts_with('.') {
            continue;
        }
        out.push(RuntimeLogFile {
            name: match prefix {
                Some(prefix) => format!("{prefix}/{file_name}"),
                None => file_name,
            },
            bytes: metadata.len(),
            modified_ms: modified_ms(&metadata),
        });
    }
}

fn modified_ms(metadata: &fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|since| i64::try_from(since.as_millis()).ok())
        .unwrap_or(0)
}

/// Resolves a listed name back to a path.
///
/// Only the two shapes `list_files` produces are accepted, so a name from the wire cannot reach
/// outside the log directory however it is spelled.
fn resolve(logs: &Path, name: &str) -> PathBuf {
    match name.split_once('/') {
        Some((OLD_DIRECTORY, file)) => logs.join(OLD_DIRECTORY).join(file),
        _ => logs.join(name),
    }
}

#[derive(Default)]
struct Tail {
    text: String,
    truncated: bool,
    total_bytes: u64,
}

/// The last `budget` bytes of a file, starting at a line boundary.
fn read_tail(path: &Path, budget: u64) -> Tail {
    let Ok(mut file) = File::open(path) else {
        return Tail::default();
    };
    let Ok(total_bytes) = file.metadata().map(|meta| meta.len()) else {
        return Tail::default();
    };

    let truncated = total_bytes > budget;
    if truncated && file.seek(SeekFrom::End(-(budget as i64))).is_err() {
        return Tail {
            total_bytes,
            ..Tail::default()
        };
    }

    let mut bytes = Vec::new();
    if file.take(budget).read_to_end(&mut bytes).is_err() {
        return Tail {
            total_bytes,
            ..Tail::default()
        };
    }

    // Lossy rather than refusing. A log cut mid-character is still the log, and this is the one
    // request whose purpose is to work when other things do not.
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if truncated {
        // The first line is whatever the cut landed in the middle of.
        text = match text.find('\n') {
            Some(newline) => text[newline + 1..].to_owned(),
            None => String::new(),
        };
    }

    Tail {
        text,
        truncated,
        total_bytes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("log directory");
        }
        let mut file = File::create(path).expect("log file");
        file.write_all(contents.as_bytes()).expect("write");
    }

    #[test]
    fn missing_logs_are_not_an_error() {
        let directory = tempfile::tempdir().expect("tempdir");
        let log = read_runtime_log(directory.path(), None, DEFAULT_LOG_BYTES);
        assert!(log.files.is_empty());
        assert!(log.text.contains("no logs yet"));
    }

    #[test]
    fn every_writer_is_listed_including_the_previous_boot() {
        let directory = tempfile::tempdir().expect("tempdir");
        let logs = directory.path().join("logs");
        write(&logs.join("daemon.log"), "live\n");
        write(&logs.join("daemon.log.1"), "rotated\n");
        write(&logs.join("service.log"), "launcher\n");
        write(&logs.join("daemon.stderr.log"), "panic\n");
        write(&logs.join("old/daemon.log"), "previous boot\n");
        // Bookkeeping, not a log.
        write(&logs.join(".boot_id"), "abc\n");

        let log = read_runtime_log(directory.path(), None, DEFAULT_LOG_BYTES);

        let names: Vec<&str> = log.files.iter().map(|file| file.name.as_str()).collect();
        assert!(names.contains(&"daemon.log"));
        assert!(names.contains(&"daemon.log.1"));
        assert!(names.contains(&"service.log"));
        assert!(names.contains(&"daemon.stderr.log"));
        assert!(names.contains(&"old/daemon.log"), "{names:?}");
        assert!(!names.iter().any(|name| name.contains("boot_id")));
    }

    #[test]
    fn a_named_file_is_the_one_read() {
        let directory = tempfile::tempdir().expect("tempdir");
        let logs = directory.path().join("logs");
        write(&logs.join("daemon.log"), "live session\n");
        write(&logs.join("old/daemon.log"), "the boot before\n");

        let log = read_runtime_log(directory.path(), Some("old/daemon.log"), DEFAULT_LOG_BYTES);

        assert_eq!(log.name, "old/daemon.log");
        assert!(log.text.contains("the boot before"));
    }

    /// A name arrives over the wire, so it must not be able to address anything else.
    #[test]
    fn a_name_cannot_escape_the_log_directory() {
        let directory = tempfile::tempdir().expect("tempdir");
        write(&directory.path().join("logs/daemon.log"), "live\n");
        write(&directory.path().join("secret.txt"), "not a log\n");

        for attempt in ["../secret.txt", "old/../../secret.txt", "/etc/hosts"] {
            let log = read_runtime_log(directory.path(), Some(attempt), DEFAULT_LOG_BYTES);
            assert!(
                !log.text.contains("not a log"),
                "{attempt} reached outside the log directory"
            );
        }
    }

    #[test]
    fn an_unknown_name_falls_back_to_the_live_log() {
        let directory = tempfile::tempdir().expect("tempdir");
        write(&directory.path().join("logs/daemon.log"), "live\n");

        let log = read_runtime_log(directory.path(), Some("nope.log"), DEFAULT_LOG_BYTES);

        assert_eq!(log.name, "daemon.log");
        assert!(log.text.contains("live"));
    }

    /// The page opens on whatever comes first, so the order decides what it opens on.
    #[test]
    fn the_listing_leads_with_the_daemons_own_log() {
        let directory = tempfile::tempdir().expect("tempdir");
        let logs = directory.path().join("logs");
        // Written in the order that made the old mtime sort pick the wrong one.
        write(&logs.join("old/daemon.log"), "previous boot\n");
        write(&logs.join("daemon.log.2"), "older\n");
        write(&logs.join("daemon.log.1"), "rotated\n");
        write(&logs.join("daemon.stderr.log"), "panic\n");
        write(&logs.join("daemon.log"), "live\n");
        write(&logs.join("service.log"), "launcher\n");

        let log = read_runtime_log(directory.path(), None, DEFAULT_LOG_BYTES);

        let names: Vec<&str> = log.files.iter().map(|file| file.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "daemon.log",
                "daemon.log.1",
                "daemon.log.2",
                "daemon.stderr.log",
                "service.log",
                "old/daemon.log",
            ],
        );
        assert_eq!(log.name, "daemon.log");
        assert!(log.text.contains("live"));
    }

    /// A cut lands mid-line, and half a line at the top reads as corruption rather than a tail.
    #[test]
    fn a_long_log_is_cut_at_a_line_boundary() {
        let directory = tempfile::tempdir().expect("tempdir");
        let body: String = (0..2000)
            .map(|i| format!("line {i} padding padding\n"))
            .collect();
        write(&directory.path().join("logs/daemon.log"), &body);

        let log = read_runtime_log(directory.path(), None, 4 * 1024);

        assert!(log.truncated);
        assert!(log.total_bytes > 4 * 1024);
        assert!(log.text.contains("line 1999"), "the end is what is kept");
        assert!(!log.text.contains("line 0 padding"));
        for line in log.text.lines().filter(|line| !line.is_empty()) {
            assert!(line.starts_with("line "), "no half line survived: {line:?}");
        }
    }

    #[test]
    fn the_budget_is_clamped_rather_than_trusted() {
        let directory = tempfile::tempdir().expect("tempdir");
        write(&directory.path().join("logs/daemon.log"), "hello\n");

        assert!(
            read_runtime_log(directory.path(), None, 0)
                .text
                .contains("hello")
        );
        assert!(
            read_runtime_log(directory.path(), None, u64::MAX)
                .text
                .contains("hello")
        );
    }
}
