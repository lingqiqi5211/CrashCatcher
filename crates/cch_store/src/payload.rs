use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use cch_model::{PayloadCodec, PayloadSource, PayloadState, RecordId};
use tracing::warn;

use crate::StoreError;

/// Compression level for payload files.
///
/// Level 3 is zstd's default and the right end of the curve here: stack traces are
/// highly repetitive text, and the ingest path runs during a crash storm, so
/// spending CPU for a marginally smaller file is the wrong trade.
const COMPRESSION_LEVEL: i32 = 3;

/// Directory name under the store root.
const PAYLOAD_DIR: &str = "payloads";

/// Outcome of storing one payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrittenPayload {
    /// Path relative to the payload root, as stored in the database.
    pub relative_path: String,
    /// Bytes actually occupied on disk, after compression.
    pub stored_bytes: u64,
    /// Bytes of text the reader will see.
    pub text_bytes: u64,
    pub codec: PayloadCodec,
    pub state: PayloadState,
}

/// Owns the payload files that sit beside the SQLite index.
///
/// Payloads live outside the database on purpose. A tombstone or an all-threads
/// ANR dump is orders of magnitude larger than an index row; keeping them in the
/// database would evict useful index pages from the page cache on every list
/// query, and reclaiming their space after a delete would need a `VACUUM`. As
/// files, reclaiming is an `unlink` and the space comes back immediately.
#[derive(Debug, Clone)]
pub struct PayloadStore {
    root: PathBuf,
}

