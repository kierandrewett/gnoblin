//! `console`: a live prompt into the running JavaScript shell, and `console --eval` for one expression.
//!
//! The shell serves a Unix socket beside the owner socket (`gnoblin_js::console_socket_path`) and evaluates each
//! request line in its own runtime, between frames (`gnoblin-js`'s `console` module has the protocol). This module is
//! the client: it connects, sends `{"id", "expr"}` lines and prints the replies.
//!
//! The prompt is a plain line loop over standard input: the terminal's own line editing applies, and `rlwrap`
//! adds arrow-key history. `gnoblinctl` has no line-editor dependency, so the loop keeps its history itself
//! (`.history`, `!!`, `!N`). A line that ends inside an unfinished expression (an open brace) is continued on the next.
//!
//! Exit codes: 0 when the expression gave a value (or the prompt ended with `.exit` or end of input), 1 when the
//! expression failed or no shell is listening. A piped script that had any failing line exits 1.

use std::io::{self, BufRead, BufReader, IsTerminal, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gnoblin_cli_style::{
    ColorChoice, ErrorReport, LineOutcome, OutputEnvironment, OutputTheme, error_block, status_line,
};
use serde_json::{Value as Json, json};

/// How long a reply may take. The shell's budget for one expression is a fraction of a second, so a reply that is
/// this late means the shell is stuck in a frame and the person should know.
const REPLY_WAIT: Duration = Duration::from_secs(30);

/// The prompt, and the prompt of a line that continues the one before.
const PROMPT: &str = "gnoblin> ";
const CONTINUATION: &str = "... ";

/// What `.help` lists.
const HELP: &[(&str, &str)] = &[
    (".exit", "leave the console (Ctrl-D does too)"),
    (".history", "list the expressions of this session"),
    ("!!", "run the last expression again"),
    ("!N", "run expression N of `.history` again"),
    (".help", "show this list"),
];

const NO_SHELL_HINT: &str = "the session must be running, with the JavaScript shell: check that the session is up \
     and that `GNOBLIN_JS_CONSOLE` is not `0`";

/// What `gnoblinctl console` was asked.
#[derive(Debug, Clone, Copy, Default)]
pub struct ConsoleOptions<'a> {
    /// One expression to evaluate and print as JSON, instead of the prompt.
    pub eval: Option<&'a str>,
    /// Print each reply as it came over the socket (one JSON object per line), with no prompt and no styling.
    pub json: bool,
    /// Whether standard input is a terminal: it decides whether prompts are printed and whether a failing line
    /// makes the exit code 1.
    pub interactive: bool,
}

/// The console socket for the process environment, as `gnoblin_js::console_socket_path` finds it.
///
/// # Errors
///
/// The message that names the variables to set when neither decides a path.
pub fn console_socket() -> Result<PathBuf, String> {
    gnoblin_js::console_socket_path(&|name| std::env::var(name).ok())
}

/// `console` for the process: the environment's socket, standard input and the streams' own terminals.
pub(crate) fn run_process(eval: Option<&str>, json: bool, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let stdin = io::stdin();
    let options = ConsoleOptions {
        eval,
        json,
        interactive: stdin.is_terminal(),
    };
    run_console(console_socket(), &options, &mut stdin.lock(), out, err)
}

/// Runs the console against the shell at `socket`, reading lines from `input`.
///
/// Public so a test can drive the real loop over a real socket without touching the process environment.
pub fn run_console(
    socket: Result<PathBuf, String>,
    options: &ConsoleOptions<'_>,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let theme = theme(io::stderr().is_terminal());
    let path = match socket {
        Ok(path) => path,
        Err(reason) => return fail(err, &theme, "cannot find the console socket", Some(reason)),
    };
    let mut session = match ConsoleLink::connect(&path) {
        Ok(session) => session,
        Err(error) => {
            let why = format!("cannot connect to {}: {error}", path.display());
            return fail(err, &theme, "no JavaScript shell is running", Some(why));
        }
    };
    let screen = ConsoleScreen {
        theme: &theme,
        json: options.json,
    };
    match options.eval {
        Some(expr) => one_shot(&mut session, expr, &screen, out, err),
        None => prompt(&mut session, options, &screen, input, out, err),
    }
}

