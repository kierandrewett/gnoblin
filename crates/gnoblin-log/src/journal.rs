//! The journal sink of the log: one structured entry per event in the systemd journal.
//!
//! [`JournalLayer`] sends each event over the native journal protocol to `systemd-journald`, which
//! listens on `/run/systemd/journal/socket`, an `AF_UNIX`, `SOCK_DGRAM` socket. A datagram holds one
//! entry as `NAME=value` lines. A value that contains a newline is written as the name, a newline,
//! a little-endian `u64` length, the value bytes and a newline. Names that start with `_` are
//! trusted, so this module never sends one. The protocol is described at
//! <https://systemd.io/JOURNAL_NATIVE_PROTOCOL/>.
//!
//! Each entry carries `MESSAGE`; `PRIORITY`, the syslog priority of the level (see
//! [`syslog_priority`]); `SYSLOG_IDENTIFIER=gnoblin-<role>`; `GNOBLIN_ROLE`, `GNOBLIN_COMPONENT` and
//! `GNOBLIN_TARGET`; and each event field as `GNOBLIN_<NAME>`. Each byte of a field name that is
//! not `A-Z`, `0-9` or `_` becomes `_`, and the name is cut to 64 bytes. That limit is this module's
//! own choice, to keep names readable in `journalctl`.
//!
//! Each datagram is capped at [`MAX_DATAGRAM`], 64 KiB. The protocol can carry a larger entry in a
//! memfd, but this module does not use one. A record over the cap is truncated: the identifier,
//! priority, role, component and target are kept, then `MESSAGE` as far as it fits, then the event
//! fields that still fit, in the order the event recorded them.
//!
//! Failure. The journal socket is never waited on. A send that would block, or that the kernel
//! refuses for lack of buffer space, drops the record and counts it. The count is printed on
//! stderr at most once a second, at the next drop after that interval. Any other send failure,
//! such as a journal that restarted, reconnects once and sends again. If the journal is still
//! unreachable, the layer prints one warning and writes this and later records to stderr in the
//! `<N>` stream format, so nothing is lost without a trace. Each later record tries to connect
//! again, so a journal that comes back takes the records back.

use std::io::{self, Write as _};
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::__tracing::{Event, Level, Subscriber};
use crate::event::{Logged, syslog_priority};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

/// Where `systemd-journald` listens for native-protocol datagrams.
pub const SOCKET_PATH: &str = "/run/systemd/journal/socket";

/// The largest datagram this module sends. A record over it is truncated.
pub const MAX_DATAGRAM: usize = 64 * 1024;

/// The longest field name this module sends, including the `GNOBLIN_` prefix.
const MAX_NAME: usize = 64;

/// The prefix of every event field name, which also keeps a name from starting with `_`.
const PREFIX: &str = "GNOBLIN_";

/// The least time between two reports of dropped records.
const REPORT_INTERVAL: Duration = Duration::from_secs(1);

/// `ENOBUFS` on Linux: the kernel has no buffer space for the datagram.
const ENOBUFS: i32 = 105;

/// The bytes a `MESSAGE` field takes beyond its value in the binary form: the name, a newline, the
/// 8-byte length and a newline. The text form takes fewer, so this bound is always safe.
const BINARY_OVERHEAD: usize = "MESSAGE\n".len() + 8 + 1;

/// A tracing layer that writes each event as one journal entry, falling back to stderr.
pub struct JournalLayer {
    identifier: String,
    role: String,
    path: PathBuf,
    state: Mutex<State>,
}

struct State {
    /// The connected socket, or `None` when the last send failed and the next one must reconnect.
    socket: Option<UnixDatagram>,
    /// Records dropped since the last report.
    dropped: u64,
    /// When the drop count was last printed.
    last_report: Option<Instant>,
    /// Whether the warning about an unreachable journal has been printed since the last success.
    warned: bool,
}

/// What became of one datagram.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// The journal took the entry.
    Delivered,
    /// The journal's buffer was full, so the entry was dropped without waiting.
    Full,
    /// The journal could not be reached, even after reconnecting.
    Unavailable,
}

impl JournalLayer {
    /// Connects to the system journal for `role`, whose entries carry the identifier
    /// `gnoblin-<role>`.
    pub fn connect(role: &str) -> io::Result<Self> {
        let socket = open(Path::new(SOCKET_PATH))?;
        Ok(Self::with_socket(Path::new(SOCKET_PATH), role, Some(socket)))
    }

