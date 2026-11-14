//! `gnoblinctl console`: the real prompt loop and the real exit codes over a real Unix socket.
//!
//! The shell's side is a scripted server: it answers each request line from a closure, the way the shell's console
//! does (`gnoblin-js`'s `console` module has the protocol, and `crates/gnoblin/tests/js_shell.rs` proves it against a
//! real compositor). What is checked here is the client: what it sends, what it prints and which code it exits with.

use std::io::{BufRead, BufReader, Cursor, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;

use gnoblinctl::{ConsoleOptions, run_console};
use serde_json::{Value, json};

/// A console that answers every request with `answer`, and returns the request lines it saw once the client leaves.
fn serve(path: &Path, answer: impl Fn(&str) -> Value + Send + 'static) -> JoinHandle<Vec<String>> {
    // A test that starts a second shell at one path leaves the first one's socket file behind.
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path).expect("a listening socket");
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("the client connects");
        let mut reader = BufReader::new(stream.try_clone().expect("a second handle"));
        let mut writer = stream;
        let mut seen = Vec::new();
        let mut line = String::new();
        while reader.read_line(&mut line).expect("a request line") > 0 {
            let request: Value = serde_json::from_str(&line).expect("a request is JSON");
            let expr = request["expr"].as_str().expect("an expr").to_owned();
            let mut reply = answer(&expr);
            reply["id"] = request["id"].clone();
            writeln!(writer, "{reply}").expect("the reply is written");
            seen.push(expr);
            line.clear();
        }
        seen
    })
}

fn ok(value: Value, text: &str) -> Value {
    json!({"ok": true, "type": "number", "value": value, "text": text, "truncated": false})
}

fn failure(kind: &str, message: &str, line: u64, column: u64) -> Value {
    json!({"ok": false, "error": {"kind": kind, "message": message, "stack": null,
        "location": {"file": "<console>", "line": line, "column": column}}})
}

struct Run {
    code: u8,
    out: String,
    err: String,
}

fn drive(socket: PathBuf, options: &ConsoleOptions<'_>, input: &str) -> Run {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run_console(
        Ok(socket),
        options,
        &mut Cursor::new(input.as_bytes()),
        &mut out,
        &mut err,
    );
    Run {
        code,
        out: String::from_utf8(out).expect("UTF-8"),
        err: String::from_utf8(err).expect("UTF-8"),
    }
}

fn eval(expr: &str) -> ConsoleOptions<'_> {
    ConsoleOptions {
        eval: Some(expr),
        json: false,
        interactive: false,
    }
}

fn socket(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("js.sock")
}

#[test]
fn no_shell_is_a_clear_error_and_exit_1() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let run = drive(socket(&dir), &eval("1"), "");
    assert_eq!(run.code, 1);
    assert!(run.err.contains("error: no JavaScript shell is running"), "{}", run.err);
    assert!(run.err.contains("cannot connect to"), "{}", run.err);
    assert!(run.err.contains("hint:"), "{}", run.err);
    assert!(run.err.contains("GNOBLIN_SHELL_VM"), "{}", run.err);
    assert!(
        !run.err.contains("GNOBLIN_SHELL_VM=js"),
        "the hint no longer tells you to select the default: {}",
        run.err
    );
    assert_eq!(run.out, "", "nothing on standard output");

    // A prompt, not only a one-shot, says the same and does not hang waiting for input.
    let prompt = run_console(
        Ok(socket(&dir)),
        &ConsoleOptions {
            eval: None,
            json: false,
            interactive: true,
        },
        &mut Cursor::new(&b""[..]),
        &mut Vec::new(),
        &mut Vec::new(),
    );
    assert_eq!(prompt, 1);
}

#[test]
fn an_environment_that_names_no_socket_is_exit_1_with_the_variables() {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run_console(
        Err("no IPC socket: set GNOBLIN_IPC_SOCKET, or XDG_RUNTIME_DIR".to_owned()),
        &eval("1"),
        &mut Cursor::new(&b""[..]),
        &mut out,
        &mut err,
    );
    let err = String::from_utf8(err).expect("UTF-8");
    assert_eq!(code, 1);
    assert!(err.contains("XDG_RUNTIME_DIR") && err.contains("hint:"), "{err}");
}

