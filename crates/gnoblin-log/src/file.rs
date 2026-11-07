//! The file sink: one JSON object per line in `gnoblin.jsonl`, shared by every gnoblin process.
//!
//! The compositor, the portal and the other binaries append to the same file, so a batch is
//! written under an advisory lock, and rotation happens under that lock too. A reader follows
//! the file with `tail -F` or the JSON tools of its choice; the format is the one the console
//! sink's JSON mode writes, except that `time_ms` is a number and `fields` is one string.

use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::json;
use crate::record::{LogRecord, RecordSink};

/// The file the sink writes inside its directory.
pub const FILE_NAME: &str = "gnoblin.jsonl";

/// How many rotated files are kept beside the live one: `gnoblin.jsonl.1` is the newest.
pub const RETAINED: u32 = 3;

/// How many times a batch reopens the file before it gives up. Each retry follows a rotation or a
/// replacement of the file by another process, so one retry is normally enough.
const ATTEMPTS: usize = 4;

/// Appends each batch as JSON lines to `gnoblin.jsonl`, rotating it when a batch would take it
/// past `max_bytes`.
#[derive(Debug)]
pub struct FileSink {
    dir: PathBuf,
    max_bytes: Option<u64>,
    /// Set while writes fail, so a run of failures reports once rather than once a batch.
    failing: AtomicBool,
}

