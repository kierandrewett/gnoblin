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

use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use gnoblin_client::unix::socket_path;
use rquickjs::context::EvalOptions;
use rquickjs::{Ctx, Function, Object, Value};
use serde_json::{Map, Value as Json, json};
use thiserror::Error;

use crate::error::{JsError, JsErrorKind};
use crate::generation::JsGeneration;
use crate::script_console::JsConsoleLine;
use crate::shell::JsShell;

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

/// The reply to one request line.
fn answer(shell: &JsShell, line: &[u8], max_bytes: usize) -> Json {
    let request: Json = match serde_json::from_slice(line) {
        Ok(request) => request,
        Err(error) => {
            return failure(
                &Json::Null,
                "request",
                &format!("a request is one JSON object per line: {error}"),
            );
        }
    };
    let id = request.get("id").cloned().unwrap_or(Json::Null);
    let Some(expr) = request.get("expr").and_then(Json::as_str) else {
        return failure(&id, "request", "a request needs an `expr` string");
    };
    let (generation, budget) = shell.console_target();
    let (outcome, output) = evaluate_capturing(generation, budget, expr, max_bytes);
    let mut reply = match outcome {
        Ok(rendered) => match serde_json::from_str::<Map<String, Json>>(&rendered) {
            Ok(mut reply) => {
                reply.insert("id".to_owned(), id);
                reply.insert("ok".to_owned(), Json::Bool(true));
                Json::Object(reply)
            }
            Err(error) => failure(
                &id,
                "engine",
                &format!("the renderer returned what is not JSON: {error}"),
            ),
        },
        Err(error) => error_reply(&id, &error),
    };
    // What the expression logged through `console`, in order, whether it ended well or not.
    reply["output"] = output_json(&output);
    reply
}

fn output_json(lines: &[JsConsoleLine]) -> Json {
    Json::Array(
        lines
            .iter()
            .map(|line| json!({"level": line.level.name(), "site": line.site, "text": line.text}))
            .collect(),
    )
}

fn failure(id: &Json, kind: &str, message: &str) -> Json {
    json!({"id": id, "ok": false, "error": {"kind": kind, "message": message, "location": null, "stack": null}})
}

fn error_reply(id: &Json, error: &JsError) -> Json {
    let kind = match error.kind {
        JsErrorKind::Budget => "budget",
        JsErrorKind::Memory => "memory",
        JsErrorKind::Exception => "exception",
        JsErrorKind::Ts | JsErrorKind::Load | JsErrorKind::Engine => "engine",
    };
    // The place that matters is the one in what the person typed, not a frame of the library under it.
    let location = error
        .stack
        .as_deref()
        .and_then(console_location)
        .map(|(line, column)| json!({"file": CONSOLE_FILE, "line": line, "column": column}))
        .or_else(|| {
            error
                .location
                .as_ref()
                .map(|at| json!({"file": at.file.display().to_string(), "line": at.line, "column": at.column}))
        })
        .unwrap_or(Json::Null);
    json!({
        "id": id,
        "ok": false,
        "error": {"kind": kind, "message": error.message, "location": location, "stack": error.stack},
    })
}

/// The line and column of the first stack frame in a console expression.
fn console_location(stack: &str) -> Option<(u32, u32)> {
    stack.lines().find_map(|frame| {
        let rest = &frame[frame.find(CONSOLE_FILE)? + CONSOLE_FILE.len()..];
        let mut parts = rest.strip_prefix(':')?.trim_end_matches(')').split(':');
        Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
    })
}

/// Evaluates `source` as a script in the running generation of `shell` and renders its value, as the text of the
/// reply's JSON. Not counted as a strike when it runs past its budget.
pub fn evaluate_in(shell: &JsShell, source: &str, max_bytes: usize) -> Result<String, JsError> {
    let (generation, budget) = shell.console_target();
    evaluate_capturing(generation, budget, source, max_bytes).0
}

/// [`evaluate`], with the lines the expression logged through `console` kept apart from the log's own record, so the
/// reply can carry them to the prompt.
fn evaluate_capturing(
    generation: &JsGeneration,
    budget: Duration,
    source: &str,
    max_bytes: usize,
) -> (Result<String, JsError>, Vec<JsConsoleLine>) {
    generation.capture_console();
    let outcome = evaluate(generation, budget, source, max_bytes);
    (outcome, generation.take_console())
}

