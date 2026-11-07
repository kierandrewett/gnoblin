//! `log --follow`: the records after the last one printed, polled until the command is stopped.
//!
//! The cursor is the id of the newest record the follower has counted. Each poll reads the rows
//! after that id and up to the newest id at the start of the poll, then commits the cursor with
//! the rows it returns, so a row is neither printed twice nor skipped. Ids are the table's rowid.
//! Retention can empty the table, and the next row then restarts the ids; `poll` handles that.
//!
//! When the database file is replaced (rotated by a rename or a new file at the same path), its
//! identity changes and the follower starts again from the base query on the new file. While the
//! file is absent the follower prints nothing and keeps waiting. A failed read is reported and
//! retried at the next interval, because a rotation can briefly leave the path unreadable.

use std::io;
use std::time::Duration;

use gnoblin_store::{LogQuery, StoredRecord};

/// The most rows one poll reads. A poll that reads a full batch is followed at once by the next,
/// so a burst is drained without waiting for the interval.
const BATCH: u32 = 1_000;

/// Where the follower reads from. The real one is the database file; tests supply a script.
pub(crate) trait Source {
    /// Which file the rows come from, or `None` while it does not exist.
    fn identity(&self) -> Option<(u64, u64)>;
    /// The id of the newest row, `0` when there is none.
    fn newest_id(&self) -> Result<i64, String>;
    /// The rows matching `query`, in id order.
    fn read(&self, query: &LogQuery) -> Result<Vec<StoredRecord>, String>;
}

/// The result of one poll.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Poll {
    /// The rows this poll found, oldest first.
    pub(crate) records: Vec<StoredRecord>,
    /// Whether the batch was full, so another poll should follow without waiting.
    pub(crate) more: bool,
}

/// Tracks the cursor and the file identity across polls.
pub(crate) struct Follower<S> {
    source: S,
    base: LogQuery,
    /// The id the next poll reads after. `None` before the first poll, or after a rotation.
    cursor: Option<i64>,
    identity: Option<(u64, u64)>,
}

impl<S: Source> Follower<S> {
    /// A follower that starts with the records `base` matches, then follows the rest.
    pub(crate) fn new(source: S, base: LogQuery) -> Self {
        Self {
            source,
            base,
            cursor: None,
            identity: None,
        }
    }

    /// One poll. The cursor moves only when the read succeeds, and then together with the rows.
    pub(crate) fn poll(&mut self) -> Result<Poll, String> {
        let identity = self.source.identity();
        if identity != self.identity {
            self.identity = identity;
            self.cursor = None;
        }
        if identity.is_none() {
            return Ok(Poll {
                records: Vec::new(),
                more: false,
            });
        }

        let newest = self.source.newest_id()?;
        // The newest id is below the cursor only when retention emptied the table and the ids
        // restarted. Every stored row is new then, so the cursor goes back to zero and they are read.
        if self.cursor.is_some_and(|cursor| newest < cursor) {
            self.cursor = Some(0);
        }
        let (query, first) = match self.cursor {
            None => (
                LogQuery {
                    up_to_id: Some(newest),
                    ..self.base.clone()
                },
                true,
            ),
            Some(cursor) => (
                LogQuery {
                    after_id: Some(cursor),
                    up_to_id: Some(newest),
                    limit: Some(BATCH),
                    ..self.base.clone()
                },
                false,
            ),
        };
        let records = self.source.read(&query)?;
        let full = !first && records.len() == BATCH as usize;
        self.cursor = Some(match records.last() {
            Some(last) if full => last.id,
            _ => newest,
        });
        Ok(Poll { records, more: full })
    }
}

