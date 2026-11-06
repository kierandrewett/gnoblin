//! Raw output of the children the guardian starts.
//!
//! A child that logs through `gnoblin-log` writes structured lines, which its own subscriber
//! already formats. Everything else a child prints (a library panic, a Lua `print`, output from
//! a C library or from Xwayland) reaches the guardian's standard streams as bare text. This
//! module reads those streams and logs each line through the facade as component `raw`, tagged
//! with the child's role and the stream it came from.
//!
//! The readers never block the supervisor: each stream has its own thread, the lines go through
//! one bounded channel, and a full channel drops the line and counts it. The drop count is
//! logged once a second while it changes, so a flood is visible without slowing anything down.
//!
//! Which children are captured is decided by [`policy`]: an allowlist by role, so a role that is
//! not listed is read and discarded. Each captured line is scrubbed of what looks like a secret
//! before it is logged. That scrubbing is best effort: heuristics cannot recognise every secret,
//! so it never guarantees that a line is free of secrets. Only capture the output of a child that
//! does not print secrets.

use std::io::{BufRead, BufReader, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::time::Duration;

use gnoblin_log::redact::Redactor;

/// The longest line logged. A longer line is cut here and marked with [`TRUNCATED`].
pub const MAX_LINE: usize = 8 * 1024;

/// The marker appended to a truncated line.
pub const TRUNCATED: &str = " [truncated]";

/// How many raw lines may wait for the logger before new ones are dropped.
const CAPACITY: usize = 1024;

/// How often the logger reports a change in the drop count.
const REPORT_INTERVAL: Duration = Duration::from_secs(1);

/// The stream a raw line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStream {
    Stdout,
    Stderr,
}

impl OutputStream {
    /// The name the log records for the stream.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }
}

/// Whether the raw lines of one child are captured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawPolicy {
    /// The lines are read and skipped, so nothing reaches the log.
    Off,
    /// The lines are scrubbed with [`Redactor`] before they are logged.
    Redacted,
}

/// The raw capture policy of a child, by role. Only the roles listed here are captured; any
/// other role is [`RawPolicy::Off`] until someone decides otherwise. The services role is
/// captured only when `raw_services` is set.
pub fn policy(role: &str, raw_services: bool) -> RawPolicy {
    match role {
        "compositor" | "runtime" | "portal" => RawPolicy::Redacted,
        "services" if raw_services => RawPolicy::Redacted,
        _ => RawPolicy::Off,
    }
}

/// One raw line, already bounded in length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawLine {
    pub role: String,
    pub stream: OutputStream,
    pub text: String,
}

/// The sending half shared by every reader of the guardian.
#[derive(Clone)]
pub struct RawSink {
    sender: SyncSender<RawLine>,
    dropped: Arc<AtomicU64>,
}

impl RawSink {
    /// Creates the sink and the receiver the logger reads from.
    pub fn channel() -> (Self, Receiver<RawLine>, Arc<AtomicU64>) {
        let (sender, receiver) = sync_channel(CAPACITY);
        let dropped = Arc::new(AtomicU64::new(0));
        (
            Self {
                sender,
                dropped: Arc::clone(&dropped),
            },
            receiver,
            dropped,
        )
    }