impl PayloadStore {
    #[must_use]
    pub fn new(store_root: impl AsRef<Path>) -> Self {
        Self {
            root: store_root.as_ref().join(PAYLOAD_DIR),
        }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Stores a payload, truncating it at `max_bytes`.
    ///
    /// Returns `Ok(None)` when there is nothing to store. Truncation is recorded in
    /// [`WrittenPayload::state`] so the detail screen can say the text is
    /// incomplete rather than letting the user assume they are seeing everything.
    pub fn write(
        &self,
        id: &RecordId,
        source: &PayloadSource,
        max_bytes: u64,
    ) -> Result<Option<WrittenPayload>, StoreError> {
        let raw = match source {
            PayloadSource::None => return Ok(None),
            PayloadSource::Inline(bytes) if bytes.is_empty() => return Ok(None),
            PayloadSource::Inline(bytes) => bytes.clone(),
            PayloadSource::File(path) => fs::read(path).map_err(|source| StoreError::Io {
                path: path.clone(),
                source,
            })?,
        };
        if raw.is_empty() {
            return Ok(None);
        }

        let cap = usize::try_from(max_bytes).unwrap_or(usize::MAX);
        let (bytes, state) = if raw.len() > cap {
            (truncate_utf8(&raw, cap), PayloadState::Truncated)
        } else {
            (raw, PayloadState::Present)
        };

        let relative_path = relative_path_for(id);
        let absolute = self.root.join(&relative_path);
        if let Some(parent) = absolute.parent() {
            fs::create_dir_all(parent).map_err(|source| StoreError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        let compressed = zstd::encode_all(bytes.as_slice(), COMPRESSION_LEVEL)
            .map_err(|source| StoreError::Compress { source })?;

        let mut file = fs::File::create(&absolute).map_err(|source| StoreError::Io {
            path: absolute.clone(),
            source,
        })?;
        file.write_all(&compressed)
            .map_err(|source| StoreError::Io {
                path: absolute.clone(),
                source,
            })?;

        Ok(Some(WrittenPayload {
            relative_path,
            stored_bytes: compressed.len() as u64,
            text_bytes: bytes.len() as u64,
            codec: PayloadCodec::Zstd,
            state,
        }))
    }

    /// Reads a payload back as text.
    pub fn read_text(
        &self,
        relative_path: &str,
        codec: PayloadCodec,
    ) -> Result<String, StoreError> {
        let bytes = self.read_bytes(relative_path, codec)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Reads one chunk, moving the end back to a character boundary.
    ///
    /// Returns the text and the offset to resume from. Splitting a multi-byte
    /// character across two chunks would render as replacement characters in the
    /// detail view, so the boundary moves instead of the bytes.
    pub fn read_chunk(
        &self,
        relative_path: &str,
        codec: PayloadCodec,
        offset: u64,
        len: u32,
    ) -> Result<(String, u64, bool), StoreError> {
        let bytes = self.read_bytes(relative_path, codec)?;
        let start = usize::try_from(offset)
            .unwrap_or(usize::MAX)
            .min(bytes.len());
        let requested_end = start.saturating_add(len as usize).min(bytes.len());
        let slice = truncate_utf8(&bytes[start..], requested_end - start);
        let next_offset = (start + slice.len()) as u64;
        let eof = next_offset as usize >= bytes.len();
        Ok((
            String::from_utf8_lossy(&slice).into_owned(),
            next_offset,
            eof,
        ))
    }

    fn read_bytes(&self, relative_path: &str, codec: PayloadCodec) -> Result<Vec<u8>, StoreError> {
        let absolute = self.resolve(relative_path)?;
        let file = fs::File::open(&absolute).map_err(|source| StoreError::Io {
            path: absolute.clone(),
            source,
        })?;
        match codec {
            PayloadCodec::Raw => {
                let mut bytes = Vec::new();
                let mut file = file;
                file.read_to_end(&mut bytes)
                    .map_err(|source| StoreError::Io {
                        path: absolute,
                        source,
                    })?;
                Ok(bytes)
            }
            PayloadCodec::Zstd => {
                zstd::decode_all(file).map_err(|source| StoreError::Decompress { source })
            }
        }
    }

    /// Materializes a payload into an anonymous in-memory file and hands back the
    /// descriptor, positioned at the start.
    ///
    /// This is the fast path for the detail screen: the manager streams the
    /// descriptor directly, so a multi-megabyte dump never gets framed, JSON
    /// escaped, or reassembled. `memfd` rather than a temp file so nothing has to
    /// be cleaned up and the bytes never land somewhere another process could read.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    pub fn open_memfd(
        &self,
        relative_path: &str,
        codec: PayloadCodec,
    ) -> Result<std::os::fd::OwnedFd, StoreError> {
        use std::{
            io::{Seek, SeekFrom},
            os::fd::FromRawFd,
        };

        let bytes = self.read_bytes(relative_path, codec)?;

        let name = c"cch_payload";
        // SAFETY: `name` is a valid NUL-terminated C string that outlives the call,
        // and MFD_CLOEXEC is a documented flag for this syscall.
        // Call the kernel directly instead of linking bionic's `memfd_create`
        // wrapper: the syscall is available on supported Android kernels, while
        // the exported libc symbol is newer than the module's API 29 baseline.
        let raw =
            unsafe { libc::syscall(libc::SYS_memfd_create, name.as_ptr(), libc::MFD_CLOEXEC) };
        if raw < 0 {
            return Err(StoreError::Memfd {
                source: std::io::Error::last_os_error(),
            });
        }
        // SAFETY: `raw` is a fresh descriptor owned by nobody else; wrapping it
        // transfers ownership so it is closed exactly once.
        let raw = i32::try_from(raw).map_err(|_| StoreError::Memfd {
            source: std::io::Error::other("memfd descriptor did not fit in i32"),
        })?;
        let owned = unsafe { std::os::fd::OwnedFd::from_raw_fd(raw) };

        let mut file = std::fs::File::from(owned);
        file.write_all(&bytes)
            .map_err(|source| StoreError::Memfd { source })?;
        file.seek(SeekFrom::Start(0))
            .map_err(|source| StoreError::Memfd { source })?;

        // Hand out a read-only descriptor, not the writable one we just filled.
        //
        // When a descriptor crosses `SCM_RIGHTS`, SELinux checks the *receiving*
        // domain against the access the descriptor was opened with. A read-write memfd
        // therefore needs `write` on `tmpfs:file` in the receiver, which
        // `untrusted_app` does not have — so the manager got the descriptor and was
        // denied the moment it touched it:
        //
        //   avc: denied { write } for path="/memfd:cch_payload (deleted)"
        //        scontext=u:r:untrusted_app tcontext=u:object_r:tmpfs tclass=file
        //
        // Reopening through `/proc/self/fd` yields a descriptor onto the same
        // (still anonymous, still unlinked) memory with read access only, which is all
        // the reader needs and all SELinux will grant it.
        reopen_read_only(&file)
    }

    /// Removes a payload file, treating "already gone" as success.
    pub fn delete(&self, relative_path: &str) -> Result<(), StoreError> {
        let absolute = self.resolve(relative_path)?;
        match fs::remove_file(&absolute) {
            Ok(()) => Ok(()),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(StoreError::Io {
                path: absolute,
                source,
            }),
        }
    }

    /// Deletes payload files the index does not reference.
    ///
    /// Runs at open. Deleting a payload is two steps — `unlink` the file, then the
    /// row — and a crash between them leaves the file behind forever, so something
    /// has to sweep. Returns how many files were removed.
    pub fn remove_orphans(&self, referenced: &HashSet<String>) -> Result<u64, StoreError> {
        if !self.root.exists() {
            return Ok(0);
        }
        let mut removed = 0;
        for shard in read_dir(&self.root)? {
            if !shard.is_dir() {
                continue;
            }
            for file in read_dir(&shard)? {
                let Some(relative) = self.relative_of(&file) else {
                    continue;
                };
                if referenced.contains(&relative) {
                    continue;
                }
                match fs::remove_file(&file) {
                    Ok(()) => removed += 1,
                    Err(error) => warn!(
                        path = %file.display(),
                        %error,
                        "could not remove orphaned payload"
                    ),
                }
            }
        }
        Ok(removed)
    }

    /// Sum of the payload files actually on disk.
    pub fn disk_bytes(&self) -> Result<u64, StoreError> {
        if !self.root.exists() {
            return Ok(0);
        }
        let mut total = 0;
        for shard in read_dir(&self.root)? {
            if !shard.is_dir() {
                continue;
            }
            for file in read_dir(&shard)? {
                if let Ok(metadata) = fs::metadata(&file) {
                    total += metadata.len();
                }
            }
        }
        Ok(total)
    }

    /// Joins a stored relative path, refusing anything that escapes the root.
    ///
    /// The paths are generated from record ids, so this should never trigger — but
    /// the value does come back out of the database, and a store that will happily
    /// read `../../..` on request is one schema bug away from being a file-read
    /// primitive.
    fn resolve(&self, relative_path: &str) -> Result<PathBuf, StoreError> {
        let candidate = Path::new(relative_path);
        let safe = candidate
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)));
        if !safe || relative_path.is_empty() {
            return Err(StoreError::BadPayloadPath {
                path: relative_path.to_owned(),
            });
        }
        Ok(self.root.join(candidate))
    }

