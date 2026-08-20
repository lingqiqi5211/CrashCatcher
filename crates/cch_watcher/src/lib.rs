//! Startup scan and inotify discovery for Android crash artifact directories.

#![deny(unsafe_op_in_unsafe_fn)]
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};
use thiserror::Error;
pub const IN_CLOSE_WRITE_MASK: u32 = 0x8;
pub const IN_MOVED_TO_MASK: u32 = 0x80;
pub const IN_CREATE_MASK: u32 = 0x100;
pub const IN_DELETE_MASK: u32 = 0x200;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WatchKind {
    Dropbox,
    Tombstone,
    Anr,
}
impl WatchKind {
    const fn prefix(self) -> &'static str {
        match self {
            Self::Dropbox => "dropbox",
            Self::Tombstone => "tombstone",
            Self::Anr => "anr",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchRoot {
    pub kind: WatchKind,
    pub path: PathBuf,
}
impl WatchRoot {
    #[must_use]
    pub fn new(kind: WatchKind, path: impl Into<PathBuf>) -> Self {
        Self {
            kind,
            path: path.into(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceIdentity {
    pub key: String,
    pub size: u64,
    pub modified_ns: u128,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredSource {
    pub kind: WatchKind,
    pub path: PathBuf,
    pub preferred_path: PathBuf,
    pub identity: SourceIdentity,
}
pub trait IngestedRegistry {
    fn contains(&self, key: &str) -> Result<bool, String>;
}
#[derive(Debug, Clone, Copy, Default)]
pub struct EmptyRegistry;
impl IngestedRegistry for EmptyRegistry {
    fn contains(&self, _: &str) -> Result<bool, String> {
        Ok(false)
    }
}

pub fn startup_scan<R: IngestedRegistry>(
    roots: &[WatchRoot],
    registry: &R,
) -> Result<Vec<DiscoveredSource>, WatcherError> {
    let mut out = vec![];
    for root in roots {
        let entries = match fs::read_dir(&root.path) {
            Ok(v) => v,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(source) => {
                return Err(WatcherError::Io {
                    path: root.path.clone(),
                    source,
                });
            }
        };
        for entry in entries {
            let entry = entry.map_err(|source| WatcherError::Io {
                path: root.path.clone(),
                source,
            })?;
            let Some(name) = entry.file_name().to_str().map(ToOwned::to_owned) else {
                continue;
            };
            if completed_name(root.kind, &name)
                && let Some(source) = discover(root.kind, entry.path())?
                && !registry
                    .contains(&source.identity.key)
                    .map_err(WatcherError::Registry)?
            {
                out.push(source)
            }
        }
    }
    out.sort_by_key(|v| v.identity.modified_ns);
    Ok(out)
}
pub fn source_identity(kind: WatchKind, path: &Path) -> Result<SourceIdentity, WatcherError> {
    let m = fs::metadata(path).map_err(|source| WatcherError::Io {
        path: path.into(),
        source,
    })?;
    if !m.is_file() {
        return Err(WatcherError::NotAFile(path.into()));
    }
    let ns = m
        .modified()
        .map_err(|source| WatcherError::Io {
            path: path.into(),
            source,
        })?
        .duration_since(UNIX_EPOCH)
        .map_err(|_| WatcherError::BeforeUnixEpoch(path.into()))?
        .as_nanos();
    let name = path
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or_else(|| WatcherError::InvalidName(path.into()))?;
    let size = m.len();
    Ok(SourceIdentity {
        key: format!("{}:{name}:{ns}:{size}", kind.prefix()),
        size,
        modified_ns: ns,
    })
}
fn discover(kind: WatchKind, path: PathBuf) -> Result<Option<DiscoveredSource>, WatcherError> {
    match source_identity(kind, &path) {
        Ok(identity) => {
            let preferred = if kind == WatchKind::Tombstone {
                let p = path.with_extension("pb");
                if p.is_file() { p } else { path.clone() }
            } else {
                path.clone()
            };
            Ok(Some(DiscoveredSource {
                kind,
                path,
                preferred_path: preferred,
                identity,
            }))
        }
        Err(WatcherError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}
#[must_use]
pub fn is_completion_event(kind: WatchKind, name: &str, mask: u32) -> bool {
    if mask & IN_DELETE_MASK != 0 || !completed_name(kind, name) {
        return false;
    }
    match kind {
        // Tombstoned creates or truncates a numbered slot before writing its header. Treating
        // IN_CREATE as completion reads an empty prefix, reports a missing process header, and
        // never retries because the later close was not watched. Reused slots do not emit create
        // at all, so close-write is the only reliable completion signal for both cases.
        WatchKind::Tombstone => mask & (IN_CLOSE_WRITE_MASK | IN_MOVED_TO_MASK) != 0,
        WatchKind::Dropbox => mask & IN_MOVED_TO_MASK != 0,
        WatchKind::Anr => mask & (IN_CLOSE_WRITE_MASK | IN_MOVED_TO_MASK) != 0,
    }
}
fn completed_name(kind: WatchKind, name: &str) -> bool {
    match kind {
        WatchKind::Tombstone => name
            .strip_prefix("tombstone_")
            .is_some_and(|v| v.len() == 2 && v.bytes().all(|b| b.is_ascii_digit())),
        WatchKind::Dropbox => {
            name.contains('@') && (name.ends_with(".txt") || name.ends_with(".txt.gz"))
        }
        WatchKind::Anr => name.starts_with("anr_") && !name.ends_with(".tmp"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawInotifyEvent {
    pub watch_descriptor: i32,
    pub mask: u32,
    pub cookie: u32,
    pub name: String,
}
pub fn parse_inotify_events(bytes: &[u8]) -> Result<Vec<RawInotifyEvent>, WatcherError> {
    let mut out = vec![];
    let mut p = 0;
    while p < bytes.len() {
        if bytes.len() - p < 16 {
            return Err(WatcherError::MalformedEvent("truncated header"));
        }
        let h = &bytes[p..p + 16];
        let wd = i32::from_ne_bytes(copy4(&h[..4])?);
        let mask = u32::from_ne_bytes(copy4(&h[4..8])?);
        let cookie = u32::from_ne_bytes(copy4(&h[8..12])?);
        let len = usize::try_from(u32::from_ne_bytes(copy4(&h[12..16])?))
            .map_err(|_| WatcherError::MalformedEvent("name length overflow"))?;
        p += 16;
        if len > bytes.len() - p {
            return Err(WatcherError::MalformedEvent("truncated name"));
        }
        let raw = &bytes[p..p + len];
        let end = raw.iter().position(|b| *b == 0).unwrap_or(raw.len());
        let name = std::str::from_utf8(&raw[..end])
            .map_err(|_| WatcherError::MalformedEvent("non-UTF8 name"))?
            .to_owned();
        out.push(RawInotifyEvent {
            watch_descriptor: wd,
            mask,
            cookie,
            name,
        });
        p += len;
    }
    Ok(out)
}
fn copy4(b: &[u8]) -> Result<[u8; 4], WatcherError> {
    b.try_into()
        .map_err(|_| WatcherError::MalformedEvent("truncated integer"))
}

#[cfg(any(target_os = "android", target_os = "linux"))]
mod live {
    use super::*;
    use std::{
        collections::HashMap,
        ffi::CString,
        fs::File,
        io::Read,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::ffi::OsStrExt,
        },
    };
    pub struct InotifyWatcher {
        file: File,
        roots: HashMap<i32, WatchRoot>,
    }
    impl InotifyWatcher {
        pub fn new(roots: &[WatchRoot]) -> Result<Self, WatcherError> {
            // SAFETY: no pointer arguments; successful fd is transferred exactly once to File.
            let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC | libc::IN_NONBLOCK) };
            if fd < 0 {
                return Err(WatcherError::Inotify(io::Error::last_os_error()));
            } // SAFETY: fd is fresh and uniquely owned.
            let file = unsafe { File::from_raw_fd(fd) };
            let mut watcher = Self {
                file,
                roots: HashMap::new(),
            };
            for root in roots {
                watcher.add(root.clone())?
            }
            Ok(watcher)
        }
        fn add(&mut self, root: WatchRoot) -> Result<(), WatcherError> {
            let path = CString::new(root.path.as_os_str().as_bytes())
                .map_err(|_| WatcherError::InvalidName(root.path.clone()))?;
            let mask = match root.kind {
                WatchKind::Tombstone => IN_CLOSE_WRITE_MASK | IN_MOVED_TO_MASK | IN_DELETE_MASK,
                WatchKind::Dropbox => IN_MOVED_TO_MASK,
                WatchKind::Anr => IN_CLOSE_WRITE_MASK | IN_MOVED_TO_MASK,
            }; // SAFETY: fd and NUL path are valid for the call and no pointer is retained.
            let wd = unsafe { libc::inotify_add_watch(self.file.as_raw_fd(), path.as_ptr(), mask) };
            if wd < 0 {
                return Err(WatcherError::Io {
                    path: root.path,
                    source: io::Error::last_os_error(),
                });
            }
            self.roots.insert(wd, root);
            Ok(())
        }
        pub fn poll(&mut self) -> Result<Vec<DiscoveredSource>, WatcherError> {
            let mut buffer = [0; 64 * 1024];
            let n = match self.file.read(&mut buffer) {
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(vec![]),
                Err(e) => return Err(WatcherError::Inotify(e)),
            };
            let mut out = vec![];
            for e in parse_inotify_events(&buffer[..n])? {
                let Some(root) = self.roots.get(&e.watch_descriptor) else {
                    continue;
                };
                if is_completion_event(root.kind, &e.name, e.mask)
                    && let Some(v) = discover(root.kind, root.path.join(e.name))?
                {
                    out.push(v)
                }
            }
            Ok(out)
        }
    }
}
#[cfg(any(target_os = "android", target_os = "linux"))]
pub use live::InotifyWatcher;
#[cfg(not(any(target_os = "android", target_os = "linux")))]
pub struct InotifyWatcher;
#[cfg(not(any(target_os = "android", target_os = "linux")))]
impl InotifyWatcher {
    pub fn new(_: &[WatchRoot]) -> Result<Self, WatcherError> {
        Err(WatcherError::UnsupportedPlatform)
    }
    pub fn poll(&mut self) -> Result<Vec<DiscoveredSource>, WatcherError> {
        Err(WatcherError::UnsupportedPlatform)
    }
}

#[derive(Debug, Error)]
pub enum WatcherError {
    #[error("failed to access {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("not a regular file: {0}")]
    NotAFile(PathBuf),
    #[error("invalid filename: {0}")]
    InvalidName(PathBuf),
    #[error("timestamp predates Unix epoch: {0}")]
    BeforeUnixEpoch(PathBuf),
    #[error("ingested registry failed: {0}")]
    Registry(String),
    #[error("malformed inotify event: {0}")]
    MalformedEvent(&'static str),
    #[error("inotify failed: {0}")]
    Inotify(#[source] io::Error),
    #[error("inotify unsupported on this host")]
    UnsupportedPlatform,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    #[test]
    fn completion_rules() {
        assert!(!is_completion_event(
            WatchKind::Tombstone,
            "tombstone_07",
            IN_CREATE_MASK
        ));
        assert!(is_completion_event(
            WatchKind::Tombstone,
            "tombstone_07",
            IN_CLOSE_WRITE_MASK
        ));
        assert!(is_completion_event(
            WatchKind::Tombstone,
            "tombstone_07",
            IN_MOVED_TO_MASK
        ));
        assert!(!is_completion_event(
            WatchKind::Tombstone,
            "tombstone_07.pb",
            IN_CREATE_MASK
        ));
        assert!(!is_completion_event(
            WatchKind::Dropbox,
            "data_app_crash@1.txt",
            IN_CREATE_MASK
        ));
        assert!(is_completion_event(
            WatchKind::Dropbox,
            "data_app_crash@1.txt",
            IN_MOVED_TO_MASK
        ));
    }
    #[test]
    fn scan_prefers_proto() {
        let d = tempfile::tempdir().unwrap();
        let text = d.path().join("tombstone_01");
        let pb = d.path().join("tombstone_01.pb");
        File::create(&text).unwrap();
        File::create(&pb).unwrap();
        let out = startup_scan(
            &[WatchRoot::new(WatchKind::Tombstone, d.path())],
            &EmptyRegistry,
        )
        .unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].preferred_path, pb);
    }
}
