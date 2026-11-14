//! The conversation with `polkit-agent-helper-1`.
//!
//! The helper is polkit's setuid-root program that runs the PAM stack for one user. libpolkit-agent's
//! `PolkitAgentSession` is the reference for how it is driven (design/js-research/polkit.md), and this is the same
//! protocol:
//!
//! - It is started as `polkit-agent-helper-1 <user name>`, with the cookie as the first line of its standard
//!   input (never in its arguments, where other processes could read it).
//! - It writes one line per event to its standard output, with the text escaped like `g_strescape`:
//!   `PAM_PROMPT_ECHO_OFF <text>` and `PAM_PROMPT_ECHO_ON <text>` ask for an answer, `PAM_TEXT_INFO <text>` and
//!   `PAM_ERROR_MSG <text>` only tell, and the last line is `SUCCESS` or `FAILURE`.
//! - An answer to a prompt is one line on its standard input.
//! - On `SUCCESS` the helper has already told polkitd itself (`AuthenticationAgentResponse2`, which polkitd only
//!   accepts from root). Nothing here tells polkitd anything.
//!
//! # Failure modes first
//!
//! 1. **The helper is missing or not runnable.** [`locate_helper`] and [`HelperSession::spawn`] return a
//!    [`HelperError`] that names what was tried, so the dialog can say so and the request ends unauthenticated.
//! 2. **The helper ends without saying `SUCCESS`, or says something that is not the protocol.** Both are
//!    [`HelperEvent::Failure`]: only the exact line `SUCCESS` is a success.
//! 3. **The session is dropped while the helper runs** (the user cancelled, the request timed out, polkitd
//!    cancelled it). The helper's input is closed and it is killed and reaped, so none outlives its request.
//! 4. **A password leaks.** It is only ever a [`SecretText`] handed to [`HelperSession::respond`], which writes
//!    its bytes to the helper's standard input and keeps nothing; no type here formats one.

use std::fmt;
use std::io::{self, BufRead as _, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use gnoblin_secret::SecretText;

use crate::log;

/// The environment variable that names the helper to run, for tests and for a distribution that keeps it
/// somewhere unusual. When it is set and names nothing runnable, no other helper is tried.
pub const HELPER_ENV: &str = "GNOBLIN_POLKIT_HELPER";

/// Where distributions install the helper, in the order they are tried. polkit compiles its libexec directory in
/// and does not report it, so these are the known places: the NixOS setuid wrapper, then `/usr/lib` (Arch, Fedora,
/// openSUSE, current Debian), `/usr/libexec` and the older Debian name.
pub const HELPER_CANDIDATES: [&str; 4] = [
    "/run/wrappers/bin/polkit-agent-helper-1",
    "/usr/lib/polkit-1/polkit-agent-helper-1",
    "/usr/libexec/polkit-1/polkit-agent-helper-1",
    "/usr/lib/policykit-1/polkit-agent-helper-1",
];

/// The longest line read from the helper, in bytes. A PAM message is a sentence.
const MAX_LINE: u64 = 64 * 1024;

/// Why the helper could not be started or written to.
#[derive(Debug, thiserror::Error)]
pub enum HelperError {
    /// No helper is installed where it is known to live, or the override names nothing.
    #[error("polkit-agent-helper-1 was not found ({looked}); install polkit or set GNOBLIN_POLKIT_HELPER")]
    NotFound {
        /// What was looked at, for the message.
        looked: String,
    },
    /// The helper exists and could not be started.
    #[error("cannot start {path}: {source}")]
    Spawn {
        /// The helper that failed to start.
        path: String,
        /// Why.
        #[source]
        source: io::Error,
    },
    /// The helper no longer reads its input.
    #[error("the polkit helper is not accepting an answer: {0}")]
    Write(#[source] io::Error),
}

/// What the helper said, in the order it said it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HelperEvent {
    /// PAM asks for an answer. `echo` is whether it may be shown as typed (a username, a code) and not masked.
    Prompt {
        /// What PAM asks, for example `Password: `.
        text: String,
        /// Whether the answer may be echoed.
        echo: bool,
    },
    /// PAM says something: a fingerprint reader waiting, a notice.
    Info(String),
    /// PAM reports an error: a lockout, a bad password.
    Error(String),
    /// The user authenticated, and the helper told polkitd so.
    Success,
    /// The user did not authenticate, or the helper ended or misspoke. No more events follow.
    Failure,
}

/// The helper to run: `HELPER_ENV` when set, else the first of [`HELPER_CANDIDATES`] that exists.
///
/// # Errors
///
/// [`HelperError::NotFound`] when there is none.
pub fn locate_helper() -> Result<PathBuf, HelperError> {
    locate_in(std::env::var_os(HELPER_ENV).map(PathBuf::from), &HELPER_CANDIDATES)
}

fn locate_in(over: Option<PathBuf>, candidates: &[&str]) -> Result<PathBuf, HelperError> {
    if let Some(path) = over.filter(|path| !path.as_os_str().is_empty()) {
        // An override that names nothing must not fall back to the real helper: a test that points at a fake
        // one should fail, not authenticate against the machine's PAM.
        return if path.is_file() {
            Ok(path)
        } else {
            Err(HelperError::NotFound {
                looked: format!("{HELPER_ENV} is {}", path.display()),
            })
        };
    }
    candidates
        .iter()
        .map(Path::new)
        .find(|path| path.is_file())
        .map(Path::to_path_buf)
        .ok_or_else(|| HelperError::NotFound {
            looked: format!("looked in {}", candidates.join(", ")),
        })
}

/// One run of the helper for one user and one cookie.
///
/// The helper is a short conversation: it asks, [`HelperSession::respond`] answers, and it ends in
/// [`HelperEvent::Success`] or [`HelperEvent::Failure`]. A failed run is over; a retry is a new session, as it is
/// in libpolkit-agent.
pub struct HelperSession {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Option<String>>,
    done: bool,
}

impl fmt::Debug for HelperSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HelperSession")
            .field("pid", &self.child.id())
            .field("done", &self.done)
            .finish_non_exhaustive()
    }
}