    fn relative_of(&self, absolute: &Path) -> Option<String> {
        let relative = absolute.strip_prefix(&self.root).ok()?;
        let mut parts = Vec::new();
        for component in relative.components() {
            match component {
                std::path::Component::Normal(part) => {
                    parts.push(part.to_string_lossy().into_owned())
                }
                _ => return None,
            }
        }
        Some(parts.join("/"))
    }
}

fn read_dir(path: &Path) -> Result<Vec<PathBuf>, StoreError> {
    let entries = fs::read_dir(path).map_err(|source| StoreError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect())
}

/// Payload path for a record: sharded by the first two characters of the id.
///
/// Flat directories with tens of thousands of entries make every lookup and every
/// listing slower on the filesystems Android uses; two hex-ish characters give 1024
/// buckets, which is plenty for the record ceiling.
fn relative_path_for(id: &RecordId) -> String {
    let text = id.as_str();
    let shard = &text[..2];
    format!("{shard}/{text}.zst")
}

/// Truncates to at most `max_bytes` without splitting a UTF-8 character.
///
/// Falls back to the raw byte cut when the input is not valid UTF-8, so a
/// mis-detected binary payload is still stored rather than rejected.
fn truncate_utf8(bytes: &[u8], max_bytes: usize) -> Vec<u8> {
    if bytes.len() <= max_bytes {
        return bytes.to_vec();
    }
    let mut end = max_bytes;
    while end > 0 && !is_char_boundary(bytes, end) {
        end -= 1;
    }
    if end == 0 {
        end = max_bytes;
    }
    bytes[..end].to_vec()
}

fn is_char_boundary(bytes: &[u8], index: usize) -> bool {
    match bytes.get(index) {
        None => index == bytes.len(),
        // Continuation bytes are 10xxxxxx.
        Some(byte) => (*byte as i8) >= -0x40,
    }
}