    fn with_socket(path: &Path, role: &str, socket: Option<UnixDatagram>) -> Self {
        Self {
            identifier: format!("gnoblin-{role}"),
            role: role.to_owned(),
            path: path.to_owned(),
            state: Mutex::new(State {
                socket,
                dropped: 0,
                last_report: None,
                warned: false,
            }),
        }
    }

    /// Acts on what became of one record: counts a drop, warns once when the journal is
    /// unreachable, and writes such a record to stderr.
    fn settle(state: &mut State, outcome: Outcome, logged: &Logged, level: Level) {
        match outcome {
            Outcome::Delivered => state.warned = false,
            Outcome::Full => {
                state.dropped += 1;
                report_drops(state);
            }
            Outcome::Unavailable => {
                if !state.warned {
                    let _ = writeln!(
                        io::stderr().lock(),
                        "gnoblin-log: journal unavailable, writing to stderr"
                    );
                    state.warned = true;
                }
                let _ = writeln!(io::stderr().lock(), "{}", logged.stream_line(level));
            }
        }
    }
}

/// Prints the drop count, unless it was printed less than [`REPORT_INTERVAL`] ago.
fn report_drops(state: &mut State) {
    let now = Instant::now();
    if state
        .last_report
        .is_some_and(|last| now.duration_since(last) < REPORT_INTERVAL)
    {
        return;
    }
    let _ = writeln!(
        io::stderr().lock(),
        "gnoblin-log: journal busy, dropped {} record(s)",
        state.dropped
    );
    state.dropped = 0;
    state.last_report = Some(now);
}

impl<S: Subscriber> Layer<S> for JournalLayer {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let level = *event.metadata().level();
        let logged = Logged::from_event(event);
        let datagram = encode(&self.identifier, &self.role, &logged, level);
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let outcome = deliver(&self.path, &mut state, &datagram);
        Self::settle(&mut state, outcome, &logged, level);
    }
}

/// Sends `datagram` on the connected socket. A send that fails for any reason other than a full
/// buffer drops the socket and reconnects once, then sends again.
fn deliver(path: &Path, state: &mut State, datagram: &[u8]) -> Outcome {
    if let Some(socket) = state.socket.as_ref() {
        match socket.send(datagram) {
            Ok(_) => return Outcome::Delivered,
            Err(error) if is_full(&error) => return Outcome::Full,
            Err(_) => state.socket = None,
        }
    }
    let Ok(socket) = open(path) else {
        return Outcome::Unavailable;
    };
    let outcome = match socket.send(datagram) {
        Ok(_) => Outcome::Delivered,
        Err(error) if is_full(&error) => Outcome::Full,
        Err(_) => return Outcome::Unavailable,
    };
    state.socket = Some(socket);
    outcome
}

/// Whether a send failed because the journal's buffer is full, which drops the record without
/// waiting.
fn is_full(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::WouldBlock || error.raw_os_error() == Some(ENOBUFS)
}

/// Opens a non-blocking datagram socket connected to the journal at `path`.
fn open(path: &Path) -> io::Result<UnixDatagram> {
    let socket = UnixDatagram::unbound()?;
    socket.set_nonblocking(true)?;
    socket.connect(path)?;
    Ok(socket)
}

/// The datagram for one event, in the format described in the module docs.
fn encode(identifier: &str, role: &str, logged: &Logged, level: Level) -> Vec<u8> {
    let mut out = Vec::new();
    let priority = syslog_priority(level).to_string();
    push_if_fits(&mut out, "SYSLOG_IDENTIFIER", identifier);
    push_if_fits(&mut out, "PRIORITY", &priority);
    push_if_fits(&mut out, "GNOBLIN_ROLE", role);
    push_if_fits(&mut out, "GNOBLIN_COMPONENT", &logged.component);
    push_if_fits(&mut out, "GNOBLIN_TARGET", &logged.target);
    push_message(&mut out, &logged.message);
    for (name, value) in &logged.fields {
        push_if_fits(&mut out, &field_name(name), value);
    }
    out
}