impl FileSink {
    /// Opens the sink in `dir`, creating the directory and the file. `max_bytes` of `None` means
    /// the file never rotates. The open fails here, not on the first batch, so a binary that
    /// cannot write its log says so at startup.
    pub fn open(dir: &Path, max_bytes: Option<u64>) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let sink = Self {
            dir: dir.to_owned(),
            max_bytes,
            failing: AtomicBool::new(false),
        };
        drop(sink.open_append()?);
        Ok(sink)
    }

    /// The live file's path.
    pub fn path(&self) -> PathBuf {
        self.dir.join(FILE_NAME)
    }

    fn numbered(&self, index: u32) -> PathBuf {
        self.dir.join(format!("{FILE_NAME}.{index}"))
    }

    fn open_append(&self) -> io::Result<File> {
        OpenOptions::new()
            .append(true)
            .create(true)
            .mode(0o600)
            .open(self.path())
    }

    /// Writes `buffer` to the live file under its lock, rotating first when the batch would
    /// overflow it. A process that replaced or rotated the file while this one waited for the
    /// lock is detected by its inode, and the batch goes to the new file.
    fn append(&self, buffer: &[u8]) -> io::Result<()> {
        for _ in 0..ATTEMPTS {
            let file = self.open_append()?;
            file.lock()?;
            if !self.is_current(&file)? {
                continue;
            }
            let size = file.metadata()?.len();
            let overflows = self
                .max_bytes
                .is_some_and(|max| size > 0 && size.saturating_add(buffer.len() as u64) > max);
            if overflows {
                self.rotate()?;
                continue;
            }
            (&file).write_all(buffer)?;
            return Ok(());
        }
        Err(io::Error::other(format!(
            "{} kept changing under the writer",
            self.path().display()
        )))
    }

    /// Whether `file` is still the file at the live path, by device and inode.
    fn is_current(&self, file: &File) -> io::Result<bool> {
        match fs::metadata(self.path()) {
            Ok(path) => {
                let open = file.metadata()?;
                Ok(path.dev() == open.dev() && path.ino() == open.ino())
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Shifts the rotated files down by one, dropping the oldest, and moves the live file to
    /// `.1`. The caller holds the lock on the live file.
    fn rotate(&self) -> io::Result<()> {
        remove_if_exists(&self.numbered(RETAINED))?;
        for index in (1..RETAINED).rev() {
            rename_if_exists(&self.numbered(index), &self.numbered(index + 1))?;
        }
        rename_if_exists(&self.path(), &self.numbered(1))
    }

    /// Reports a failed batch on stderr, once per run of failures. The log pipeline cannot carry
    /// this report: the same write would fail again.
    fn report(&self, error: &io::Error) {
        if !self.failing.swap(true, Ordering::Relaxed) {
            eprintln!("gnoblin-log: cannot write {}: {error}", self.path().display());
        }
    }
}

impl RecordSink for FileSink {
    fn write_batch(&self, records: &[LogRecord]) {
        let buffer = encode(records);
        match self.append(buffer.as_bytes()) {
            Ok(()) => self.failing.store(false, Ordering::Relaxed),
            Err(error) => self.report(&error),
        }
    }
}

/// Opens the file sink the `logging` section names. `dir` of `None` or empty means the state
/// directory's `gnoblin/log`. `max_size_mb` of `None` or 0 means the file never rotates. A
/// setting that cannot be used is an error here, so the process reports it once the log is up.
pub fn open_from_settings(
    dir: Option<&str>,
    max_size_mb: Option<i64>,
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<FileSink, String> {
    let dir = match dir.filter(|path| !path.is_empty()) {
        Some(path) => PathBuf::from(path),
        None => default_dir(lookup)
            .ok_or_else(|| "no state directory: neither HOME nor XDG_STATE_HOME is set".to_owned())?,
    };
    let max_bytes = max_size_mb
        .and_then(|megabytes| u64::try_from(megabytes).ok())
        .filter(|&megabytes| megabytes > 0)
        .map(|megabytes| megabytes * 1024 * 1024);
    FileSink::open(&dir, max_bytes).map_err(|error| format!("no log file in {}: {error}", dir.display()))
}

/// The default log directory: `$XDG_STATE_HOME/gnoblin/log`, or `$HOME/.local/state/...` when
/// `XDG_STATE_HOME` is unset or relative. `None` when neither is set.
pub fn default_dir(lookup: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let non_empty = |value: Option<String>| value.filter(|text| !text.is_empty());
    let state_home = match non_empty(lookup("XDG_STATE_HOME")) {
        Some(path) if Path::new(&path).is_absolute() => PathBuf::from(path),
        _ => PathBuf::from(non_empty(lookup("HOME"))?).join(".local/state"),
    };
    Some(state_home.join("gnoblin").join("log"))
}

/// Encodes `records` as JSON lines, oldest first, each ending in a newline.
fn encode(records: &[LogRecord]) -> String {
    let mut out = String::new();
    for record in records {
        encode_one(&mut out, record);
        out.push('\n');
    }
    out
}

fn encode_one(out: &mut String, record: &LogRecord) {
    out.push('{');
    let _ = write!(out, "\"time_ms\":{}", record.time_ms);
    for (key, value) in [
        ("level", record.level),
        ("role", record.role),
        ("component", record.component.as_str()),
        ("target", record.target.as_str()),
        ("message", record.message.as_str()),
        ("fields", record.fields.as_str()),
    ] {
        out.push(',');
        json::pair(out, key, value);
    }
    out.push('}');
}

fn remove_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

fn rename_if_exists(from: &Path, to: &Path) -> io::Result<()> {
    match fs::rename(from, to) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh directory under the system temp dir, removed when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("gnoblin-log-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn record(message: &str) -> LogRecord {
        LogRecord {
            time_ms: 42,
            level: "INFO",
            component: "wayland".to_owned(),
            role: "compositor",
            target: "gnoblin::wayland".to_owned(),
            message: message.to_owned(),
            fields: "id=3".to_owned(),
        }
    }

    #[test]
    fn a_record_encodes_as_one_json_object_with_a_numeric_time() {
        let line = encode(&[record("mapped \"x\"")]);
        assert_eq!(
            line,
            "{\"time_ms\":42,\"level\":\"INFO\",\"role\":\"compositor\",\"component\":\"wayland\",\
             \"target\":\"gnoblin::wayland\",\"message\":\"mapped \\\"x\\\"\",\"fields\":\"id=3\"}\n"
        );
    }

    #[test]
    fn batches_append_to_the_live_file() {
        let scratch = Scratch::new("append");
        let sink = FileSink::open(&scratch.0, None).expect("open");
        sink.write_batch(&[record("one"), record("two")]);
        sink.write_batch(&[record("three")]);
        let text = fs::read_to_string(sink.path()).expect("read");
        assert_eq!(text.lines().count(), 3);
        assert!(text.lines().last().expect("line").contains("\"message\":\"three\""));
    }

    #[test]
    fn a_batch_that_would_overflow_rotates_the_file_first() {
        let scratch = Scratch::new("rotate");
        let one = encode(&[record("filler")]).len() as u64;
        let sink = FileSink::open(&scratch.0, Some(one + one / 2)).expect("open");
        sink.write_batch(&[record("filler")]);
        sink.write_batch(&[record("filler")]);
        assert!(sink.numbered(1).exists(), "the first batch moves to .1");
        let live = fs::read_to_string(sink.path()).expect("read");
        assert_eq!(live.lines().count(), 1, "the second batch starts a new file");
    }

    #[test]
    fn rotation_keeps_three_files_and_drops_the_oldest() {
        let scratch = Scratch::new("retain");
        let one = encode(&[record("filler")]).len() as u64;
        let sink = FileSink::open(&scratch.0, Some(one)).expect("open");
        for index in 0..5 {
            sink.write_batch(&[record(&format!("batch {index}"))]);
        }
        assert!(sink.numbered(1).exists());
        assert!(sink.numbered(2).exists());
        assert!(sink.numbered(3).exists());
        assert!(!sink.numbered(4).exists());
        let oldest = fs::read_to_string(sink.numbered(3)).expect("read");
        assert!(oldest.contains("batch 1"), "the oldest kept file holds batch 1");
    }

    #[test]
    fn a_batch_larger_than_the_limit_is_written_whole() {
        let scratch = Scratch::new("oversize");
        let sink = FileSink::open(&scratch.0, Some(4)).expect("open");
        sink.write_batch(&[record("far too large for four bytes")]);
        let text = fs::read_to_string(sink.path()).expect("read");
        assert!(text.contains("far too large"));
    }

    #[test]
    fn a_replaced_file_is_followed_to_the_new_one() {
        let scratch = Scratch::new("replaced");
        let sink = FileSink::open(&scratch.0, None).expect("open");
        sink.write_batch(&[record("before")]);
        fs::rename(sink.path(), sink.numbered(1)).expect("rename");
        sink.write_batch(&[record("after")]);
        let live = fs::read_to_string(sink.path()).expect("read");
        assert!(live.contains("after") && !live.contains("before"));
    }
}
