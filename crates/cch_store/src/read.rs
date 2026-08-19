use std::collections::HashSet;

use cch_model::{PayloadCodec, PayloadState, RecordId};
use cch_wire::{
    Cursor, CursorAnchor, DeleteTarget, ExceptionCount, GroupSummary, KindCount, Page, PageRequest,
    RecordDetail, RecordSummary, SortKey, Stats, StorageStatus, TrendBucket,
};
use rusqlite::{OptionalExtension, params, params_from_iter};

use crate::{Store, StoreError, sql};

/// Per-package aggregate for the apps screen.
///
/// The store contributes crash counts; the daemon adds labels and the installed-app
/// list from the bridge, since neither is knowable from storage alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageRollup {
    pub package_name: String,
    pub user_id: i32,
    pub is_system_app: bool,
    pub group_count: u64,
    pub occurrence: u64,
    pub last_seen_ms: i64,
}

/// Default number of trend buckets when the caller does not pick a width.
const DEFAULT_TREND_BUCKETS: i64 = 24;

impl Store {
    /// One page of crash groups.
    ///
    /// Single table, no join, no payload read — the property that makes the list
    /// open instantly regardless of history size. One extra row is fetched to learn
    /// whether a next page exists without a second `COUNT(*)`.
    pub fn list_groups(&self, page: &PageRequest) -> Result<Page<GroupSummary>, StoreError> {
        page.validate()?;
        let limit = page.effective_limit();

        let mut predicates = sql::group_filter(&page.filter);
        if let Some(cursor) = &page.cursor {
            predicates.extend(sql::cursor_predicate(cursor, page.sort)?);
        }

        let statement_sql = format!(
            "SELECT {columns} FROM crash_group{where_clause}{order_by} LIMIT ?",
            columns = sql::GROUP_COLUMNS,
            where_clause = predicates.where_clause(),
            order_by = sql::group_order_by(page.sort),
        );

        let mut params = predicates.params();
        params.push(rusqlite::types::Value::Integer(i64::from(limit) + 1));

        let connection = self.connection()?;
        let mut statement = connection.prepare(&statement_sql)?;
        let mut items = statement
            .query_map(params_from_iter(params), sql::map_group)?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let next_cursor = if items.len() > limit as usize {
            items.truncate(limit as usize);
            items
                .last()
                .map(|group| sql::cursor_after(group, page.sort))
        } else {
            None
        };

        Ok(Page::new(items, next_cursor))
    }

    /// One page of occurrences inside a group, newest first.
    ///
    /// Record pages are always time-ordered, so the cursor's anchor is
    /// `happened_at_ms` and its tiebreak is the record id.
    pub fn list_records(
        &self,
        group_id: &str,
        page: &PageRequest,
    ) -> Result<Page<RecordSummary>, StoreError> {
        let limit = page.effective_limit();

        let mut clauses = String::from(" WHERE group_id = ?1");
        let mut params: Vec<rusqlite::types::Value> =
            vec![rusqlite::types::Value::Text(group_id.to_owned())];

        if let Some(cursor) = &page.cursor {
            cursor.validate_for(SortKey::LastSeenDesc)?;
            let CursorAnchor::Int(anchor) = cursor.anchor else {
                return Err(StoreError::Request(cch_wire::WireError::cursor_invalidated(
                    "record pages anchor on a timestamp",
                )));
            };
            clauses.push_str(" AND (happened_at_ms < ?2 OR (happened_at_ms = ?2 AND id < ?3))");
            params.push(rusqlite::types::Value::Integer(anchor));
            params.push(rusqlite::types::Value::Text(cursor.tiebreak.clone()));
        }

        let statement_sql = format!(
            "SELECT {columns} FROM crash_record{clauses} \
             ORDER BY happened_at_ms DESC, id DESC LIMIT ?{index}",
            columns = sql::RECORD_COLUMNS,
            index = params.len() + 1,
        );
        params.push(rusqlite::types::Value::Integer(i64::from(limit) + 1));

        let connection = self.connection()?;
        let mut statement = connection.prepare(&statement_sql)?;
        let mut items = statement
            .query_map(params_from_iter(params), sql::map_record)?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let next_cursor = if items.len() > limit as usize {
            items.truncate(limit as usize);
            items.last().map(|record| {
                Cursor::new(
                    SortKey::LastSeenDesc,
                    CursorAnchor::Int(record.happened_at_ms),
                    record.id.as_str(),
                )
            })
        } else {
            None
        };

        Ok(Page::new(items, next_cursor))
    }

