//! Crash storage: a SQLite index alongside compressed payload files.
//!
//! The split is the whole point. The list screen reads `crash_group` only — one
//! indexed range scan, no join, no payload — so opening it costs the same with ten
//! records or ten thousand. The bulky text lives in files, reached by descriptor,
//! so a multi-megabyte ANR dump never has to be framed or parsed to draw a row.
//!
//! This is the direct answer to how the tool being replaced behaves: it keeps one
//! JSON file per crash, deserializes every one of them at start-up, and ships the
//! entire history in a single message. All three costs scale with history, which is
//! why its list takes seconds to open.
//!
//! Aggregates (`occurrence`, `first_seen_ms`, `last_seen_ms`) are maintained on
//! write, never computed on read.

#![forbid(unsafe_op_in_unsafe_fn)]

mod payload;
mod read;
mod retention;
mod schema;
mod sql;
#[cfg(test)]
mod test_support;
mod write;

use std::{
    io,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use cch_model::RecordIdGenerator;
use cch_wire::WireError;
use rusqlite::{Connection, OptionalExtension};
use tracing::info;

pub use payload::{PayloadStore, WrittenPayload};
pub use read::PackageRollup;
pub use retention::SweepOutcome;
pub use schema::SCHEMA_VERSION;
pub use write::Inserted;

/// File name of the index database inside the store root.
pub const DATABASE_FILE_NAME: &str = "crashes.db";

/// Everything that can go wrong in the store.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io failed at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to compress a payload: {source}")]
    Compress {
        #[source]
        source: io::Error,
    },
    #[error("failed to decompress a payload: {source}")]
    Decompress {
        #[source]
        source: io::Error,
    },
    #[error("failed to materialize a payload in memory: {source}")]
    Memfd {
        #[source]
        source: io::Error,
    },
    /// A stored payload path was not a plain relative path.
    #[error("refusing payload path {path:?}")]
    BadPayloadPath { path: String },
    #[error("database schema is version {found}, this build supports {supported}")]
    SchemaTooNew { found: i64, supported: i64 },
    #[error("no record with id {id}")]
    RecordNotFound { id: String },
    #[error("no crash group with id {id}")]
    GroupNotFound { id: String },
    #[error("record {id} has no readable payload")]
    PayloadUnavailable { id: String },
    /// A request-shaped failure, surfaced so the daemon can forward the code as is.
    #[error("{0}")]
    Request(#[from] WireError),
    /// A previous panic poisoned the connection lock.
    #[error("store lock was poisoned by an earlier failure")]
    Poisoned,
}

impl StoreError {
    /// Maps onto the wire vocabulary so the daemon does not have to guess.
    #[must_use]
    pub fn to_wire(&self) -> WireError {
        use cch_wire::ErrorCode;
        match self {
            Self::Request(error) => error.clone(),
            Self::RecordNotFound { .. }
            | Self::GroupNotFound { .. }
            | Self::PayloadUnavailable { .. } => {
                WireError::new(ErrorCode::NotFound, self.to_string())
            }
            _ => WireError::new(ErrorCode::Internal, self.to_string()),
        }
    }
}

/// The crash store.
///
/// Cheap to share: the connection sits behind a mutex and WAL lets a reader
/// proceed while a write is in flight, so the manager's queries are not blocked by
/// ingest.
#[derive(Debug)]
pub struct Store {
    connection: Mutex<Connection>,
    payloads: PayloadStore,
    ids: Mutex<RecordIdGenerator>,
}

impl Store {
    /// Opens (creating if needed) the store rooted at `root`.
    ///
    /// Also sweeps payload files the index no longer references. Deleting a payload
    /// is two steps — unlink the file, delete the row — so an interrupted delete
    /// leaves a file nothing points at. Start-up is the natural place to notice.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, StoreError> {
        let root = root.as_ref();
        std::fs::create_dir_all(root).map_err(|source| StoreError::Io {
            path: root.to_path_buf(),
            source,
        })?;

        let connection = Connection::open(root.join(DATABASE_FILE_NAME))?;
        schema::initialize(&connection)?;

        let store = Self {
            connection: Mutex::new(connection),
            payloads: PayloadStore::new(root),
            ids: Mutex::new(RecordIdGenerator::new()),
        };

        // Resume the id sequence past whatever is already stored. Ids are derived from
        // the crash's own timestamp, so a generator that restarted at zero would re-emit
        // an existing id the moment the same crash was re-read — and the insert would fail
        // on the primary key, dropping the crash with only a log line to show for it.
        store.resume_ids()?;

        let referenced = store.referenced_payload_paths()?;
        let removed = store.payloads.remove_orphans(&referenced)?;
        if removed > 0 {
            info!(
                removed,
                "swept payload files the index no longer references"
            );
        }

        Ok(store)
    }

    /// Opens an in-memory index with payloads under `root`. Tests only.
    #[doc(hidden)]
    pub fn open_in_memory(root: impl AsRef<Path>) -> Result<Self, StoreError> {
        let root = root.as_ref();
        std::fs::create_dir_all(root).map_err(|source| StoreError::Io {
            path: root.to_path_buf(),
            source,
        })?;
        let connection = Connection::open_in_memory()?;
        schema::initialize(&connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
            payloads: PayloadStore::new(root),
            ids: Mutex::new(RecordIdGenerator::new()),
        })
    }

    #[must_use]
    pub fn payloads(&self) -> &PayloadStore {
        &self.payloads
    }

    pub(crate) fn connection(&self) -> Result<MutexGuard<'_, Connection>, StoreError> {
        self.connection.lock().map_err(|_| StoreError::Poisoned)
    }

    pub(crate) fn next_id(&self, at_ms: i64) -> Result<cch_model::RecordId, StoreError> {
        let mut generator = self.ids.lock().map_err(|_| StoreError::Poisoned)?;
        Ok(generator.next(u64::try_from(at_ms).unwrap_or(0)))
    }

    /// Points the id generator past the largest id in the index.
    ///
    /// `MAX(id)` is the right question because ids sort lexicographically in the same
    /// order as they sort numerically — that is what the fixed-width base32 encoding is
    /// for — so the textual maximum is also the newest.
    fn resume_ids(&self) -> Result<(), StoreError> {
        let latest: Option<String> = self
            .connection()?
            .query_row("SELECT MAX(id) FROM crash_record", [], |row| row.get(0))
            .optional()?
            .flatten();

        if let Some(id) = latest.as_deref().and_then(cch_model::RecordId::parse) {
            self.ids
                .lock()
                .map_err(|_| StoreError::Poisoned)?
                .resume_after(&id);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_twice_reuses_the_same_database() {
        let dir = tempfile::tempdir().expect("tempdir");
        {
            let store = Store::open(dir.path()).expect("first open");
            drop(store);
        }
        let store = Store::open(dir.path()).expect("second open");
        assert_eq!(store.storage_status().expect("status").record_count, 0);
        assert!(dir.path().join(DATABASE_FILE_NAME).exists());
    }

    #[test]
    fn store_errors_map_onto_the_wire_vocabulary() {
        use cch_wire::ErrorCode;

        let not_found = StoreError::RecordNotFound { id: "x".into() };
        assert_eq!(not_found.to_wire().code, ErrorCode::NotFound);

        let unavailable = StoreError::PayloadUnavailable { id: "x".into() };
        assert_eq!(unavailable.to_wire().code, ErrorCode::NotFound);

        let poisoned = StoreError::Poisoned;
        assert_eq!(poisoned.to_wire().code, ErrorCode::Internal);

        // A cursor problem must keep its own code rather than becoming Internal,
        // so the client knows to restart the query instead of retrying blindly.
        let cursor = StoreError::Request(WireError::cursor_invalidated("stale"));
        assert_eq!(cursor.to_wire().code, ErrorCode::CursorInvalidated);
    }
}
