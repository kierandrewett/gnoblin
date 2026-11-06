//! Reads stored records back, for `gnoblinctl log` and for anything else that inspects the log.

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, Row, params};

/// How long a read waits for a writer holding the database.
const READ_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Which records to read. Every filter that is `None` matches everything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    /// Records at or after this time, in milliseconds since the epoch.
    pub since_ms: Option<i64>,
    /// Records before this time, in milliseconds since the epoch.
    pub until_ms: Option<i64>,
    /// Records at this level or above, named as the writer names them (`INFO`, `WARN`, ...).
    pub min_level: Option<String>,
    /// Records written by this process role, in the `role` column.
    pub role: Option<String>,
    /// Records whose event target matches exactly.
    pub target: Option<String>,
    /// Only records with an id above this, oldest first. Used to follow the log.
    pub after_id: Option<i64>,
    /// The most records to return.
    pub limit: u32,
}

/// One stored record, as read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogRecord {
    pub id: i64,
    pub time_ms: i64,
    pub level: String,
    /// The component that logged the event, as declared by `gnoblin_log::component!`.
    pub component: String,
    /// The process role that logged the event.
    pub role: String,
    pub target: String,
    pub message: String,
    pub fields: String,
}

/// Reads the records matching `query` from the database at `path`, oldest first.
///
/// The database is opened read-only, so reading never changes it.
pub fn read_entries(path: &Path, query: &Query) -> Result<Vec<LogRecord>, String> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    connection
        .busy_timeout(READ_BUSY_TIMEOUT)
        .map_err(|error| format!("cannot set the busy timeout: {error}"))?;
    read_from(&connection, query).map_err(|error| format!("cannot read {}: {error}", path.display()))
}

/// Reads the records matching `query` from an open connection, oldest first.
pub(crate) fn read_from(connection: &Connection, query: &Query) -> rusqlite::Result<Vec<LogRecord>> {
    let limit = i64::from(query.limit.max(1));
    let min_rank = query.min_level.as_deref().map_or(0, level_rank);
    let role = query.role.as_deref();
    let target = query.target.as_deref();
    if let Some(after_id) = query.after_id {
        let mut statement = connection.prepare_cached(FOLLOW_SQL)?;
        let rows = statement.query_map(
            params![after_id, query.since_ms, query.until_ms, min_rank, role, target, limit],
            read_record,
        )?;
        return rows.collect();
    }
    let mut statement = connection.prepare_cached(NEWEST_SQL)?;
    let rows = statement.query_map(
        params![query.since_ms, query.until_ms, min_rank, role, target, limit],
        read_record,
    )?;
    let mut entries = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    entries.reverse();
    Ok(entries)
}

/// The rank of a level name: higher is more severe. Names the writer does not produce rank as
/// `ERROR`, so a filter never hides them by accident.
pub(crate) fn level_rank(name: &str) -> i64 {
    match name.to_ascii_uppercase().as_str() {
        "TRACE" => 0,
        "DEBUG" => 1,
        "INFO" => 2,
        "WARN" => 3,
        _ => 4,
    }
}

/// The newest `?6` records matching the filters. `?1`/`?2` bound the time, `?3` is the minimum
/// level rank, `?4` the role and `?5` the target; a `NULL` parameter disables its filter.
const NEWEST_SQL: &str = "SELECT id, time_ms, level, component, role, target, message, fields FROM log_records
    WHERE (?1 IS NULL OR time_ms >= ?1)
      AND (?2 IS NULL OR time_ms < ?2)
      AND (CASE level WHEN 'TRACE' THEN 0 WHEN 'DEBUG' THEN 1 WHEN 'INFO' THEN 2 WHEN 'WARN' THEN 3 ELSE 4 END) >= ?3
      AND (?4 IS NULL OR role = ?4)
      AND (?5 IS NULL OR target = ?5)
    ORDER BY id DESC
    LIMIT ?6";