    /// One group by id.
    pub fn get_group(&self, group_id: &str) -> Result<GroupSummary, StoreError> {
        let connection = self.connection()?;
        connection
            .query_row(
                &format!(
                    "SELECT {} FROM crash_group WHERE group_id = ?1",
                    sql::GROUP_COLUMNS
                ),
                params![group_id],
                sql::map_group,
            )
            .optional()?
            .ok_or_else(|| StoreError::GroupNotFound {
                id: group_id.to_owned(),
            })
    }

    /// One record together with its group, for the detail screen's header.
    pub fn get_record(&self, id: &RecordId) -> Result<RecordDetail, StoreError> {
        let connection = self.connection()?;
        let record: Option<RecordSummary> = connection
            .query_row(
                &format!(
                    "SELECT {} FROM crash_record WHERE id = ?1",
                    sql::RECORD_COLUMNS
                ),
                params![id.as_str()],
                sql::map_record,
            )
            .optional()?;
        let record = record.ok_or_else(|| StoreError::RecordNotFound {
            id: id.as_str().to_owned(),
        })?;

        let group = connection.query_row(
            &format!(
                "SELECT {} FROM crash_group WHERE group_id = ?1",
                sql::GROUP_COLUMNS
            ),
            params![&record.group_id],
            sql::map_group,
        )?;

        Ok(RecordDetail { record, group })
    }

    /// Where a record's payload lives, if it still has one.
    fn payload_location(&self, id: &RecordId) -> Result<(String, PayloadCodec), StoreError> {
        let connection = self.connection()?;
        let row: Option<(Option<String>, i64, i64)> = connection
            .query_row(
                "SELECT payload_path, payload_codec, payload_state FROM crash_record WHERE id = ?1",
                params![id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;

        let (path, codec, state) = row.ok_or_else(|| StoreError::RecordNotFound {
            id: id.as_str().to_owned(),
        })?;

        let readable = PayloadState::from_i64(state).is_some_and(PayloadState::is_readable);
        match path {
            Some(path) if readable => Ok((path, sql::codec_from_i64(codec))),
            _ => Err(StoreError::PayloadUnavailable {
                id: id.as_str().to_owned(),
            }),
        }
    }

    /// Reads a record's payload as text.
    pub fn read_payload_text(&self, id: &RecordId) -> Result<String, StoreError> {
        let (path, codec) = self.payload_location(id)?;
        self.payloads.read_text(&path, codec)
    }

    /// Reads one chunk of a record's payload. See [`PayloadStore::read_chunk`].
    pub fn read_payload_chunk(
        &self,
        id: &RecordId,
        offset: u64,
        len: u32,
    ) -> Result<(String, u64, bool), StoreError> {
        let (path, codec) = self.payload_location(id)?;
        self.payloads.read_chunk(&path, codec, offset, len)
    }

    /// Materializes a payload into an anonymous file and returns the descriptor.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    pub fn open_payload_fd(&self, id: &RecordId) -> Result<std::os::fd::OwnedFd, StoreError> {
        let (path, codec) = self.payload_location(id)?;
        self.payloads.open_memfd(&path, codec)
    }

    /// Text byte count of a payload, for `open_payload`'s reply.
    pub fn payload_text_bytes(&self, id: &RecordId) -> Result<u64, StoreError> {
        Ok(self.read_payload_text(id)?.len() as u64)
    }

    /// Deletes records or groups at the user's request.
    ///
    /// Unlike retention eviction, this removes rows: the user asked for the history
    /// to be gone, so a group left with no occurrences goes too. Payload files are
    /// unlinked first — the reverse order would strand them if the transaction failed.
    pub fn delete(&self, target: &DeleteTarget) -> Result<(u64, u64), StoreError> {
        let paths = self.payload_paths_for(target)?;
        for path in &paths {
            self.payloads.delete(path)?;
        }

        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;

        let removed_records;
        let removed_groups;

        match target {
            DeleteTarget::All => {
                removed_records = transaction.execute("DELETE FROM crash_record", [])? as u64;
                removed_groups = transaction.execute("DELETE FROM crash_group", [])? as u64;
            }
            DeleteTarget::Group { group_id } => {
                removed_records = transaction.execute(
                    "DELETE FROM crash_record WHERE group_id = ?1",
                    params![group_id],
                )? as u64;
                removed_groups = transaction.execute(
                    "DELETE FROM crash_group WHERE group_id = ?1",
                    params![group_id],
                )? as u64;
            }
            DeleteTarget::Ids { ids } => {
                if ids.is_empty() {
                    transaction.commit()?;
                    return Ok((0, 0));
                }
                let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
                removed_records = transaction.execute(
                    &format!("DELETE FROM crash_record WHERE id IN ({placeholders})"),
                    params_from_iter(ids.iter().map(|id| id.as_str())),
                )? as u64;
                // A group with nothing left in it is not history any more.
                removed_groups = transaction.execute(
                    "DELETE FROM crash_group WHERE group_id NOT IN
                         (SELECT DISTINCT group_id FROM crash_record)",
                    [],
                )? as u64;
            }
        }

        transaction.commit()?;
        Ok((removed_records, removed_groups))
    }

    fn payload_paths_for(&self, target: &DeleteTarget) -> Result<Vec<String>, StoreError> {
        let connection = self.connection()?;
        let mut paths = Vec::new();
        match target {
            DeleteTarget::All => {
                let mut statement = connection.prepare(
                    "SELECT payload_path FROM crash_record WHERE payload_path IS NOT NULL",
                )?;
                let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
                for row in rows {
                    paths.push(row?);
                }
            }
            DeleteTarget::Group { group_id } => {
                let mut statement = connection.prepare(
                    "SELECT payload_path FROM crash_record
                     WHERE group_id = ?1 AND payload_path IS NOT NULL",
                )?;
                let rows = statement.query_map(params![group_id], |row| row.get::<_, String>(0))?;
                for row in rows {
                    paths.push(row?);
                }
            }
            DeleteTarget::Ids { ids } => {
                if ids.is_empty() {
                    return Ok(paths);
                }
                let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
                let mut statement = connection.prepare(&format!(
                    "SELECT payload_path FROM crash_record
                     WHERE id IN ({placeholders}) AND payload_path IS NOT NULL"
                ))?;
                let rows = statement
                    .query_map(params_from_iter(ids.iter().map(|id| id.as_str())), |row| {
                        row.get::<_, String>(0)
                    })?;
                for row in rows {
                    paths.push(row?);
                }
            }
        }
        Ok(paths)
    }

    /// Every payload path the index still references.
    pub(crate) fn referenced_payload_paths(&self) -> Result<HashSet<String>, StoreError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare("SELECT payload_path FROM crash_record WHERE payload_path IS NOT NULL")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut paths = HashSet::new();
        for row in rows {
            paths.insert(row?);
        }
        Ok(paths)
    }

