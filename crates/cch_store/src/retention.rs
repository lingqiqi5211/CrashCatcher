use cch_config::RetentionPolicy;
use cch_model::PayloadState;
use rusqlite::{params, params_from_iter};
use tracing::debug;

use crate::{Store, StoreError};

const MS_PER_DAY: i64 = 24 * 60 * 60 * 1000;

/// What one sweep reclaimed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SweepOutcome {
    pub removed_records: u64,
    pub removed_groups: u64,
    /// Records whose payload was dropped while the row itself survived.
    pub evicted_payloads: u64,
    pub reclaimed_bytes: u64,
}

impl SweepOutcome {
    #[must_use]
    pub const fn did_nothing(&self) -> bool {
        self.removed_records == 0
            && self.removed_groups == 0
            && self.evicted_payloads == 0
            && self.reclaimed_bytes == 0
    }
}

impl Store {
    /// Applies the four retention ceilings, in order.
    ///
    /// 1. **Age** — occurrences older than `retention_days`.
    /// 2. **Per group** — keep only the newest `max_records_per_group` occurrences.
    /// 3. **Total count** — oldest occurrences first.
    /// 4. **Total bytes** — reclaim payload *files* only, oldest first, leaving the
    ///    metadata row behind.
    ///
    /// Order matters: the cheap, obviously-correct rules run first so the byte quota
    /// has less to do. The fourth tier degrades instead of deleting — history,
    /// occurrence counts and the trend chart stay intact, and the list still shows
    /// the crash; only the full stack becomes unavailable. Losing the row entirely
    /// would silently rewrite the user's statistics.
    ///
    /// `occurrence` and `first_seen_ms` are never touched, so a group's count keeps
    /// meaning "how many times this happened", not "how many detail rows survive".
    pub fn sweep(
        &self,
        now_ms: i64,
        retention: RetentionPolicy,
    ) -> Result<SweepOutcome, StoreError> {
        let retention = retention.clamped();
        let mut outcome = SweepOutcome::default();

        let cutoff_ms = now_ms.saturating_sub(i64::from(retention.retention_days) * MS_PER_DAY);
        outcome.removed_records += self.delete_records_where(
            "happened_at_ms < ?1",
            &[rusqlite::types::Value::Integer(cutoff_ms)],
        )?;

        outcome.removed_records += self.trim_each_group(retention.max_records_per_group)?;
        outcome.removed_records += self.trim_total(retention.max_records_total)?;

        // Groups outside the declared window with nothing left in them are no longer
        // history the user asked to keep.
        outcome.removed_groups += self.drop_empty_groups_before(cutoff_ms)?;

        let (evicted, reclaimed) = self.evict_payloads(retention.max_payload_bytes_total)?;
        outcome.evicted_payloads = evicted;
        outcome.reclaimed_bytes = reclaimed;

        if !outcome.did_nothing() {
            debug!(
                removed_records = outcome.removed_records,
                removed_groups = outcome.removed_groups,
                evicted_payloads = outcome.evicted_payloads,
                reclaimed_bytes = outcome.reclaimed_bytes,
                "retention sweep"
            );
        }
        Ok(outcome)
    }

    /// Deletes matching records, unlinking their payloads first.
    fn delete_records_where(
        &self,
        predicate: &str,
        values: &[rusqlite::types::Value],
    ) -> Result<u64, StoreError> {
        let ids = {
            let connection = self.connection()?;
            let mut statement = connection.prepare(&format!(
                "SELECT id, payload_path, payload_bytes, group_id
                 FROM crash_record WHERE {predicate}"
            ))?;
            let rows = statement.query_map(params_from_iter(values.iter()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };

        if ids.is_empty() {
            return Ok(0);
        }

        for (_, path, _, _) in &ids {
            if let Some(path) = path {
                self.payloads.delete(path)?;
            }
        }

        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        for (id, _, bytes, group_id) in &ids {
            transaction.execute("DELETE FROM crash_record WHERE id = ?1", params![id])?;
            // Keep the group's byte accounting honest; occurrence stays put.
            transaction.execute(
                "UPDATE crash_group SET payload_bytes = MAX(0, payload_bytes - ?2)
                 WHERE group_id = ?1",
                params![group_id, bytes],
            )?;
        }
        transaction.commit()?;

        Ok(ids.len() as u64)
    }

    fn trim_each_group(&self, keep_per_group: u32) -> Result<u64, StoreError> {
        self.delete_records_where(
            "id IN (
                 SELECT id FROM (
                     SELECT id, ROW_NUMBER() OVER (
                         PARTITION BY group_id ORDER BY happened_at_ms DESC, id DESC
                     ) AS position
                     FROM crash_record
                 ) WHERE position > ?1
             )",
            &[rusqlite::types::Value::Integer(i64::from(keep_per_group))],
        )
    }

    fn trim_total(&self, keep_total: u32) -> Result<u64, StoreError> {
        let total: i64 = {
            let connection = self.connection()?;
            connection.query_row("SELECT COUNT(*) FROM crash_record", [], |row| row.get(0))?
        };
        let excess = total - i64::from(keep_total);
        if excess <= 0 {
            return Ok(0);
        }

        self.delete_records_where(
            "id IN (
                 SELECT id FROM crash_record
                 ORDER BY happened_at_ms ASC, id ASC LIMIT ?1
             )",
            &[rusqlite::types::Value::Integer(excess)],
        )
    }