/// The oldest `?7` records after id `?1` matching the filters, with the same parameters as
/// [`NEWEST_SQL`] shifted by one.
const FOLLOW_SQL: &str = "SELECT id, time_ms, level, component, role, target, message, fields FROM log_records
    WHERE id > ?1
      AND (?2 IS NULL OR time_ms >= ?2)
      AND (?3 IS NULL OR time_ms < ?3)
      AND (CASE level WHEN 'TRACE' THEN 0 WHEN 'DEBUG' THEN 1 WHEN 'INFO' THEN 2 WHEN 'WARN' THEN 3 ELSE 4 END) >= ?4
      AND (?5 IS NULL OR role = ?5)
      AND (?6 IS NULL OR target = ?6)
    ORDER BY id ASC
    LIMIT ?7";

fn read_record(row: &Row<'_>) -> rusqlite::Result<LogRecord> {
    Ok(LogRecord {
        id: row.get(0)?,
        time_ms: row.get(1)?,
        level: row.get(2)?,
        component: row.get(3)?,
        role: row.get(4)?,
        target: row.get(5)?,
        message: row.get(6)?,
        fields: row.get(7)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> Connection {
        let connection = Connection::open_in_memory().expect("an in-memory database");
        connection.execute_batch(crate::SCHEMA).expect("the schema is valid");
        connection
    }

    fn insert(connection: &Connection, time_ms: i64, level: &str, role: &str, target: &str, message: &str) {
        connection
            .execute(
                "INSERT INTO log_records (time_ms, level, component, role, target, message, fields) \
                 VALUES (?1, ?2, 'wayland', ?3, ?4, ?5, '')",
                params![time_ms, level, role, target, message],
            )
            .expect("an insert");
    }

    fn sample() -> Connection {
        let connection = database();
        insert(&connection, 1_000, "DEBUG", "gnoblin", "a::one", "debug one");
        insert(&connection, 2_000, "INFO", "gnoblin", "a::two", "info two");
        insert(&connection, 3_000, "WARN", "portal", "b::three", "warn three");
        insert(&connection, 4_000, "ERROR", "gnoblin", "a::four", "error four");
        connection
    }

    fn messages(entries: &[LogRecord]) -> Vec<&str> {
        entries.iter().map(|entry| entry.message.as_str()).collect()
    }

    #[test]
    fn the_newest_records_come_back_oldest_first() {
        let connection = sample();
        let query = Query {
            limit: 2,
            ..Query::default()
        };
        let entries = read_from(&connection, &query).expect("a read");
        assert_eq!(messages(&entries), ["warn three", "error four"]);
    }

    #[test]
    fn the_level_filter_keeps_that_level_and_above() {
        let connection = sample();
        let query = Query {
            min_level: Some("info".to_owned()),
            limit: 10,
            ..Query::default()
        };
        let entries = read_from(&connection, &query).expect("a read");
        assert_eq!(messages(&entries), ["info two", "warn three", "error four"]);
    }

    #[test]
    fn the_role_and_target_filters_match_exactly() {
        let connection = sample();
        let by_role = Query {
            role: Some("portal".to_owned()),
            limit: 10,
            ..Query::default()
        };
        assert_eq!(
            messages(&read_from(&connection, &by_role).expect("a read")),
            ["warn three"]
        );
        let by_target = Query {
            target: Some("a::two".to_owned()),
            limit: 10,
            ..Query::default()
        };
        assert_eq!(
            messages(&read_from(&connection, &by_target).expect("a read")),
            ["info two"]
        );
    }

    #[test]
    fn the_time_window_is_half_open() {
        let connection = sample();
        let query = Query {
            since_ms: Some(2_000),
            until_ms: Some(4_000),
            limit: 10,
            ..Query::default()
        };
        let entries = read_from(&connection, &query).expect("a read");
        assert_eq!(messages(&entries), ["info two", "warn three"]);
    }

    #[test]
    fn following_returns_the_records_after_an_id_oldest_first() {
        let connection = sample();
        let first = read_from(
            &connection,
            &Query {
                limit: 10,
                ..Query::default()
            },
        )
        .expect("a read");
        let query = Query {
            after_id: Some(first[1].id),
            limit: 10,
            ..Query::default()
        };
        let entries = read_from(&connection, &query).expect("a read");
        assert_eq!(messages(&entries), ["warn three", "error four"]);
    }

    #[test]
    fn unknown_levels_rank_as_errors() {
        assert_eq!(level_rank("bogus"), 4);
        assert!(level_rank("warn") < level_rank("error"));
    }
}