    /// Occupancy, for the home screen and the storage settings.
    pub fn storage_status(&self) -> Result<StorageStatus, StoreError> {
        let payload_bytes = self.payloads.disk_bytes()?;
        let connection = self.connection()?;

        let group_count: i64 =
            connection.query_row("SELECT COUNT(*) FROM crash_group", [], |row| row.get(0))?;
        let record_count: i64 =
            connection.query_row("SELECT COUNT(*) FROM crash_record", [], |row| row.get(0))?;
        let evicted: i64 = connection.query_row(
            "SELECT COUNT(*) FROM crash_record WHERE payload_state = ?1",
            params![PayloadState::Evicted.as_i64()],
            |row| row.get(0),
        )?;

        let page_count: i64 =
            connection.pragma_query_value(None, "page_count", |row| row.get(0))?;
        let page_size: i64 = connection.pragma_query_value(None, "page_size", |row| row.get(0))?;

        Ok(StorageStatus {
            group_count: group_count.max(0) as u64,
            record_count: record_count.max(0) as u64,
            payload_bytes,
            database_bytes: (page_count * page_size).max(0) as u64,
            evicted_payload_count: evicted.max(0) as u64,
        })
    }

    /// Aggregate statistics over a time window.
    ///
    /// `installed_app_count` is left at zero: only the bridge can know it, so the
    /// daemon fills it in rather than the store guessing.
    pub fn stats(
        &self,
        time_from_ms: Option<i64>,
        time_to_ms: Option<i64>,
        bucket_ms: i64,
    ) -> Result<Stats, StoreError> {
        let from = time_from_ms.unwrap_or(i64::MIN);
        let to = time_to_ms.unwrap_or(i64::MAX);
        let connection = self.connection()?;

        let total: i64 = connection.query_row(
            "SELECT COUNT(*) FROM crash_record WHERE happened_at_ms >= ?1 AND happened_at_ms < ?2",
            params![from, to],
            |row| row.get(0),
        )?;

        let mut by_kind = Vec::new();
        {
            let mut statement = connection.prepare(
                "SELECT g.kind, COUNT(*) FROM crash_record r
                 JOIN crash_group g ON g.group_id = r.group_id
                 WHERE r.happened_at_ms >= ?1 AND r.happened_at_ms < ?2
                 GROUP BY g.kind ORDER BY COUNT(*) DESC",
            )?;
            let rows = statement.query_map(params![from, to], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            })?;
            for row in rows {
                let (kind, count) = row?;
                if let Some(kind) = cch_model::CrashKind::from_i64(kind) {
                    by_kind.push(KindCount {
                        kind,
                        count: count.max(0) as u64,
                    });
                }
            }
        }

