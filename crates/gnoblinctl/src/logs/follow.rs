//! The reader behind `log`: the newest records across the rotated files and the live one, and
//! `--follow`, which keeps printing the records written after them.
//!
//! The live file stays open, and a poll reads what the handle has gained since the last poll.
//! When the path no longer names the file the handle holds, the writer has rotated it (or another
//! process replaced it). The handle is drained once more, because the writer may have appended to
//! it before the rename, and then the new live file is read from its start. So a rotation loses no
//! record. A missing live file is waited for, and a failed poll is reported and retried at the next
//! interval, because a rotation can briefly leave the path unreadable.

use std::fs::File;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::records::{self, Filter, Record};

/// Reads the log directory `dir` for the records `filter` keeps.
pub(crate) struct Follower {
    dir: PathBuf,
    filter: Filter,
    limit: usize,
    live: Option<Live>,
    started: bool,
}

/// The live file as the follower last saw it: the open handle, how far it has been read, and the
/// device and inode that name it.
struct Live {
    file: File,
    offset: u64,
    identity: (u64, u64),
}

impl Live {
    /// Opens the live file in `dir` at offset zero, or `None` when there is none.
    fn open(dir: &Path) -> io::Result<Option<Self>> {
        let file = match File::open(records::live_path(dir)) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let metadata = file.metadata()?;
        Ok(Some(Self {
            file,
            offset: 0,
            identity: (metadata.dev(), metadata.ino()),
        }))
    }
}

impl Follower {
    /// A follower that reads `dir` for the records `filter` keeps, and prints at most `limit`
    /// of them at the start.
    pub(crate) fn new(dir: PathBuf, filter: Filter, limit: usize) -> Self {
        Self {
            dir,
            filter,
            limit,
            live: None,
            started: false,
        }
    }

    /// The newest `limit` records the filter keeps, oldest first. It also positions the follower
    /// at the end of the live file, so the next poll returns only what is written after this.
    ///
    /// The live file is opened before the rotated ones are read. A rotation in between can make
    /// the same records appear twice, but never drops one.
    pub(crate) fn start(&mut self) -> io::Result<Vec<Record>> {
        let mut live = Live::open(&self.dir)?;
        let mut records = records::read_rotated(&self.dir, &self.filter)?;
        if let Some(live) = live.as_mut() {
            live.offset = records::read_lines(&mut live.file, 0, &self.filter, &mut records)?;
        }
        self.live = live;
        self.started = true;
        Ok(records::keep_newest(records, self.limit))
    }

    /// The records written since the last poll, oldest first. The first poll is `start`.
    pub(crate) fn poll(&mut self) -> io::Result<Vec<Record>> {
        if !self.started {
            return self.start();
        }
        let path = records::live_path(&self.dir);
        let rotated = match (&self.live, records::identity(&path)?) {
            (Some(live), Some(now)) => live.identity != now,
            (None, None) => false,
            _ => true,
        };
        let mut out = Vec::new();
        if let Some(live) = self.live.as_mut() {
            live.offset = records::read_lines(&mut live.file, live.offset, &self.filter, &mut out)?;
        }
        if rotated {
            self.live = Live::open(&self.dir)?;
            if let Some(live) = self.live.as_mut() {
                live.offset = records::read_lines(&mut live.file, 0, &self.filter, &mut out)?;
            }
        }
        Ok(out)
    }
}

/// Prints what `follower` finds, polling every `interval`, until `emit` fails. The first poll
/// prints the newest records, as `start` does. Only an output error ends the loop; a failed read
/// is reported once when it starts, then retried.
pub(crate) fn follow(
    follower: &mut Follower,
    interval: Duration,
    emit: &mut dyn FnMut(&[Record]) -> io::Result<()>,
    report: &mut dyn FnMut(&str),
) -> io::Result<()> {
    let mut failing = false;
    loop {
        match follower.poll() {
            Ok(records) => {
                failing = false;
                if !records.is_empty() {
                    emit(&records)?;
                }
            }
            Err(error) => {
                if !failing {
                    report(&format!("cannot read the log: {error}"));
                }
                failing = true;
            }
        }
        std::thread::sleep(interval);
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use gnoblin_log::file::FILE_NAME;

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gnoblinctl-follow-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir
    }

    fn line(time_ms: i64) -> String {
        format!(
            "{{\"time_ms\":{time_ms},\"level\":\"INFO\",\"role\":\"compositor\",\
             \"component\":\"log\",\"target\":\"t\",\"message\":\"m\",\"fields\":\"\"}}\n"
        )
    }

    fn append(path: &Path, text: &str) {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("opened");
        file.write_all(text.as_bytes()).expect("written");
    }

    fn times(records: &[Record]) -> Vec<i64> {
        records.iter().map(|record| record.time_ms).collect()
    }

    #[test]
    fn start_prints_the_newest_limit_then_poll_prints_only_new_records() {
        let dir = scratch("start");
        let live = records::live_path(&dir);
        append(&live, &(1..=3).map(line).collect::<String>());
        let mut follower = Follower::new(dir.clone(), Filter::default(), 2);
        assert_eq!(times(&follower.start().expect("start")), vec![2, 3]);
        assert!(follower.poll().expect("poll").is_empty());
        append(&live, &line(4));
        assert_eq!(times(&follower.poll().expect("poll")), vec![4]);
    }

    #[test]
    fn a_rotation_drains_the_old_file_and_reads_the_new_one_from_its_start() {
        let dir = scratch("rotate");
        let live = records::live_path(&dir);
        append(&live, &line(1));
        let mut follower = Follower::new(dir.clone(), Filter::default(), 10);
        follower.start().expect("start");
        append(&live, &line(2));
        std::fs::rename(&live, dir.join(format!("{FILE_NAME}.1"))).expect("renamed");
        append(&live, &line(3));
        assert_eq!(times(&follower.poll().expect("poll")), vec![2, 3]);
    }

    #[test]
    fn a_missing_live_file_is_waited_for() {
        let dir = scratch("missing");
        let mut follower = Follower::new(dir.clone(), Filter::default(), 10);
        assert!(follower.start().expect("start").is_empty());
        assert!(follower.poll().expect("poll").is_empty());
        append(&records::live_path(&dir), &line(5));
        assert_eq!(times(&follower.poll().expect("poll")), vec![5]);
    }

    #[test]
    fn the_filter_applies_to_the_followed_records() {
        let dir = scratch("filter");
        let live = records::live_path(&dir);
        let filter = Filter {
            role: Some("portal".to_owned()),
            ..Filter::default()
        };
        let mut follower = Follower::new(dir.clone(), filter, 10);
        follower.start().expect("start");
        append(&live, &line(1));
        assert!(follower.poll().expect("poll").is_empty());
    }
}