/// Writes `MESSAGE`, cut at a character boundary so that it fits in what is left of the datagram.
fn push_message(out: &mut Vec<u8>, message: &str) {
    let room = MAX_DATAGRAM - out.len();
    let mut end = message.len().min(room.saturating_sub(BINARY_OVERHEAD));
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    push_field(out, "MESSAGE", &message[..end]);
}

/// Writes one field if the whole field fits in what is left of the datagram.
fn push_if_fits(out: &mut Vec<u8>, key: &str, value: &str) {
    if field_len(key, value) <= MAX_DATAGRAM - out.len() {
        push_field(out, key, value);
    }
}

/// The bytes one field takes in the text form, or in the binary form when the value has a newline.
fn field_len(key: &str, value: &str) -> usize {
    if value.contains('\n') {
        key.len() + 1 + 8 + value.len() + 1
    } else {
        key.len() + 1 + value.len() + 1
    }
}

fn push_field(out: &mut Vec<u8>, key: &str, value: &str) {
    out.extend_from_slice(key.as_bytes());
    if value.contains('\n') {
        out.push(b'\n');
        out.extend_from_slice(&u64::try_from(value.len()).unwrap_or(u64::MAX).to_le_bytes());
    } else {
        out.push(b'=');
    }
    out.extend_from_slice(value.as_bytes());
    out.push(b'\n');
}