        let mut top_packages = Vec::new();
        {
            let mut statement = connection.prepare(
                "SELECT g.package_name, COUNT(*) FROM crash_record r
                 JOIN crash_group g ON g.group_id = r.group_id
                 WHERE r.happened_at_ms >= ?1 AND r.happened_at_ms < ?2
                 GROUP BY g.package_name ORDER BY COUNT(*) DESC, g.package_name ASC LIMIT 10",
            )?;
            let rows = statement.query_map(params![from, to], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?;
            for row in rows {
                let (package_name, count) = row?;
                top_packages.push(cch_wire::PackageCount {
                    package_name,
                    label: None,
                    count: count.max(0) as u64,
                });
            }
        }

        let mut top_exceptions = Vec::new();
        {
            let mut statement = connection.prepare(
                "SELECT g.summary_class, COUNT(*) FROM crash_record r
                 JOIN crash_group g ON g.group_id = r.group_id
                 WHERE r.happened_at_ms >= ?1 AND r.happened_at_ms < ?2
                   AND g.summary_class IS NOT NULL
                 GROUP BY g.summary_class ORDER BY COUNT(*) DESC, g.summary_class ASC LIMIT 10",
            )?;
            let rows = statement.query_map(params![from, to], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?;
            for row in rows {
                let (class_name, count) = row?;
                top_exceptions.push(ExceptionCount {
                    class_name,
                    count: count.max(0) as u64,
                });
            }
        }

        let crashed_app_count: i64 = connection.query_row(
            "SELECT COUNT(DISTINCT g.package_name) FROM crash_record r
             JOIN crash_group g ON g.group_id = r.group_id
             WHERE r.happened_at_ms >= ?1 AND r.happened_at_ms < ?2",
            params![from, to],
            |row| row.get(0),
        )?;

        let trend = self.trend(&connection, from, to, bucket_ms)?;

        Ok(Stats {
            total: total.max(0) as u64,
            by_kind,
            top_packages,
            top_exceptions,
            trend,
            crashed_app_count: crashed_app_count.max(0) as u64,
            installed_app_count: 0,
        })
    }

    fn trend(
        &self,
        connection: &rusqlite::Connection,
        from: i64,
        to: i64,
        bucket_ms: i64,
    ) -> Result<Vec<TrendBucket>, StoreError> {
        // Resolve the window before bucketing: with no explicit range the sentinels
        // would produce a nonsensical bucket width.
        let (start, end): (Option<i64>, Option<i64>) = connection.query_row(
            "SELECT MIN(happened_at_ms), MAX(happened_at_ms) FROM crash_record
             WHERE happened_at_ms >= ?1 AND happened_at_ms < ?2",
            params![from, to],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let (Some(start), Some(end)) = (start, end) else {
            return Ok(Vec::new());
        };

        let span = (end - start).max(1);
        let width = if bucket_ms > 0 {
            bucket_ms
        } else {
            (span / DEFAULT_TREND_BUCKETS).max(1)
        };

        let mut statement = connection.prepare(
            "SELECT ((happened_at_ms - ?3) / ?4) AS bucket, COUNT(*)
             FROM crash_record
             WHERE happened_at_ms >= ?1 AND happened_at_ms < ?2
             GROUP BY bucket ORDER BY bucket ASC",
        )?;
        let rows = statement.query_map(params![from, to, start, width], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })?;

        let mut buckets = Vec::new();
        for row in rows {
            let (index, count) = row?;
            buckets.push(TrendBucket {
                from_ms: start + index * width,
                count: count.max(0) as u64,
            });
        }
        Ok(buckets)
    }

    /// Per-package crash aggregates for the apps screen.
    pub fn package_rollups(
        &self,
        include_system_apps: bool,
        limit: u32,
    ) -> Result<Vec<PackageRollup>, StoreError> {
        let effective_limit = if limit == 0 {
            i64::from(cch_wire::MAX_PAGE_LIMIT)
        } else {
            i64::from(limit.min(cch_wire::MAX_PAGE_LIMIT))
        };

        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT package_name, user_id, MAX(is_system_app), COUNT(*), SUM(occurrence),
                    MAX(last_seen_ms)
             FROM crash_group
             WHERE (?1 = 1 OR is_system_app = 0)
             GROUP BY package_name, user_id
             ORDER BY MAX(last_seen_ms) DESC
             LIMIT ?2",
        )?;
        let rows = statement.query_map(params![include_system_apps, effective_limit], |row| {
            Ok(PackageRollup {
                package_name: row.get(0)?,
                user_id: row.get(1)?,
                is_system_app: row.get(2)?,
                group_count: row.get::<_, i64>(3)?.max(0) as u64,
                occurrence: row.get::<_, i64>(4)?.max(0) as u64,
                last_seen_ms: row.get(5)?,
            })
        })?;

        let mut rollups = Vec::new();
        for row in rows {
            rollups.push(row?);
        }
        Ok(rollups)
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support::{
        TestStore, anr_record, fixture, java_record, native_record, record_for,
    };
    use cch_model::{CrashKind, PayloadSource};
    use cch_wire::{CrashFilter, DeleteTarget, PageRequest, SortKey};

    fn all_apps_filter() -> CrashFilter {
        CrashFilter {
            include_system_apps: true,
            ..CrashFilter::default()
        }
    }

    fn page(limit: u32) -> PageRequest {
        PageRequest {
            filter: all_apps_filter(),
            limit,
            ..PageRequest::default()
        }
    }

    #[test]
    fn an_empty_store_lists_nothing_and_offers_no_cursor() {
        let store = TestStore::new();
        let result = store.store.list_groups(&page(50)).expect("lists");
        assert!(result.items.is_empty());
        assert_eq!(result.next_cursor, None);
    }

    #[test]
    fn groups_come_back_newest_first() {
        let store = TestStore::new();
        store
            .insert_default(&record_for("com.a", 1_000))
            .expect("a");
        store
            .insert_default(&record_for("com.b", 3_000))
            .expect("b");
        store
            .insert_default(&record_for("com.c", 2_000))
            .expect("c");

        let names: Vec<String> = store
            .store
            .list_groups(&page(50))
            .expect("lists")
            .items
            .into_iter()
            .map(|group| group.package_name)
            .collect();
        assert_eq!(names, vec!["com.b", "com.c", "com.a"]);
    }

    #[test]
    fn keyset_pagination_walks_every_group_exactly_once() {
        let store = TestStore::new();
        for index in 0..25 {
            store
                .insert_default(&record_for(&format!("com.app{index:02}"), 1_000 + index))
                .expect("inserts");
        }

        let mut seen = Vec::new();
        let mut request = page(7);
        loop {
            let result = store.store.list_groups(&request).expect("lists");
            assert!(result.items.len() <= 7);
            seen.extend(result.items.iter().map(|group| group.group_id.clone()));
            match result.next_cursor {
                Some(cursor) => request.cursor = Some(cursor),
                None => break,
            }
        }

        assert_eq!(seen.len(), 25, "every group must appear");
        let unique: std::collections::HashSet<_> = seen.iter().collect();
        assert_eq!(unique.len(), 25, "no group may appear twice");
    }

    #[test]
    fn pagination_holds_up_when_many_groups_share_a_sort_value() {
        let store = TestStore::new();
        // Same timestamp for all: only the group-id tiebreak separates them.
        for index in 0..12 {
            store
                .insert_default(&record_for(&format!("com.same{index:02}"), 5_000))
                .expect("inserts");
        }

        let mut seen = Vec::new();
        let mut request = page(5);
        loop {
            let result = store.store.list_groups(&request).expect("lists");
            seen.extend(result.items.iter().map(|group| group.group_id.clone()));
            match result.next_cursor {
                Some(cursor) => request.cursor = Some(cursor),
                None => break,
            }
        }

        let unique: std::collections::HashSet<_> = seen.iter().collect();
        assert_eq!(unique.len(), 12, "the tiebreak must keep the walk total");
    }

    #[test]
    fn the_last_page_reports_no_cursor() {
        let store = TestStore::new();
        store.insert_default(&java_record(1_000)).expect("inserts");
        let result = store.store.list_groups(&page(50)).expect("lists");
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.next_cursor, None);
    }