#[test]
fn eval_prints_the_value_as_json_and_exits_0() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let server = serve(&socket(&dir), |_| ok(json!({"n": 2, "names": ["a", "b"]}), "{ n: 2 }"));
    let run = drive(socket(&dir), &eval("gnoblin.clients.length"), "");
    assert_eq!(
        (run.code, run.out.as_str()),
        (0, "{\"n\":2,\"names\":[\"a\",\"b\"]}\n"),
        "{}",
        run.err
    );
    assert_eq!(run.err, "");
    assert_eq!(server.join().expect("the server ends"), ["gnoblin.clients.length"]);
}

#[test]
fn eval_of_undefined_prints_null_and_json_keeps_the_envelope() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let server = serve(
        &socket(&dir),
        |_| json!({"ok": true, "type": "undefined", "text": "undefined", "truncated": false}),
    );
    let plain = drive(socket(&dir), &eval("undefined"), "");
    assert_eq!((plain.code, plain.out.as_str()), (0, "null\n"));
    server.join().expect("the server ends");

    let server = serve(
        &socket(&dir),
        |_| json!({"ok": true, "type": "undefined", "text": "undefined", "truncated": false}),
    );
    let options = ConsoleOptions {
        eval: Some("undefined"),
        json: true,
        interactive: false,
    };
    let with_json = drive(socket(&dir), &options, "");
    let envelope: Value = serde_json::from_str(&with_json.out).expect("one JSON document");
    assert_eq!(envelope["type"], "undefined");
    assert_eq!(envelope["id"], 1);
    server.join().expect("the server ends");
}

#[test]
fn eval_of_a_failing_expression_exits_1_with_the_message_and_the_place() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let server = serve(&socket(&dir), |_| {
        failure("exception", "unexpected token in expression: ')'", 1, 5)
    });
    let run = drive(socket(&dir), &eval("foo(1,)"), "");
    assert_eq!(run.code, 1);
    assert_eq!(run.out, "");
    assert!(
        run.err.contains("error: unexpected token in expression: ')'"),
        "{}",
        run.err
    );
    assert!(run.err.contains("at <console>:1:5"), "{}", run.err);
    assert!(run.err.contains("hint:"), "{}", run.err);
    server.join().expect("the server ends");

    let server = serve(&socket(&dir), |_| {
        failure("budget", "the script ran past its 100 ms budget", 1, 1)
    });
    let run = drive(socket(&dir), &eval("while (true) {}"), "");
    assert_eq!(run.code, 1);
    // The hint wraps at the terminal width, so the words are compared and not the lines.
    let words = run.err.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        words.contains("interrupted") && words.contains("the shell is unaffected"),
        "{}",
        run.err
    );
    server.join().expect("the server ends");
}

#[test]
fn eval_json_prints_a_failure_as_the_reply_and_still_exits_1() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let server = serve(&socket(&dir), |_| failure("exception", "boom", 1, 1));
    let options = ConsoleOptions {
        eval: Some("throw 1"),
        json: true,
        interactive: false,
    };
    let run = drive(socket(&dir), &options, "");
    assert_eq!(run.code, 1);
    let reply: Value = serde_json::from_str(&run.out).expect("one JSON document");
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["error"]["message"], "boom");
    assert_eq!(run.err, "", "the failure is the document");
    server.join().expect("the server ends");
}

#[test]
fn a_truncated_value_is_flagged_on_standard_error() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let server = serve(
        &socket(&dir),
        |_| json!({"ok": true, "type": "array", "value": [1], "text": "[ 1, ... 9 more ]", "truncated": true}),
    );
    let run = drive(socket(&dir), &eval("big"), "");
    assert_eq!((run.code, run.out.as_str()), (0, "[1]\n"));
    assert!(run.err.contains("warn") && run.err.contains("size cap"), "{}", run.err);
    server.join().expect("the server ends");
}

