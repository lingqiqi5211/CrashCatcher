use cch_config::RetentionPolicy;
use cch_model::{CrashRecord, PayloadState, RecordId};
use cch_wire::{GroupSummary, RecordSummary};
use rusqlite::{OptionalExtension, params};

use crate::{Store, StoreError, payload::WrittenPayload, sql};

/// What an insert produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inserted {
    pub record: RecordSummary,
    pub group: GroupSummary,
    /// First occurrence of this fingerprint, so the list gains a row rather than
    /// incrementing one.
    pub is_new_group: bool,
}

impl Store {
    /// Records one occurrence.
    ///
    /// The payload file is written first and removed again if the transaction
    /// fails, so a failed insert cannot leave a file nothing references. The
    /// reverse order would leave the row pointing at a file that was never written.
    pub fn insert(
        &self,
        record: &CrashRecord,
        retention: RetentionPolicy,
    ) -> Result<Inserted, StoreError> {
        self.insert_with_sources(record, retention, &[], 0)
    }

    /// Records one occurrence and confirms every contributing source in the same transaction.
    ///
    /// A daemon crash can therefore leave either both the record and its source keys, or neither;
    /// it cannot leave a key that makes the next daemon skip a record which never reached SQLite.
    pub fn insert_with_sources(
        &self,
        record: &CrashRecord,
        retention: RetentionPolicy,
        source_keys: &[String],
        ingested_at_ms: i64,
    ) -> Result<Inserted, StoreError> {
        let id = self.next_id(record.happened_at_ms)?;
        let written =
            self.payloads
                .write(&id, &record.payload, retention.max_payload_bytes_per_record)?;

        match self.insert_rows(&id, record, written.as_ref(), source_keys, ingested_at_ms) {
            Ok(inserted) => Ok(inserted),
            Err(error) => {
                if let Some(written) = &written {
                    // Best effort: the sweep at next open would catch it anyway.
                    let _ = self.payloads.delete(&written.relative_path);
                }
                Err(error)
            }
        }
    }

    fn insert_rows(
        &self,
        id: &RecordId,
        record: &CrashRecord,
        written: Option<&WrittenPayload>,
        source_keys: &[String],
        ingested_at_ms: i64,
    ) -> Result<Inserted, StoreError> {
        let group_id = record.group_id();
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;

        let existing: Option<i64> = transaction
            .query_row(
                "SELECT occurrence FROM crash_group WHERE group_id = ?1",
                params![&group_id],
                |row| row.get(0),
            )
            .optional()?;
        let is_new_group = existing.is_none();
        // Anything after the first sighting is a repeat. Derived from our own
        // history rather than read off a framework field, which is what keeps it
        // meaningful across reboots.
        let is_repeating = !is_new_group;

        let payload_bytes = written.map_or(0, |written| written.stored_bytes);

        transaction.execute(
            "INSERT INTO crash_group (
                 group_id, package_name, process_name, user_id, kind,
                 is_system_app, package_installed, is_main_process, self_handled,
                 summary_class, summary_text,
                 occurrence, first_seen_ms, last_seen_ms, payload_bytes, muted_until_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1, ?12, ?12, ?13, NULL)
             ON CONFLICT(group_id) DO UPDATE SET
                 occurrence    = occurrence + 1,
                 first_seen_ms = MIN(first_seen_ms, excluded.first_seen_ms),
                 last_seen_ms  = MAX(last_seen_ms, excluded.last_seen_ms),
                 payload_bytes = payload_bytes + excluded.payload_bytes,
                 -- Keep the newest wording and the newest self-handled verdict: they
                 -- describe how the app behaves now, not how it behaved first.
                 summary_class = COALESCE(excluded.summary_class, summary_class),
                 summary_text  = COALESCE(excluded.summary_text, summary_text),
                 self_handled  = excluded.self_handled,
                 -- Both are re-resolved on every sighting: an app can be installed after a
                 -- native process of the same name crashed, or updated into /system.
                 is_system_app = excluded.is_system_app,
                 package_installed = excluded.package_installed",
            params![
                &group_id,
                &record.package_name,
                &record.process_name,
                record.user_id,
                record.kind.as_i64(),
                record.is_system_app,
                record.package_installed,
                record.is_main_process(),
                record.self_handled,
                &record.summary.class_name,
                &record.summary.text,
                record.happened_at_ms,
                payload_bytes as i64,
            ],
        )?;