    /// Sends one line without waiting. A full channel counts the line as dropped.
    pub fn send(&self, line: RawLine) {
        if self.sender.try_send(line).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Reads `reader` to its end on a new thread, sending each raw line to `sink` under `policy`.
pub fn capture(role: &str, stream: OutputStream, policy: RawPolicy, reader: impl Read + Send + 'static, sink: RawSink) {
    let role = role.to_owned();
    let spawned = std::thread::Builder::new()
        .name(format!("gnoblin-raw-{role}-{}", stream.as_str()))
        .spawn(move || read_lines(&role, stream, policy, reader, &sink));
    if let Err(error) = spawned {
        // Without a reader the child would block once its pipe fills, so this is worth a log
        // line even though the guardian carries on.
        gnoblin_log::warn!("cannot read the {} of a child: {error}", stream.as_str());
    }
}

/// Writes a structured line to the guardian's stream of the same name, unchanged. A failed
/// write is ignored: the guardian's own output is not worth stopping the session for.
fn forward(stream: OutputStream, line: &[u8]) {
    match stream {
        OutputStream::Stdout => {
            let _ = std::io::stdout().lock().write_all(line);
        }
        OutputStream::Stderr => {
            let _ = std::io::stderr().lock().write_all(line);
        }
    }
}

/// Reads lines until the end of `reader`. Under [`RawPolicy::Redacted`] each raw line goes to
/// `sink`, scrubbed. Under [`RawPolicy::Off`] each line is only read and skipped, so the child
/// still drains its pipe.
pub fn read_lines(role: &str, stream: OutputStream, policy: RawPolicy, reader: impl Read, sink: &RawSink) {
    let mut reader = BufReader::new(reader);
    let mut buffer = Vec::new();
    let mut redactor = Redactor::new();
    loop {
        buffer.clear();
        match reader.read_until(b'\n', &mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        if policy == RawPolicy::Off {
            continue;
        }
        let line = String::from_utf8_lossy(&buffer);
        let text = bound(&redactor.redact(line.trim_end_matches(['\n', '\r'])));
        if is_structured(&text) {
            // The child formatted this line itself; it goes out redacted and otherwise unchanged.
            forward(stream, format!("{text}\n").as_bytes());
            continue;
        }
        sink.send(RawLine {
            role: role.to_owned(),
            stream,
            text,
        });
    }
}

/// Drops the line terminator and cuts the line to [`MAX_LINE`] bytes on a character boundary.
pub fn bound(line: &str) -> String {
    let line = line.trim_end_matches(['\n', '\r']);
    if line.len() <= MAX_LINE {
        return line.to_owned();
    }
    let mut end = MAX_LINE;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{TRUNCATED}", &line[..end])
}

/// Whether a line is already a record of the gnoblin log, as the console subscriber prints it:
/// a level padded to five characters, then the role in brackets.
pub fn is_structured(line: &str) -> bool {
    ["ERROR", "WARN", "INFO", "DEBUG", "TRACE"].iter().any(|level| {
        line.strip_prefix(level)
            .and_then(|rest| rest.trim_start().strip_prefix('['))
            .is_some_and(|rest| rest.contains("] "))
    })
}

/// Logs raw lines until every sender is gone, reporting drops once a second while they change.
pub fn run_logger(receiver: &Receiver<RawLine>, dropped: &AtomicU64) {
    let mut reported = 0;
    loop {
        match receiver.recv_timeout(REPORT_INTERVAL) {
            Ok(line) => log_line(&line),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        let total = dropped.load(Ordering::Relaxed);
        if total != reported {
            gnoblin_log::warn!(component: "raw", "dropped {} raw child lines", total - reported);
            reported = total;
        }
    }
}

/// How long the guardian waits for the logger to print what the children wrote before exiting.
const FLUSH_WAIT: Duration = Duration::from_secs(2);

/// Drops this sender and waits, for at most [`FLUSH_WAIT`], until the logger has printed every
/// line already sent. A reader still holding a pipe (a grandchild kept it open) cannot hold the
/// guardian's exit past the bound.
pub fn finish(logger: std::thread::JoinHandle<()>, sink: RawSink) {
    drop(sink);
    let (done, waited) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = logger.join();
        let _ = done.send(());
    });
    let _ = waited.recv_timeout(FLUSH_WAIT);
}

fn log_line(line: &RawLine) {
    match line.stream {
        OutputStream::Stdout => {
            gnoblin_log::info!(component: "raw", role = %line.role, stream = "stdout", "{}", line.text);
        }
        OutputStream::Stderr => {
            gnoblin_log::warn!(component: "raw", role = %line.role, stream = "stderr", "{}", line.text);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::process::{Command, Stdio};
    use std::time::Duration;

    use super::*;

    fn collect(role: &str, stream: OutputStream, reader: impl Read) -> Vec<RawLine> {
        let (sink, receiver, _) = RawSink::channel();
        read_lines(role, stream, RawPolicy::Redacted, reader, &sink);
        drop(sink);
        receiver.try_iter().collect()
    }

    #[test]
    fn a_fake_child_is_read_on_both_streams() {
        let mut child = Command::new("sh")
            .args(["-c", "echo from-out; echo from-err >&2"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("sh is available");
        let stdout = child.stdout.take().expect("piped");
        let stderr = child.stderr.take().expect("piped");
        let out = collect("compositor", OutputStream::Stdout, stdout);
        let err = collect("compositor", OutputStream::Stderr, stderr);
        child.wait().expect("sh finishes");
        assert_eq!(
            out.iter().map(|line| line.text.as_str()).collect::<Vec<_>>(),
            ["from-out"]
        );
        assert_eq!(
            err.iter().map(|line| line.text.as_str()).collect::<Vec<_>>(),
            ["from-err"]
        );
        assert_eq!(err[0].role, "compositor");
        assert_eq!(err[0].stream, OutputStream::Stderr);
    }

    #[test]
    fn a_long_line_is_cut_and_marked() {
        let long = "x".repeat(MAX_LINE + 10);
        let bounded = bound(&long);
        assert_eq!(bounded.len(), MAX_LINE + TRUNCATED.len());
        assert!(bounded.ends_with(TRUNCATED));
    }

    #[test]
    fn the_cut_never_splits_a_character() {
        let line = format!("{}é", "a".repeat(MAX_LINE - 1));
        let bounded = bound(&line);
        assert!(bounded.ends_with(TRUNCATED));
        assert!(bounded.is_char_boundary(bounded.len() - TRUNCATED.len()));
    }

    #[test]
    fn structured_console_lines_are_not_raw() {
        assert!(is_structured("INFO  [compositor] gnoblin_wm: started"));
        assert!(is_structured("WARN  [guardian] gnoblin_session: slow"));
        assert!(!is_structured("panicked at src/main.rs:3"));
        assert!(!is_structured("INFO started without a role"));
    }

    #[test]
    fn a_full_channel_drops_lines_without_blocking() {
        let (sink, _receiver, dropped) = RawSink::channel();
        for index in 0..CAPACITY + 5 {
            sink.send(RawLine {
                role: "wm".to_owned(),
                stream: OutputStream::Stdout,
                text: index.to_string(),
            });
        }
        assert_eq!(dropped.load(Ordering::Relaxed), 5);
        std::thread::sleep(Duration::from_millis(1));
    }

    #[test]
    fn a_secret_in_a_raw_line_is_redacted_before_it_is_sent() {
        let (sink, receiver, _) = RawSink::channel();
        read_lines(
            "compositor",
            OutputStream::Stdout,
            RawPolicy::Redacted,
            "password=hunter2 ok\n".as_bytes(),
            &sink,
        );
        drop(sink);
        let texts: Vec<String> = receiver.try_iter().map(|line| line.text).collect();
        assert_eq!(texts, ["password=[redacted] ok"]);
    }

    #[test]
    fn a_role_without_raw_capture_drops_nothing_and_logs_nothing() {
        let (sink, receiver, dropped) = RawSink::channel();
        read_lines(
            "services",
            OutputStream::Stdout,
            RawPolicy::Off,
            "one\ntwo\n".as_bytes(),
            &sink,
        );
        drop(sink);
        assert_eq!(receiver.try_iter().count(), 0);
        assert_eq!(dropped.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn only_the_listed_roles_are_captured() {
        assert_eq!(policy("compositor", false), RawPolicy::Redacted);
        assert_eq!(policy("runtime", false), RawPolicy::Redacted);
        assert_eq!(policy("portal", false), RawPolicy::Redacted);
        assert_eq!(policy("services", false), RawPolicy::Off);
        assert_eq!(policy("services", true), RawPolicy::Redacted);
        assert_eq!(policy("front", true), RawPolicy::Off);
        assert_eq!(policy("guardian", true), RawPolicy::Off);
    }
}