#[test]
fn the_prompt_prints_each_text_keeps_history_and_leaves_on_exit() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let server = serve(&socket(&dir), |expr| {
        ok(json!(expr.len()), &format!("len {}", expr.len()))
    });
    let options = ConsoleOptions {
        eval: None,
        json: false,
        interactive: true,
    };
    let run = drive(
        socket(&dir),
        &options,
        "1 + 1\n!!\n\n.history\n!1\n!9\n.help\n.exit\nnever sent\n",
    );
    assert_eq!(run.code, 0, "{}", run.err);
    // Prompts, results, the echo of a recalled line, and the history list.
    assert!(
        run.out.starts_with("gnoblin> len 5\ngnoblin> 1 + 1\nlen 5\n"),
        "{:?}",
        run.out
    );
    assert!(run.out.contains("   1  1 + 1\n   2  1 + 1\n"), "{:?}", run.out);
    assert!(run.out.contains(".exit"), "`.help` lists the commands: {:?}", run.out);
    assert!(run.err.contains("`.history` has no expression 9"), "{}", run.err);
    assert_eq!(
        server.join().expect("the server ends"),
        ["1 + 1", "1 + 1", "1 + 1"],
        "`!!` and `!1` send the expression again; `.exit` stops reading"
    );
}

#[test]
fn a_line_that_ends_inside_an_expression_continues_on_the_next() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let server = serve(&socket(&dir), |expr| {
        if expr.trim_end().ends_with('}') {
            ok(json!(1), "done")
        } else {
            failure("exception", "unexpected token in expression: 'end of input'", 1, 1)
        }
    });
    let options = ConsoleOptions {
        eval: None,
        json: false,
        interactive: true,
    };
    let run = drive(socket(&dir), &options, "const a = {\n  b: 1,\n}\n");
    assert_eq!(run.code, 0, "{}", run.err);
    assert_eq!(run.out, "gnoblin> ... ... done\ngnoblin> \n");
    assert_eq!(run.err, "", "an unfinished line is not an error");
    assert_eq!(
        server.join().expect("the server ends"),
        ["const a = {", "const a = {\n  b: 1,", "const a = {\n  b: 1,\n}"]
    );
}

#[test]
fn a_piped_script_with_a_failing_line_exits_1_and_a_terminal_session_does_not() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let answer = |expr: &str| {
        if expr == "bad" {
            failure("exception", "bad is not defined", 1, 1)
        } else {
            ok(json!(1), "1")
        }
    };
    let server = serve(&socket(&dir), answer);
    let piped = drive(socket(&dir), &ConsoleOptions::default(), "1\nbad\n1\n");
    assert_eq!(piped.code, 1);
    assert_eq!(piped.out, "1\n1\n", "no prompts when standard input is not a terminal");
    assert!(piped.err.contains("bad is not defined"), "{}", piped.err);
    server.join().expect("the server ends");

    let server = serve(&socket(&dir), answer);
    let options = ConsoleOptions {
        interactive: true,
        ..ConsoleOptions::default()
    };
    let typed = drive(socket(&dir), &options, "bad\n");
    assert_eq!(typed.code, 0, "a mistake at the prompt is not a failed session");
    server.join().expect("the server ends");
}

#[test]
fn json_mode_prints_one_reply_per_line_without_prompts() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let server = serve(&socket(&dir), |_| ok(json!(2), "2"));
    let options = ConsoleOptions {
        json: true,
        interactive: true,
        ..ConsoleOptions::default()
    };
    let run = drive(socket(&dir), &options, "1 + 1\n1 + 1\n");
    let lines: Vec<Value> = run
        .out
        .lines()
        .map(|line| serde_json::from_str(line).expect("JSON"))
        .collect();
    assert_eq!(lines.len(), 2, "{}", run.out);
    assert_eq!(lines[1]["id"], 2);
    server.join().expect("the server ends");
}

#[test]
fn a_shell_that_goes_away_mid_session_is_an_error_not_a_hang() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let listener = UnixListener::bind(socket(&dir)).expect("a listening socket");
    let hangup = std::thread::spawn(move || {
        // Accept, read the request, and close without answering.
        let (stream, _) = listener.accept().expect("the client connects");
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).expect("the request");
    });
    let run = drive(socket(&dir), &eval("1"), "");
    assert_eq!(run.code, 1);
    assert!(run.err.contains("the shell closed the console"), "{}", run.err);
    hangup.join().expect("the server ends");
}
