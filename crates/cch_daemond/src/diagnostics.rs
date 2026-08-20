//! The daemon's own logs, for a user looking at why something is not working.
//!
//! Two files, both written by the module rather than by any Android facility:
//!
//! - `logs/daemon.log` — this process's `tracing` output, redirected by `service.sh`.
//! - `logs/service.log` — the launcher's own record of exits and restart attempts, which is the
//!   only place a daemon that died before it could log anything leaves a trace.
//!
//! Read from the tail. Nothing rotates these, and the answer to "why did it stop working" is
//! always at the end.

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

/// Files are read from the end; this is how much of each is kept.
pub const DEFAULT_LOG_BYTES: u64 = 128 * 1024;
/// Ceiling on what one request can ask for, so the answer still fits in a frame.
pub const MAX_LOG_BYTES: u64 = 512 * 1024;

pub struct RuntimeLog {
    pub text: String,
    /// Whether anything was cut from the front.
    pub truncated: bool,
    /// Size of the logs on disk, which is what tells a reader the tail is a tail.
    pub total_bytes: u64,
}

/// Reads the tail of both logs, newest file last.
pub fn read_runtime_log(state_dir: &Path, max_bytes: u64) -> RuntimeLog {
    let budget = max_bytes.clamp(4 * 1024, MAX_LOG_BYTES);
    let logs = state_dir.join("logs");

    // Half the budget each, so a chatty daemon cannot push the launcher's exit codes out of the
    // answer — those are the lines that explain a daemon which is not running at all.
    let service = read_tail(&logs.join("service.log"), budget / 2);
    let daemon = read_tail(&logs.join("daemon.log"), budget - budget / 2);

    let mut text = String::new();
    if let Some(section) = section("service.log", &service) {
        text.push_str(&section);
    }
    if let Some(section) = section("daemon.log", &daemon) {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&section);
    }
    if text.is_empty() {
        text.push_str("(no logs yet)\n");
    }

    RuntimeLog {
        text,
        truncated: service.truncated || daemon.truncated,
        total_bytes: service.total_bytes + daemon.total_bytes,
    }
}

fn section(name: &str, tail: &Tail) -> Option<String> {
    let body = tail.text.trim_end();
    if body.is_empty() {
        return None;
    }
    let head = if tail.truncated {
        format!(
            "===== {name} (last {} bytes of {}) =====",
            body.len(),
            tail.total_bytes
        )
    } else {
        format!("===== {name} ({} bytes) =====", tail.total_bytes)
    };
    Some(format!("{head}\n{body}\n"))
}

#[derive(Default)]
struct Tail {
    text: String,
    truncated: bool,
    total_bytes: u64,
}

/// The last `budget` bytes of a file, starting at a line boundary.
///
/// A missing file is not an error: `service.log` only exists once the launcher has had something
/// to say, and asking for logs is what a user does when things are already odd.
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

    // Lossy rather than refusing: a log cut mid-character is still the log, and this is the one
    // request whose whole purpose is to work when other things do not.
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
            std::fs::create_dir_all(parent).expect("log directory");
        }
        let mut file = File::create(path).expect("log file");
        file.write_all(contents.as_bytes()).expect("write");
    }

    #[test]
    fn missing_logs_are_not_an_error() {
        let directory = tempfile::tempdir().expect("tempdir");
        let log = read_runtime_log(directory.path(), DEFAULT_LOG_BYTES);
        assert!(!log.truncated);
        assert_eq!(log.total_bytes, 0);
        assert!(log.text.contains("no logs yet"));
    }

    #[test]
    fn both_files_appear_with_the_launcher_first() {
        let directory = tempfile::tempdir().expect("tempdir");
        write(
            &directory.path().join("logs/service.log"),
            "2026-08-20 daemon exited code=1 attempt=1\n",
        );
        write(
            &directory.path().join("logs/daemon.log"),
            "INFO catcherd: crashcatcher daemon ready\n",
        );

        let log = read_runtime_log(directory.path(), DEFAULT_LOG_BYTES);

        let service = log.text.find("service.log").expect("service section");
        let daemon = log.text.find("daemon.log").expect("daemon section");
        assert!(service < daemon, "the launcher's record comes first");
        assert!(log.text.contains("daemon exited code=1"));
        assert!(log.text.contains("crashcatcher daemon ready"));
        assert!(!log.truncated);
    }

    /// A cut lands mid-line, and half a line at the top reads as corruption rather than as a
    /// tail. It is dropped, and the header says the file is longer than what is shown.
    #[test]
    fn a_long_log_is_cut_at_a_line_boundary() {
        let directory = tempfile::tempdir().expect("tempdir");
        let body: String = (0..2000)
            .map(|i| format!("line {i} padding padding\n"))
            .collect();
        write(&directory.path().join("logs/daemon.log"), &body);

        let log = read_runtime_log(directory.path(), 4 * 1024);

        assert!(log.truncated);
        assert!(log.total_bytes > 4 * 1024);
        assert!(log.text.contains("line 1999"), "the end is what is kept");
        assert!(!log.text.contains("line 0 padding"));
        for line in log.text.lines().skip(1) {
            assert!(
                line.is_empty() || line.starts_with("line ") || line.starts_with("====="),
                "no half line survived: {line:?}"
            );
        }
    }

    #[test]
    fn the_budget_is_clamped_rather_than_trusted() {
        let directory = tempfile::tempdir().expect("tempdir");
        write(&directory.path().join("logs/daemon.log"), "hello\n");

        // Absurd in both directions; neither may panic or read the whole disk.
        assert!(read_runtime_log(directory.path(), 0).text.contains("hello"));
        assert!(
            read_runtime_log(directory.path(), u64::MAX)
                .text
                .contains("hello")
        );
    }
}