fn theme(is_terminal: bool) -> OutputTheme {
    OutputTheme::resolve(ColorChoice::Auto, &OutputEnvironment::detect(is_terminal))
}

fn fail(err: &mut dyn Write, theme: &OutputTheme, summary: &str, why: Option<String>) -> u8 {
    let report = ErrorReport {
        summary: summary.to_owned(),
        why,
        hints: vec![NO_SHELL_HINT.to_owned()],
    };
    let _ = writeln!(err, "{}", error_block(theme, &report));
    1
}

/// The error theme and the output mode a reply is printed with.
struct ConsoleScreen<'a> {
    theme: &'a OutputTheme,
    json: bool,
}

/// One connection to the console socket.
struct ConsoleLink {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
    next_id: u64,
}

impl ConsoleLink {
    fn connect(path: &Path) -> io::Result<Self> {
        let writer = UnixStream::connect(path)?;
        writer.set_read_timeout(Some(REPLY_WAIT))?;
        let reader = BufReader::new(writer.try_clone()?);
        Ok(Self {
            writer,
            reader,
            next_id: 1,
        })
    }

    /// Sends `expr` and returns the reply object. An `Err` is a broken conversation, not a failed expression.
    fn ask(&mut self, expr: &str) -> Result<Json, String> {
        let id = self.next_id;
        self.next_id += 1;
        let mut request = json!({"id": id, "expr": expr}).to_string();
        request.push('\n');
        self.writer
            .write_all(request.as_bytes())
            .map_err(|error| format!("cannot send to the shell: {error}"))?;
        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            Ok(0) => Err("the shell closed the console".to_owned()),
            Ok(_) => {
                let reply: Json =
                    serde_json::from_str(&line).map_err(|error| format!("the shell sent what is not JSON: {error}"))?;
                if reply.get("id") == Some(&json!(id)) {
                    Ok(reply)
                } else {
                    Err("the shell answered another request".to_owned())
                }
            }
            Err(error) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => Err(format!(
                "the shell did not answer within {} s; it may be stuck in a frame",
                REPLY_WAIT.as_secs()
            )),
            Err(error) => Err(format!("cannot read from the shell: {error}")),
        }
    }
}

fn is_ok(reply: &Json) -> bool {
    reply.get("ok") == Some(&Json::Bool(true))
}

/// The hint for a failed expression, by the kind the shell gave it.
fn hint_for(kind: &str) -> &'static str {
    match kind {
        "budget" => {
            "the expression ran past the shell's call budget and was interrupted; the shell is unaffected, so break \
             the work into smaller expressions"
        }
        "memory" => "the expression hit the runtime's memory limit; keep what it builds smaller",
        "request" => "this is a bug in the console client: the request was not an expression",
        _ => "fix the expression and run it again; a syntax error names the column in the expression",
    }
}

/// An error block for a failed reply.
fn report_failure(reply: &Json, theme: &OutputTheme, err: &mut dyn Write) {
    let error = &reply["error"];
    let message = error["message"].as_str().unwrap_or("the expression failed");
    let kind = error["kind"].as_str().unwrap_or("exception");
    let at = match (error["location"]["file"].as_str(), error["location"]["line"].as_u64()) {
        (Some(file), Some(line)) => Some(match error["location"]["column"].as_u64() {
            Some(column) => format!("at {file}:{line}:{column}"),
            None => format!("at {file}:{line}"),
        }),
        _ => None,
    };
    let report = ErrorReport {
        summary: message.to_owned(),
        why: at,
        hints: vec![hint_for(kind).to_owned()],
    };
    let _ = writeln!(err, "{}", error_block(theme, &report));
}

/// What the expression printed through `console`, before its value: each line's text, and whether it is a `warn` or
/// an `error`, which a terminal shows on standard error.
fn printed_lines(reply: &Json) -> impl Iterator<Item = (bool, &str)> {
    reply["output"].as_array().into_iter().flatten().map(|line| {
        let to_err = matches!(line["level"].as_str(), Some("warn" | "error"));
        (to_err, line["text"].as_str().unwrap_or_default())
    })
}

