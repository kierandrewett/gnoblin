//! Example 11: hot reload of a TypeScript shell and of a Python shell, in a real session with a Wayland client that stays
//! connected. The shell file and a module it imports are edited: the new behaviour is in force, the windows are the same windows,
//! no listener of an earlier generation runs again, and a broken edit is reported at its line while the shell that serves stays.

use std::fs;
use std::path::Path;

use crate::support::Mapper;
use crate::support::native_shell::{Gnoblin, Start, scene_ids, start_script, wait_for_window};
use crate::{near, save, screen_of, serving};

/// The language of a shell under test: its file names and the text of its files.
struct Language {
    shell: &'static str,
    module: &'static str,
    /// The shell of generation `tag`, which records every window it is told of in `marker`.
    shell_text: fn(tag: &str, marker: &Path) -> String,
    /// The module the shell imports: columns with no gap, or (edited) rows 24 apart.
    module_text: fn(edited: bool) -> String,
    /// A file the language cannot read, and the line its error is on.
    broken: fn(good: &str) -> (String, usize),
}

const TYPESCRIPT: Language = Language {
    shell: "shell.ts",
    module: "place.ts",
    shell_text: |tag, marker| {
        format!(
            r#"import {{ CollectionListener, gnoblin }} from "gnoblin-core";
import type {{ Client }} from "gnoblin-core";

import {{ place }} from "./place.ts";

// Every window the listener hears of adds a line to the marker file, so a listener that outlives its generation shows as a
// second line for one window.
class Tiler extends CollectionListener<Client> {{
    onAdded(client: Client): void {{
        gnoblin.spawn(["sh", "-c", "echo {tag}:" + client.appId + " >> {marker}"]);
        place();
    }}
    onRemoved(): void {{
        place();
    }}
}}
gnoblin.clients.addListener(new Tiler());
"#,
            marker = marker.display()
        )
    },
    module_text: |edited| {
        if edited {
            r#"import { gnoblin } from "gnoblin-core";

export function place(): void {
    const output = gnoblin.outputs.at(0);
    if (output === undefined) {
        return;
    }
    const windows = [...gnoblin.clients];
    const gap = 24;
    const height = (output.height - gap * (windows.length - 1)) / windows.length;
    windows.forEach((client, at) => {
        client.bounds = { x: output.x, y: output.y + at * (height + gap), width: output.width, height };
    });
}
"#
            .to_owned()
        } else {
            r#"import { gnoblin } from "gnoblin-core";

export function place(): void {
    const output = gnoblin.outputs.at(0);
    if (output === undefined) {
        return;
    }
    const windows = [...gnoblin.clients];
    const width = output.width / windows.length;
    windows.forEach((client, at) => {
        client.bounds = { x: output.x + at * width, y: output.y, width, height: output.height };
    });
}
"#
            .to_owned()
        }
    },
    broken: |good| {
        let line = good.lines().count() + 2;
        (format!("{good}\nexport const broken: number = ;\n"), line)
    },
};

const PYTHON: Language = Language {
    shell: "shell.py",
    module: "place.py",
    shell_text: |tag, marker| {
        format!(
            r#"from gnoblin import gnoblin

import place


def record(window):
    gnoblin.spawn(["sh", "-c", "echo {tag}:%s >> {marker}" % window.app_id])


@gnoblin.on_client_created
def added(window):
    record(window)
    place.place()


@gnoblin.on_client_destroyed
def removed(window):
    place.place()


for window in gnoblin.windows:
    record(window)
place.place()
"#,
            marker = marker.display()
        )
    },
    // The edit keeps the module's size: a stale `.pyc`, which Python keys on the source's size and its time in seconds, would run the
    // old code.
    module_text: |edited| {
        let gap = if edited { 24 } else { 10 };
        format!(
            r#"from gnoblin import Screen, gnoblin

GAP = {gap}


def place():
    screen = next(node for node in gnoblin.root.children() if isinstance(node, Screen))
    windows = gnoblin.windows
    if not windows:
        return
    if GAP == 10:
        step = screen.width / len(windows)
        for at, window in enumerate(windows):
            window.x, window.y, window.width, window.height = screen.x + at * step, screen.y, step, screen.height
    else:
        height = (screen.height - GAP * (len(windows) - 1)) / len(windows)
        for at, window in enumerate(windows):
            window.x, window.y, window.width, window.height = screen.x, screen.y + at * (height + GAP), screen.width, height
"#
        )
    },
    broken: |good| {
        let line = good.lines().count() + 2;
        (format!("{good}\ndef broken(:\n"), line)
    },
};

fn lines_of(marker: &Path, wanted: &str) -> usize {
    fs::read_to_string(marker)
        .unwrap_or_default()
        .lines()
        .filter(|line| *line == wanted)
        .count()
}

#[track_caller]
fn wait_marker(session: &mut Gnoblin, marker: &Path, wanted: &str) {
    session.wait_for(&format!("the marker says `{wanted}`"), |_| {
        lines_of(marker, wanted) >= 1
    });
}