    #[test]
    fn a_cursor_from_a_different_sort_order_is_refused() {
        let store = TestStore::new();
        store.insert_default(&java_record(1_000)).expect("inserts");
        store
            .insert_default(&record_for("com.b", 2_000))
            .expect("inserts");

        let first = store
            .store
            .list_groups(&PageRequest {
                filter: all_apps_filter(),
                sort: SortKey::LastSeenDesc,
                limit: 1,
                ..PageRequest::default()
            })
            .expect("lists");
        let cursor = first.next_cursor.expect("has a next page");

        let error = store
            .store
            .list_groups(&PageRequest {
                filter: all_apps_filter(),
                sort: SortKey::OccurrenceDesc,
                cursor: Some(cursor),
                limit: 1,
            })
            .map(|_| ())
            .unwrap_err();
        assert_eq!(error.to_wire().code, cch_wire::ErrorCode::CursorInvalidated);
    }

    #[test]
    fn filters_narrow_by_package_kind_and_time() {
        let store = TestStore::new();
        store.insert_default(&java_record(1_000)).expect("java");
        store.insert_default(&native_record(2_000)).expect("native");
        store.insert_default(&anr_record(3_000)).expect("anr");
        store
            .insert_default(&record_for("com.other", 4_000))
            .expect("other");

        let by_package = store
            .store
            .list_groups(&PageRequest {
                filter: CrashFilter {
                    packages: vec!["com.other".into()],
                    include_system_apps: true,
                    ..CrashFilter::default()
                },
                limit: 50,
                ..PageRequest::default()
            })
            .expect("lists");
        assert_eq!(by_package.items.len(), 1);

        let by_kind = store
            .store
            .list_groups(&PageRequest {
                filter: CrashFilter {
                    kinds: vec![CrashKind::Anr],
                    include_system_apps: true,
                    ..CrashFilter::default()
                },
                limit: 50,
                ..PageRequest::default()
            })
            .expect("lists");
        assert_eq!(by_kind.items.len(), 1);
        assert_eq!(by_kind.items[0].kind, CrashKind::Anr);

        let by_time = store
            .store
            .list_groups(&PageRequest {
                filter: CrashFilter {
                    time_from_ms: Some(2_500),
                    include_system_apps: true,
                    ..CrashFilter::default()
                },
                limit: 50,
                ..PageRequest::default()
            })
            .expect("lists");
        assert_eq!(by_time.items.len(), 2);
    }