fn evaluate(generation: &JsGeneration, budget: Duration, source: &str, max_bytes: usize) -> Result<String, JsError> {
    let source = as_expression(source);
    generation.call_budgeted(budget, |ctx| {
        // Sloppy mode, as a prompt expects: `x = 5` makes a global and does not throw.
        let mut options = EvalOptions::default();
        options.strict = false;
        options.filename = Some(CONSOLE_FILE.to_owned());
        let value: Value<'_> = ctx.eval_with_options(source, options)?;
        let settled = settle(ctx, value)?;
        let host: Object<'_> = ctx.globals().get("__gnoblinHost")?;
        let entry: Object<'_> = host.get("entry")?;
        entry
            .get::<_, Function<'_>>("render")?
            .call::<_, String>((settled, max_bytes))
    })
}

/// A value after the promise jobs it started have run: a promise that is settled is its result (a rejected one
/// throws its reason), anything else is itself.
fn settle<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<Value<'js>> {
    for _ in 0..MAX_JOBS {
        if !ctx.execute_pending_job() {
            break;
        }
    }
    match value.as_promise().map(|promise| promise.result::<Value<'js>>()) {
        Some(Some(result)) => result,
        _ => Ok(value),
    }
}

/// `source`, in parentheses when it starts with `{` and reads as an object literal that way: at a prompt `{a: 1}`
/// is the object, not a block holding a label. A `{ ... }` that is a block is left alone.
fn as_expression(source: &str) -> String {
    if source.trim_start().starts_with('{') {
        let wrapped = format!("({source}\n)");
        if parses(&wrapped) {
            return wrapped;
        }
    }
    source.to_owned()
}

fn parses(source: &str) -> bool {
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, SourceType::cjs()).parse();
    parsed.diagnostics.errors().next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_shells_own_uid_is_served() {
        assert!(peer_allowed(Some(1000), 1000));
        assert!(!peer_allowed(Some(0), 1000));
        assert!(!peer_allowed(Some(1001), 1000));
        assert!(
            !peer_allowed(None, 1000),
            "a peer whose credentials cannot be read is refused"
        );
    }

    #[test]
    fn a_connection_from_this_process_has_this_uid() {
        let (near, _far) = UnixStream::pair().expect("a socket pair");
        assert!(
            stream_allowed(&near),
            "the kernel's record of a local peer is our own uid"
        );
    }

    #[test]
    fn the_console_socket_sits_beside_the_owner_socket() {
        let env = |name: &str| (name == "GNOBLIN_IPC_SOCKET").then(|| "/run/user/1000/gnoblin/ipc.sock".to_owned());
        assert_eq!(
            console_socket_path(&env).expect("a path"),
            PathBuf::from("/run/user/1000/gnoblin/js.sock")
        );
        let runtime = |name: &str| (name == "XDG_RUNTIME_DIR").then(|| "/run/user/1000".to_owned());
        assert_eq!(
            console_socket_path(&runtime).expect("a path"),
            PathBuf::from("/run/user/1000/gnoblin/js.sock")
        );
        assert!(console_socket_path(&|_: &str| None).is_err());
    }

    #[test]
    fn the_console_is_on_unless_the_variable_is_zero() {
        let with = |value: &'static str| move |name: &str| (name == CONSOLE_VARIABLE).then(|| value.to_owned());
        assert!(console_enabled(&|_: &str| None));
        assert!(console_enabled(&with("1")));
        assert!(!console_enabled(&with("0")));
    }

    #[test]
    fn a_stack_frame_in_the_console_gives_the_location() {
        assert_eq!(
            console_location("    at <eval> (<console>:3:14)\n    at f (gnoblin/host:9:1)"),
            Some((3, 14))
        );
        assert_eq!(console_location("    at <console>:1:5\n"), Some((1, 5)));
        assert_eq!(console_location("    at f (gnoblin/host:9:1)"), None);
    }

    #[test]
    fn an_object_literal_at_the_prompt_is_an_object_and_a_block_is_a_block() {
        assert_eq!(as_expression("{a: 1}"), "({a: 1}\n)");
        assert_eq!(as_expression("{ let a = 1; a }"), "{ let a = 1; a }");
        assert_eq!(as_expression("1 + 1"), "1 + 1");
    }

    #[test]
    fn a_stale_socket_file_is_replaced_and_a_live_one_is_refused() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("js.sock");
        // A socket nobody listens on any more: bind, then drop the listener without removing the file.
        drop(UnixListener::bind(&path).expect("a first socket"));
        assert!(path.exists());
        let first = JsConsole::bind(&path).expect("a stale socket is replaced");
        assert_eq!(
            std::fs::metadata(&path).expect("the socket").permissions().mode() & 0o777,
            0o600,
            "the socket is private to its owner"
        );
        assert!(matches!(JsConsole::bind(&path), Err(JsConsoleError::Busy { .. })));
        drop(first);
        assert!(!path.exists(), "the socket file goes with the console");
    }
}