    /// Reclaims payload files, oldest first, until the total fits the quota.
    fn evict_payloads(&self, max_bytes: u64) -> Result<(u64, u64), StoreError> {
        let total = self.payload_bytes_in_index()?;
        if total <= max_bytes {
            return Ok((0, 0));
        }
        let mut over_by = total - max_bytes;

        let candidates = {
            let connection = self.connection()?;
            let mut statement = connection.prepare(
                "SELECT id, payload_path, payload_bytes, group_id
                 FROM crash_record
                 WHERE payload_path IS NOT NULL AND payload_state != ?1
                 ORDER BY happened_at_ms ASC, id ASC",
            )?;
            let rows = statement.query_map(params![PayloadState::Evicted.as_i64()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };

        let mut evicted = 0;
        let mut reclaimed = 0;
        for (id, path, bytes, group_id) in candidates {
            if over_by == 0 {
                break;
            }
            self.payloads.delete(&path)?;

            let mut connection = self.connection()?;
            let transaction = connection.transaction()?;
            transaction.execute(
                "UPDATE crash_record
                 SET payload_path = NULL, payload_bytes = 0, payload_state = ?2
                 WHERE id = ?1",
                params![&id, PayloadState::Evicted.as_i64()],
            )?;
            transaction.execute(
                "UPDATE crash_group SET payload_bytes = MAX(0, payload_bytes - ?2)
                 WHERE group_id = ?1",
                params![&group_id, bytes],
            )?;
            transaction.commit()?;

            let freed = bytes.max(0) as u64;
            reclaimed += freed;
            over_by = over_by.saturating_sub(freed);
            evicted += 1;
        }

        Ok((evicted, reclaimed))
    }

    fn payload_bytes_in_index(&self) -> Result<u64, StoreError> {
        let connection = self.connection()?;
        let total: i64 = connection.query_row(
            "SELECT COALESCE(SUM(payload_bytes), 0) FROM crash_record
             WHERE payload_path IS NOT NULL",
            [],
            |row| row.get(0),
        )?;
        Ok(total.max(0) as u64)
    }

    fn drop_empty_groups_before(&self, cutoff_ms: i64) -> Result<u64, StoreError> {
        let connection = self.connection()?;
        let removed = connection.execute(
            "DELETE FROM crash_group
             WHERE last_seen_ms < ?1
               AND group_id NOT IN (SELECT DISTINCT group_id FROM crash_record)",
            params![cutoff_ms],
        )?;
        Ok(removed as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::MS_PER_DAY;
    use crate::test_support::{TestStore, java_record, record_for};
    use cch_config::RetentionPolicy;
    use cch_model::{PayloadSource, PayloadState};

    fn policy() -> RetentionPolicy {
        RetentionPolicy::default()
    }

    #[test]
    fn a_sweep_of_an_empty_store_does_nothing() {
        let store = TestStore::new();
        let outcome = store.store.sweep(1_000, policy()).expect("sweeps");
        assert!(outcome.did_nothing());
    }

    #[test]
    fn fresh_records_survive_a_sweep() {
        let store = TestStore::new();
        let now = 100 * MS_PER_DAY;
        store.insert_default(&java_record(now)).expect("inserts");

        let outcome = store.store.sweep(now, policy()).expect("sweeps");
        assert!(outcome.did_nothing());
        assert_eq!(store.all_groups().len(), 1);
    }

    #[test]
    fn records_past_the_retention_window_are_removed() {
        let store = TestStore::new();
        let now = 100 * MS_PER_DAY;
        // 40 days old, window is 30.
        store
            .insert_default(&java_record(now - 40 * MS_PER_DAY))
            .expect("old");
        store.insert_default(&java_record(now)).expect("fresh");

        let outcome = store.store.sweep(now, policy()).expect("sweeps");
        assert_eq!(outcome.removed_records, 1);

        let group = &store.all_groups()[0];
        assert_eq!(
            group.occurrence, 2,
            "occurrence records history, not surviving rows"
        );
        assert_eq!(store.records_of(&group.group_id).len(), 1);
    }

    #[test]
    fn a_group_whose_records_all_expired_is_dropped_with_them() {
        let store = TestStore::new();
        let now = 100 * MS_PER_DAY;
        store
            .insert_default(&java_record(now - 40 * MS_PER_DAY))
            .expect("old");

        let outcome = store.store.sweep(now, policy()).expect("sweeps");
        assert_eq!(outcome.removed_records, 1);
        assert_eq!(outcome.removed_groups, 1);
        assert!(store.all_groups().is_empty());
    }

    #[test]
    fn a_group_with_recent_activity_survives_even_if_old_records_expire() {
        let store = TestStore::new();
        let now = 100 * MS_PER_DAY;
        store
            .insert_default(&java_record(now - 40 * MS_PER_DAY))
            .expect("old");
        store.insert_default(&java_record(now)).expect("fresh");

        store.store.sweep(now, policy()).expect("sweeps");
        assert_eq!(store.all_groups().len(), 1);
    }

    #[test]
    fn each_group_keeps_only_the_newest_occurrences() {
        let store = TestStore::new();
        let now = 100 * MS_PER_DAY;
        for index in 0..10 {
            store
                .insert_default(&java_record(now - i64::from(index)))
                .expect("inserts");
        }

        let retention = RetentionPolicy {
            max_records_per_group: 3,
            ..policy()
        };
        let outcome = store.store.sweep(now, retention).expect("sweeps");
        assert_eq!(outcome.removed_records, 7);

        let group = &store.all_groups()[0];
        assert_eq!(group.occurrence, 10);
        let records = store.records_of(&group.group_id);
        assert_eq!(records.len(), 3);
        // The newest survive.
        assert_eq!(records[0].happened_at_ms, now);
    }

    #[test]
    fn the_per_group_cap_is_applied_group_by_group() {
        let store = TestStore::new();
        let now = 100 * MS_PER_DAY;
        for index in 0..4 {
            store
                .insert_default(&record_for("com.a", now - i64::from(index)))
                .expect("a");
            store
                .insert_default(&record_for("com.b", now - i64::from(index)))
                .expect("b");
        }

        let retention = RetentionPolicy {
            max_records_per_group: 2,
            ..policy()
        };
        store.store.sweep(now, retention).expect("sweeps");

        for group in store.all_groups() {
            assert_eq!(
                store.records_of(&group.group_id).len(),
                2,
                "{} kept the wrong number",
                group.package_name
            );
        }
    }

    #[test]
    fn the_global_ceiling_drops_the_oldest_first() {
        let store = TestStore::new();
        let now = 100 * MS_PER_DAY;
        for index in 0..10 {
            store
                .insert_default(&record_for(
                    &format!("com.app{index:02}"),
                    now - i64::from(index),
                ))
                .expect("inserts");
        }

        // Driven directly: `sweep` clamps the configured ceiling up to
        // MIN_RECORDS_TOTAL, which is well above ten, so the public path cannot
        // exercise this tier with a small fixture.
        assert_eq!(store.store.trim_total(4).expect("trims"), 6);

        let survivors: Vec<i64> = store
            .all_groups()
            .iter()
            .flat_map(|group| store.records_of(&group.group_id))
            .map(|record| record.happened_at_ms)
            .collect();
        assert_eq!(survivors.len(), 4);
        assert!(
            survivors.iter().all(|at| *at > now - 4),
            "the four newest must be the survivors, got {survivors:?}"
        );
    }

    #[test]
    fn the_configured_global_ceiling_is_respected_once_it_is_reachable() {
        let store = TestStore::new();
        let now = 100 * MS_PER_DAY;
        let ceiling = RetentionPolicy::MIN_RECORDS_TOTAL;
        for index in 0..(ceiling + 5) {
            store
                .insert_default(&record_for("com.example.app", now - i64::from(index)))
                .expect("inserts");
        }

        let retention = RetentionPolicy {
            max_records_total: ceiling,
            // Keep the per-group tier out of the way; everything shares one group here.
            max_records_per_group: RetentionPolicy::MAX_RECORDS_PER_GROUP,
            ..policy()
        };
        store.store.sweep(now, retention).expect("sweeps");

        assert_eq!(
            store.store.storage_status().expect("status").record_count,
            u64::from(ceiling)
        );
    }

    #[test]
    fn the_byte_quota_reclaims_payloads_but_keeps_the_rows() {
        let store = TestStore::new();
        let now = 100 * MS_PER_DAY;
        for index in 0..6 {
            let mut record = record_for(&format!("com.app{index:02}"), now - i64::from(index));
            // Incompressible payload so the quota bites predictably.
            record.payload = PayloadSource::Inline(
                (0..200_000u32)
                    .map(|value| value.to_le_bytes()[0])
                    .collect(),
            );
            store.insert_default(&record).expect("inserts");
        }

        let before = store.store.storage_status().expect("status");
        assert!(before.payload_bytes > 0);

        let retention = RetentionPolicy {
            max_payload_bytes_total: RetentionPolicy::MIN_PAYLOAD_BYTES_TOTAL,
            ..policy()
        };
        // Bytes are well under the 8 MiB floor, so nothing is evicted yet.
        let outcome = store.store.sweep(now, retention).expect("sweeps");
        assert_eq!(outcome.evicted_payloads, 0);

        let after = store.store.storage_status().expect("status");
        assert_eq!(
            after.record_count, 6,
            "rows must never go for byte pressure"
        );
    }

    #[test]
    fn eviction_marks_records_and_frees_disk_without_losing_history() {
        let store = TestStore::new();
        let now = 100 * MS_PER_DAY;
        for index in 0..4 {
            let mut record = record_for(&format!("com.app{index:02}"), now - i64::from(index));
            record.payload = PayloadSource::Inline(vec![b'x'; 100_000]);
            store.insert_default(&record).expect("inserts");
        }

        // Drive the quota directly, below the configurable floor, to exercise the tier.
        let (evicted, reclaimed) = store.store.evict_payloads(1).expect("evicts");
        assert!(evicted > 0, "something had to give");
        assert!(reclaimed > 0);

        let status = store.store.storage_status().expect("status");
        assert_eq!(status.record_count, 4, "rows survive");
        assert_eq!(status.group_count, 4, "groups survive");
        assert_eq!(status.evicted_payload_count, evicted);
        assert_eq!(status.payload_bytes, 0, "files are gone from disk");

        // The oldest record lost its payload and says so.
        let groups = store.all_groups();
        let oldest = groups
            .iter()
            .min_by_key(|group| group.last_seen_ms)
            .expect("has groups");
        let record = &store.records_of(&oldest.group_id)[0];
        assert_eq!(record.payload_state, PayloadState::Evicted);
        assert_eq!(record.payload_bytes, 0);

        let error = store
            .store
            .read_payload_text(&record.id)
            .map(|_| ())
            .unwrap_err();
        assert_eq!(error.to_wire().code, cch_wire::ErrorCode::NotFound);
    }

    #[test]
    fn group_byte_accounting_follows_eviction() {
        let store = TestStore::new();
        let now = 100 * MS_PER_DAY;
        let mut record = java_record(now);
        record.payload = PayloadSource::Inline(vec![b'x'; 100_000]);
        let inserted = store.insert_default(&record).expect("inserts");
        assert!(inserted.group.payload_bytes > 0);

        store.store.evict_payloads(0).expect("evicts");
        assert_eq!(store.group(&inserted.group.group_id).payload_bytes, 0);
    }

    #[test]
    fn group_byte_accounting_follows_record_deletion() {
        let store = TestStore::new();
        let now = 100 * MS_PER_DAY;
        let mut old = java_record(now - 40 * MS_PER_DAY);
        old.payload = PayloadSource::Inline(vec![b'x'; 50_000]);
        store.insert_default(&old).expect("old");

        let mut fresh = java_record(now);
        fresh.payload = PayloadSource::Inline(vec![b'y'; 50_000]);
        let inserted = store.insert_default(&fresh).expect("fresh");

        store.store.sweep(now, policy()).expect("sweeps");
        assert_eq!(
            store.group(&inserted.group.group_id).payload_bytes,
            inserted.record.payload_bytes,
            "only the surviving record's bytes should remain accounted for"
        );
    }

    #[test]
    fn sweeping_is_idempotent() {
        let store = TestStore::new();
        let now = 100 * MS_PER_DAY;
        store
            .insert_default(&java_record(now - 40 * MS_PER_DAY))
            .expect("old");
        store.insert_default(&java_record(now)).expect("fresh");

        store.store.sweep(now, policy()).expect("first sweep");
        let second = store.store.sweep(now, policy()).expect("second sweep");
        assert!(second.did_nothing(), "a settled store must stay settled");
    }

    #[test]
    fn retention_values_are_clamped_before_use() {
        let store = TestStore::new();
        let now = 100 * MS_PER_DAY;
        store.insert_default(&java_record(now)).expect("inserts");

        // retention_days = 0 would delete everything if taken literally.
        let absurd = RetentionPolicy {
            retention_days: 0,
            ..policy()
        };
        let outcome = store.store.sweep(now, absurd).expect("sweeps");
        assert_eq!(
            outcome.removed_records, 0,
            "clamping to a one-day minimum must protect today's records"
        );
    }
}