    #[test]
    fn system_apps_are_hidden_unless_asked_for() {
        let store = TestStore::new();
        let mut system = java_record(1_000);
        system.is_system_app = true;
        system.package_name = "com.android.systemui".to_owned();
        system.process_name = "com.android.systemui".to_owned();
        store.insert_default(&system).expect("inserts");

        let hidden = store
            .store
            .list_groups(&PageRequest {
                limit: 50,
                ..PageRequest::default()
            })
            .expect("lists");
        assert!(hidden.items.is_empty());

        let shown = store.store.list_groups(&page(50)).expect("lists");
        assert_eq!(shown.items.len(), 1);
    }

    #[test]
    fn a_text_query_matches_package_and_summary() {
        let store = TestStore::new();
        store.insert_default(&java_record(1_000)).expect("inserts");

        for query in ["example", "IllegalState", "Fragment already"] {
            let result = store
                .store
                .list_groups(&PageRequest {
                    filter: CrashFilter {
                        query: Some(query.into()),
                        include_system_apps: true,
                        ..CrashFilter::default()
                    },
                    limit: 50,
                    ..PageRequest::default()
                })
                .expect("lists");
            assert_eq!(result.items.len(), 1, "query {query:?} should match");
        }

        let miss = store
            .store
            .list_groups(&PageRequest {
                filter: CrashFilter {
                    query: Some("definitely-not-present".into()),
                    include_system_apps: true,
                    ..CrashFilter::default()
                },
                limit: 50,
                ..PageRequest::default()
            })
            .expect("lists");
        assert!(miss.items.is_empty());
    }

    #[test]
    fn only_self_handled_isolates_the_crashes_nothing_else_sees() {
        let store = TestStore::new();
        store.insert_default(&java_record(1_000)).expect("ordinary");

        let mut swallowed = java_record(2_000);
        swallowed.self_handled = true;
        swallowed.fingerprint = fixture::other_fingerprint();
        store.insert_default(&swallowed).expect("swallowed");

        let result = store
            .store
            .list_groups(&PageRequest {
                filter: CrashFilter {
                    only_self_handled: true,
                    include_system_apps: true,
                    ..CrashFilter::default()
                },
                limit: 50,
                ..PageRequest::default()
            })
            .expect("lists");
        assert_eq!(result.items.len(), 1);
        assert!(result.items[0].self_handled);
    }