/// Reopens an open file as read-only through `/proc/self/fd`.
///
/// Works for an anonymous memfd as well as a named file: the `/proc/self/fd` entry is
/// a magic link the kernel resolves back to the underlying inode, so the new
/// descriptor shares the same memory and the same unlinked identity, and it starts at
/// offset zero regardless of where the writable handle was left.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn reopen_read_only(file: &std::fs::File) -> Result<std::os::fd::OwnedFd, StoreError> {
    use std::os::fd::AsRawFd;

    let path = format!("/proc/self/fd/{}", file.as_raw_fd());
    std::fs::File::open(&path)
        .map(Into::into)
        .map_err(|source| StoreError::Memfd { source })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cch_model::RecordIdGenerator;

    fn store() -> (tempfile::TempDir, PayloadStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = PayloadStore::new(dir.path());
        (dir, store)
    }

    fn id(now_ms: u64) -> RecordId {
        RecordIdGenerator::new().next(now_ms)
    }

    #[test]
    fn nothing_is_written_for_an_absent_payload() {
        let (_dir, store) = store();
        assert_eq!(
            store
                .write(&id(1), &PayloadSource::None, 1024)
                .expect("writes"),
            None
        );
        assert_eq!(
            store
                .write(&id(1), &PayloadSource::Inline(Vec::new()), 1024)
                .expect("writes"),
            None
        );
    }

    #[test]
    fn payloads_round_trip_through_compression() {
        let (_dir, store) = store();
        let text = "java.lang.IllegalStateException: boom\n\tat com.example.Foo.bar(Foo.kt:1)\n";
        let written = store
            .write(
                &id(1),
                &PayloadSource::Inline(text.as_bytes().to_vec()),
                1 << 20,
            )
            .expect("writes")
            .expect("wrote something");

        assert_eq!(written.state, PayloadState::Present);
        assert_eq!(written.codec, PayloadCodec::Zstd);
        assert_eq!(written.text_bytes, text.len() as u64);
        assert_eq!(
            store
                .read_text(&written.relative_path, written.codec)
                .expect("reads"),
            text
        );
    }

    #[test]
    fn repetitive_traces_actually_get_smaller() {
        let (_dir, store) = store();
        // A real stack trace is dozens of near-identical frames.
        let text = "\tat com.example.app.Repo.load(Repo.kt:88)\n".repeat(200);
        let written = store
            .write(
                &id(1),
                &PayloadSource::Inline(text.clone().into_bytes()),
                1 << 20,
            )
            .expect("writes")
            .expect("wrote something");
        assert!(
            written.stored_bytes < written.text_bytes / 4,
            "expected real compression, got {} from {}",
            written.stored_bytes,
            written.text_bytes
        );
    }

    #[test]
    fn oversized_payloads_are_truncated_and_flagged() {
        let (_dir, store) = store();
        let text = "x".repeat(5_000);
        let written = store
            .write(&id(1), &PayloadSource::Inline(text.into_bytes()), 1_000)
            .expect("writes")
            .expect("wrote something");

        assert_eq!(written.state, PayloadState::Truncated);
        assert_eq!(written.text_bytes, 1_000);
        assert_eq!(
            store
                .read_text(&written.relative_path, written.codec)
                .expect("reads")
                .len(),
            1_000
        );
    }

    #[test]
    fn truncation_does_not_split_a_multibyte_character() {
        let (_dir, store) = store();
        // Each `の` is 3 bytes, so a 10-byte cap lands mid-character.
        let text = "の".repeat(10);
        let written = store
            .write(&id(1), &PayloadSource::Inline(text.into_bytes()), 10)
            .expect("writes")
            .expect("wrote something");

        let read_back = store
            .read_text(&written.relative_path, written.codec)
            .expect("reads");
        // A 10-byte cap walks back to the boundary at 9, which holds exactly three
        // three-byte characters.
        assert_eq!(read_back, "ののの");
        assert_eq!(read_back.len(), 9);
        assert!(!read_back.contains('\u{fffd}'));
    }

    #[test]
    fn payloads_can_come_from_a_file() {
        let (dir, store) = store();
        let source = dir.path().join("tombstone_00");
        fs::write(&source, b"signal 11 (SIGSEGV)").expect("writes source");

        let written = store
            .write(&id(1), &PayloadSource::File(source), 1 << 20)
            .expect("writes")
            .expect("wrote something");
        assert_eq!(
            store
                .read_text(&written.relative_path, written.codec)
                .expect("reads"),
            "signal 11 (SIGSEGV)"
        );
    }

    #[test]
    fn chunked_reads_cover_the_whole_payload_without_splitting_characters() {
        let (_dir, store) = store();
        let text = "のabc".repeat(50);
        let written = store
            .write(
                &id(1),
                &PayloadSource::Inline(text.clone().into_bytes()),
                1 << 20,
            )
            .expect("writes")
            .expect("wrote something");

        let mut assembled = String::new();
        let mut offset = 0;
        loop {
            let (chunk, next, eof) = store
                .read_chunk(&written.relative_path, written.codec, offset, 7)
                .expect("reads chunk");
            assert!(!chunk.contains('\u{fffd}'), "chunk split a character");
            assembled.push_str(&chunk);
            if eof {
                break;
            }
            assert!(next > offset, "chunk reader must make progress");
            offset = next;
        }
        assert_eq!(assembled, text);
    }

    #[test]
    fn reading_past_the_end_reports_eof_without_erroring() {
        let (_dir, store) = store();
        let written = store
            .write(&id(1), &PayloadSource::Inline(b"short".to_vec()), 1 << 20)
            .expect("writes")
            .expect("wrote something");

        let (chunk, next, eof) = store
            .read_chunk(&written.relative_path, written.codec, 9_999, 100)
            .expect("reads past the end");
        assert!(chunk.is_empty());
        assert!(eof);
        assert_eq!(next, 5);
    }

    #[test]
    fn payloads_are_sharded_rather_than_piled_into_one_directory() {
        let (_dir, store) = store();
        let record_id = id(1);
        let written = store
            .write(&record_id, &PayloadSource::Inline(b"x".to_vec()), 1 << 20)
            .expect("writes")
            .expect("wrote something");
        assert_eq!(
            written.relative_path,
            format!("{}/{}.zst", &record_id.as_str()[..2], record_id.as_str())
        );
    }

    #[test]
    fn deleting_is_idempotent() {
        let (_dir, store) = store();
        let written = store
            .write(&id(1), &PayloadSource::Inline(b"x".to_vec()), 1 << 20)
            .expect("writes")
            .expect("wrote something");

        store.delete(&written.relative_path).expect("first delete");
        store
            .delete(&written.relative_path)
            .expect("deleting twice must not fail");
    }

    #[test]
    fn orphans_are_swept_and_referenced_files_are_kept() {
        let (_dir, store) = store();
        let keep = store
            .write(&id(1), &PayloadSource::Inline(b"keep".to_vec()), 1 << 20)
            .expect("writes")
            .expect("wrote something");
        let orphan = store
            .write(&id(2), &PayloadSource::Inline(b"orphan".to_vec()), 1 << 20)
            .expect("writes")
            .expect("wrote something");

        let referenced: HashSet<String> = [keep.relative_path.clone()].into_iter().collect();
        assert_eq!(store.remove_orphans(&referenced).expect("sweeps"), 1);

        assert!(store.read_text(&keep.relative_path, keep.codec).is_ok());
        assert!(
            store
                .read_text(&orphan.relative_path, orphan.codec)
                .is_err()
        );
    }

    #[test]
    fn sweeping_an_empty_store_is_not_an_error() {
        let (_dir, store) = store();
        assert_eq!(store.remove_orphans(&HashSet::new()).expect("sweeps"), 0);
        assert_eq!(store.disk_bytes().expect("measures"), 0);
    }

    #[test]
    fn disk_usage_reflects_what_was_written() {
        let (_dir, store) = store();
        let written = store
            .write(&id(1), &PayloadSource::Inline(vec![b'x'; 4096]), 1 << 20)
            .expect("writes")
            .expect("wrote something");
        assert_eq!(store.disk_bytes().expect("measures"), written.stored_bytes);
    }

    #[test]
    fn a_traversing_path_is_refused() {
        let (_dir, store) = store();
        for path in ["../secret", "a/../../secret", "", "/etc/passwd"] {
            assert!(
                store.read_text(path, PayloadCodec::Zstd).is_err(),
                "{path} must not resolve"
            );
        }
    }
}