fn run(language: &Language) {
    let dir = tempfile::tempdir().expect("a directory for the shell");
    let marker = dir.path().join("marker");
    let shell = dir.path().join(language.shell);
    let module = dir.path().join(language.module);
    fs::write(&shell, (language.shell_text)("first", &marker)).expect("the shell");
    fs::write(&module, (language.module_text)(false)).expect("the module");
    let mut session = start_script(Start::Compositor, &shell, &[]);
    serving(&mut session);
    let (width, height) = screen_of(&session);
    let gap = 24.0;

    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("a", "A");
    mapper.map_window("b", "B");
    wait_for_window(&mut session, "columns with no gap: the first window", "a", |window| {
        near(window.x, 0.0) && near(window.width, width / 2.0) && near(window.height, height)
    });
    wait_for_window(&mut session, "the second window", "b", |window| {
        near(window.x, width / 2.0) && near(window.width, width / 2.0)
    });
    let before = scene_ids(&session.ipc());
    let copies = |session: &Gnoblin| -> Vec<String> {
        session
            .mapped_files()
            .into_iter()
            .filter(|path| path.contains("/gen-"))
            .collect()
    };
    let copies_before = copies(&session);
    assert!(!copies_before.is_empty(), "the runtime is mapped from its own copy");

    // Both files change, as when one save writes the module and another the entry: the module's edit is in force (rows 24 apart),
    // so the module graph was read again and not served from a stale cache. The window is moved by the new code, in place.
    save(&module, &(language.module_text)(true));
    save(&shell, &(language.shell_text)("second", &marker));
    let rows = (height - gap) / 2.0;
    wait_for_window(
        &mut session,
        "rows: the first window on top, in full width",
        "a",
        |window| near(window.y, 0.0) && near(window.width, width) && near(window.height, rows),
    );
    wait_for_window(&mut session, "rows: the second below", "b", |window| {
        near(window.y, rows + gap) && near(window.width, width)
    });
    // The client's connection and its windows lived through it: the same window nodes, and a connection that still answers.
    mapper.roundtrip();
    assert_eq!(before, scene_ids(&session.ipc()), "the windows are the same windows");
    // A reload maps nothing: the loader's copies are what they were, the runtime's one among them.
    assert_eq!(copies_before, copies(&session), "a reload maps nothing");

    // A window that opens after the reload is placed by the new code, once: the old generation's listener never runs again, and
    // the new one is told of it once however many reloads the saves made.
    mapper.map_window("c", "C");
    let third = (height - 2.0 * gap) / 3.0;
    wait_for_window(
        &mut session,
        "a third window is placed by the edited module",
        "c",
        |window| near(window.height, third) && near(window.width, width),
    );
    wait_marker(&mut session, &marker, "second:c");
    assert_eq!(
        lines_of(&marker, "second:c"),
        1,
        "no listener twice: {}",
        session.log_text()
    );
    assert_eq!(
        lines_of(&marker, "first:c"),
        0,
        "the old generation's listener never runs again"
    );
    assert_eq!(lines_of(&marker, "first:a"), 1);
    assert_eq!(lines_of(&marker, "first:b"), 1);

    // A broken edit is reported at its line and the generation that serves stays: a window is still placed, in four rows.
    let (broken, line) = (language.broken)(&(language.module_text)(true));
    save(&module, &broken);
    session.wait_log("the broken edit is reported at its file and line", |text| {
        text.contains(&format!("{}:{line}", language.module)) && text.contains("did not reload")
    });
    mapper.map_window("d", "D");
    let fourth = (height - 3.0 * gap) / 4.0;
    wait_for_window(
        &mut session,
        "the serving generation still places windows",
        "d",
        |window| near(window.height, fourth),
    );
    assert_eq!(lines_of(&marker, "second:d"), 1, "{}", session.log_text());

    // A fix loads as the next generation.
    let reloads = |session: &Gnoblin| {
        session
            .log()
            .iter()
            .filter(|line| line.text.contains("the shell reloaded"))
            .count()
    };
    let reloaded = reloads(&session);
    save(&shell, &(language.shell_text)("third", &marker));
    save(&module, &(language.module_text)(true));
    session.wait_for("the fix is reloaded as the next generation", |session| {
        reloads(session) > reloaded
    });
    mapper.map_window("e", "E");
    wait_marker(&mut session, &marker, "third:e");
    assert_eq!(lines_of(&marker, "third:e"), 1);
    assert_eq!(lines_of(&marker, "second:e"), 0);
    mapper.roundtrip();
}

#[test]
fn example_11_a_ts_shell_and_the_module_it_imports_reload_in_place_while_a_client_stays_connected() {
    run(&TYPESCRIPT);
}

#[test]
fn example_11_a_python_shell_and_the_module_it_imports_reload_in_place_while_a_client_stays_connected() {
    run(&PYTHON);
}
