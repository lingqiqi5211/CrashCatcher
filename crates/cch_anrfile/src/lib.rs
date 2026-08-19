//! Parser for Android `/data/anr/anr_<timestamp>` thread dumps.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MAX_ANR_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnrThread {
    pub name: String,
    pub tid: Option<i32>,
    pub state: Option<String>,
    pub lines: Vec<String>,
}

impl AnrThread {
    #[must_use]
    pub fn top_frame(&self) -> Option<&str> {
        self.lines
            .iter()
            .map(String::as_str)
            .map(str::trim)
            .find(|line| line.starts_with("at ") || line.starts_with("native:"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnrReport {
    pub pid: i32,
    pub process_name: String,
    pub timestamp: Option<String>,
    pub threads: Vec<AnrThread>,
    pub raw: String,
}

impl AnrReport {
    #[must_use]
    pub fn package_name(&self) -> &str {
        self.process_name
            .split(':')
            .next()
            .unwrap_or(&self.process_name)
    }
    #[must_use]
    pub fn main_thread(&self) -> Option<&AnrThread> {
        self.threads.iter().find(|thread| thread.name == "main")
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AnrError {
    #[error("ANR dump exceeds the {0}-byte safety limit")]
    TooLarge(usize),
    #[error("ANR dump is missing its pid header")]
    MissingPid,
    #[error("ANR dump is missing Cmd line")]
    MissingProcess,
    #[error("ANR pid is malformed")]
    InvalidPid,
}

pub fn parse_anr(bytes: &[u8]) -> Result<AnrReport, AnrError> {
    if bytes.len() > MAX_ANR_BYTES {
        return Err(AnrError::TooLarge(MAX_ANR_BYTES));
    }
    let raw = String::from_utf8_lossy(bytes).replace("\r\n", "\n");
    let header = raw
        .lines()
        .find(|line| line.starts_with("----- pid "))
        .ok_or(AnrError::MissingPid)?;
    let (pid, timestamp) = parse_pid_header(header)?;
    let process_name = raw
        .lines()
        .find_map(|line| line.trim().strip_prefix("Cmd line:"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or(AnrError::MissingProcess)?;
    Ok(AnrReport {
        pid,
        process_name,
        timestamp,
        threads: parse_threads(&raw),
        raw,
    })
}

fn parse_pid_header(header: &str) -> Result<(i32, Option<String>), AnrError> {
    let body = header
        .strip_prefix("----- pid ")
        .ok_or(AnrError::MissingPid)?;
    let (pid_text, rest) = body.split_once(' ').ok_or(AnrError::InvalidPid)?;
    let pid = pid_text.parse::<i32>().map_err(|_| AnrError::InvalidPid)?;
    let timestamp = rest
        .strip_prefix("at ")
        .and_then(|v| v.strip_suffix(" -----"))
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned);
    Ok((pid, timestamp))
}

fn parse_threads(raw: &str) -> Vec<AnrThread> {
    let mut threads = Vec::new();
    let mut current = None;
    for line in raw.lines() {
        if let Some(thread) = parse_thread_header(line) {
            if let Some(previous) = current.replace(thread) {
                threads.push(previous);
            }
        } else if let Some(thread) = current.as_mut() {
            if line.starts_with("----- end ") {
                if let Some(previous) = current.take() {
                    threads.push(previous);
                }
            } else {
                thread.lines.push(line.to_owned());
            }
        }
    }
    if let Some(thread) = current {
        threads.push(thread);
    }
    threads
}

fn parse_thread_header(line: &str) -> Option<AnrThread> {
    let body = line.trim_start().strip_prefix('"')?;
    let quote = body.find('"')?;
    let metadata = body[quote + 1..].trim();
    let tid = metadata
        .split_whitespace()
        .find_map(|part| part.strip_prefix("tid="))
        .and_then(|value| value.parse::<i32>().ok());
    Some(AnrThread {
        name: body[..quote].to_owned(),
        tid,
        state: metadata.split_whitespace().last().map(ToOwned::to_owned),
        lines: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_main_thread_and_process() {
        let sample = br#"----- pid 1234 at 2026-08-17 10:11:12.345 -----
Cmd line: com.example:worker
"main" prio=5 tid=1 Native
  at com.example.Home.onCreate(Home.kt:20)
"FinalizerWatchdogDaemon" daemon prio=5 tid=7 Waiting
  at java.lang.Object.wait(Native method)
----- end 1234 -----
"#;
        let report = parse_anr(sample).unwrap();
        assert_eq!(report.package_name(), "com.example");
        assert_eq!(report.threads.len(), 2);
        assert_eq!(
            report.main_thread().and_then(AnrThread::top_frame),
            Some("at com.example.Home.onCreate(Home.kt:20)")
        );
    }
}
