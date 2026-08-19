use cch_model::{CrashKind, PayloadCodec, PayloadState, RecordId, SourceMask};
use cch_wire::{
    CrashFilter, Cursor, CursorAnchor, GroupSummary, RecordSummary, SortKey, WireError,
};
use rusqlite::{Row, types::Value};

/// A SQL fragment together with the values to bind to it.
///
/// Only column names and operators are ever concatenated into `sql`; every value
/// the caller supplied travels as a bound parameter.
#[derive(Debug, Default)]
pub(crate) struct Predicates {
    clauses: Vec<String>,
    params: Vec<Value>,
}

impl Predicates {
    pub(crate) fn push(
        &mut self,
        clause: impl Into<String>,
        values: impl IntoIterator<Item = Value>,
    ) {
        self.clauses.push(clause.into());
        self.params.extend(values);
    }

    /// Renders as a `WHERE` clause, or an empty string when unrestricted.
    pub(crate) fn where_clause(&self) -> String {
        if self.clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", self.clauses.join(" AND "))
        }
    }

    pub(crate) fn params(self) -> Vec<Value> {
        self.params
    }

    pub(crate) fn extend(&mut self, other: Self) {
        self.clauses.extend(other.clauses);
        self.params.extend(other.params);
    }
}

/// Builds the `crash_group` predicates for a filter.
///
/// The time range is interpreted as *interval overlap*: a group matches when its
/// `[first_seen, last_seen]` span intersects the requested window. Testing the
/// group's own columns keeps this index-driven; the exact alternative
/// (`EXISTS (SELECT 1 FROM crash_record …)`) would add a correlated subquery per
/// row for a distinction the UI's own ranges — last day, last week, last month —
/// never surfaces.
pub(crate) fn group_filter(filter: &CrashFilter) -> Predicates {
    let mut predicates = Predicates::default();

    if !filter.packages.is_empty() {
        predicates.push(
            format!("package_name IN ({})", placeholders(filter.packages.len())),
            filter
                .packages
                .iter()
                .map(|package| Value::Text(package.clone())),
        );
    }

    if !filter.kinds.is_empty() {
        predicates.push(
            format!("kind IN ({})", placeholders(filter.kinds.len())),
            filter
                .kinds
                .iter()
                .map(|kind| Value::Integer(kind.as_i64())),
        );
    }

    if !filter.user_ids.is_empty() {
        predicates.push(
            format!("user_id IN ({})", placeholders(filter.user_ids.len())),
            filter
                .user_ids
                .iter()
                .map(|user| Value::Integer(i64::from(*user))),
        );
    }

    if let Some(from) = filter.time_from_ms {
        predicates.push("last_seen_ms >= ?", [Value::Integer(from)]);
    }
    if let Some(to) = filter.time_to_ms {
        predicates.push("first_seen_ms < ?", [Value::Integer(to)]);
    }

    if !filter.include_system_apps {
        predicates.push("is_system_app = 0", []);
    }
    if filter.only_main_process {
        predicates.push("is_main_process = 1", []);
    }
    if filter.only_self_handled {
        predicates.push("self_handled = 1", []);
    }

    if let Some(query) = filter
        .query
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty())
    {
        let pattern = like_pattern(query);
        predicates.push(
            r"(package_name LIKE ? ESCAPE '\'
               OR COALESCE(summary_class, '') LIKE ? ESCAPE '\'
               OR COALESCE(summary_text, '') LIKE ? ESCAPE '\')",
            [
                Value::Text(pattern.clone()),
                Value::Text(pattern.clone()),
                Value::Text(pattern),
            ],
        );
    }

    predicates
}

/// Builds the keyset predicate that resumes after `cursor`.
///
/// Anchoring on `(sort column, group_id)` rather than using `OFFSET`: offset
/// re-walks every skipped row, so page N costs N times page 1. This costs the same
/// for every page.
pub(crate) fn cursor_predicate(cursor: &Cursor, sort: SortKey) -> Result<Predicates, WireError> {
    cursor.validate_for(sort)?;

    let column = group_sort_column(sort);
    let (comparison, tiebreak_comparison) = if sort.is_descending() {
        ("<", "<")
    } else {
        (">", ">")
    };

    let anchor = match &cursor.anchor {
        CursorAnchor::Int(value) => Value::Integer(*value),
        CursorAnchor::Text(value) => Value::Text(value.clone()),
    };

    let mut predicates = Predicates::default();
    predicates.push(
        format!("({column} {comparison} ? OR ({column} = ? AND group_id {tiebreak_comparison} ?))"),
        [anchor.clone(), anchor, Value::Text(cursor.tiebreak.clone())],
    );
    Ok(predicates)
}