/// At the prompt `log`, `info` and `debug` go to standard output and `warn` and `error` to standard error, as a Node
/// prompt does.
fn print_output(reply: &Json, out: &mut dyn Write, err: &mut dyn Write) {
    for (to_err, text) in printed_lines(reply) {
        let _ = if to_err {
            writeln!(err, "{text}")
        } else {
            writeln!(out, "{text}")
        };
    }
}

fn warn_truncated(reply: &Json, theme: &OutputTheme, err: &mut dyn Write) {
    if reply["truncated"] == Json::Bool(true) {
        let text = "the value was cut at the size cap; ask for less (`.slice`, `.map`) to see the rest";
        let _ = writeln!(err, "{}", status_line(theme, LineOutcome::Warn, text));
    }
}

/// `--eval`: one expression, its value as JSON.
fn one_shot(
    session: &mut ConsoleLink,
    expr: &str,
    screen: &ConsoleScreen<'_>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let reply = match session.ask(expr) {
        Ok(reply) => reply,
        Err(reason) => return fail(err, screen.theme, "the console conversation broke", Some(reason)),
    };
    if !screen.json {
        // Standard output is the value alone, so what the expression printed goes to standard error.
        for (_, text) in printed_lines(&reply) {
            let _ = writeln!(err, "{text}");
        }
    }
    if is_ok(&reply) {
        // `undefined` has no JSON form; it prints as `null`, and `--json` keeps the difference (`type`).
        let printed = if screen.json {
            reply.to_string()
        } else {
            reply.get("value").unwrap_or(&Json::Null).to_string()
        };
        let _ = writeln!(out, "{printed}");
        warn_truncated(&reply, screen.theme, err);
        0
    } else {
        if screen.json {
            let _ = writeln!(out, "{reply}");
        } else {
            report_failure(&reply, screen.theme, err);
        }
        1
    }
}

