//! The console: a live line into the running JavaScript shell.
//!
//! [`JsConsole`] listens on a Unix socket beside the owner socket (`js.sock` in the same directory, see
//! [`console_socket_path`]) and evaluates what a connection sends in the shell's own generation, with the same
//! `gnoblin` global as `init.ts`. It is what `gnoblinctl console` talks to.
//!
//! # Protocol
//!
//! Newline-delimited JSON. A request is `{"id": <any>, "expr": "<source>"}`; the reply carries the same `id`:
//!
//! - `{"id", "ok": true, "type", "value", "text", "truncated"}`: `value` is the result as plain JSON data (absent
//!   for `undefined`), `text` is how a terminal shows it, `type` names it, `truncated` says the size cap cut
//!   something off. The renderer is `data/js/gnoblin/inspect.ts`.
//! - `{"id", "ok": false, "error": {"kind", "message", "location", "stack"}}`: `kind` is `exception`, `budget`,
//!   `memory`, `request` or `engine`, `location` is `{"file", "line", "column"}` (the file of a console expression
//!   is `<console>`) or null.
//! - Both replies carry `output`, what the expression printed through `console.log` and its siblings: a list of
//!   `{"level", "site", "text"}` in the order they were logged (`level` is `debug`, `info`, `warn` or `error`; `site`
//!   is `<console>:line:col` or null). The same lines are in the log; `output` is how the prompt shows them.
//!
//! # Where an expression runs
//!
//! The server is polled by the role's loop ([`JsRole::tick`](crate::JsRole::tick)) before each frame and never
//! from another thread, so an expression runs between frames and never beside one: the state it reads is the
//! state the next frame sees, and a write it makes (`gnoblin.clients.at(0).x = 10`) travels in that frame's
//! transaction. Each expression has the shell's entry budget; one that runs past it (`while (true) {}`) is
//! interrupted and answered with a `budget` error. A console expression is not a strike against the generation:
//! the person at the prompt is not a listener the supervisor should restage the shell over.
//!
//! A `let`, `const` or `var` at the prompt, or an assignment to an undeclared name, is a global of the running
//! generation and stays for the next expression, from any connection. A reload starts a new generation, so what
//! the console defined is gone with the old one; nothing the console does is written to disk.
//!
//! # Who may connect
//!
//! The socket is mode 0600 and the peer's uid is read from the kernel (`SO_PEERCRED`) on every accept: only the
//! uid the shell runs as is served. Another uid, or a peer whose credentials cannot be read, is closed without a
//! word. A request line is at most [`MAX_REQUEST`] bytes, and a connection that does not read its replies is
//! not evaluated for.
//!
//! The module is split by concern. This file is the socket: the listener, the connections and the framing; `eval`
//! evaluates one request in the generation and renders the answer.

use std::io;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use gnoblin_client::unix::socket_path;
use serde_json::Value as Json;
use thiserror::Error;

use self::eval::{answer, failure};
use crate::shell::JsShell;

mod eval;

pub use self::eval::evaluate_in;

gnoblin_log::logger!(log = "js");

/// The console socket's file name, in the directory of the owner socket.
pub const CONSOLE_SOCKET: &str = "js.sock";

/// The variable that turns the console off when it is `0`.
pub const CONSOLE_VARIABLE: &str = "GNOBLIN_JS_CONSOLE";

/// The name a console expression has in a stack trace and in an error's location.
pub const CONSOLE_FILE: &str = "<console>";

/// The longest request line served.
pub const MAX_REQUEST: usize = 1 << 20;

/// The most text a rendering holds before it is cut off, unless the console is built with another limit.
pub const DEFAULT_MAX_BYTES: usize = 64 * 1024;

/// How many connections are held at once.
const MAX_CONNECTIONS: usize = 16;

/// How many expressions one [`JsConsole::serve`] evaluates, so a busy connection cannot starve the frame loop.
const REQUESTS_PER_TURN: usize = 4;

/// How many promise jobs an expression may run before its value is read; a promise that is settled by then shows
/// its result, one that is not shows as pending.
const MAX_JOBS: usize = 1000;

