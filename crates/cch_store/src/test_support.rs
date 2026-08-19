//! Shared fixtures for the store's own tests.
//!
//! Hand-written rather than generated: a fixture that is obvious to read is worth
//! more in a storage test than one that is clever.

use cch_config::RetentionPolicy;
use cch_model::{CrashKind, CrashRecord, CrashSummary, Fingerprint, PayloadSource, SourceMask};
use cch_wire::{GroupSummary, PageRequest, RecordSummary};

use crate::{Inserted, Store, StoreError};

/// A store on a temp directory that cleans up with the test.
pub struct TestStore {
    pub store: Store,
    // Held so the directory outlives the store.
    _dir: tempfile::TempDir,
}

impl TestStore {
    #[must_use]
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::open(dir.path()).expect("opens store");
        Self { store, _dir: dir }
    }

    pub fn insert_default(&self, record: &CrashRecord) -> Result<Inserted, StoreError> {
        self.store.insert(record, RetentionPolicy::default())
    }

    /// Reads one group back, failing the test if it is gone.
    #[must_use]
    pub fn group(&self, group_id: &str) -> GroupSummary {
        self.store
            .list_groups(&PageRequest {
                filter: cch_wire::CrashFilter {
                    include_system_apps: true,
                    ..cch_wire::CrashFilter::default()
                },
                limit: 200,
                ..PageRequest::default()
            })
            .expect("lists groups")
            .items
            .into_iter()
            .find(|group| group.group_id == group_id)
            .expect("group is present")
    }

    /// Every group, newest first.
    #[must_use]
    pub fn all_groups(&self) -> Vec<GroupSummary> {
        self.store
            .list_groups(&PageRequest {
                filter: cch_wire::CrashFilter {
                    include_system_apps: true,
                    ..cch_wire::CrashFilter::default()
                },
                limit: 200,
                ..PageRequest::default()
            })
            .expect("lists groups")
            .items
    }

    /// Every record in a group, newest first.
    #[must_use]
    pub fn records_of(&self, group_id: &str) -> Vec<RecordSummary> {
        self.store
            .list_records(
                group_id,
                &PageRequest {
                    limit: 200,
                    ..PageRequest::default()
                },
            )
            .expect("lists records")
            .items
    }
}

impl Default for TestStore {
    fn default() -> Self {
        Self::new()
    }
}

/// A plain Java crash from a user app, with a small inline payload.
#[must_use]
pub fn java_record(happened_at_ms: i64) -> CrashRecord {
    CrashRecord {
        kind: CrashKind::JavaException,
        package_name: "com.example.app".to_owned(),
        process_name: "com.example.app".to_owned(),
        user_id: 0,
        pid: 12_874,
        happened_at_ms,
        app_version_name: Some("1.4.2".to_owned()),
        app_version_code: Some(10_402),
        is_system_app: false,
        package_installed: true,
        is_foreground: Some(true),
        self_handled: false,
        dropped_count: None,
        sources: SourceMask::EVENTS.union(SourceMask::CRASH_BUFFER),
        summary: CrashSummary::new(
            Some("java.lang.IllegalStateException".to_owned()),
            Some("Fragment already added".to_owned()),
        ),
        fingerprint: fixture::default_fingerprint(),
        payload: PayloadSource::Inline(
            b"java.lang.IllegalStateException: Fragment already added\n\tat com.example.app.MainActivity.onCreate(MainActivity.kt:37)\n"
                .to_vec(),
        ),
    }
}

/// A record for a different package, so package filters have something to separate.
#[must_use]
pub fn record_for(package: &str, happened_at_ms: i64) -> CrashRecord {
    let mut record = java_record(happened_at_ms);
    record.package_name = package.to_owned();
    record.process_name = package.to_owned();
    record
}

/// A native crash, for kind filters and for the four-sources merge case.
#[must_use]
pub fn native_record(happened_at_ms: i64) -> CrashRecord {
    let mut record = java_record(happened_at_ms);
    record.kind = CrashKind::NativeCrash;
    record.summary = CrashSummary::new(Some("SIGSEGV".to_owned()), Some("SEGV_MAPERR".to_owned()));
    record.fingerprint = Fingerprint::from_raw_frames(
        CrashKind::NativeCrash,
        "SIGSEGV",
        &["      #00 pc 0000000000001ac4  /data/app/lib/arm64/libnative.so (crash+16)".to_owned()],
    );
    record.sources = SourceMask::EVENTS.union(SourceMask::TOMBSTONE);
    record
}

/// An ANR, whose payload stands in for an all-threads dump.
#[must_use]
pub fn anr_record(happened_at_ms: i64) -> CrashRecord {
    let mut record = java_record(happened_at_ms);
    record.kind = CrashKind::Anr;
    record.summary = CrashSummary::new(
        Some("Input dispatching timed out".to_owned()),
        Some("com.example.app/.MainActivity".to_owned()),
    );
    record.fingerprint = Fingerprint::from_raw_frames(
        CrashKind::Anr,
        "Input dispatching timed out",
        &["at com.example.app.Repo.blockingLoad(Repo.kt:88)".to_owned()],
    );
    record.sources = SourceMask::EVENTS.union(SourceMask::ANR_FILE);
    record
}

pub mod fixture {
    use super::*;

    #[must_use]
    pub fn default_fingerprint() -> Fingerprint {
        Fingerprint::from_raw_frames(
            CrashKind::JavaException,
            "java.lang.IllegalStateException",
            &["at com.example.app.MainActivity.onCreate(MainActivity.kt:37)".to_owned()],
        )
    }

    /// A second, distinct fingerprint for the same package.
    #[must_use]
    pub fn other_fingerprint() -> Fingerprint {
        Fingerprint::from_raw_frames(
            CrashKind::JavaException,
            "java.lang.NullPointerException",
            &["at com.example.app.Other.run(Other.kt:9)".to_owned()],
        )
    }
}
