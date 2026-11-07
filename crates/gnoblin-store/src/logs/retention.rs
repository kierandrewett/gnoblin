//! Keeps the `log` kind within the limits the configuration sets, when the file opens.
//!
//! Deleting rows does not shrink the file: SQLite keeps the freed pages for new records. So the
//! size limit bounds the pages in use, and the file keeps its high-water mark.

use rusqlite::{Connection, params};

/// Milliseconds in a day, for turning the age limit into a cutoff.
const DAY_MS: i64 = 86_400_000;

/// The limits on what the `log` kind keeps. A zero age or an absent size is no limit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Retention {
    /// Records older than this many days are deleted; zero keeps every record.
    pub max_age_days: u32,
    /// The pages in use may not exceed this many bytes; the oldest records go first.
    pub max_bytes: Option<u64>,
}

/// Applies `retention` to `connection` as of `now_ms`, the time in milliseconds since the epoch.
///
/// The age pass runs first, then the size pass trims the oldest rows until the pages in use fit.
pub(crate) fn apply(connection: &Connection, retention: Retention, now_ms: i64) -> rusqlite::Result<()> {
    if retention.max_age_days > 0 {
        let cutoff = now_ms - i64::from(retention.max_age_days) * DAY_MS;
        connection.execute("DELETE FROM log_records WHERE time_ms < ?1", params![cutoff])?;
    }
    if let Some(limit) = retention.max_bytes {
        // Each pass deletes the oldest rows the excess bytes account for, rounded up, so the loop
        // converges on the limit without emptying the table. Deleting rows only frees pages, so
        // the bytes in use are read again after every pass.
        loop {
            let used = used_bytes(connection)?;
            if used <= limit {
                break;
            }
            let rows = row_count(connection)?;
            if rows == 0 {
                break;
            }
            let excess = used - limit;
            let batch = rows.saturating_mul(excess).div_ceil(used).clamp(1, rows);
            let deleted = connection.execute(
                "DELETE FROM log_records WHERE id IN (SELECT id FROM log_records ORDER BY id LIMIT ?1)",
                params![i64::try_from(batch).unwrap_or(i64::MAX)],
            )?;
            if deleted == 0 {
                break;
            }
        }
    }
    Ok(())
}

/// Rows in the table, which the size trim needs to estimate how many rows a byte excess is.
fn row_count(connection: &Connection) -> rusqlite::Result<u64> {
    let rows: i64 = connection.query_row("SELECT COUNT(*) FROM log_records", [], |row| row.get(0))?;
    Ok(u64::try_from(rows.max(0)).unwrap_or(0))
}

/// The bytes of pages the database uses: its pages less the ones it has freed.
fn used_bytes(connection: &Connection) -> rusqlite::Result<u64> {
    let page_size: i64 = connection.query_row("PRAGMA page_size", [], |row| row.get(0))?;
    let page_count: i64 = connection.query_row("PRAGMA page_count", [], |row| row.get(0))?;
    let free_pages: i64 = connection.query_row("PRAGMA freelist_count", [], |row| row.get(0))?;
    Ok(u64::try_from((page_count - free_pages).max(0) * page_size).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logs::LOG_STEPS;

    /// The instant the tests treat as now: day 100, so day-old records have a known cutoff.
    const NOW: i64 = 100 * DAY_MS;

    /// A database with the `log` kind's table and no rows.
    fn database() -> Connection {
        let connection = Connection::open_in_memory().expect("memory database");
        connection.execute_batch(LOG_STEPS[0]).expect("schema");
        connection
    }

    /// Inserts one record with `message` at `time_ms`.
    fn insert(connection: &Connection, time_ms: i64, message: &str) {
        connection
            .execute(
                "INSERT INTO log_records (time_ms, level, component, role, target, message, fields) \
                 VALUES (?1, 'INFO', 'test', 'test', 'gnoblin::test', ?2, '')",
                params![time_ms, message],
            )
            .expect("insert");
    }

    fn count(connection: &Connection) -> i64 {
        connection
            .query_row("SELECT COUNT(*) FROM log_records", [], |row| row.get(0))
            .expect("count")
    }

    #[test]
    fn no_limits_keep_every_record() {
        let connection = database();
        insert(&connection, 0, "ancient");
        insert(&connection, NOW, "now");
        apply(&connection, Retention::default(), NOW).expect("apply");
        assert_eq!(count(&connection), 2);
    }

    #[test]
    fn records_older_than_the_age_limit_are_deleted() {
        let connection = database();
        insert(&connection, NOW - 10 * DAY_MS, "too old");
        insert(&connection, NOW - 2 * DAY_MS, "kept");
        let retention = Retention {
            max_age_days: 5,
            max_bytes: None,
        };
        apply(&connection, retention, NOW).expect("apply");
        assert_eq!(count(&connection), 1);
    }

    #[test]
    fn the_size_limit_deletes_the_oldest_records_first() {
        let connection = database();
        let message = "x".repeat(4000);
        for index in 0..400 {
            insert(&connection, NOW + index, &message);
        }
        let before = used_bytes(&connection).expect("bytes");
        let limit = before / 2;
        apply(
            &connection,
            Retention {
                max_age_days: 0,
                max_bytes: Some(limit),
            },
            NOW,
        )
        .expect("apply");
        assert!(used_bytes(&connection).expect("bytes") <= limit);
        let oldest: i64 = connection
            .query_row("SELECT MIN(time_ms) FROM log_records", [], |row| row.get(0))
            .expect("oldest");
        assert!(oldest > NOW, "the oldest rows must go first");
    }
}