        transaction.execute(
            "INSERT INTO crash_record (
                 id, group_id, happened_at_ms, pid, sources,
                 app_version_name, app_version_code, is_foreground, is_repeating,
                 dropped_count, payload_path, payload_bytes, payload_codec, payload_state
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                id.as_str(),
                &group_id,
                record.happened_at_ms,
                record.pid,
                i64::from(record.sources.bits()),
                &record.app_version_name,
                record.app_version_code,
                record.is_foreground,
                is_repeating,
                record.dropped_count.map(i64::from),
                written.map(|written| written.relative_path.as_str()),
                payload_bytes as i64,
                written.map_or(0, |written| written.codec.as_i64()),
                // `Absent`, not `Evicted`: nothing was written because there was nothing to
                // write. A crash seen only in the events buffer arrives without a stack, and
                // reporting that as reclaimed-for-quota blames retention for a payload that
                // never existed.
                written
                    .map_or(PayloadState::Absent, |written| written.state)
                    .as_i64(),
            ],
        )?;

        // Read both rows back so the caller is handed what was stored, not what was
        // requested — the aggregates in particular are computed by the upsert.
        let group = transaction.query_row(
            &format!(
                "SELECT {} FROM crash_group WHERE group_id = ?1",
                sql::GROUP_COLUMNS
            ),
            params![&group_id],
            sql::map_group,
        )?;
        let stored_record = transaction.query_row(
            &format!(
                "SELECT {} FROM crash_record WHERE id = ?1",
                sql::RECORD_COLUMNS
            ),
            params![id.as_str()],
            sql::map_record,
        )?;

        for source_key in source_keys {
            transaction.execute(
                "INSERT INTO ingested_source (source_key, ingested_at_ms) VALUES (?1, ?2)
                 ON CONFLICT(source_key) DO NOTHING",
                params![source_key, ingested_at_ms],
            )?;
        }

        transaction.commit()?;

        Ok(Inserted {
            record: stored_record,
            group,
            is_new_group,
        })
    }

    /// Every package name the index has a group for.
    ///
    /// Tens of rows, not the thousands `packages.list` holds: the caller is re-deciding
    /// classifications, and only packages that actually crashed have one to re-decide.
    pub fn group_package_names(&self) -> Result<Vec<String>, StoreError> {
        let connection = self.connection()?;
        let mut statement =
            connection.prepare("SELECT DISTINCT package_name FROM crash_group ORDER BY 1")?;
        let names = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(names)
    }

    /// Rewrites the platform/app verdict for groups whose package the caller has since resolved.
    ///
    /// The insert only re-resolves it when a group is *seen again*, so a crash that happens once
    /// keeps whatever the index said at the time — and during early boot the index says every
    /// package is an ordinary app. The caller passes only what PackageManager has now confirmed.
    pub fn apply_package_system_flags(&self, flags: &[(String, bool)]) -> Result<u64, StoreError> {
        if flags.is_empty() {
            return Ok(0);
        }
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let mut changed = 0;
        {
            let mut statement = transaction.prepare(
                "UPDATE crash_group SET is_system_app = ?2, package_installed = 1
                 WHERE package_name = ?1 AND (is_system_app <> ?2 OR package_installed <> 1)",
            )?;
            for (name, is_system) in flags {
                changed += statement.execute(params![name, is_system])?;
            }
        }
        transaction.commit()?;
        Ok(changed as u64)
    }

    /// The stored record this one is a second view of, if there is one.
    ///
    /// `CrashMerger` only folds fragments alive in the same process at the same time, and the
    /// artefact collectors rescan `/data/tombstones` from scratch at every start — so a tombstone
    /// read days after DropBox reported the same crash lands as a second record in a second group,
    /// the fingerprint being built from frames the two sources normalise differently.
    ///
    /// Identity is the pid, which a dead process cannot reuse inside the window. Two conditions,
    /// each catching what the other does not: the *same* millisecond whatever the sources is one
    /// artefact read twice; *near* it with no source in common is one crash seen from two places.
    /// Both are in the query rather than applied to its result, so the nearest *qualifying* row
    /// wins — filtering afterwards would drop a match because a closer row failed the test.
    /// A pid of zero identifies nothing and is never folded.
    pub fn companion_record(
        &self,
        record: &CrashRecord,
        window_ms: i64,
    ) -> Result<Option<RecordSummary>, StoreError> {
        if record.pid == 0 {
            return Ok(None);
        }
        let connection = self.connection()?;
        let found = connection
            .query_row(
                &format!(
                    "SELECT {} FROM crash_record r
                     JOIN crash_group g ON g.group_id = r.group_id
                     WHERE r.pid = ?1
                       AND g.process_name = ?2
                       AND g.kind = ?3
                       AND g.user_id = ?4
                       AND r.happened_at_ms BETWEEN ?5 - ?6 AND ?5 + ?6
                       AND (r.happened_at_ms = ?5 OR (r.sources & ?7) = 0)
                     ORDER BY ABS(r.happened_at_ms - ?5) ASC
                     LIMIT 1",
                    sql::record_columns_qualified("r")
                ),
                params![
                    record.pid,
                    &record.process_name,
                    record.kind.as_i64(),
                    record.user_id,
                    record.happened_at_ms,
                    window_ms,
                    i64::from(record.sources.bits()),
                ],
                sql::map_record,
            )
            .optional()?;

        Ok(found)
    }

    /// Adds a second source's evidence to a record that is already stored.
    ///
    /// Not an insert: the crash happened once, so `occurrence` must not move. The payload is
    /// adopted only when the stored record has none — the shape of a crash seen only in the
    /// events buffer, where the arriving tombstone is the stack the detail screen was missing.
    pub fn fold_into_record(
        &self,
        existing: &RecordSummary,
        record: &CrashRecord,
        retention: RetentionPolicy,
        source_keys: &[String],
        ingested_at_ms: i64,
    ) -> Result<RecordSummary, StoreError> {
        let adopt = matches!(existing.payload_state, PayloadState::Absent)
            .then(|| {
                self.payloads.write(
                    &existing.id,
                    &record.payload,
                    retention.max_payload_bytes_per_record,
                )
            })
            .transpose()?
            .flatten();

        match self.fold_rows(
            existing,
            record,
            adopt.as_ref(),
            source_keys,
            ingested_at_ms,
        ) {
            Ok(folded) => Ok(folded),
            Err(error) => {
                if let Some(adopt) = &adopt {
                    let _ = self.payloads.delete(&adopt.relative_path);
                }
                Err(error)
            }
        }
    }

    fn fold_rows(
        &self,
        existing: &RecordSummary,
        record: &CrashRecord,
        adopted: Option<&WrittenPayload>,
        source_keys: &[String],
        ingested_at_ms: i64,
    ) -> Result<RecordSummary, StoreError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;

        let sources = existing.sources.union(record.sources);
        transaction.execute(
            "UPDATE crash_record SET sources = ?2 WHERE id = ?1",
            params![existing.id.as_str(), i64::from(sources.bits())],
        )?;

        if let Some(adopted) = adopted {
            transaction.execute(
                "UPDATE crash_record
                 SET payload_path = ?2, payload_bytes = ?3, payload_codec = ?4, payload_state = ?5
                 WHERE id = ?1",
                params![
                    existing.id.as_str(),
                    adopted.relative_path.as_str(),
                    adopted.stored_bytes as i64,
                    adopted.codec.as_i64(),
                    adopted.state.as_i64(),
                ],
            )?;
            transaction.execute(
                "UPDATE crash_group SET payload_bytes = payload_bytes + ?2 WHERE group_id = ?1",
                params![&existing.group_id, adopted.stored_bytes as i64],
            )?;
        }

        for source_key in source_keys {
            transaction.execute(
                "INSERT INTO ingested_source (source_key, ingested_at_ms) VALUES (?1, ?2)
                 ON CONFLICT(source_key) DO NOTHING",
                params![source_key, ingested_at_ms],
            )?;
        }

        let folded = transaction.query_row(
            &format!(
                "SELECT {} FROM crash_record WHERE id = ?1",
                sql::RECORD_COLUMNS
            ),
            params![existing.id.as_str()],
            sql::map_record,
        )?;
        transaction.commit()?;
        Ok(folded)
    }

    /// Whether this source artefact has already been turned into a record.
    ///
    /// The key must include mtime and size, not just the file name: tombstone slots
    /// are reused round-robin, so `tombstone_07` is a different crash an hour later.
    pub fn was_ingested(&self, source_key: &str) -> Result<bool, StoreError> {
        let connection = self.connection()?;
        let found: Option<i64> = connection
            .query_row(
                "SELECT 1 FROM ingested_source WHERE source_key = ?1",
                params![source_key],
                |row| row.get(0),
            )
            .optional()?;
        Ok(found.is_some())
    }

    /// Marks a source artefact ingested. Returns false when it already was.
    ///
    /// Lets the caller claim an artefact and detect a duplicate in one step, which
    /// is what the start-up backfill needs when it overlaps with live inotify events.
    pub fn mark_ingested(&self, source_key: &str, now_ms: i64) -> Result<bool, StoreError> {
        let connection = self.connection()?;
        let changed = connection.execute(
            "INSERT INTO ingested_source (source_key, ingested_at_ms) VALUES (?1, ?2)
             ON CONFLICT(source_key) DO NOTHING",
            params![source_key, now_ms],
        )?;
        Ok(changed > 0)
    }

    /// Mutes a group until a wall-clock instant, or clears the mute with `None`.
    ///
    /// The store only knows an instant. "Until unlock" and "until restart" are the
    /// daemon's business, cleared when it sees the corresponding event — the same
    /// deliberately volatile treatment the tool being replaced gives them.
    pub fn set_group_mute(
        &self,
        group_id: &str,
        muted_until_ms: Option<i64>,
    ) -> Result<bool, StoreError> {
        let connection = self.connection()?;
        let changed = connection.execute(
            "UPDATE crash_group SET muted_until_ms = ?2 WHERE group_id = ?1",
            params![group_id, muted_until_ms],
        )?;
        Ok(changed > 0)
    }

    /// Mutes every group belonging to a package. Returns how many were affected.
    pub fn set_package_mute(
        &self,
        package_name: &str,
        muted_until_ms: Option<i64>,
    ) -> Result<u64, StoreError> {
        let connection = self.connection()?;
        let changed = connection.execute(
            "UPDATE crash_group SET muted_until_ms = ?2 WHERE package_name = ?1",
            params![package_name, muted_until_ms],
        )?;
        Ok(changed as u64)
    }

    /// Clears every mute. Called at boot for "until restart" scopes.
    pub fn clear_all_mutes(&self) -> Result<u64, StoreError> {
        let connection = self.connection()?;
        let changed = connection.execute(
            "UPDATE crash_group SET muted_until_ms = NULL WHERE muted_until_ms IS NOT NULL",
            [],
        )?;
        Ok(changed as u64)
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support::{TestStore, fixture, java_record, native_record};
    use cch_config::RetentionPolicy;
    use cch_model::{PayloadSource, PayloadState, SourceMask};

    #[test]
    fn the_first_occurrence_creates_a_group_and_is_not_repeating() {
        let store = TestStore::new();
        let inserted = store.insert_default(&java_record(1_000)).expect("inserts");

        assert!(inserted.is_new_group);
        assert!(!inserted.record.is_repeating);
        assert_eq!(inserted.group.occurrence, 1);
        assert_eq!(inserted.group.first_seen_ms, 1_000);
        assert_eq!(inserted.group.last_seen_ms, 1_000);
    }

    #[test]
    fn a_second_occurrence_increments_rather_than_adding_a_row() {
        let store = TestStore::new();
        store.insert_default(&java_record(1_000)).expect("first");
        let second = store.insert_default(&java_record(2_000)).expect("second");

        assert!(!second.is_new_group);
        assert!(second.record.is_repeating);
        assert_eq!(second.group.occurrence, 2);
        assert_eq!(second.group.first_seen_ms, 1_000, "first sighting is kept");
        assert_eq!(second.group.last_seen_ms, 2_000, "latest sighting wins");
    }

    #[test]
    fn an_out_of_order_occurrence_does_not_move_the_bounds_backwards() {
        let store = TestStore::new();
        store.insert_default(&java_record(5_000)).expect("newer");
        let older = store.insert_default(&java_record(1_000)).expect("older");

        assert_eq!(older.group.first_seen_ms, 1_000);
        assert_eq!(older.group.last_seen_ms, 5_000);
    }

    #[test]
    fn different_fingerprints_get_different_groups() {
        let store = TestStore::new();
        let first = store.insert_default(&java_record(1_000)).expect("first");

        let mut other = java_record(2_000);
        other.fingerprint = cch_model::Fingerprint::from_raw_frames(
            cch_model::CrashKind::JavaException,
            "java.lang.NullPointerException",
            &["at com.example.app.Other.run(Other.kt:9)".to_owned()],
        );
        let second = store.insert_default(&other).expect("second");

        assert!(second.is_new_group);
        assert_ne!(first.group.group_id, second.group.group_id);
    }

    #[test]
    fn payload_bytes_accumulate_on_the_group() {
        let store = TestStore::new();
        let mut record = java_record(1_000);
        record.payload = PayloadSource::Inline(vec![b'x'; 4_096]);

        let first = store.insert_default(&record).expect("first");
        let second = store.insert_default(&record).expect("second");

        assert!(first.record.payload_bytes > 0);
        assert_eq!(
            second.group.payload_bytes,
            first.record.payload_bytes + second.record.payload_bytes
        );
    }

    #[test]
    fn a_record_without_a_payload_is_stored_as_absent() {
        // Not Evicted: that state means retention reclaimed a stack, and the UI says so. A
        // record that never had one has to be able to say that instead.
        let store = TestStore::new();
        let mut record = java_record(1_000);
        record.payload = PayloadSource::None;

        let inserted = store.insert_default(&record).expect("inserts");
        assert_eq!(inserted.record.payload_bytes, 0);
        assert_eq!(inserted.record.payload_state, PayloadState::Absent);
    }

    #[test]
    fn an_oversized_payload_is_marked_truncated() {
        let store = TestStore::new();
        let mut record = java_record(1_000);
        record.payload = PayloadSource::Inline(vec![b'x'; 500_000]);

        let retention = RetentionPolicy {
            max_payload_bytes_per_record: RetentionPolicy::MIN_PAYLOAD_BYTES_PER_RECORD,
            ..RetentionPolicy::default()
        };
        let inserted = store.store.insert(&record, retention).expect("inserts");
        assert_eq!(inserted.record.payload_state, PayloadState::Truncated);
    }

    #[test]
    fn source_masks_survive_the_round_trip() {
        let store = TestStore::new();
        let mut record = java_record(1_000);
        record.sources = SourceMask::EVENTS
            .union(SourceMask::CRASH_BUFFER)
            .union(SourceMask::DROPBOX);

        let inserted = store.insert_default(&record).expect("inserts");
        assert!(inserted.record.sources.contains(SourceMask::EVENTS));
        assert!(inserted.record.sources.contains(SourceMask::CRASH_BUFFER));
        assert!(inserted.record.sources.contains(SourceMask::DROPBOX));
        assert!(!inserted.record.sources.contains(SourceMask::TOMBSTONE));
    }

    #[test]
    fn the_dropbox_rate_limiter_count_is_preserved() {
        let store = TestStore::new();
        let mut record = java_record(1_000);
        record.dropped_count = Some(12);

        let inserted = store.insert_default(&record).expect("inserts");
        assert_eq!(inserted.record.dropped_count, Some(12));
    }

    #[test]
    fn the_group_reflects_the_latest_self_handled_verdict() {
        let store = TestStore::new();
        let mut record = java_record(1_000);
        record.self_handled = false;
        store.insert_default(&record).expect("first");

        record.self_handled = true;
        let second = store.insert_default(&record).expect("second");
        assert!(second.group.self_handled);
    }

    #[test]
    fn ingested_markers_claim_an_artefact_exactly_once() {
        let store = TestStore::new();
        let key = "tombstone_07:1755440000:4096";

        assert!(!store.store.was_ingested(key).expect("checks"));
        assert!(store.store.mark_ingested(key, 1).expect("claims"));
        assert!(store.store.was_ingested(key).expect("checks"));
        assert!(
            !store.store.mark_ingested(key, 2).expect("second claim"),
            "a second claim must report the duplicate"
        );
    }

    #[test]
    fn a_record_and_its_source_keys_commit_together() {
        let store = TestStore::new();
        let source_keys = vec!["events:1".to_owned(), "crash:1".to_owned()];

        store
            .store
            .insert_with_sources(
                &java_record(1_000),
                RetentionPolicy::default(),
                &source_keys,
                1_001,
            )
            .expect("inserts record and keys");

        assert!(store.store.was_ingested("events:1").expect("checks"));
        assert!(store.store.was_ingested("crash:1").expect("checks"));
    }

    #[test]
    fn a_reused_tombstone_slot_is_a_different_artefact() {
        let store = TestStore::new();
        // Same name, different mtime and size.
        assert!(
            store
                .store
                .mark_ingested("tombstone_07:100:512", 1)
                .expect("first")
        );
        assert!(
            store
                .store
                .mark_ingested("tombstone_07:900:777", 2)
                .expect("second")
        );
    }

    #[test]
    fn mutes_apply_and_clear() {
        let store = TestStore::new();
        let inserted = store.insert_default(&java_record(1_000)).expect("inserts");
        let group_id = inserted.group.group_id;

        assert!(
            store
                .store
                .set_group_mute(&group_id, Some(9_999))
                .expect("mutes")
        );
        assert_eq!(store.group(&group_id).muted_until_ms, Some(9_999));

        assert_eq!(store.store.clear_all_mutes().expect("clears"), 1);
        assert_eq!(store.group(&group_id).muted_until_ms, None);
    }

    #[test]
    fn muting_a_package_covers_all_of_its_groups() {
        let store = TestStore::new();
        store.insert_default(&java_record(1_000)).expect("first");

        let mut other = java_record(2_000);
        other.fingerprint = fixture::other_fingerprint();
        store.insert_default(&other).expect("second");

        assert_eq!(
            store
                .store
                .set_package_mute("com.example.app", Some(1))
                .expect("mutes"),
            2
        );
    }

    #[test]
    fn muting_an_unknown_group_reports_no_change() {
        let store = TestStore::new();
        assert!(
            !store
                .store
                .set_group_mute("nonexistent", Some(1))
                .expect("no-op")
        );
    }

    /// DropBox reported the crash; a later daemon rescanned `/data/tombstones` and read the
    /// tombstone for it. Same pid and millisecond, different groups — the two sources normalise
    /// to different frames.
    #[test]
    fn a_tombstone_read_days_later_folds_into_the_crash_dropbox_already_reported() {
        let store = TestStore::new();
        let mut dropbox = native_record(1_000);
        dropbox.sources = SourceMask::EVENTS.union(SourceMask::DROPBOX);
        let first = store.insert_default(&dropbox).expect("inserts");

        let mut tombstone = native_record(1_000);
        tombstone.sources = SourceMask::TOMBSTONE;
        tombstone.fingerprint = fixture::other_fingerprint();

        let companion = store
            .store
            .companion_record(&tombstone, 10_000)
            .expect("looks for a companion")
            .expect("the stored crash is one");
        assert_eq!(companion.id, first.record.id);

        store
            .store
            .fold_into_record(
                &companion,
                &tombstone,
                RetentionPolicy::default(),
                &[],
                2_000,
            )
            .expect("folds");

        assert_eq!(store.all_groups().len(), 1, "one crash is one group");
        let group = store.group(&first.group.group_id);
        assert_eq!(group.occurrence, 1, "folding is not a second occurrence");
        let records = store.records_of(&first.group.group_id);
        assert_eq!(records.len(), 1);
        assert!(
            records[0].sources.contains(SourceMask::TOMBSTONE),
            "the late source has to show on the record it belongs to"
        );
    }

    /// The events buffer reports `am_crash` with no stack at all. When the tombstone turns up it
    /// is the only copy of the backtrace, so folding has to adopt it rather than keep the nothing
    /// that was stored first.
    #[test]
    fn folding_adopts_a_payload_when_the_stored_record_has_none() {
        let store = TestStore::new();
        let mut events_only = native_record(1_000);
        events_only.sources = SourceMask::EVENTS;
        events_only.payload = PayloadSource::None;
        let first = store.insert_default(&events_only).expect("inserts");
        assert_eq!(first.record.payload_state, PayloadState::Absent);

        let mut tombstone = native_record(1_000);
        tombstone.sources = SourceMask::TOMBSTONE;
        tombstone.payload = PayloadSource::Inline(b"backtrace".to_vec());
        let companion = store
            .store
            .companion_record(&tombstone, 10_000)
            .expect("looks")
            .expect("finds");
        let folded = store
            .store
            .fold_into_record(
                &companion,
                &tombstone,
                RetentionPolicy::default(),
                &[],
                2_000,
            )
            .expect("folds");

        assert_eq!(folded.payload_state, PayloadState::Present);
        assert!(folded.payload_bytes > 0);
        assert_eq!(
            store.group(&first.group.group_id).payload_bytes,
            folded.payload_bytes,
            "the group's byte total has to follow the payload it gained"
        );
    }

    /// Two sightings that share a source are two sightings. Only an artefact read twice lands on
    /// the same millisecond, and that case still folds.
    #[test]
    fn a_second_crash_from_the_same_source_is_not_a_companion() {
        let store = TestStore::new();
        let record = native_record(1_000);
        store.insert_default(&record).expect("inserts");

        let mut later = native_record(3_000);
        later.sources = record.sources;
        assert!(
            store
                .store
                .companion_record(&later, 10_000)
                .expect("looks")
                .is_none(),
            "a later crash reported by the same source is a new occurrence"
        );

        let mut reread = native_record(1_000);
        reread.sources = record.sources;
        assert!(
            store
                .store
                .companion_record(&reread, 10_000)
                .expect("looks")
                .is_some(),
            "the same artefact read twice is not"
        );
    }

    /// A pid of zero is what a source reports when it did not see one, and every such record
    /// would otherwise look like every other.
    #[test]
    fn records_without_a_pid_are_never_folded() {
        let store = TestStore::new();
        let mut first = native_record(1_000);
        first.pid = 0;
        store.insert_default(&first).expect("inserts");

        let mut second = native_record(1_000);
        second.pid = 0;
        second.sources = SourceMask::TOMBSTONE;
        assert!(
            store
                .store
                .companion_record(&second, 10_000)
                .expect("looks")
                .is_none()
        );
    }

    /// Groups written before `cmd package` answered carry the wrong verdict, and an insert only
    /// re-resolves it when the same crash happens again.
    #[test]
    fn a_completed_package_index_repairs_the_boot_time_verdict() {
        let store = TestStore::new();
        let mut early = java_record(1_000);
        early.package_name = "com.android.systemui".to_owned();
        early.process_name = "com.android.systemui".to_owned();
        early.is_system_app = false;
        let inserted = store.insert_default(&early).expect("inserts");
        assert!(!store.group(&inserted.group.group_id).is_system_app);

        let names = store.store.group_package_names().expect("reads names");
        assert_eq!(names, vec!["com.android.systemui".to_owned()]);

        let changed = store
            .store
            .apply_package_system_flags(&[("com.android.systemui".to_owned(), true)])
            .expect("applies");

        assert_eq!(changed, 1);
        assert!(store.group(&inserted.group.group_id).is_system_app);
    }

    #[test]
    fn re_applying_the_same_verdict_changes_nothing() {
        let store = TestStore::new();
        let inserted = store.insert_default(&java_record(1_000)).expect("inserts");
        let flags = [("com.example.app".to_owned(), false)];

        assert_eq!(
            store
                .store
                .apply_package_system_flags(&flags)
                .expect("applies"),
            0
        );
        assert!(!store.group(&inserted.group.group_id).is_system_app);
    }
}
