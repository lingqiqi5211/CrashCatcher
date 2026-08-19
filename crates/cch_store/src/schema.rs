use rusqlite::Connection;

use crate::StoreError;

/// Schema version this build expects.
///
/// 2 added `crash_group.package_installed`.
pub const SCHEMA_VERSION: i64 = 2;

/// Table and index definitions.
///
/// Every column the filter vocabulary can restrict on has an index. Without that
/// a "show me only ANRs from this app" query degrades into a full scan, which is
/// the behaviour this design exists to avoid.
///
/// `crash_group` is deliberately self-sufficient: the list screen reads it with no
/// join and no payload access, so opening the list costs one index range scan
/// regardless of how much history is stored.
const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS crash_group (
  group_id        TEXT    PRIMARY KEY,
  package_name    TEXT    NOT NULL,
  process_name    TEXT    NOT NULL,
  user_id         INTEGER NOT NULL,
  kind            INTEGER NOT NULL,
  is_system_app   INTEGER NOT NULL,
  package_installed INTEGER NOT NULL DEFAULT 1,
  is_main_process INTEGER NOT NULL,
  self_handled    INTEGER NOT NULL,
  summary_class   TEXT,
  summary_text    TEXT,
  occurrence      INTEGER NOT NULL,
  first_seen_ms   INTEGER NOT NULL,
  last_seen_ms    INTEGER NOT NULL,
  payload_bytes   INTEGER NOT NULL DEFAULT 0,
  muted_until_ms  INTEGER
);

CREATE INDEX IF NOT EXISTS idx_group_last_seen
  ON crash_group(last_seen_ms DESC, group_id DESC);
CREATE INDEX IF NOT EXISTS idx_group_first_seen
  ON crash_group(first_seen_ms DESC, group_id DESC);
CREATE INDEX IF NOT EXISTS idx_group_occurrence
  ON crash_group(occurrence DESC, group_id DESC);
CREATE INDEX IF NOT EXISTS idx_group_package
  ON crash_group(package_name ASC, group_id ASC);
CREATE INDEX IF NOT EXISTS idx_group_kind
  ON crash_group(kind, last_seen_ms DESC);