/// What went wrong setting the console up. None of these ends the shell: it runs without a console.
#[derive(Debug, Error)]
pub enum JsConsoleError {
    /// The socket path could not be decided.
    #[error("{0}")]
    Path(String),
    /// Another shell already answers on the path.
    #[error("another JavaScript shell already serves its console at {}", path.display())]
    Busy {
        /// The socket that is in use.
        path: PathBuf,
    },
    /// The socket could not be created.
    #[error("cannot listen on {}: {source}", path.display())]
    Bind {
        /// The socket path.
        path: PathBuf,
        /// What the system said.
        source: io::Error,
    },
}

/// Where a role's console listens.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum JsConsoleSite {
    /// Beside the owner socket the environment names, unless `GNOBLIN_JS_CONSOLE` is `0`. A role that is not
    /// connected through an environment (a test over a fake link) has no console under this setting.
    #[default]
    Environment,
    /// No console.
    Off,
    /// At this path.
    At(PathBuf),
}

/// Whether the environment leaves the console on: it is, unless `GNOBLIN_JS_CONSOLE` is `0`.
pub fn console_enabled(env: &dyn Fn(&str) -> Option<String>) -> bool {
    env(CONSOLE_VARIABLE).as_deref() != Some("0")
}

/// The console socket for `env`: `js.sock` in the directory of the owner socket, so it follows
/// `GNOBLIN_IPC_SOCKET` and `XDG_RUNTIME_DIR` exactly as every client finds the owner socket.
///
/// # Errors
///
/// The message `socket_path` gives when neither variable is set.
pub fn console_socket_path(env: &dyn Fn(&str) -> Option<String>) -> Result<PathBuf, String> {
    socket_path(env).map(|owner| owner.with_file_name(CONSOLE_SOCKET))
}

/// Whether a peer with `peer_uid` may use the console of a shell running as `own_uid`. No credentials is no.
pub const fn peer_allowed(peer_uid: Option<u32>, own_uid: u32) -> bool {
    match peer_uid {
        Some(peer) => peer == own_uid,
        None => false,
    }
}

/// The kernel's record of who is on the other end of `stream`, checked against this process's uid.
fn stream_allowed(stream: &UnixStream) -> bool {
    let peer = rustix::net::sockopt::socket_peercred(stream)
        .ok()
        .map(|credentials| credentials.uid.as_raw());
    peer_allowed(peer, rustix::process::getuid().as_raw())
}

/// One client of the console.
struct JsConsoleConnection {
    stream: UnixStream,
    /// Bytes read and not yet a whole line.
    input: Vec<u8>,
    /// Replies not yet written.
    output: Vec<u8>,
    /// The peer has closed its end, or must be closed after the output drains.
    closing: bool,
}

impl JsConsoleConnection {
    /// Reads what the peer has sent, up to the request limit.
    fn fill(&mut self) {
        let mut chunk = [0u8; 8192];
        while self.input.len() <= MAX_REQUEST {
            match self.stream.read(&mut chunk) {
                Ok(0) => {
                    self.closing = true;
                    break;
                }
                Ok(count) => self.input.extend_from_slice(&chunk[..count]),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => {
                    self.closing = true;
                    self.output.clear();
                    break;
                }
            }
        }
    }