    #[test]
    fn records_of_a_group_come_back_newest_first_and_paginate() {
        let store = TestStore::new();
        for index in 0..9 {
            store
                .insert_default(&java_record(1_000 + index))
                .expect("inserts");
        }
        let group_id = store.all_groups()[0].group_id.clone();

        let mut seen = Vec::new();
        let mut request = PageRequest {
            limit: 4,
            ..PageRequest::default()
        };
        loop {
            let result = store
                .store
                .list_records(&group_id, &request)
                .expect("lists records");
            seen.extend(result.items.iter().map(|record| record.happened_at_ms));
            match result.next_cursor {
                Some(cursor) => request.cursor = Some(cursor),
                None => break,
            }
        }

        assert_eq!(seen.len(), 9);
        let mut sorted = seen.clone();
        sorted.sort_unstable_by(|a, b| b.cmp(a));
        assert_eq!(seen, sorted, "records must be newest first");
    }

    #[test]
    fn a_group_can_be_fetched_by_id() {
        let store = TestStore::new();
        let inserted = store.insert_default(&java_record(1_000)).expect("inserts");

        let group = store
            .store
            .get_group(&inserted.group.group_id)
            .expect("reads");
        assert_eq!(group, inserted.group);
    }

    #[test]
    fn fetching_an_unknown_group_says_not_found() {
        let store = TestStore::new();
        let error = store.store.get_group("nope").map(|_| ()).unwrap_err();
        assert_eq!(error.to_wire().code, cch_wire::ErrorCode::NotFound);
    }

    #[test]
    fn a_record_detail_carries_its_group() {
        let store = TestStore::new();
        let inserted = store.insert_default(&java_record(1_000)).expect("inserts");

        let detail = store.store.get_record(&inserted.record.id).expect("reads");
        assert_eq!(detail.record.id, inserted.record.id);
        assert_eq!(detail.group.group_id, inserted.group.group_id);
    }

    #[test]
    fn asking_for_an_unknown_record_says_not_found() {
        let store = TestStore::new();
        let id = cch_model::RecordIdGenerator::new().next(1);
        let error = store.store.get_record(&id).map(|_| ()).unwrap_err();
        assert_eq!(error.to_wire().code, cch_wire::ErrorCode::NotFound);
    }

    #[test]
    fn payload_text_round_trips_through_the_store() {
        let store = TestStore::new();
        let inserted = store.insert_default(&java_record(1_000)).expect("inserts");
        let text = store
            .store
            .read_payload_text(&inserted.record.id)
            .expect("reads payload");
        assert!(text.contains("IllegalStateException"));
    }

    #[test]
    fn a_record_with_no_payload_reports_it_rather_than_erroring_obscurely() {
        let store = TestStore::new();
        let mut record = java_record(1_000);
        record.payload = PayloadSource::None;
        let inserted = store.insert_default(&record).expect("inserts");

        let error = store
            .store
            .read_payload_text(&inserted.record.id)
            .map(|_| ())
            .unwrap_err();
        assert_eq!(error.to_wire().code, cch_wire::ErrorCode::NotFound);
    }

    #[test]
    fn deleting_a_group_removes_its_records_and_payload_files() {
        let store = TestStore::new();
        store.insert_default(&java_record(1_000)).expect("first");
        store.insert_default(&java_record(2_000)).expect("second");
        let group_id = store.all_groups()[0].group_id.clone();

        let (records, groups) = store
            .store
            .delete(&DeleteTarget::Group {
                group_id: group_id.clone(),
            })
            .expect("deletes");
        assert_eq!(records, 2);
        assert_eq!(groups, 1);
        assert!(store.all_groups().is_empty());
        assert_eq!(store.store.payloads().disk_bytes().expect("measures"), 0);
    }

    #[test]
    fn deleting_the_last_record_of_a_group_removes_the_group_too() {
        let store = TestStore::new();
        let inserted = store.insert_default(&java_record(1_000)).expect("inserts");

        let (records, groups) = store
            .store
            .delete(&DeleteTarget::Ids {
                ids: vec![inserted.record.id],
            })
            .expect("deletes");
        assert_eq!(records, 1);
        assert_eq!(groups, 1);
        assert!(store.all_groups().is_empty());
    }

    #[test]
    fn deleting_one_of_several_records_keeps_the_group() {
        let store = TestStore::new();
        let first = store.insert_default(&java_record(1_000)).expect("first");
        store.insert_default(&java_record(2_000)).expect("second");

        let (records, groups) = store
            .store
            .delete(&DeleteTarget::Ids {
                ids: vec![first.record.id],
            })
            .expect("deletes");
        assert_eq!(records, 1);
        assert_eq!(groups, 0);
        assert_eq!(store.all_groups().len(), 1);
    }