CREATE TABLE IF NOT EXISTS crash_record (
  id               TEXT    PRIMARY KEY,
  group_id         TEXT    NOT NULL REFERENCES crash_group(group_id) ON DELETE CASCADE,
  happened_at_ms   INTEGER NOT NULL,
  pid              INTEGER NOT NULL,
  sources          INTEGER NOT NULL,
  app_version_name TEXT,
  app_version_code INTEGER,
  is_foreground    INTEGER,
  is_repeating     INTEGER NOT NULL,
  dropped_count    INTEGER,
  payload_path     TEXT,
  payload_bytes    INTEGER NOT NULL DEFAULT 0,
  payload_codec    INTEGER NOT NULL DEFAULT 0,
  payload_state    INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_record_group
  ON crash_record(group_id, happened_at_ms DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_record_time
  ON crash_record(happened_at_ms ASC);
-- Drives byte-quota eviction: oldest readable payload first.
CREATE INDEX IF NOT EXISTS idx_record_payload_state
  ON crash_record(payload_state, happened_at_ms ASC);

CREATE TABLE IF NOT EXISTS ingested_source (
  source_key     TEXT    PRIMARY KEY,
  ingested_at_ms INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_ingested_at
  ON ingested_source(ingested_at_ms ASC);
"#;

/// Applies pragmas, creates the schema, and records the version.
pub fn initialize(connection: &Connection) -> Result<(), StoreError> {
    // WAL lets the manager read a page while a collector is still writing.
    // NORMAL rather than FULL: FULL fsyncs on every commit, and during a crash
    // storm the ingest path commits constantly. WAL + NORMAL can lose the last
    // transaction on power loss, which for crash history is an acceptable trade —
    // the source directories are still there to re-ingest from.
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    connection.pragma_update(None, "foreign_keys", true)?;
    connection.busy_timeout(std::time::Duration::from_secs(5))?;

    // Read before creating anything: `CREATE TABLE IF NOT EXISTS` leaves an existing table
    // exactly as it was, so this is the only point where a fresh database and an old one can
    // still be told apart.
    let found: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if found > SCHEMA_VERSION {
        return Err(StoreError::SchemaTooNew {
            found,
            supported: SCHEMA_VERSION,
        });
    }

    connection.execute_batch(SCHEMA_SQL)?;
    if found > 0 && found < SCHEMA_VERSION {
        migrate(connection)?;
    }
    if found != SCHEMA_VERSION {
        connection.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }

    Ok(())
}

/// Brings an existing database up to [`SCHEMA_VERSION`].
///
/// Written against the columns rather than against the version numbers: the added columns all
/// have defaults, so the work is the same whichever old version is on disk, and doing it this
/// way means a database that was created before versions were tracked properly still converges.
///
/// Deliberately does not revisit how existing rows were classified. The origin of a crash is a
/// fact about the device rather than about the row, so stored ones can be wrong — but rewriting
/// history would move records a user has already seen, and the filters only need to be right
/// from here on.
fn migrate(connection: &Connection) -> Result<(), StoreError> {
    if !has_column(connection, "crash_group", "package_installed")? {
        // Existing rows predate the distinction; 1 keeps them exactly as they were.
        connection.execute_batch(
            "ALTER TABLE crash_group ADD COLUMN package_installed INTEGER NOT NULL DEFAULT 1",
        )?;
    }
    Ok(())
}

fn has_column(connection: &Connection, table: &str, column: &str) -> Result<bool, StoreError> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        if row.get::<_, String>(1)? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory() -> Connection {
        Connection::open_in_memory().expect("in-memory database")
    }

    #[test]
    fn initialize_is_idempotent() {
        let connection = memory();
        initialize(&connection).expect("first run");
        initialize(&connection).expect("second run must not fail");
    }

    #[test]
    fn schema_version_is_recorded() {
        let connection = memory();
        initialize(&connection).expect("initializes");
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("reads user_version");
        assert_eq!(version, SCHEMA_VERSION);
    }

    /// The v1 table, without `package_installed`, as shipped before the platform-process split.
    const V1_CRASH_GROUP: &str = "
        CREATE TABLE crash_group (
          group_id TEXT PRIMARY KEY, package_name TEXT NOT NULL, process_name TEXT NOT NULL,
          user_id INTEGER NOT NULL, kind INTEGER NOT NULL, is_system_app INTEGER NOT NULL,
          is_main_process INTEGER NOT NULL, self_handled INTEGER NOT NULL, summary_class TEXT,
          summary_text TEXT, occurrence INTEGER NOT NULL, first_seen_ms INTEGER NOT NULL,
          last_seen_ms INTEGER NOT NULL, payload_bytes INTEGER NOT NULL DEFAULT 0,
          muted_until_ms INTEGER)";

    #[test]
    fn a_v1_database_gains_the_column_with_its_rows_intact() {
        let connection = memory();
        connection.execute_batch(V1_CRASH_GROUP).expect("v1 table");
        connection
            .execute(
                "INSERT INTO crash_group (group_id, package_name, process_name, user_id, kind,
                     is_system_app, is_main_process, self_handled, occurrence,
                     first_seen_ms, last_seen_ms)
                 VALUES ('g1', 'com.example.app', 'com.example.app', 0, 0, 0, 1, 0, 7, 10, 20)",
                [],
            )
            .expect("v1 row");
        connection
            .pragma_update(None, "user_version", 1)
            .expect("marks v1");

        initialize(&connection).expect("migrates");

        let (installed, occurrence): (i64, i64) = connection
            .query_row(
                "SELECT package_installed, occurrence FROM crash_group WHERE group_id = 'g1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("reads the migrated row");
        assert_eq!(installed, 1, "existing rows must stay visible");
        assert_eq!(occurrence, 7, "migration must not touch the history");
    }

    /// Even the rows that prompted the column — an audio HAL binary filed as an app — are left
    /// as they were found. The filters apply from here on; the history stays what the user saw.
    #[test]
    fn migration_leaves_existing_classifications_alone() {
        let connection = memory();
        connection.execute_batch(V1_CRASH_GROUP).expect("v1 table");
        connection
            .execute(
                "INSERT INTO crash_group (group_id, package_name, process_name, user_id, kind,
                     is_system_app, is_main_process, self_handled, occurrence,
                     first_seen_ms, last_seen_ms)
                 VALUES ('hal', '/vendor/bin/hw/android.hardware.audio.service_64',
                         '/vendor/bin/hw/android.hardware.audio.service_64', 0, 0, 0, 1, 0,
                         108, 10, 20)",
                [],
            )
            .expect("v1 row");
        connection
            .pragma_update(None, "user_version", 1)
            .expect("marks v1");

        initialize(&connection).expect("migrates");

        let (installed, system): (i64, i64) = connection
            .query_row(
                "SELECT package_installed, is_system_app FROM crash_group WHERE group_id = 'hal'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("reads");
        assert_eq!(installed, 1, "the new column defaults, nothing more");
        assert_eq!(system, 0, "and the old one is not rewritten");
    }

    #[test]
    fn migrating_twice_is_harmless() {
        let connection = memory();
        connection.execute_batch(V1_CRASH_GROUP).expect("v1 table");
        connection
            .pragma_update(None, "user_version", 1)
            .expect("marks v1");
        initialize(&connection).expect("first migration");
        initialize(&connection).expect("second run must not re-apply the ALTER");
    }

    #[test]
    fn a_future_schema_is_refused_rather_than_corrupted() {
        let connection = memory();
        initialize(&connection).expect("initializes");
        connection
            .pragma_update(None, "user_version", SCHEMA_VERSION + 5)
            .expect("bumps version");

        match initialize(&connection) {
            Err(StoreError::SchemaTooNew { found, supported }) => {
                assert_eq!(found, SCHEMA_VERSION + 5);
                assert_eq!(supported, SCHEMA_VERSION);
            }
            other => panic!("expected SchemaTooNew, got {other:?}"),
        }
    }

    #[test]
    fn deleting_a_group_cascades_to_its_records() {
        let connection = memory();
        initialize(&connection).expect("initializes");
        connection
            .execute(
                "INSERT INTO crash_group (group_id, package_name, process_name, user_id, kind,
                     is_system_app, is_main_process, self_handled, occurrence,
                     first_seen_ms, last_seen_ms)
                 VALUES ('g1', 'com.example.app', 'com.example.app', 0, 0, 0, 1, 0, 1, 10, 10)",
                [],
            )
            .expect("insert group");
        connection
            .execute(
                "INSERT INTO crash_record (id, group_id, happened_at_ms, pid, sources, is_repeating)
                 VALUES ('r1', 'g1', 10, 1, 1, 0)",
                [],
            )
            .expect("insert record");

        connection
            .execute("DELETE FROM crash_group WHERE group_id = 'g1'", [])
            .expect("delete group");

        let remaining: i64 = connection
            .query_row("SELECT COUNT(*) FROM crash_record", [], |row| row.get(0))
            .expect("counts");
        assert_eq!(remaining, 0, "foreign_keys pragma must be on");
    }

    #[test]
    fn every_filterable_column_has_an_index() {
        let connection = memory();
        initialize(&connection).expect("initializes");
        let mut statement = connection
            .prepare("SELECT name FROM sqlite_master WHERE type = 'index' AND name LIKE 'idx_%'")
            .expect("prepares");
        let names: Vec<String> = statement
            .query_map([], |row| row.get::<_, String>(0))
            .expect("queries")
            .filter_map(Result::ok)
            .collect();

        for expected in [
            "idx_group_last_seen",
            "idx_group_first_seen",
            "idx_group_occurrence",
            "idx_group_package",
            "idx_group_kind",
            "idx_record_group",
            "idx_record_time",
            "idx_record_payload_state",
            "idx_ingested_at",
        ] {
            assert!(
                names.iter().any(|name| name == expected),
                "missing index {expected}; a filter without one becomes a full scan"
            );
        }
    }

    #[test]
    fn the_list_query_uses_an_index_rather_than_scanning() {
        let connection = memory();
        initialize(&connection).expect("initializes");
        let plan: String = connection
            .query_row(
                "EXPLAIN QUERY PLAN
                 SELECT group_id FROM crash_group ORDER BY last_seen_ms DESC, group_id DESC LIMIT 50",
                [],
                |row| row.get(3),
            )
            .expect("explains");
        assert!(
            plan.contains("idx_group_last_seen"),
            "the default list ordering must be index-driven, got: {plan}"
        );
    }
}