pub(crate) fn group_sort_column(sort: SortKey) -> &'static str {
    match sort {
        SortKey::LastSeenDesc => "last_seen_ms",
        SortKey::FirstSeenDesc => "first_seen_ms",
        SortKey::OccurrenceDesc => "occurrence",
        SortKey::PackageAsc => "package_name",
    }
}

pub(crate) fn group_order_by(sort: SortKey) -> String {
    let column = group_sort_column(sort);
    let direction = if sort.is_descending() { "DESC" } else { "ASC" };
    format!(" ORDER BY {column} {direction}, group_id {direction}")
}

/// Builds the cursor that resumes after `group`.
pub(crate) fn cursor_after(group: &GroupSummary, sort: SortKey) -> Cursor {
    let anchor = match sort {
        SortKey::LastSeenDesc => CursorAnchor::Int(group.last_seen_ms),
        SortKey::FirstSeenDesc => CursorAnchor::Int(group.first_seen_ms),
        SortKey::OccurrenceDesc => CursorAnchor::Int(group.occurrence as i64),
        SortKey::PackageAsc => CursorAnchor::Text(group.package_name.clone()),
    };
    Cursor::new(sort, anchor, group.group_id.clone())
}

/// Escapes the LIKE metacharacters so a user typing `%` searches for `%`.
fn like_pattern(query: &str) -> String {
    let mut escaped = String::with_capacity(query.len() + 2);
    escaped.push('%');
    for character in query.chars() {
        if matches!(character, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped.push('%');
    escaped
}

fn placeholders(count: usize) -> String {
    let mut out = String::with_capacity(count * 2);
    for index in 0..count {
        if index > 0 {
            out.push(',');
        }
        out.push('?');
    }
    out
}

/// Columns selected for a [`GroupSummary`], in the order [`map_group`] reads them.
pub(crate) const GROUP_COLUMNS: &str = "group_id, package_name, process_name, user_id, kind, \
     is_system_app, is_main_process, self_handled, summary_class, summary_text, \
     occurrence, first_seen_ms, last_seen_ms, payload_bytes, muted_until_ms";

pub(crate) fn map_group(row: &Row<'_>) -> rusqlite::Result<GroupSummary> {
    let kind_value: i64 = row.get(4)?;
    Ok(GroupSummary {
        group_id: row.get(0)?,
        package_name: row.get(1)?,
        process_name: row.get(2)?,
        user_id: row.get(3)?,
        kind: CrashKind::from_i64(kind_value).unwrap_or(CrashKind::JavaException),
        is_system_app: row.get(5)?,
        is_main_process: row.get(6)?,
        self_handled: row.get(7)?,
        summary_class: row.get(8)?,
        summary_text: row.get(9)?,
        occurrence: row.get::<_, i64>(10)?.max(0) as u64,
        first_seen_ms: row.get(11)?,
        last_seen_ms: row.get(12)?,
        payload_bytes: row.get::<_, i64>(13)?.max(0) as u64,
        muted_until_ms: row.get(14)?,
    })
}

/// Columns selected for a [`RecordSummary`], in the order [`map_record`] reads them.
pub(crate) const RECORD_COLUMNS: &str = "id, group_id, happened_at_ms, pid, sources, \
     app_version_name, app_version_code, is_foreground, is_repeating, dropped_count, \
     payload_bytes, payload_state";

pub(crate) fn map_record(row: &Row<'_>) -> rusqlite::Result<RecordSummary> {
    let id_text: String = row.get(0)?;
    let sources: i64 = row.get(4)?;
    let payload_state: i64 = row.get(11)?;
    Ok(RecordSummary {
        // A row whose id does not parse would be a schema bug; fall back rather
        // than failing the whole page, so one bad row cannot hide the rest.
        id: RecordId::parse(&id_text).unwrap_or_else(fallback_id),
        group_id: row.get(1)?,
        happened_at_ms: row.get(2)?,
        pid: row.get(3)?,
        sources: SourceMask::from_bits_truncate(u32::try_from(sources).unwrap_or(0)),
        app_version_name: row.get(5)?,
        app_version_code: row.get(6)?,
        is_foreground: row.get(7)?,
        is_repeating: row.get(8)?,
        dropped_count: row
            .get::<_, Option<i64>>(9)?
            .map(|count| u32::try_from(count).unwrap_or(u32::MAX)),
        payload_bytes: row.get::<_, i64>(10)?.max(0) as u64,
        payload_state: PayloadState::from_i64(payload_state).unwrap_or(PayloadState::Evicted),
    })
}

fn fallback_id() -> RecordId {
    cch_model::RecordIdGenerator::new().next(0)
}

pub(crate) fn codec_from_i64(value: i64) -> PayloadCodec {
    PayloadCodec::from_i64(value).unwrap_or(PayloadCodec::Zstd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cch_wire::ErrorCode;

    #[test]
    fn an_unrestricted_filter_produces_no_where_clause_beyond_system_apps() {
        let filter = CrashFilter {
            include_system_apps: true,
            ..CrashFilter::default()
        };
        assert_eq!(group_filter(&filter).where_clause(), "");
    }

    #[test]
    fn system_apps_are_excluded_by_default() {
        let predicates = group_filter(&CrashFilter::default());
        assert_eq!(predicates.where_clause(), " WHERE is_system_app = 0");
    }

    #[test]
    fn list_filters_bind_one_placeholder_per_value() {
        let filter = CrashFilter {
            packages: vec!["a".into(), "b".into(), "c".into()],
            include_system_apps: true,
            ..CrashFilter::default()
        };
        let predicates = group_filter(&filter);
        assert!(
            predicates
                .where_clause()
                .contains("package_name IN (?,?,?)")
        );
        assert_eq!(predicates.params().len(), 3);
    }

    #[test]
    fn a_time_range_becomes_an_overlap_test() {
        let filter = CrashFilter {
            time_from_ms: Some(100),
            time_to_ms: Some(200),
            include_system_apps: true,
            ..CrashFilter::default()
        };
        let clause = group_filter(&filter).where_clause();
        assert!(clause.contains("last_seen_ms >= ?"));
        assert!(clause.contains("first_seen_ms < ?"));
    }

    #[test]
    fn like_metacharacters_are_escaped() {
        assert_eq!(like_pattern("100%"), r"%100\%%");
        assert_eq!(like_pattern("a_b"), r"%a\_b%");
        assert_eq!(like_pattern(r"back\slash"), r"%back\\slash%");
        assert_eq!(like_pattern("plain"), "%plain%");
    }

    #[test]
    fn a_blank_query_is_not_a_filter() {
        let filter = CrashFilter {
            query: Some("   ".into()),
            include_system_apps: true,
            ..CrashFilter::default()
        };
        assert_eq!(group_filter(&filter).where_clause(), "");
    }

    #[test]
    fn a_query_searches_package_class_and_text() {
        let filter = CrashFilter {
            query: Some("boom".into()),
            include_system_apps: true,
            ..CrashFilter::default()
        };
        let predicates = group_filter(&filter);
        let clause = predicates.where_clause();
        assert!(clause.contains("package_name LIKE"));
        assert!(clause.contains("summary_class"));
        assert!(clause.contains("summary_text"));
        assert_eq!(predicates.params().len(), 3);
    }

    #[test]
    fn descending_and_ascending_sorts_get_matching_comparisons() {
        let descending = cursor_predicate(
            &Cursor::new(SortKey::LastSeenDesc, CursorAnchor::Int(10), "g"),
            SortKey::LastSeenDesc,
        )
        .expect("builds");
        assert!(descending.where_clause().contains("last_seen_ms < ?"));
        assert!(descending.where_clause().contains("group_id < ?"));

        let ascending = cursor_predicate(
            &Cursor::new(SortKey::PackageAsc, CursorAnchor::Text("com.a".into()), "g"),
            SortKey::PackageAsc,
        )
        .expect("builds");
        assert!(ascending.where_clause().contains("package_name > ?"));
        assert!(ascending.where_clause().contains("group_id > ?"));
    }

    #[test]
    fn a_mismatched_cursor_is_rejected_before_any_sql_is_built() {
        let error = cursor_predicate(
            &Cursor::new(SortKey::LastSeenDesc, CursorAnchor::Int(10), "g"),
            SortKey::PackageAsc,
        )
        .map(|_| ())
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::CursorInvalidated);
    }

    #[test]
    fn order_by_matches_the_index_definitions() {
        assert_eq!(
            group_order_by(SortKey::LastSeenDesc),
            " ORDER BY last_seen_ms DESC, group_id DESC"
        );
        assert_eq!(
            group_order_by(SortKey::PackageAsc),
            " ORDER BY package_name ASC, group_id ASC"
        );
    }

    #[test]
    fn the_cursor_for_a_group_anchors_on_the_sorted_column() {
        let group = GroupSummary {
            group_id: "g1".into(),
            package_name: "com.example.app".into(),
            process_name: "com.example.app".into(),
            user_id: 0,
            kind: CrashKind::JavaException,
            is_system_app: false,
            is_main_process: true,
            self_handled: false,
            summary_class: None,
            summary_text: None,
            occurrence: 7,
            first_seen_ms: 100,
            last_seen_ms: 900,
            payload_bytes: 0,
            muted_until_ms: None,
        };

        assert_eq!(
            cursor_after(&group, SortKey::LastSeenDesc).anchor,
            CursorAnchor::Int(900)
        );
        assert_eq!(
            cursor_after(&group, SortKey::OccurrenceDesc).anchor,
            CursorAnchor::Int(7)
        );
        assert_eq!(
            cursor_after(&group, SortKey::PackageAsc).anchor,
            CursorAnchor::Text("com.example.app".into())
        );
    }
}
