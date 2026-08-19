//! Parser for crash-related files published by Android's DropBox service.

#![forbid(unsafe_code)]

use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};
use thiserror::Error;

pub const MAX_DROPBOX_ENTRY_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DropboxKind {
    JavaCrash,
    Anr,
    NativeCrash,
    NativeRecoverableCrash,
    Wtf,
    StrictMode,
    LowMemory,
    Watchdog,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DropboxFileName {
    pub tag: String,
    pub happened_at_ms: i64,
    pub compressed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DropboxEntry {
    pub file_name: DropboxFileName,
    pub kind: DropboxKind,
    pub process_name: Option<String>,
    pub package_name: Option<String>,
    pub pid: Option<i32>,
    pub uid: Option<i32>,
    pub foreground: Option<bool>,
    pub crash_handler: Option<String>,
    pub dropped_count: Option<u32>,
    pub headers: BTreeMap<String, String>,
    pub body: String,
}

#[derive(Debug, Error)]
pub enum DropboxError {
    #[error("invalid dropbox filename {0}")]
    InvalidFileName(String),
    #[error("dropbox tag contains malformed percent encoding")]
    InvalidTagEncoding,
    #[error("dropbox entry exceeds the {0}-byte safety limit")]
    TooLarge(usize),
    #[error("dropbox entry is not valid UTF-8")]
    InvalidUtf8,
    #[error("failed to read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

pub fn parse_file_name(path: &Path) -> Result<DropboxFileName, DropboxError> {
    let name = path
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or_else(|| DropboxError::InvalidFileName(path.display().to_string()))?;
    let (stem, compressed) = if let Some(v) = name.strip_suffix(".txt.gz") {
        (v, true)
    } else if let Some(v) = name.strip_suffix(".txt") {
        (v, false)
    } else {
        return Err(DropboxError::InvalidFileName(name.to_owned()));
    };
    let (tag, timestamp) = stem
        .rsplit_once('@')
        .ok_or_else(|| DropboxError::InvalidFileName(name.to_owned()))?;
    let happened_at_ms = timestamp
        .parse()
        .map_err(|_| DropboxError::InvalidFileName(name.to_owned()))?;
    let tag = percent_decode(tag)?;
    if tag.is_empty() {
        return Err(DropboxError::InvalidFileName(name.to_owned()));
    }
    Ok(DropboxFileName {
        tag,
        happened_at_ms,
        compressed,
    })
}

pub fn parse_path(path: &Path) -> Result<DropboxEntry, DropboxError> {
    let file_name = parse_file_name(path)?;
    let file = File::open(path).map_err(|source| DropboxError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let reader: Box<dyn Read> = if file_name.compressed {
        Box::new(GzDecoder::new(file))
    } else {
        Box::new(file)
    };
    let bytes = read_limited(reader, path)?;
    parse_bytes(file_name, &bytes)
}

pub fn parse_bytes(file_name: DropboxFileName, bytes: &[u8]) -> Result<DropboxEntry, DropboxError> {
    if bytes.len() > MAX_DROPBOX_ENTRY_BYTES {
        return Err(DropboxError::TooLarge(MAX_DROPBOX_ENTRY_BYTES));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| DropboxError::InvalidUtf8)?;
    let normalized = text.replace("\r\n", "\n");
    let (header_block, body) = normalized
        .split_once("\n\n")
        .map_or(("", normalized.as_str()), |v| v);
    let headers = parse_headers(header_block);
    let process_name = header(&headers, "Process").map(ToOwned::to_owned);
    let package_name = header(&headers, "Package")
        .and_then(|v| v.split_whitespace().next())
        .map(ToOwned::to_owned)
        .or_else(|| process_name.as_deref().map(package_from_process));
    Ok(DropboxEntry {
        kind: kind_from_tag(&file_name.tag),
        file_name,
        process_name,
        package_name,
        pid: parse_header(&headers, "PID"),
        uid: parse_header(&headers, "UID"),
        foreground: header(&headers, "Foreground").and_then(parse_bool),
        crash_handler: header(&headers, "Crash-Handler").map(ToOwned::to_owned),
        dropped_count: parse_header(&headers, "Dropped-Count"),
        headers,
        body: body.to_owned(),
    })
}

fn read_limited(mut reader: Box<dyn Read>, path: &Path) -> Result<Vec<u8>, DropboxError> {
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take((MAX_DROPBOX_ENTRY_BYTES as u64) + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| DropboxError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    if bytes.len() > MAX_DROPBOX_ENTRY_BYTES {
        return Err(DropboxError::TooLarge(MAX_DROPBOX_ENTRY_BYTES));
    }
    Ok(bytes)
}

fn parse_headers(block: &str) -> BTreeMap<String, String> {
    block
        .lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect()
}
fn header<'a>(headers: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}
fn parse_header<T: std::str::FromStr>(headers: &BTreeMap<String, String>, name: &str) -> Option<T> {
    header(headers, name)?.parse().ok()
}
fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "yes" | "true" | "1" => Some(true),
        "no" | "false" | "0" => Some(false),
        _ => None,
    }
}
fn package_from_process(process: &str) -> String {
    process.split(':').next().unwrap_or(process).to_owned()
}
fn kind_from_tag(tag: &str) -> DropboxKind {
    if tag.ends_with("native_recoverable_crash") {
        DropboxKind::NativeRecoverableCrash
    } else if tag.ends_with("native_crash") || tag.starts_with("SYSTEM_TOMBSTONE") {
        DropboxKind::NativeCrash
    } else if tag.ends_with("_crash") {
        DropboxKind::JavaCrash
    } else if tag.ends_with("_anr") {
        DropboxKind::Anr
    } else if tag.ends_with("_wtf") {
        DropboxKind::Wtf
    } else if tag.ends_with("_strictmode") {
        DropboxKind::StrictMode
    } else if tag.ends_with("_lowmem") {
        DropboxKind::LowMemory
    } else if tag.ends_with("_watchdog") || tag.ends_with("_pre_watchdog") {
        DropboxKind::Watchdog
    } else {
        DropboxKind::Unknown
    }
}
fn percent_decode(value: &str) -> Result<String, DropboxError> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let h = bytes
                .get(i + 1)
                .and_then(|b| hex(*b))
                .ok_or(DropboxError::InvalidTagEncoding)?;
            let l = bytes
                .get(i + 2)
                .and_then(|b| hex(*b))
                .ok_or(DropboxError::InvalidTagEncoding)?;
            out.push((h << 4) | l);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| DropboxError::InvalidTagEncoding)
}
const fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_name_and_headers() {
        let name =
            parse_file_name(Path::new("data_app_crash%3Asecondary@1720000000123.txt.gz")).unwrap();
        assert_eq!(name.tag, "data_app_crash:secondary");
        let entry = parse_bytes(
            DropboxFileName {
                tag: "data_app_crash".into(),
                happened_at_ms: 1,
                compressed: false,
            },
            b"Process: com.example:worker\nPID: 42\nForeground: Yes\nDropped-Count: 7\n\nError",
        )
        .unwrap();
        assert_eq!(entry.package_name.as_deref(), Some("com.example"));
        assert_eq!(entry.dropped_count, Some(7));
    }
    #[test]
    fn rejects_unknown_extensions() {
        assert!(matches!(
            parse_file_name(Path::new("data_app_crash@1.tmp")),
            Err(DropboxError::InvalidFileName(_))
        ));
    }
}
