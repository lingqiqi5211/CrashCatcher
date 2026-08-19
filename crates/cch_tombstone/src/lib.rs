//! Android tombstone parser: protobuf first, legacy rendered text as fallback.

#![forbid(unsafe_code)]
mod proto;

use prost::Message;
pub use proto::{Architecture, BacktraceFrame, Signal, Thread, Tombstone};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MAX_TOMBSTONE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TombstoneFormat {
    Protobuf,
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeFrame {
    pub relative_pc: u64,
    pub function_name: Option<String>,
    pub function_offset: Option<u64>,
    pub file_name: Option<String>,
    pub build_id: Option<String>,
}
impl NativeFrame {
    #[must_use]
    pub fn normalized(&self) -> String {
        match (&self.function_name, &self.file_name) {
            (Some(f), Some(file)) => format!("{file}!{f}"),
            (Some(f), None) => f.clone(),
            (None, Some(file)) => format!("{file}+0x{:x}", self.relative_pc),
            (None, None) => format!("pc+0x{:x}", self.relative_pc),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TombstoneReport {
    pub format: TombstoneFormat,
    pub pid: i32,
    pub tid: i32,
    pub uid: Option<i32>,
    pub process_name: String,
    pub thread_name: Option<String>,
    pub signal_number: Option<i32>,
    pub signal_name: Option<String>,
    pub signal_code: Option<i32>,
    pub signal_code_name: Option<String>,
    pub abort_message: Option<String>,
    pub cause: Option<String>,
    pub frames: Vec<NativeFrame>,
    pub raw_text: Option<String>,
}
impl TombstoneReport {
    #[must_use]
    pub fn package_name(&self) -> &str {
        self.process_name
            .split(':')
            .next()
            .unwrap_or(&self.process_name)
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TombstoneError {
    #[error("tombstone exceeds the {0}-byte safety limit")]
    TooLarge(usize),
    #[error("invalid tombstone protobuf: {0}")]
    InvalidProtobuf(String),
    #[error("text tombstone is missing pid/tid/process header")]
    MissingProcessHeader,
    #[error("text tombstone pid or tid is malformed")]
    InvalidProcessHeader,
}

pub fn parse_proto(bytes: &[u8]) -> Result<TombstoneReport, TombstoneError> {
    if bytes.len() > MAX_TOMBSTONE_BYTES {
        return Err(TombstoneError::TooLarge(MAX_TOMBSTONE_BYTES));
    }
    let t = Tombstone::decode(bytes).map_err(|e| TombstoneError::InvalidProtobuf(e.to_string()))?;
    let thread = t.threads.get(&t.tid);
    let signal = t.signal_info.as_ref();
    let frames = thread
        .map(|v| v.current_backtrace.as_slice())
        .unwrap_or_default()
        .iter()
        .map(|f| NativeFrame {
            relative_pc: f.rel_pc,
            function_name: non_empty(f.function_name.clone()),
            function_offset: (f.function_offset != 0).then_some(f.function_offset),
            file_name: non_empty(f.file_name.clone()),
            build_id: non_empty(f.build_id.clone()),
        })
        .collect();
    Ok(TombstoneReport {
        format: TombstoneFormat::Protobuf,
        pid: i32::try_from(t.pid).unwrap_or(i32::MAX),
        tid: i32::try_from(t.tid).unwrap_or(i32::MAX),
        uid: i32::try_from(t.uid).ok(),
        process_name: t.process_name,
        thread_name: thread.map(|v| v.name.clone()),
        signal_number: signal.map(|v| v.number),
        signal_name: signal.and_then(|v| non_empty(v.name.clone())),
        signal_code: signal.map(|v| v.code),
        signal_code_name: signal.and_then(|v| non_empty(v.code_name.clone())),
        abort_message: non_empty(t.abort_message),
        cause: t.cause.and_then(|v| non_empty(v.human_readable)),
        frames,
        raw_text: None,
    })
}

pub fn parse_text(bytes: &[u8]) -> Result<TombstoneReport, TombstoneError> {
    if bytes.len() > MAX_TOMBSTONE_BYTES {
        return Err(TombstoneError::TooLarge(MAX_TOMBSTONE_BYTES));
    }
    let raw = String::from_utf8_lossy(bytes).replace("\r\n", "\n");
    let header = raw
        .lines()
        .find(|l| l.trim_start().starts_with("pid:"))
        .ok_or(TombstoneError::MissingProcessHeader)?;
    let (pid, tid, thread_name, process_name) = parse_process_header(header)?;
    let sig = raw
        .lines()
        .find(|l| l.trim_start().starts_with("signal "))
        .map(parse_signal)
        .unwrap_or_default();
    let abort_message = raw
        .lines()
        .find_map(|l| l.trim().strip_prefix("Abort message:"))
        .map(str::trim)
        .map(trim_quotes)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned);
    let cause = raw
        .lines()
        .find_map(|l| l.trim().strip_prefix("Cause:"))
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned);
    let frames = raw.lines().filter_map(parse_frame).collect();
    Ok(TombstoneReport {
        format: TombstoneFormat::Text,
        pid,
        tid,
        uid: raw
            .lines()
            .find_map(|l| l.trim().strip_prefix("uid:")?.trim().parse().ok()),
        process_name,
        thread_name,
        signal_number: sig.number,
        signal_name: sig.name,
        signal_code: sig.code,
        signal_code_name: sig.code_name,
        abort_message,
        cause,
        frames,
        raw_text: Some(raw),
    })
}

fn non_empty(v: String) -> Option<String> {
    (!v.is_empty()).then_some(v)
}
fn parse_process_header(line: &str) -> Result<(i32, i32, Option<String>, String), TombstoneError> {
    let parts: Vec<&str> = line.split(',').map(str::trim).collect();
    let pid = parts
        .iter()
        .find_map(|p| p.strip_prefix("pid:"))
        .and_then(|v| v.trim().parse().ok())
        .ok_or(TombstoneError::InvalidProcessHeader)?;
    let tid = parts
        .iter()
        .find_map(|p| p.strip_prefix("tid:"))
        .and_then(|v| v.trim().parse().ok())
        .ok_or(TombstoneError::InvalidProcessHeader)?;
    let thread = parts
        .iter()
        .find_map(|p| p.strip_prefix("name:"))
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned);
    let process = line
        .split_once(">>>")
        .and_then(|(_, r)| r.split_once("<<<"))
        .map(|(v, _)| v.trim())
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| thread.clone())
        .ok_or(TombstoneError::MissingProcessHeader)?;
    Ok((pid, tid, thread, process))
}
#[derive(Default)]
struct ParsedSignal {
    number: Option<i32>,
    name: Option<String>,
    code: Option<i32>,
    code_name: Option<String>,
}
fn parse_signal(line: &str) -> ParsedSignal {
    let mut r = ParsedSignal::default();
    let t = line.trim();
    if let Some(v) = t.strip_prefix("signal ") {
        let first = v.split(',').next().unwrap_or(v);
        r.number = first.split_whitespace().next().and_then(|x| x.parse().ok());
        r.name = between(first).map(ToOwned::to_owned);
    }
    if let Some(c) = t.split(',').map(str::trim).find(|p| p.starts_with("code ")) {
        let v = c.strip_prefix("code ").unwrap_or(c);
        r.code = v.split_whitespace().next().and_then(|x| x.parse().ok());
        r.code_name = between(v).map(ToOwned::to_owned);
    }
    r
}
fn between(v: &str) -> Option<&str> {
    let s = v.find('(')? + 1;
    let e = s + v[s..].find(')')?;
    v.get(s..e)
}
fn trim_quotes(v: &str) -> &str {
    v.strip_prefix('"')
        .and_then(|x| x.strip_suffix('"'))
        .unwrap_or(v)
}
fn parse_frame(line: &str) -> Option<NativeFrame> {
    let t = line.trim();
    let after = t
        .strip_prefix('#')?
        .split_once(' ')?
        .1
        .trim()
        .strip_prefix("pc ")?;
    let mut p = after.split_whitespace();
    let pc = u64::from_str_radix(p.next()?, 16).ok()?;
    let file = p.next().map(ToOwned::to_owned);
    let block = after
        .find(" (")
        .and_then(|i| after.get(i + 2..))
        .and_then(|v| v.split(')').next());
    let (function, offset) = block.map_or((None, None), |b| {
        if b.starts_with("BuildId:") {
            (None, None)
        } else {
            b.rsplit_once('+')
                .map_or((non_empty(b.trim().to_owned()), None), |(n, o)| {
                    (non_empty(n.trim().to_owned()), o.parse().ok())
                })
        }
    });
    let build = after
        .split("BuildId:")
        .nth(1)
        .and_then(|v| v.split(')').next())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned);
    Some(NativeFrame {
        relative_pc: pc,
        function_name: function,
        function_offset: offset,
        file_name: file,
        build_id: build,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    #[test]
    fn parses_proto() {
        let mut threads = HashMap::new();
        threads.insert(
            42,
            Thread {
                id: 42,
                name: "worker".into(),
                registers: vec![],
                current_backtrace: vec![BacktraceFrame {
                    rel_pc: 1,
                    pc: 0,
                    sp: 0,
                    function_name: "abort".into(),
                    function_offset: 4,
                    file_name: "/lib/libc.so".into(),
                    file_map_offset: 0,
                    build_id: "a".into(),
                }],
                memory_dump: vec![],
            },
        );
        let t = Tombstone {
            arch: 1,
            build_fingerprint: String::new(),
            revision: String::new(),
            timestamp: String::new(),
            pid: 41,
            tid: 42,
            uid: 10123,
            selinux_label: String::new(),
            process_name: "com.example:native".into(),
            signal_info: Some(Signal {
                number: 11,
                name: "SIGSEGV".into(),
                code: 1,
                code_name: "SEGV_MAPERR".into(),
                has_sender: false,
                sender_uid: 0,
                sender_pid: 0,
                has_fault_address: true,
                fault_address: 0,
            }),
            abort_message: String::new(),
            cause: None,
            threads,
            memory_mappings: vec![],
            log_buffers: vec![],
            open_fds: vec![],
        };
        let r = parse_proto(&t.encode_to_vec()).unwrap();
        assert_eq!(r.signal_name.as_deref(), Some("SIGSEGV"));
        assert_eq!(r.frames[0].normalized(), "/lib/libc.so!abort");
    }
    #[test]
    fn parses_legacy() {
        let r=parse_text(b"pid: 7, tid: 8, name: w  >>> com.example:w <<<\nuid: 10123\nsignal 11 (SIGSEGV), code 1 (SEGV_MAPERR)\n#00 pc 00000123 /lib/libc.so (abort+4)\n").unwrap();
        assert_eq!(r.package_name(), "com.example");
        assert_eq!(r.frames.len(), 1);
    }
}