impl HelperSession {
    /// Starts `helper` for `user`, naming the authentication by `cookie`.
    ///
    /// # Errors
    ///
    /// [`HelperError::Spawn`] when the helper cannot be started, and [`HelperError::Write`] when it ends before it
    /// reads its cookie.
    pub fn spawn(helper: &Path, user: &str, cookie: &str) -> Result<Self, HelperError> {
        let mut child = Command::new(helper)
            .arg(user)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|source| HelperError::Spawn {
                path: helper.display().to_string(),
                source,
            })?;
        let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(HelperError::Write(io::Error::other("the helper has no pipes")));
        };
        // The cookie goes on standard input so that it cannot be read from the process list.
        let sent = stdin
            .write_all(cookie.as_bytes())
            .and_then(|()| stdin.write_all(b"\n"))
            .and_then(|()| stdin.flush());
        if let Err(source) = sent {
            let _ = child.kill();
            let _ = child.wait();
            return Err(HelperError::Write(source));
        }
        let (sender, lines) = channel();
        let reader = std::thread::Builder::new()
            .name("polkit-helper".to_owned())
            .spawn(move || {
                let mut reader = io::BufReader::new(stdout);
                loop {
                    let mut raw = Vec::new();
                    let read = reader.by_ref().take(MAX_LINE).read_until(b'\n', &mut raw);
                    match read {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            let line = String::from_utf8_lossy(&raw).trim_end_matches('\n').to_owned();
                            if sender.send(Some(line)).is_err() {
                                return;
                            }
                        }
                    }
                }
                let _ = sender.send(None);
            });
        if let Err(source) = reader {
            let _ = child.kill();
            let _ = child.wait();
            return Err(HelperError::Spawn {
                path: helper.display().to_string(),
                source,
            });
        }
        log::debug!(pid = child.id(), "started the polkit helper");
        Ok(Self {
            child,
            stdin: Some(stdin),
            lines,
            done: false,
        })
    }

    /// Everything the helper said since the last call, without waiting. After a result nothing more is returned.
    pub fn poll(&mut self) -> Vec<HelperEvent> {
        let mut events = Vec::new();
        while !self.done {
            match self.lines.try_recv() {
                Ok(Some(line)) => {
                    let event = parse_line(&line);
                    self.done = matches!(event, HelperEvent::Success | HelperEvent::Failure);
                    events.push(event);
                }
                // The output ended with no result: the helper died or was killed.
                Ok(None) | Err(TryRecvError::Disconnected) => {
                    self.done = true;
                    events.push(HelperEvent::Failure);
                }
                Err(TryRecvError::Empty) => break,
            }
        }
        if self.done {
            // The helper has nothing left to read: let it go before the session is dropped.
            self.stdin = None;
        }
        events
    }

    /// Whether the helper has reported its result.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Answers the prompt the helper is waiting on. The text is written to the helper and kept nowhere.
    ///
    /// # Errors
    ///
    /// [`HelperError::Write`] when the helper has ended.
    pub fn respond(&mut self, answer: &SecretText) -> Result<(), HelperError> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| HelperError::Write(io::Error::from(io::ErrorKind::BrokenPipe)))?;
        stdin
            .write_all(answer.expose().as_bytes())
            .and_then(|()| stdin.write_all(b"\n"))
            .and_then(|()| stdin.flush())
            .map_err(HelperError::Write)
    }
}

