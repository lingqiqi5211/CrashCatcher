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
        let id = self.next_id(record.happened_at_ms)?;
        let written =
            self.payloads
                .write(&id, &record.payload, retention.max_payload_bytes_per_record)?;

        match self.insert_rows(&id, record, written.as_ref()) {
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

        transaction.commit()?;

        Ok(Inserted {
            record: stored_record,
            group,
            is_new_group,
        })
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
    use crate::test_support::{TestStore, fixture, java_record};
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
}