/// Whether a failed reply says the expression ended before it was complete, so the next line may finish it.
fn is_incomplete(reply: &Json) -> bool {
    reply["error"]["kind"] == "exception"
        && reply["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("end of input"))
}

/// The prompt: lines in, results out, until `.exit` or end of input.
fn prompt(
    session: &mut ConsoleLink,
    options: &ConsoleOptions<'_>,
    screen: &ConsoleScreen<'_>,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let show_prompt = options.interactive && !screen.json;
    let mut history: Vec<String> = Vec::new();
    let mut pending = String::new();
    let mut unfinished: Option<Json> = None;
    let mut failed = false;
    loop {
        if show_prompt {
            let _ = write!(out, "{}", if pending.is_empty() { PROMPT } else { CONTINUATION });
            let _ = out.flush();
        }
        let mut raw = String::new();
        match input.read_line(&mut raw) {
            Ok(0) => {
                if show_prompt {
                    let _ = writeln!(out);
                }
                break;
            }
            Ok(_) => {}
            Err(error) => return fail(err, screen.theme, "cannot read the prompt", Some(error.to_string())),
        }
        let mut expr = raw.trim_end_matches(['\n', '\r']).to_owned();
        if pending.is_empty() {
            let word = expr.trim();
            match word {
                ".exit" | ".quit" => break,
                ".help" => {
                    for (command, text) in HELP {
                        let _ = writeln!(out, "{command:<10} {text}");
                    }
                    continue;
                }
                ".history" => {
                    for (index, entry) in history.iter().enumerate() {
                        let _ = writeln!(out, "{:>4}  {}", index + 1, entry.replace('\n', " "));
                    }
                    continue;
                }
                "" => continue,
                _ => {}
            }
            if let Some(recalled) = recall(word, &history) {
                match recalled {
                    Ok(entry) => {
                        // Echo what runs, as a shell does, so the transcript shows it.
                        let _ = writeln!(out, "{entry}");
                        expr = entry;
                    }
                    Err(problem) => {
                        let _ = writeln!(err, "{}", status_line(screen.theme, LineOutcome::Fail, &problem));
                        continue;
                    }
                }
            }
        }
        if !pending.is_empty() {
            pending.push('\n');
        }
        pending.push_str(&expr);
        let reply = match session.ask(&pending) {
            Ok(reply) => reply,
            Err(reason) => return fail(err, screen.theme, "the console conversation broke", Some(reason)),
        };
        if !is_ok(&reply) && is_incomplete(&reply) {
            unfinished = Some(reply);
            continue;
        }
        unfinished = None;
        history.push(std::mem::take(&mut pending));
        if screen.json {
            let _ = writeln!(out, "{reply}");
            failed |= !is_ok(&reply);
        } else if is_ok(&reply) {
            print_output(&reply, out, err);
            let _ = writeln!(out, "{}", reply["text"].as_str().unwrap_or("undefined"));
            warn_truncated(&reply, screen.theme, err);
        } else {
            failed = true;
            print_output(&reply, out, err);
            report_failure(&reply, screen.theme, err);
        }
    }
    if let Some(reply) = unfinished {
        // The input ended inside an expression: say why it never finished.
        failed = true;
        if screen.json {
            let _ = writeln!(out, "{reply}");
        } else {
            report_failure(&reply, screen.theme, err);
        }
    }
    u8::from(failed && !options.interactive)
}

/// `!!` and `!N`: the expression to run again, or why there is none. `None` when `word` is not a recall.
fn recall(word: &str, history: &[String]) -> Option<Result<String, String>> {
    let spec = word.strip_prefix('!')?;
    if spec == "!" {
        return Some(
            history
                .last()
                .cloned()
                .ok_or_else(|| "there is no earlier expression to repeat".to_owned()),
        );
    }
    let number: usize = spec.parse().ok()?;
    Some(
        number
            .checked_sub(1)
            .and_then(|index| history.get(index))
            .cloned()
            .ok_or_else(|| format!("`.history` has no expression {number}")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recall_is_the_last_expression_or_a_numbered_one() {
        let history = vec!["1 + 1".to_owned(), "gnoblin.clients.length".to_owned()];
        assert_eq!(recall("!!", &history), Some(Ok("gnoblin.clients.length".to_owned())));
        assert_eq!(recall("!1", &history), Some(Ok("1 + 1".to_owned())));
        assert!(matches!(recall("!3", &history), Some(Err(_))));
        assert!(matches!(recall("!0", &history), Some(Err(_))));
        assert!(matches!(recall("!!", &[]), Some(Err(_))));
        assert_eq!(
            recall("!important", &history),
            None,
            "a `!` that is not a recall is an expression"
        );
        assert_eq!(recall("1 + 1", &history), None);
    }

    #[test]
    fn only_an_unfinished_expression_continues_on_the_next_line() {
        let reply = |kind: &str, message: &str| json!({"ok": false, "error": {"kind": kind, "message": message}});
        assert!(is_incomplete(&reply(
            "exception",
            "unexpected token in expression: 'end of input'"
        )));
        assert!(!is_incomplete(&reply("exception", "x is not defined")));
        assert!(!is_incomplete(&reply("budget", "end of input")));
    }

    #[test]
    fn the_command_line_takes_eval_and_nothing_else() {
        let parse = |args: &[&str]| {
            crate::cli::parse(
                std::iter::once("gnoblinctl")
                    .chain(args.iter().copied())
                    .map(std::ffi::OsString::from),
            )
        };
        match parse(&["console", "--eval", "gnoblin.clients.length"]) {
            Ok(crate::cli::Invocation::Console { eval, json }) => {
                assert_eq!((eval.as_deref(), json), (Some("gnoblin.clients.length"), false));
            }
            _ => panic!("expected a console invocation"),
        }
        match parse(&["console", "--json"]) {
            Ok(crate::cli::Invocation::Console { eval, json }) => assert_eq!((eval, json), (None, true)),
            _ => panic!("expected a console invocation"),
        }
        assert!(parse(&["console", "--nonsense"]).is_err());
    }
}