impl Drop for HelperSession {
    fn drop(&mut self) {
        // The helper's input is closed first so a helper that is reading ends by itself, then it is killed in
        // case it is not, and reaped so it does not stay a zombie.
        self.stdin = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Reads one line the helper wrote.
///
/// A line that is not part of the protocol is a [`HelperEvent::Failure`], as it is for libpolkit-agent: a helper
/// that speaks something else is not one to trust with a success.
#[must_use]
pub fn parse_line(raw: &str) -> HelperEvent {
    let line = compress(raw);
    if let Some(text) = line.strip_prefix("PAM_PROMPT_ECHO_OFF ") {
        HelperEvent::Prompt {
            text: text.to_owned(),
            echo: false,
        }
    } else if let Some(text) = line.strip_prefix("PAM_PROMPT_ECHO_ON ") {
        HelperEvent::Prompt {
            text: text.to_owned(),
            echo: true,
        }
    } else if let Some(text) = line.strip_prefix("PAM_ERROR_MSG ") {
        HelperEvent::Error(text.to_owned())
    } else if let Some(text) = line.strip_prefix("PAM_TEXT_INFO ") {
        HelperEvent::Info(text.to_owned())
    } else if line == "SUCCESS" {
        HelperEvent::Success
    } else {
        HelperEvent::Failure
    }
}

/// Undoes `g_strescape`, which is how the helper writes a message: `\n`, `\t` and the like, `\"`, `\\`, and a
/// backslash with up to three octal digits for any other byte.
fn compress(raw: &str) -> String {
    let mut bytes: Vec<u8> = Vec::with_capacity(raw.len());
    let mut chars = raw.bytes().peekable();
    while let Some(byte) = chars.next() {
        if byte != b'\\' {
            bytes.push(byte);
            continue;
        }
        let Some(next) = chars.next() else {
            bytes.push(b'\\');
            break;
        };
        match next {
            b'b' => bytes.push(0x08),
            b'f' => bytes.push(0x0c),
            b'n' => bytes.push(b'\n'),
            b'r' => bytes.push(b'\r'),
            b't' => bytes.push(b'\t'),
            b'v' => bytes.push(0x0b),
            b'0'..=b'7' => {
                let mut value = u32::from(next - b'0');
                for _ in 0..2 {
                    match chars.peek().copied() {
                        Some(digit @ b'0'..=b'7') => {
                            value = value * 8 + u32::from(digit - b'0');
                            chars.next();
                        }
                        _ => break,
                    }
                }
                bytes.push(u8::try_from(value & 0xff).unwrap_or(b'?'));
            }
            other => bytes.push(other),
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_line_of_the_protocol_is_read() {
        assert_eq!(
            parse_line("PAM_PROMPT_ECHO_OFF Password: "),
            HelperEvent::Prompt {
                text: "Password: ".to_owned(),
                echo: false
            }
        );
        assert_eq!(
            parse_line("PAM_PROMPT_ECHO_ON Code: "),
            HelperEvent::Prompt {
                text: "Code: ".to_owned(),
                echo: true
            }
        );
        assert_eq!(
            parse_line("PAM_TEXT_INFO Place your finger on the reader"),
            HelperEvent::Info("Place your finger on the reader".to_owned())
        );
        assert_eq!(
            parse_line("PAM_ERROR_MSG Authentication failure"),
            HelperEvent::Error("Authentication failure".to_owned())
        );
        assert_eq!(parse_line("SUCCESS"), HelperEvent::Success);
        assert_eq!(parse_line("FAILURE"), HelperEvent::Failure);
    }

    #[test]
    fn only_the_exact_success_line_is_a_success() {
        for line in [
            "SUCCESSFUL",
            "success",
            "SUCCESS ",
            " SUCCESS",
            "",
            "PAM_TEXT_INFO",
            "something else",
        ] {
            assert_eq!(parse_line(line), HelperEvent::Failure, "{line:?}");
        }
    }

    #[test]
    fn escapes_are_undone_as_g_strcompress_does() {
        assert_eq!(
            parse_line(r#"PAM_ERROR_MSG Account \"locked\"\nTry in 5\tminutes \\ \303\251"#),
            HelperEvent::Error("Account \"locked\"\nTry in 5\tminutes \\ \u{e9}".to_owned())
        );
    }

    #[test]
    fn an_override_that_names_nothing_does_not_fall_back() {
        let error = locate_in(Some(PathBuf::from("/nonexistent/helper")), &["/bin/sh"]).expect_err("nothing there");
        assert!(error.to_string().contains("/nonexistent/helper"), "{error}");
        assert_eq!(
            locate_in(None, &["/nonexistent/helper", "/bin/sh"]).expect("the second candidate exists"),
            PathBuf::from("/bin/sh")
        );
        assert!(locate_in(None, &["/nonexistent/helper"]).is_err());
    }
}