/// The journal field name for an event field `name`: `GNOBLIN_` and the name in upper case, with
/// each character that is not an ASCII letter or digit as `_`, cut to 64 bytes. Every character is
/// then ASCII, so the cut is always on a character boundary.
fn field_name(name: &str) -> String {
    let mut key = String::from(PREFIX);
    for character in name.chars() {
        key.push(if character.is_ascii_alphanumeric() {
            character.to_ascii_uppercase()
        } else {
            '_'
        });
    }
    key.truncate(MAX_NAME);
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(message: &str, fields: Vec<(String, String)>) -> Logged {
        Logged {
            component: "wayland".to_owned(),
            message: message.to_owned(),
            target: "gnoblin_wayland".to_owned(),
            fields,
        }
    }

    fn text(datagram: &[u8]) -> String {
        String::from_utf8_lossy(datagram).into_owned()
    }

    /// A path in the temp directory that no other test or run uses.
    fn scratch_path(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("gnoblin-log-journal-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&directory);
        directory.join(name)
    }

    #[test]
    fn an_entry_carries_the_identifier_priority_component_and_message() {
        let datagram = encode(
            "gnoblin-compositor",
            "compositor",
            &sample("ready", Vec::new()),
            Level::INFO,
        );
        let text = text(&datagram);
        assert!(text.contains("SYSLOG_IDENTIFIER=gnoblin-compositor\n"));
        assert!(text.contains("PRIORITY=6\n"));
        assert!(text.contains("GNOBLIN_ROLE=compositor\n"));
        assert!(text.contains("GNOBLIN_COMPONENT=wayland\n"));
        assert!(text.contains("GNOBLIN_TARGET=gnoblin_wayland\n"));
        assert!(text.contains("MESSAGE=ready\n"));
    }

    #[test]
    fn a_message_with_a_newline_uses_the_binary_form() {
        let datagram = encode("x", "x", &sample("one\ntwo", Vec::new()), Level::ERROR);
        let mut expected = b"MESSAGE\n".to_vec();
        expected.extend_from_slice(&7_u64.to_le_bytes());
        expected.extend_from_slice(b"one\ntwo\n");
        assert!(
            datagram
                .windows(expected.len())
                .any(|window| window == expected.as_slice())
        );
    }

    #[test]
    fn field_names_are_upper_case_and_valid() {
        assert_eq!(field_name("output"), "GNOBLIN_OUTPUT");
        assert_eq!(field_name("output-name"), "GNOBLIN_OUTPUT_NAME");
        assert_eq!(field_name("1st"), "GNOBLIN_1ST");
        assert_eq!(field_name("w\u{e9}"), "GNOBLIN_W_");
        assert_eq!(field_name(&"a".repeat(100)).len(), MAX_NAME);
    }

    #[test]
    fn an_oversized_record_is_truncated_to_the_cap_with_its_message() {
        let fields = vec![("big".to_owned(), "b".repeat(MAX_DATAGRAM))];
        let datagram = encode("x", "x", &sample(&"a".repeat(MAX_DATAGRAM * 2), fields), Level::INFO);
        assert!(datagram.len() <= MAX_DATAGRAM);
        assert!(text(&datagram).contains("MESSAGE=aaa"));
        assert!(!text(&datagram).contains("GNOBLIN_BIG="));
    }

    #[test]
    fn a_message_cut_in_the_middle_of_a_character_stays_valid() {
        let datagram = encode(
            "x",
            "x",
            &sample(&"\u{e9}".repeat(MAX_DATAGRAM), Vec::new()),
            Level::INFO,
        );
        assert!(datagram.len() <= MAX_DATAGRAM);
        assert!(std::str::from_utf8(&datagram).is_ok());
    }

    #[test]
    fn a_missing_journal_is_unavailable_then_delivers_once_it_returns() {
        let path = scratch_path("missing.sock");
        let _ = std::fs::remove_file(&path);
        let mut state = State {
            socket: None,
            dropped: 0,
            last_report: None,
            warned: false,
        };
        let datagram = encode("x", "x", &sample("early", Vec::new()), Level::INFO);
        assert_eq!(deliver(&path, &mut state, &datagram), Outcome::Unavailable);
        assert!(state.socket.is_none());

        let receiver = UnixDatagram::bind(&path).expect("bind receiver");
        assert_eq!(deliver(&path, &mut state, &datagram), Outcome::Delivered);
        let mut buffer = vec![0; MAX_DATAGRAM];
        let received = receiver.recv(&mut buffer).expect("receive");
        assert!(text(&buffer[..received]).contains("MESSAGE=early\n"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_full_journal_drops_the_record_without_blocking_and_counts_it() {
        let path = scratch_path("full.sock");
        let _ = std::fs::remove_file(&path);
        let _receiver = UnixDatagram::bind(&path).expect("bind receiver");
        let layer = JournalLayer::with_socket(&path, "x", Some(open(&path).expect("open")));
        let datagram = vec![b'a'; MAX_DATAGRAM];
        let mut state = layer.state.lock().expect("lock");
        let mut outcome = Outcome::Delivered;
        for _ in 0..10_000 {
            outcome = deliver(&path, &mut state, &datagram);
            if outcome != Outcome::Delivered {
                break;
            }
        }
        assert_eq!(outcome, Outcome::Full, "the unread receiver must fill up");
        let logged = sample("late", Vec::new());
        JournalLayer::settle(&mut state, Outcome::Full, &logged, Level::INFO);
        JournalLayer::settle(&mut state, Outcome::Full, &logged, Level::INFO);
        JournalLayer::settle(&mut state, Outcome::Full, &logged, Level::INFO);
        // The first drop is reported at once and resets the count; the next two fall inside the
        // report interval, so they are counted and not printed.
        assert_eq!(state.dropped, 2);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_restarted_journal_is_reconnected_on_the_next_send() {
        let path = scratch_path("restart.sock");
        let _ = std::fs::remove_file(&path);
        let first = UnixDatagram::bind(&path).expect("bind first receiver");
        let mut state = State {
            socket: Some(open(&path).expect("open")),
            dropped: 0,
            last_report: None,
            warned: false,
        };
        drop(first);
        std::fs::remove_file(&path).expect("remove the old socket");

        let second = UnixDatagram::bind(&path).expect("bind second receiver");
        let datagram = encode("x", "x", &sample("after restart", Vec::new()), Level::INFO);
        assert_eq!(deliver(&path, &mut state, &datagram), Outcome::Delivered);
        let mut buffer = vec![0; MAX_DATAGRAM];
        let received = second.recv(&mut buffer).expect("receive");
        assert!(text(&buffer[..received]).contains("MESSAGE=after restart\n"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_warning_is_printed_once_and_rearmed_by_a_delivery() {
        let logged = sample("lost", Vec::new());
        let mut state = State {
            socket: None,
            dropped: 0,
            last_report: None,
            warned: false,
        };
        JournalLayer::settle(&mut state, Outcome::Unavailable, &logged, Level::WARN);
        assert!(state.warned);
        JournalLayer::settle(&mut state, Outcome::Unavailable, &logged, Level::WARN);
        assert!(state.warned);
        JournalLayer::settle(&mut state, Outcome::Delivered, &logged, Level::WARN);
        assert!(!state.warned);
    }
}