    /// Writes as much of the output as the socket takes. `false` when the peer is gone.
    fn flush(&mut self) -> bool {
        while !self.output.is_empty() {
            match self.stream.write(&self.output) {
                Ok(0) => return false,
                Ok(count) => {
                    self.output.drain(..count);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return true,
                Err(_) => return false,
            }
        }
        true
    }

    /// The next whole request line, without its newline; an empty line is skipped.
    fn take_line(&mut self) -> Option<Vec<u8>> {
        loop {
            let end = self.input.iter().position(|byte| *byte == b'\n')?;
            let mut line: Vec<u8> = self.input.drain(..=end).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if !line.iter().all(u8::is_ascii_whitespace) {
                return Some(line);
            }
        }
    }

    fn reply(&mut self, reply: &Json) {
        // A `Value` always serialises.
        if let Ok(mut text) = serde_json::to_vec(reply) {
            text.push(b'\n');
            self.output.extend_from_slice(&text);
        }
    }
}

/// The console server: a listening socket and the connections it has accepted.
pub struct JsConsole {
    listener: UnixListener,
    path: PathBuf,
    connections: Vec<JsConsoleConnection>,
    max_bytes: usize,
}

impl JsConsole {
    /// Listens on `path`, mode 0600. A socket file left behind by a shell that died is replaced; one that still
    /// answers is [`JsConsoleError::Busy`].
    ///
    /// # Errors
    ///
    /// The path is in use, or the socket could not be made.
    pub fn bind(path: &Path) -> Result<Self, JsConsoleError> {
        let bind_error = |source| JsConsoleError::Bind {
            path: path.to_path_buf(),
            source,
        };
        if path.symlink_metadata().is_ok() {
            match UnixStream::connect(path) {
                Ok(_) => {
                    return Err(JsConsoleError::Busy {
                        path: path.to_path_buf(),
                    });
                }
                Err(error) if matches!(error.kind(), io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound) => {
                    std::fs::remove_file(path).map_err(bind_error)?;
                }
                Err(error) => return Err(bind_error(error)),
            }
        }
        let listener = UnixListener::bind(path).map_err(bind_error)?;
        // The runtime directory is 0700, so nobody else reaches the socket between the bind and this; the
        // credential check on every accept is what holds even if the directory were not.
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(bind_error)?;
        listener.set_nonblocking(true).map_err(bind_error)?;
        Ok(Self {
            listener,
            path: path.to_path_buf(),
            connections: Vec::new(),
            max_bytes: DEFAULT_MAX_BYTES,
        })
    }

    /// Sets the size cap of a rendering, in bytes of text.
    #[must_use]
    pub const fn with_max_bytes(mut self, max_bytes: usize) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    /// Where the console listens.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// How many connections are open.
    pub const fn connections(&self) -> usize {
        self.connections.len()
    }

    /// Accepts connections, reads requests and evaluates up to a handful of them in `shell`, between frames.
    /// Never blocks: the loop calls it every turn.
    pub fn serve(&mut self, shell: &JsShell) {
        self.accept();
        let mut served = 0;
        for connection in &mut self.connections {
            connection.fill();
            if !connection.flush() {
                connection.closing = true;
                connection.output.clear();
                continue;
            }
            while connection.output.is_empty() && served < REQUESTS_PER_TURN {
                if connection.input.len() > MAX_REQUEST && !connection.input.contains(&b'\n') {
                    connection.input.clear();
                    connection.closing = true;
                    connection.reply(&failure(
                        &Json::Null,
                        "request",
                        &format!("a request is at most {MAX_REQUEST} bytes"),
                    ));
                    break;
                }
                let Some(line) = connection.take_line() else { break };
                let reply = answer(shell, &line, self.max_bytes);
                connection.reply(&reply);
                served += 1;
                if !connection.flush() {
                    connection.closing = true;
                    connection.output.clear();
                }
            }
        }
        self.connections
            .retain(|connection| !(connection.closing && connection.output.is_empty()));
    }

    fn accept(&mut self) {
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    if self.connections.len() >= MAX_CONNECTIONS || !stream_allowed(&stream) {
                        log::warn!(area = "console", "a console connection was refused");
                        continue;
                    }
                    if stream.set_nonblocking(true).is_err() {
                        continue;
                    }
                    self.connections.push(JsConsoleConnection {
                        stream,
                        input: Vec::new(),
                        output: Vec::new(),
                        closing: false,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return,
                Err(error) => {
                    log::warn!(area = "console", "accepting a console connection failed: {error}");
                    return;
                }
            }
        }
    }
}

impl Drop for JsConsole {
    fn drop(&mut self) {
        // The file is ours: `bind` replaced any stale one and refused a live one.
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests;