/// Polls until `wait` returns false, handing each non-empty batch to `emit`.
///
/// A failed poll is passed to `report` once per distinct message, and the loop waits and tries
/// again. An error from `emit` (the output went away) ends the loop. `wait` sleeps for the
/// interval and returns `false` to stop; tests pass a fake, so the loop runs without sleeping.
pub(crate) fn run(
    follower: &mut Follower<impl Source>,
    interval: Duration,
    mut emit: impl FnMut(&[StoredRecord]) -> io::Result<()>,
    mut report: impl FnMut(&str),
    mut wait: impl FnMut(Duration) -> bool,
) -> io::Result<()> {
    let mut reported: Option<String> = None;
    loop {
        match follower.poll() {
            Ok(poll) => {
                reported = None;
                if !poll.records.is_empty() {
                    emit(&poll.records)?;
                }
                if poll.more {
                    continue;
                }
            }
            Err(message) => {
                if reported.as_deref() != Some(message.as_str()) {
                    report(&message);
                    reported = Some(message);
                }
            }
        }
        if !wait(interval) {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;

    use super::*;

    /// A store that answers from a script: each read returns the rows whose id is in the
    /// query's window, and the identity and failures are set by the test between polls.
    struct Fake {
        rows: RefCell<Vec<StoredRecord>>,
        identity: RefCell<Option<(u64, u64)>>,
        fail_next_read: RefCell<bool>,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                rows: RefCell::new(Vec::new()),
                identity: RefCell::new(Some((1, 1))),
                fail_next_read: RefCell::new(false),
            }
        }

        fn append(&self, count: usize) {
            let mut rows = self.rows.borrow_mut();
            for _ in 0..count {
                let id = rows.last().map_or(0, |row| row.id) + 1;
                rows.push(record(id));
            }
        }

        /// Removes the oldest rows, as retention does, leaving ids alone.
        fn retain_from(&self, id: i64) {
            self.rows.borrow_mut().retain(|row| row.id >= id);
        }
    }

    fn record(id: i64) -> StoredRecord {
        StoredRecord {
            id,
            time_ms: id * 10,
            level: "INFO".to_owned(),
            component: "wayland".to_owned(),
            role: "compositor".to_owned(),
            target: "t".to_owned(),
            message: format!("m{id}"),
            fields: String::new(),
        }
    }

    impl Source for &Fake {
        fn identity(&self) -> Option<(u64, u64)> {
            *self.identity.borrow()
        }

        fn newest_id(&self) -> Result<i64, String> {
            Ok(self.rows.borrow().last().map_or(0, |row| row.id))
        }

        fn read(&self, query: &LogQuery) -> Result<Vec<StoredRecord>, String> {
            if std::mem::take(&mut *self.fail_next_read.borrow_mut()) {
                return Err("cannot read".to_owned());
            }
            let rows = self.rows.borrow();
            let after = query.after_id.unwrap_or(0);
            let up_to = query.up_to_id.unwrap_or(i64::MAX);
            let mut matched: Vec<StoredRecord> = rows
                .iter()
                .filter(|row| row.id > after && row.id <= up_to)
                .cloned()
                .collect();
            let limit = query.limit.map_or(usize::MAX, |limit| limit as usize);
            if query.after_id.is_none() {
                // The base query reads the newest rows, as the store does.
                let skip = matched.len().saturating_sub(limit);
                matched.drain(..skip);
            } else {
                matched.truncate(limit);
            }
            Ok(matched)
        }
    }

    fn ids(records: &[StoredRecord]) -> Vec<i64> {
        records.iter().map(|row| row.id).collect()
    }

    fn base(limit: u32) -> LogQuery {
        LogQuery {
            limit: Some(limit),
            ..LogQuery::default()
        }
    }

    /// Runs the real loop for `waits` intervals, collecting what it printed.
    fn follow_for(fake: &Fake, limit: u32, waits: usize) -> Vec<i64> {
        let mut follower = Follower::new(fake, base(limit));
        let mut printed = Vec::new();
        let mut remaining = waits;
        run(
            &mut follower,
            Duration::from_millis(500),
            |batch| {
                printed.extend(ids(batch));
                Ok(())
            },
            |message| panic!("no read should fail: {message}"),
            |interval| {
                assert_eq!(interval, Duration::from_millis(500));
                if remaining == 0 {
                    return false;
                }
                remaining -= 1;
                true
            },
        )
        .expect("the fake never fails to write");
        printed
    }

    #[test]
    fn the_first_poll_prints_the_newest_matching_records_then_only_new_ones() {
        let fake = Fake::new();
        fake.append(3);
        let mut follower = Follower::new(&fake, base(2));
        assert_eq!(ids(&follower.poll().unwrap().records), [2, 3]);
        assert!(follower.poll().unwrap().records.is_empty());
        fake.append(2);
        assert_eq!(ids(&follower.poll().unwrap().records), [4, 5]);
    }

    #[test]
    fn a_row_is_neither_skipped_nor_repeated_across_polls() {
        let fake = Fake::new();
        fake.append(1);
        assert_eq!(follow_for(&fake, 100, 0), [1]);

        fake.append(3);
        let mut follower = Follower::new(&fake, base(100));
        let mut seen = Vec::new();
        for _ in 0..4 {
            seen.extend(ids(&follower.poll().unwrap().records));
            fake.append(1);
        }
        // Each poll sees each row once, in order, however the rows were appended.
        assert_eq!(seen, [1, 2, 3, 4, 5, 6, 7]);
    }

    #[test]
    fn a_full_batch_is_followed_without_waiting_and_the_rest_comes_next() {
        let fake = Fake::new();
        let mut follower = Follower::new(&fake, base(1));
        fake.append(1);
        assert_eq!(ids(&follower.poll().unwrap().records), [1]);
        fake.append(BATCH as usize + 5);
        let mut waits = 0;
        let mut printed = Vec::new();
        run(
            &mut follower,
            Duration::from_millis(500),
            |batch| {
                printed.extend(ids(batch));
                Ok(())
            },
            |_| {},
            |_| {
                waits += 1;
                false
            },
        )
        .unwrap();
        // The first poll read a full batch, so it followed at once; the second read the rest
        // and was short, so it waited once before stopping.
        assert_eq!(printed.len(), BATCH as usize + 5);
        assert_eq!(waits, 1);
    }

    #[test]
    fn retention_deleting_old_rows_neither_repeats_nor_rewinds() {
        let fake = Fake::new();
        fake.append(5);
        let mut follower = Follower::new(&fake, base(100));
        assert_eq!(ids(&follower.poll().unwrap().records), [1, 2, 3, 4, 5]);
        fake.retain_from(4);
        fake.append(2);
        assert_eq!(ids(&follower.poll().unwrap().records), [6, 7]);
    }

    #[test]
    fn rows_stored_after_retention_emptied_the_table_are_read_from_their_restarted_ids() {
        let fake = Fake::new();
        fake.append(5);
        let mut follower = Follower::new(&fake, base(100));
        assert_eq!(ids(&follower.poll().unwrap().records), [1, 2, 3, 4, 5]);
        // Retention removed every row, so the next rows are ids 1 and 2, below the cursor of 5.
        fake.retain_from(6);
        fake.append(2);
        assert_eq!(ids(&follower.poll().unwrap().records), [1, 2]);
        fake.append(1);
        assert_eq!(ids(&follower.poll().unwrap().records), [3]);
    }

    #[test]
    fn a_failed_read_leaves_the_cursor_where_it_was() {
        let fake = Fake::new();
        fake.append(2);
        let mut follower = Follower::new(&fake, base(100));
        assert_eq!(ids(&follower.poll().unwrap().records), [1, 2]);
        fake.append(1);
        *fake.fail_next_read.borrow_mut() = true;
        assert!(follower.poll().is_err());
        assert_eq!(ids(&follower.poll().unwrap().records), [3]);
    }

    #[test]
    fn a_failing_read_is_reported_once_and_retried_until_it_succeeds() {
        let fake = Fake::new();
        fake.append(1);
        *fake.fail_next_read.borrow_mut() = true;
        let mut follower = Follower::new(&fake, base(100));
        let mut reports = Vec::new();
        let mut printed = Vec::new();
        let mut waits = 0;
        run(
            &mut follower,
            Duration::from_millis(500),
            |batch| {
                printed.extend(ids(batch));
                Ok(())
            },
            |message| reports.push(message.to_owned()),
            |_| {
                waits += 1;
                waits < 3
            },
        )
        .unwrap();
        // The first poll fails and is reported; the next two find the row once the read works.
        assert_eq!(reports, ["cannot read"]);
        assert_eq!(printed, [1]);
        assert_eq!(waits, 3);
    }

    #[test]
    fn a_replaced_file_starts_again_from_the_base_query() {
        let fake = Fake::new();
        fake.append(3);
        let mut follower = Follower::new(&fake, base(2));
        assert_eq!(ids(&follower.poll().unwrap().records), [2, 3]);
        let replaced = Fake::new();
        replaced.append(4);
        *replaced.identity.borrow_mut() = Some((1, 2));
        let mut follower = Follower::new(&replaced, base(2));
        assert_eq!(ids(&follower.poll().unwrap().records), [3, 4]);
    }

    #[test]
    fn a_new_file_at_the_same_path_is_read_from_its_start() {
        let fake = Fake::new();
        fake.append(2);
        let mut follower = Follower::new(&fake, base(100));
        assert_eq!(ids(&follower.poll().unwrap().records), [1, 2]);
        *fake.identity.borrow_mut() = Some((9, 9));
        fake.rows.borrow_mut().clear();
        fake.append(1);
        assert_eq!(ids(&follower.poll().unwrap().records), [1]);
    }

    #[test]
    fn a_missing_file_prints_nothing_and_resumes_when_it_appears() {
        let fake = Fake::new();
        fake.append(2);
        let mut follower = Follower::new(&fake, base(100));
        assert_eq!(ids(&follower.poll().unwrap().records), [1, 2]);
        *fake.identity.borrow_mut() = None;
        assert!(follower.poll().unwrap().records.is_empty());
        *fake.identity.borrow_mut() = Some((1, 1));
        fake.append(1);
        // The same identity returns, so the cursor was reset: the rows are read from the start.
        assert_eq!(ids(&follower.poll().unwrap().records), [1, 2, 3]);
    }

    #[test]
    fn the_loop_stops_when_wait_says_so() {
        let fake = Fake::new();
        fake.append(1);
        let mut follower = Follower::new(&fake, base(100));
        let mut calls = VecDeque::from([true, false]);
        run(
            &mut follower,
            Duration::from_millis(500),
            |_| Ok(()),
            |_| {},
            |_| calls.pop_front().unwrap(),
        )
        .unwrap();
        assert!(calls.is_empty());
    }

    #[test]
    fn a_closed_output_ends_the_loop_with_the_write_error() {
        let fake = Fake::new();
        fake.append(1);
        let mut follower = Follower::new(&fake, base(100));
        let error = run(
            &mut follower,
            Duration::from_millis(500),
            |_| Err(io::Error::from(io::ErrorKind::BrokenPipe)),
            |_| {},
            |_| false,
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }
}
