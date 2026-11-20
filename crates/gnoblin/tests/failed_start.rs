//! A shell that fails to start, in a real session (`design/modular-framework.md`, 30: a failing shell is a failed start).
//!
//! A `shell.ts` or a `shell.py` with a syntax error leaves the session on the compositor's own window management. The
//! failure is not silent and not final: the guardian's log carries the error text at its file and line, `gnoblinctl shell
//! status` shows `failed` with the same text, and the file that is fixed afterwards is loaded by `gnoblinctl shell reload`
//! (or by the file watcher) in the same process, without restarting the session.

use std::ffi::OsString;
use std::fs;
use std::time::Duration;

use gnoblinctl::SocketConnection;

mod support;
use support::Mapper;
use support::native_shell::*;

/// One language's shell: a file that fails to start, one that works, and what the error has to say.
struct Script {
    entry: &'static str,
    /// Fails at its third line.
    broken: &'static str,
    /// What the error names: the file and line, in the user's own coordinates.
    error_mentions: &'static str,
    /// Places every window at (`ORIGIN`, `ORIGIN`).
    working: fn(origin: u32) -> String,
}

const TYPESCRIPT: Script = Script {
    entry: "shell.ts",
    broken: "import { gnoblin } from \"gnoblin-core\";\n\nconst broken: number = = 5;\n",
    error_mentions: "shell.ts:3",
    working: |origin| {
        format!(
            "import {{ gnoblin }} from \"gnoblin-core\";\n\n\
             gnoblin.onClientCreated((client) => {{\n    client.bounds = {{ x: {origin}, y: {origin}, width: 500, height: 300 }};\n}});\n"
        )
    },
};

const PYTHON: Script = Script {
    entry: "shell.py",
    broken: "from gnoblin import gnoblin\n\nbroken = (\n",
    error_mentions: "shell.py",
    working: |origin| {
        format!(
            "from gnoblin import gnoblin\n\n\n@gnoblin.on_client_created\ndef placed(window):\n    \
             window.x, window.y, window.width, window.height = {origin}.0, {origin}.0, 500.0, 300.0\n"
        )
    },
};

fn at(window: &support::Window, origin: u32) -> bool {
    (window.x - f64::from(origin)).abs() < 0.5 && (window.y - f64::from(origin)).abs() < 0.5
}

/// A session whose config directory holds `entry`, started the way a login does: no path on the command line.
fn start(script: &Script, text: &str, env: &[(&str, &str)]) -> Gnoblin {
    let home = Home::new();
    home.write(&format!("config/gnoblin/{}", script.entry), text);
    let mut launch = Launch::guardian(&[]).with_env("PYTHONDONTWRITEBYTECODE", "1");
    for (name, value) in script_environment() {
        launch = launch.with_env(name, &value);
    }
    for (name, value) in env {
        launch = launch.with_env(name, value);
    }
    let mut session = Gnoblin::launch(home, launch);
    session.wait_serving();
    session
}

/// `gnoblinctl ARGS` against the session, through the real entry point: the exit code, standard output and standard error.
fn ctl(session: &Gnoblin, args: &[&str]) -> (u8, String, String) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = gnoblinctl::run(
        args.iter().map(OsString::from),
        SocketConnection::at(session.ipc()),
        &mut out,
        &mut err,
    );
    (
        code,
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
    )
}

/// The error text of the failed start as the guardian logged it.
fn guardian_report(session: &mut Gnoblin) -> String {
    session
        .wait_log("the guardian is told the shell failed to start", |line| {
            line.contains("[guardian] configuration revision 0 rejected: the shell failed to start:")
        })
        .text
}

