//! Reloading a script shell in a real session (`design/modular-framework.md`, section 8.2; example 11).
//!
//! A TypeScript shell and a Python shell, each of two files, run by the language runtimes in the compositor's process, with a real
//! Wayland client connected throughout. Editing the **imported** module (not the entry) changes where the next window is placed:
//! a stale module cache would keep the old place, and a listener the reload did not remove would place every window twice,
//! which the log line each placement writes (with the origin that generation had) would show. A broken edit is reported at its line and the running shell
//! serves on; the fix then loads.

use std::fs;
use std::time::Duration;

mod support;
use support::Mapper;
use support::native_shell::*;

/// What a script shell is made of: the entry, the module it imports, and the line a placement writes to the log.
struct Script {
    entry: &'static str,
    module: &'static str,
    /// Environment the runtime needs besides the runtime directory.
    environment: fn() -> Vec<(&'static str, String)>,
    entry_text: fn() -> String,
    module_text: fn(origin: u32) -> String,
    broken: &'static str,
    error_mentions: &'static str,
    /// Whether a listener of the new generation runs for the windows that are already open (`gnoblin.onClientCreated` in
    /// TypeScript does, as `addListener` does for the members there are; Python's decorator runs for windows created later only).
    replays_open_windows: bool,
}

const TYPESCRIPT: Script = Script {
    entry: "shell.ts",
    module: "origin.ts",
    environment: Vec::new,
    entry_text: || {
        "import { gnoblin } from \"gnoblin-core\";\nimport { ORIGIN } from \"./origin.ts\";\n\n\
         gnoblin.onClientCreated((client) => {\n    client.bounds = { x: ORIGIN, y: ORIGIN, width: 500, height: 300 };\n    \
         console.log(`placed ${client.appId} origin=${ORIGIN}`);\n});\n"
            .to_owned()
    },
    module_text: |origin| format!("export const ORIGIN: number = {origin};\n"),
    broken: "export const ORIGIN: number = ;\n",
    error_mentions: "origin.ts:1",
    replays_open_windows: true,
};

const PYTHON: Script = Script {
    entry: "shell.py",
    module: "origin.py",
    // The package is embedded in the runtime: no directory of it is named, and the session reads none.
    environment: Vec::new,
    entry_text: || {
        "from gnoblin import gnoblin\nfrom origin import ORIGIN\n\n\n@gnoblin.on_client_created\ndef placed(window):\n    \
         window.x, window.y, window.width, window.height = ORIGIN, ORIGIN, 500.0, 300.0\n    \
         gnoblin.log(\"placed %s origin=%s\" % (window.app_id, int(ORIGIN)), \"info\")\n"
            .to_owned()
    },
    module_text: |origin| format!("ORIGIN = {origin}.0\n"),
    broken: "ORIGIN = (\n",
    error_mentions: "origin.py",
    replays_open_windows: false,
};

fn at(window: &support::Window, origin: u32) -> bool {
    (window.x - f64::from(origin)).abs() < 0.5 && (window.y - f64::from(origin)).abs() < 0.5
}

/// The log lines a placement of `app` wrote, whatever origin it had.
fn placements(session: &Gnoblin, app: &str) -> Vec<String> {
    let wanted = format!("placed {app} origin=");
    session
        .log()
        .into_iter()
        .filter_map(|line| {
            line.text
                .split_once(&wanted)
                .map(|(_, origin)| origin.chars().take_while(char::is_ascii_digit).collect())
        })
        .collect()
}

fn run(script: &Script) {
    let home = Home::new();
    let shell = home.write(&format!("config/gnoblin/{}", script.entry), &(script.entry_text)());
    let module = shell.with_file_name(script.module);
    fs::write(&module, (script.module_text)(100)).expect("the module");
    let mut launch = Launch::guardian(&[]).with_env("GNOBLIN_RUNTIME_DIR", &runtimes().display().to_string());
    for (key, value) in (script.environment)() {
        launch = launch.with_env(key, &value);
    }
    let mut session = Gnoblin::launch(home, launch);
    session.wait_serving();
    session.wait_log("the script shell serves", |line| {
        line.contains("serves in process (generation 1)")
    });
    // The watcher of the script's runtime looks at the file system on its own clock; let it settle on the first versions.
    std::thread::sleep(Duration::from_millis(500));

    let mut client = Mapper::connect(&session.wayland());
    client.map_window("first", "First");
    let first = wait_for_window(
        &mut session,
        "generation 1 places the first window",
        "first",
        |window| at(window, 100),
    );
    session.wait_for("the first placement was logged", |session| {
        placements(session, "first") == ["100"]
    });

    // Edit the imported module: a new origin. The entry file is untouched, so this is the reload of a file the generation imported.
    fs::write(&module, (script.module_text)(300)).expect("the edit");
    session.wait_for("the shell reloaded for the edit of the module", |session| {
        session
            .log()
            .iter()
            .any(|line| line.text.contains("the shell reloaded"))
    });
    client.roundtrip();
    client.map_window("second", "Second");
    wait_for_window(
        &mut session,
        "the new origin is in force (a stale module cache would keep 100)",
        "second",
        |window| at(window, 300),
    );
    session.wait_for("the second placement was logged", |session| {
        !placements(session, "second").is_empty()
    });
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(
        placements(&session, "second"),
        ["300"],
        "the window was placed once, by generation 2: no listener of generation 1 is left running\n{}",
        session.log_text()
    );
    let windows = support::scene(&session.ipc());
    let still = windows
        .iter()
        .find(|window| window.app_id == "first")
        .expect("the first window is there");
    assert_eq!(still.id, first.id, "the client's window is the node it was");
    if script.replays_open_windows {
        // The new generation's listener ran for the window that was open, once, with its own origin.
        assert_eq!(placements(&session, "first"), ["100", "300"], "{}", session.log_text());
        assert!(at(still, 300), "{still:?}");
    } else {
        assert_eq!(
            placements(&session, "first"),
            ["100"],
            "the first window was not placed again"
        );
        assert!(at(still, 100), "and it did not move: {still:?}");
    }

    // A broken edit: reported at its file, and the running generation serves on.
    let reloaded = session
        .log()
        .iter()
        .filter(|line| line.text.contains("the shell reloaded"))
        .count();
    fs::write(&module, script.broken).expect("the broken edit");
    session.wait_log("the broken edit is refused with its place", |line| {
        line.contains("did not reload") && line.contains(script.error_mentions)
    });
    client.roundtrip();
    client.map_window("third", "Third");
    wait_for_window(&mut session, "generation 2 still places windows", "third", |window| {
        at(window, 300)
    });
    assert_eq!(
        session
            .log()
            .iter()
            .filter(|line| line.text.contains("the shell reloaded"))
            .count(),
        reloaded,
        "no reload committed for the broken edit"
    );

    // The fix loads as the next generation.
    fs::write(&module, (script.module_text)(500)).expect("the fix");
    session.wait_for("the fix reloaded", |session| {
        session
            .log()
            .iter()
            .filter(|line| line.text.contains("the shell reloaded"))
            .count()
            > reloaded
    });
    client.roundtrip();
    client.map_window("fourth", "Fourth");
    wait_for_window(&mut session, "the fixed origin is in force", "fourth", |window| {
        at(window, 500)
    });
    assert!(session.exited().is_none(), "{}", session.log_text());
}

#[test]
fn example_11_a_typescript_shell_reloads_when_a_module_it_imports_is_edited() {
    run(&TYPESCRIPT);
}

#[test]
fn example_11_a_python_shell_reloads_when_a_module_it_imports_is_edited() {
    run(&PYTHON);
}