    #[test]
    fn deleting_an_empty_id_list_is_a_no_op_not_a_purge() {
        let store = TestStore::new();
        store.insert_default(&java_record(1_000)).expect("inserts");

        let (records, groups) = store
            .store
            .delete(&DeleteTarget::Ids { ids: Vec::new() })
            .expect("deletes");
        assert_eq!((records, groups), (0, 0));
        assert_eq!(
            store.all_groups().len(),
            1,
            "an empty list must not mean 'all'"
        );
    }

    #[test]
    fn deleting_everything_clears_both_tables_and_the_payload_directory() {
        let store = TestStore::new();
        store.insert_default(&java_record(1_000)).expect("first");
        store
            .insert_default(&record_for("com.other", 2_000))
            .expect("second");

        let (records, groups) = store.store.delete(&DeleteTarget::All).expect("deletes");
        assert_eq!(records, 2);
        assert_eq!(groups, 2);
        assert_eq!(store.store.payloads().disk_bytes().expect("measures"), 0);
    }

    #[test]
    fn storage_status_counts_what_is_stored() {
        let store = TestStore::new();
        store.insert_default(&java_record(1_000)).expect("first");
        store
            .insert_default(&record_for("com.other", 2_000))
            .expect("second");

        let status = store.store.storage_status().expect("status");
        assert_eq!(status.group_count, 2);
        assert_eq!(status.record_count, 2);
        assert!(status.payload_bytes > 0);
        assert!(status.database_bytes > 0);
        assert_eq!(status.evicted_payload_count, 0);
    }

    #[test]
    fn stats_break_crashes_down_by_kind_and_package() {
        let store = TestStore::new();
        store.insert_default(&java_record(1_000)).expect("java");
        store
            .insert_default(&java_record(1_100))
            .expect("java again");
        store.insert_default(&native_record(2_000)).expect("native");
        store
            .insert_default(&record_for("com.other", 3_000))
            .expect("other");

        let stats = store.store.stats(None, None, 0).expect("stats");
        assert_eq!(stats.total, 4);
        assert_eq!(stats.crashed_app_count, 2);

        let java_count = stats
            .by_kind
            .iter()
            .find(|entry| entry.kind == CrashKind::JavaException)
            .map(|entry| entry.count);
        assert_eq!(java_count, Some(3));

        let top = stats.top_packages.first().expect("has a top package");
        assert_eq!(top.package_name, "com.example.app");
        assert_eq!(top.count, 3);

        assert!(!stats.trend.is_empty());
        assert_eq!(
            stats.installed_app_count, 0,
            "the store cannot know this; the daemon fills it in"
        );
    }

    #[test]
    fn stats_over_an_empty_window_are_zero_rather_than_an_error() {
        let store = TestStore::new();
        store.insert_default(&java_record(1_000)).expect("inserts");

        let stats = store
            .store
            .stats(Some(50_000), Some(60_000), 0)
            .expect("stats");
        assert_eq!(stats.total, 0);
        assert!(stats.trend.is_empty());
    }

    #[test]
    fn package_rollups_aggregate_across_groups() {
        let store = TestStore::new();
        store.insert_default(&java_record(1_000)).expect("first");
        store.insert_default(&java_record(1_500)).expect("repeat");

        let mut other_fingerprint = java_record(2_000);
        other_fingerprint.fingerprint = fixture::other_fingerprint();
        store
            .insert_default(&other_fingerprint)
            .expect("second group");

        let rollups = store.store.package_rollups(true, 0).expect("rolls up");
        let entry = rollups
            .iter()
            .find(|entry| entry.package_name == "com.example.app")
            .expect("has the package");
        assert_eq!(entry.group_count, 2);
        assert_eq!(entry.occurrence, 3);
        assert_eq!(entry.last_seen_ms, 2_000);
    }

    #[test]
    fn package_rollups_respect_the_system_app_toggle() {
        let store = TestStore::new();
        let mut system = java_record(1_000);
        system.is_system_app = true;
        system.package_name = "com.android.systemui".to_owned();
        system.process_name = "com.android.systemui".to_owned();
        store.insert_default(&system).expect("inserts");

        assert!(
            store
                .store
                .package_rollups(false, 0)
                .expect("rolls up")
                .is_empty()
        );
        assert_eq!(
            store
                .store
                .package_rollups(true, 0)
                .expect("rolls up")
                .len(),
            1
        );
    }
}