fn failed_start_is_visible_and_a_reload_fixes_it(script: &Script) {
    let mut session = start(script, script.broken, &[("GNOBLIN_JS_WATCH", "0")]);

    // The guardian says why, at the file and line, and what the session runs on meanwhile.
    let report = guardian_report(&mut session);
    assert!(
        report.contains(script.error_mentions),
        "the report names the file and line: {report}"
    );
    assert!(report.contains("window management"), "{report}");
    assert!(report.contains("gnoblinctl shell reload"), "{report}");

    // `shell status` shows the failure and its text.
    let (code, out, err) = ctl(&session, &["shell", "status"]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    assert!(out.contains("state: failed"), "{out}");
    assert!(out.contains(script.error_mentions), "status carries the error: {out}");
    let (code, out, err) = ctl(&session, &["--json", "shell", "status"]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    let status: serde_json::Value = serde_json::from_str(&out).expect("status is JSON");
    assert_eq!(status["state"], "failed", "{status}");
    assert!(
        status["error"]
            .as_str()
            .is_some_and(|error| error.contains(script.error_mentions)),
        "{status}"
    );
    assert_eq!(status["generation"], serde_json::Value::Null, "{status}");

    // The session is up on the compositor's own window management: a client maps a window, and no shell moved it.
    let mut client = Mapper::connect(&session.wayland());
    client.map_window("before", "Before");
    wait_for_window(&mut session, "the compositor places the window", "before", |window| {
        !at(window, 300)
    });

    // A reload of a file that still fails stays failed, and says why on the command line.
    let (code, out, err) = ctl(&session, &["shell", "reload"]);
    assert_eq!(code, 1, "stdout: {out}\nstderr: {err}");
    assert!(
        err.contains(script.error_mentions),
        "the refusal names the file and line: {err}"
    );
    let (_, out, _) = ctl(&session, &["shell", "status"]);
    assert!(out.contains("state: failed"), "still failed: {out}");

    // The fixed file is picked up by `shell reload` in the same session.
    fs::write(session.home().config().join(script.entry), (script.working)(300)).expect("the fix");
    let (code, out, err) = ctl(&session, &["shell", "reload"]);
    assert_eq!((code, err.as_str()), (0, ""), "stdout: {out}\n{}", session.log_text());
    assert!(out.starts_with("reloaded: generation"), "{out}");
    let (_, out, _) = ctl(&session, &["shell", "status"]);
    assert!(!out.contains("state: failed"), "{out}");
    assert!(out.contains("is running"), "{out}");
    session.wait_log("the shell serves after the fix", |line| {
        line.contains("serves in process (generation")
    });
    client.roundtrip();
    client.map_window("after", "After");
    wait_for_window(&mut session, "the fixed shell places the window", "after", |window| {
        at(window, 300)
    });
    let log = session.log_text();
    assert_eq!(
        log.matches("started compositor (").count(),
        1,
        "the session was not restarted:\n{log}"
    );
    assert!(session.exited().is_none(), "{log}");
}

fn failed_start_is_fixed_by_saving_the_file(script: &Script) {
    // The watcher is on (the default for a script): saving the file is the request, with no `shell reload`.
    let mut session = start(script, script.broken, &[]);
    guardian_report(&mut session);
    std::thread::sleep(Duration::from_millis(500));
    fs::write(session.home().config().join(script.entry), (script.working)(300)).expect("the fix");
    session.wait_log("the shell serves once the file is saved", |line| {
        line.contains("serves in process (generation")
    });
    let mut client = Mapper::connect(&session.wayland());
    client.map_window("after", "After");
    wait_for_window(&mut session, "the fixed shell places the window", "after", |window| {
        at(window, 300)
    });
    let (_, out, _) = ctl(&session, &["--json", "shell", "status"]);
    let status: serde_json::Value = serde_json::from_str(&out).expect("status is JSON");
    assert_eq!(status["state"], "running", "{status}");
}

#[test]
fn a_typescript_shell_that_fails_to_start_is_reported_shown_and_fixed_by_a_reload() {
    failed_start_is_visible_and_a_reload_fixes_it(&TYPESCRIPT);
}

#[test]
fn a_python_shell_that_fails_to_start_is_reported_shown_and_fixed_by_a_reload() {
    failed_start_is_visible_and_a_reload_fixes_it(&PYTHON);
}

#[test]
fn a_typescript_shell_that_failed_to_start_loads_when_the_file_is_saved() {
    failed_start_is_fixed_by_saving_the_file(&TYPESCRIPT);
}

#[test]
fn a_python_shell_that_failed_to_start_loads_when_the_file_is_saved() {
    failed_start_is_fixed_by_saving_the_file(&PYTHON);
}
