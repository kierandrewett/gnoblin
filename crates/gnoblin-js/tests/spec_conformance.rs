//! How much of the owner's specification the JS shell runs today (design/js-research/owner-spec.md).
//!
//! The owner's reference `init.js` and the 128 code fences of the specification live verbatim in `tests/spec/` (as
//! `*.txt`, so no formatter or linter rewrites the owner's input). Each one is loaded into a freshly booted `JsShell`
//! over the fake compositor of `tests/common`, behind a prelude that declares the variables a snippet assumes (the
//! specification's section 19: `workspace`, `client`, `display`, `camera` and the rest exist in the user's scope). The
//! outcome is one of `runs`, `fails: <first error>` or `skipped: <reason>`, written to `design/js-spec-conformance.md`.
//!
//! The test never fails on a `fails` entry: that is the gap between the shell and the specification, recorded, not
//! asserted. It asserts that the harness works (the number of snippets) and that the table in the repository is current.
//! `UPDATE_SPEC=1 cargo test -p gnoblin-js --test spec_conformance` rewrites the table.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use gnoblin_js::JsShell;
use gnoblin_tree_client::tree_mirror::MirrorValue;

mod common;
use common::{FakeCompositor, ROOT, timed_config};

/// The number of code fences the specification's archive holds, besides `init.js`.
const SNIPPET_COUNT: usize = 128;

/// The longest error text a table row keeps.
const ERROR_WIDTH: usize = 200;

/// What a snippet may assume: the specification's section 19 says the workspace, display, client and camera variables
/// exist in the user's scope, and the fragments name more (profiles, layouts, panels). Everything the library exports is
/// global too, as for a script that imported all of it. Each fixture is built on first use, and a fixture whose class
/// the shell lacks throws the name of that class, which is the failure the table should show.
const PRELUDE: &str = r##"import * as G from "gnoblin";

const { gnoblin: _runtime, ...exported } = G;
Object.assign(globalThis, exported);

const REGISTER = __REGISTER__;

const need = (name) => {
    if (typeof G[name] !== "function") {
        throw new Error(`gnoblin exports no class ${name}`);
    }
    return G[name];
};

const lazy = (name, build) => {
    const settle = (value) => Object.defineProperty(globalThis, name, { value, writable: true, configurable: true });
    Object.defineProperty(globalThis, name, {
        configurable: true,
        get() {
            let value;
            try {
                value = build();
            } catch (error) {
                throw new Error(`fixture ${name}: ${error.message}`);
            }
            settle(value);
            return value;
        },
        set: settle,
    });
};

// The global style of the specification's section 6, for a snippet that indexes into a list that section builds.
if (__STYLE__) {
    gnoblin.clients.style.cornerRadius = 12;
    gnoblin.clients.style.borders = [
        { width: 1, color: "#ffffff40" },
        { width: 2, color: "#7c3aed", offset: 3 },
    ];
    gnoblin.clients.style.shadows = [
        { x: 0, y: 8, blur: 24, color: "#00000066" },
        { x: 0, y: 0, blur: 12, spread: 2, color: "#8b5cf655" },
        { inset: true, x: 0, y: 0, blur: 4, color: "#ffffff22" },
    ];
}

// Workspaces: the snippets read them by id and by name. A snippet that makes its own workspaces registers them itself.
const workspaceIds = ["desktop", "development", "web", "gaming", "dev", "work", "media"];
const layouts = {
    development: () => new (need("MasterStackLayout"))({ ratio: 0.65, gap: 12 }),
    web: () => new (need("GridLayout"))({ columns: "auto", gap: 8 }),
};
const workspaces = {};
let workspaceFailure = null;
try {
    for (const id of workspaceIds) {
        const layout = (layouts[id] ?? (() => new (need("FloatingLayout"))({ placement: "smart" })))();
        workspaces[id] = new (need("Workspace"))({ id, name: id, layout });
        if (REGISTER) {
            gnoblin.workspaces.add(workspaces[id]);
        }
    }
} catch (error) {
    workspaceFailure = error;
}
for (const id of workspaceIds) {
    lazy(id, () => {
        if (workspaceFailure) {
            throw workspaceFailure;
        }
        return workspaces[id];
    });
}
lazy("workspace", () => globalThis.desktop);

// Clients: the fake compositor maps a Firefox, a terminal and a Nautilus window before the shell boots.
const clientOf = (appId) => {
    const found = gnoblin.clients.find((client) => client.appId === appId);
    if (!found) {
        throw new Error(`no client with app id ${appId}`);
    }
    return found;
};
lazy("client", () => clientOf("firefox"));
lazy("firefox", () => clientOf("firefox"));
lazy("firefoxClient", () => clientOf("firefox"));
lazy("terminal", () => clientOf("com.mitchellh.ghostty"));

// Displays and the objects around them.
lazy("display", () => {
    if (!gnoblin.displays) {
        throw new Error("gnoblin.displays does not exist");
    }
    const found = gnoblin.input.primary.activeDisplay ?? gnoblin.displays.at(0);
    if (!found) {
        throw new Error("no Display exists");
    }
    return found;
});
lazy("camera2D", () => new (need("Camera2D"))());
lazy("camera3D", () => new (need("Camera3D"))({ projection: "perspective", fov: 65, position: { x: 0, y: 0, z: 1600 } }));
lazy("camera", () => globalThis.camera3D);
lazy("arcLayout", () => new (need("ArcLayout"))({ radius: 1200, spacing: 0.35 }));
lazy("spatialRoom", () => new (need("Node"))({ id: "xr-room" }));
lazy("room", () => new (need("Node"))({ id: "xr-room" }));
lazy("background", () => new (need("Node"))({ id: "background" }));
lazy("panel", () => new (need("View"))({ content: globalThis.client, size: { width: 0.9, height: 0.55 } }));
lazy("initialTransform", () => ({ position: { x: 0, y: 0, z: 0 }, rotation: { x: 0, y: 0, z: 0 }, scale: 1 }));
lazy("runThrowSimulation", () => () => {});

const profile = (match, position) => new (need("OutputProfile"))({ match, mode: "preferred", position });
lazy("ultrawide", () => profile({ connector: "DP-1" }, { x: 0, y: 0 }));
lazy("side", () => profile({ connector: "HDMI-A-1" }, { x: 3440, y: 0 }));
lazy("mainProfile", () => profile({ connector: "DP-1" }, { x: 0, y: 0 }));
lazy("secondProfile", () => profile({ connector: "HDMI-A-1" }, { x: 3440, y: 0 }));
const displayOver = (output, content) =>
    new (need("Display"))({
        outputs: [output],
        viewport: new (need("Viewport"))({ content, camera: new (need("Camera2D"))() }),
    });
lazy("leftDisplay", () => displayOver(globalThis.ultrawide, globalThis.development));
lazy("rightDisplay", () => displayOver(globalThis.side, globalThis.web));

// XR: the extension is optional, so a snippet that uses it unguarded needs it loaded.
lazy("xr", () => {
    const extension = gnoblin.extensions?.xr;
    if (!extension) {
        throw new Error("gnoblin.extensions.xr is not available");
    }
    return extension;
});
// `spatial` is the XR extension's layout in the XR fences and a plain arc layout in the 2D-to-3D ones.
lazy("spatial", () =>
    __XR__ ? new globalThis.xr.SpatialLayout({ radius: 1.6, spacing: 0.35 }) : globalThis.arcLayout,
);
"##;

/// A module a snippet imports from the user's own tree (specification section 3).
const DEVELOPMENT_WORKSPACE: &str = r#"import { Workspace, MasterStackLayout } from "gnoblin";

export class DevelopmentWorkspace extends Workspace {
    constructor() {
        super({ id: "dev", layout: new MasterStackLayout({ ratio: 0.7, gap: 10 }) });
    }
}
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    Runs,
    Fails(String),
    Skipped(String),
}

struct Row {
    file: String,
    outcome: Outcome,
    note: String,
}

fn spec_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("spec")
}

fn table_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("design")
        .join("js-spec-conformance.md")
}

/// Boots a shell over a fake compositor with a screen and three windows, loads `init.js` of `files`, runs a few frames
/// and returns the value of `probe` (a script expression) afterwards. An error is the first thing that went wrong.
fn boot(files: &[(&str, String)], probe: Option<&str>, session_first: bool) -> Result<Option<String>, String> {
    let dir = tempfile::tempdir().map_err(|error| error.to_string())?;
    let root = fs::canonicalize(dir.path()).map_err(|error| error.to_string())?;
    for (name, text) in files {
        let path = root.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(&path, text).map_err(|error| error.to_string())?;
    }
    let comp = FakeCompositor::new();
    comp.add_node(
        20,
        ROOT,
        "screen",
        vec![
            ("name", MirrorValue::Text("DP-1".to_owned())),
            ("width", MirrorValue::Double(1920.0)),
            ("height", MirrorValue::Double(1080.0)),
            ("primary", MirrorValue::Bool(true)),
        ],
    );
    // The desktop's seat has the Firefox window focused and three devices; the lock is the one the shell expects.
    comp.add_node(
        21,
        ROOT,
        "seat",
        vec![
            ("name", MirrorValue::Text("seat0".to_owned())),
            ("pointer_x", MirrorValue::Double(0.0)),
            ("pointer_y", MirrorValue::Double(0.0)),
            ("focus", MirrorValue::Node(3)),
            (
                "device",
                MirrorValue::Texts(vec![
                    "8:Logitech MX Master:pointer".to_owned(),
                    "9:USB Keyboard:keyboard".to_owned(),
                    "10:Touchpad:pointer,touchpad".to_owned(),
                ]),
            ),
        ],
    );
    comp.add_node(22, ROOT, "lock", Vec::new());
    // The XR program's session: a seat named `xr` that belongs to no parent. With it, `gnoblin.extensions.xr` exists,
    // which the specification's XR fences assume.
    comp.add_node(50, 0, "seat", vec![("name", MirrorValue::Text("xr".to_owned()))]);
    for (id, title, app_id) in [
        (3, "Mozilla Firefox", "firefox"),
        (4, "Terminal", "com.mitchellh.ghostty"),
        (5, "Files", "org.gnome.Nautilus"),
    ] {
        let mut props = vec![
            ("title", MirrorValue::Text(title.to_owned())),
            ("app_id", MirrorValue::Text(app_id.to_owned())),
            ("decoration", MirrorValue::Text("server".to_owned())),
            ("width", MirrorValue::Double(400.0)),
            ("height", MirrorValue::Double(300.0)),
        ];
        if id == 3 {
            props.push(("focused", MirrorValue::Bool(true)));
        }
        comp.add_node(id, ROOT, "window", props);
    }
    let (config, _clock) = timed_config(&root);
    let outcome = (|| {
        let mut shell = JsShell::new(Box::new(comp.clone()), config).map_err(|error| error.to_string())?;
        // The frames before the entry let the shell see the tree it booted over: the XR program's session is a seat in
        // it, and `gnoblin.extensions.xr` exists only after the shell has. They also give a window its default
        // workspace, so a fence that makes workspaces of its own must load first.
        if session_first {
            for _ in 0..6 {
                shell.frame().map_err(|error| error.to_string())?;
            }
        }
        let mut diagnostics = shell
            .load_entry(Path::new("init.js"))
            .map_err(|error| error.to_string())?;
        for _ in 0..6 {
            diagnostics.extend(shell.frame().map_err(|error| error.to_string())?.diagnostics);
        }
        // The fake compositor applies no effects, so "not applied" notices are the rig, not the snippet.
        if let Some(first) = diagnostics.iter().find(|line| !line.starts_with("not applied")) {
            return Err(format!("diagnostic: {first}"));
        }
        match probe {
            Some(expression) => shell.eval(expression).map_err(|error| error.to_string()),
            None => Ok(None),
        }
    })();
    let root_text = root.to_string_lossy().into_owned();
    outcome.map_err(|text| text.replace(&root_text, "<root>"))
}

/// The names `import { .. } from "gnoblin"` can bring in, and those of `gnoblin/xr`.
fn exports() -> (BTreeSet<String>, BTreeSet<String>) {
    let entry = "import * as core from \"gnoblin\";\nimport * as xr from \"gnoblin/xr\";\n\
                 globalThis.__names = Object.keys(core).join(\",\") + \"\\n\" + Object.keys(xr).join(\",\");\n";
    let text = boot(&[("init.js", entry.to_owned())], Some("globalThis.__names"), false)
        .expect("the library modules load")
        .expect("the probe has a value");
    let mut lines = text.lines();
    let set = |line: Option<&str>| {
        line.unwrap_or_default()
            .split(',')
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .collect::<BTreeSet<_>>()
    };
    (set(lines.next()), set(lines.next()))
}

/// One line of an error as a table cell keeps it: ASCII, no markup characters, bounded.
fn clean(text: &str) -> String {
    let line = text.lines().find(|line| !line.trim().is_empty()).unwrap_or_default();
    let mut out = String::new();
    for ch in line.split_whitespace().collect::<Vec<_>>().join(" ").chars() {
        out.push(match ch {
            '`' => '\'',
            '|' => '/',
            ch if ch.is_ascii() && !ch.is_ascii_control() => ch,
            _ => '?',
        });
    }
    if out.len() > ERROR_WIDTH {
        out.truncate(ERROR_WIDTH - 3);
        out.push_str("...");
    }
    out
}

/// The names a snippet imports from `gnoblin` that the library does not export, in order.
fn missing_imports(source: &str, core: &BTreeSet<String>) -> Vec<String> {
    let mut missing = Vec::new();
    for (names, _) in gnoblin_imports(source) {
        for name in names {
            if !core.contains(&name) && !missing.contains(&name) {
                missing.push(name);
            }
        }
    }
    missing
}

/// Each `import { a, b } from "gnoblin";` of `source`: the names and the byte range of the statement.
fn gnoblin_imports(source: &str) -> Vec<(Vec<String>, std::ops::Range<usize>)> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = source[from..].find("import") {
        let start = from + at;
        from = start + "import".len();
        let rest = &source[from..];
        let Some(open) = rest.find('{') else { break };
        if !rest[..open].trim().is_empty() {
            continue;
        }
        let Some(close) = rest.find('}') else { break };
        let tail = &rest[close + 1..];
        let Some(end) = tail.find(';') else { break };
        if tail[..end].trim().trim_start_matches("from").trim() != "\"gnoblin\"" {
            continue;
        }
        let names = rest[open + 1..close]
            .split(',')
            .map(|name| name.trim().split(" as ").last().unwrap_or_default().trim().to_owned())
            .filter(|name| !name.is_empty())
            .collect();
        found.push((names, start..from + close + 1 + end + 1));
        from += close + 1 + end + 1;
    }
    found
}

/// `source` with every name of its `gnoblin` imports that the library lacks declared as a class that throws on use, so
/// that the first error past the import shows.
fn stub_missing_imports(source: &str, core: &BTreeSet<String>) -> String {
    let mut out = String::new();
    let mut at = 0;
    for (names, range) in gnoblin_imports(source) {
        out.push_str(&source[at..range.start]);
        let (present, absent): (Vec<_>, Vec<_>) =
            names.iter().map(String::as_str).partition(|name| core.contains(*name));
        // One line stands for the statement's lines, so a later error keeps the snippet's own line number.
        if !present.is_empty() {
            out.push_str(&format!("import {{ {} }} from \"gnoblin\"; ", present.join(", ")));
        }
        for name in absent {
            out.push_str(&format!(
                "const {name} = class {{ constructor() {{ throw new Error(\"gnoblin exports no class {name}\"); }} }}; "
            ));
        }
        out.push_str(&"\n".repeat(source[range.clone()].matches('\n').count()));
        at = range.end;
    }
    out.push_str(&source[at..]);
    out
}

/// Whether a fence makes workspaces of its own, rather than reading ones that exist.
fn makes_workspaces(source: &str) -> bool {
    source.contains("workspaces.add") || source.contains("new Workspace") || source.contains("extends Workspace")
}

/// Loads one JS fragment: as the entry `init.js` itself (`entry`), or as a module behind the prelude.
fn run_js(source: &str, entry: bool) -> Result<(), String> {
    let mut files = vec![("workspaces/DevelopmentWorkspace.js", DEVELOPMENT_WORKSPACE.to_owned())];
    let own_workspaces =
        source.contains("workspaces.add") || source.contains("new Workspace") || source.contains("extends Workspace");
    // A fence about the XR extension needs the session the shell sees after its first frames; one that makes
    // workspaces of its own must load before them (see `boot`), so for it the extension is absent at load, as it is for
    // an `init.js` that runs before the headset is on.
    let session_first = source.contains("xr") && !own_workspaces;
    if entry {
        files.push(("init.js", source.to_owned()));
    } else {
        let styled = source.contains("borders[") || source.contains("shadows[");
        // A snippet that makes its own workspaces registers them; the others read workspaces that exist.
        let register = !own_workspaces && !session_first;
        files.push((
            "prelude.js",
            PRELUDE
                .replace("__REGISTER__", if register { "true" } else { "false" })
                .replace("__STYLE__", if styled { "true" } else { "false" })
                .replace("__XR__", if source.contains("xr.") { "true" } else { "false" }),
        ));
        files.push(("snippet.js", source.to_owned()));
        files.push((
            "init.js",
            "import \"./prelude.js\";\nimport \"./snippet.js\";\n".to_owned(),
        ));
    }
    boot(&files, None, session_first && !entry).map(drop)
}

/// An error as a row keeps it: the root and the prelude's own location are noise, and a QuickJS "not a function" names
/// nothing, so the line of the snippet it points at follows.
fn tidy(error: &str, source: &str) -> String {
    // A class the library lacks is declared as a stub that throws: the name is the finding, not where it threw.
    if let Some(at) = error.find("gnoblin exports no class ") {
        let name = error[at..].split_whitespace().nth(4).unwrap_or_default();
        return clean(&format!(
            "gnoblin exports no class {}",
            name.trim_matches(|ch: char| !ch.is_alphanumeric())
        ));
    }
    let mut text = clean(&error.replace("<root>/prelude.js:", "prelude.js:"));
    if let Some(rest) = text.strip_prefix("prelude.js:") {
        // `prelude.js:LINE:COL: message`: the position is the harness's, the message is what matters.
        text = rest.splitn(3, ':').nth(2).unwrap_or(rest).trim().to_owned();
    }
    let error = error.replace("<root>/", "");
    for file in ["snippet.js", "init.js"] {
        let Some(rest) = error.strip_prefix(&format!("{file}:")) else {
            continue;
        };
        let mut parts = rest.splitn(3, ':');
        let line = parts.next().and_then(|line| line.parse::<usize>().ok());
        if let (Some(line), Some(message)) = (line, parts.nth(1)) {
            let shown = source.lines().nth(line - 1).unwrap_or_default().trim();
            return clean(&format!("{file}:{line}:{message}  @ {shown}"));
        }
    }
    text
}

fn judge_js(source: &str, entry: bool, core: &BTreeSet<String>) -> (Outcome, String) {
    let mut note = String::new();
    // A fence that makes workspaces loads before the shell has seen the XR session, so its `xr?.session` guard is false.
    let guarded = source.contains("xr?.session") && (entry || makes_workspaces(source));
    let outcome = match run_js(source, entry) {
        Ok(()) => {
            if guarded {
                note =
                    "the body is guarded by xr?.session, which is absent while the file loads, so it is not exercised"
                        .to_owned();
            }
            Outcome::Runs
        }
        Err(error) => {
            let missing = missing_imports(source, core);
            if !missing.is_empty() {
                // What a user sees first is the import; what the snippet does past it is worth knowing too.
                let stubbed = stub_missing_imports(source, core);
                note = match run_js(&stubbed, entry) {
                    Ok(()) => "past the imports it runs".to_owned(),
                    Err(next) => format!("past the imports: {}", tidy(&next, source)),
                };
            }
            Outcome::Fails(tidy(&error, source))
        }
    };
    (outcome, note)
}

/// A `.ts` fence is a declaration, not a program: check that the classes it declares exist where the specification
/// puts them (members are what the JS fences exercise).
fn judge_ts(source: &str, core: &BTreeSet<String>, xr: &BTreeSet<String>) -> (Outcome, String) {
    let declared: Vec<&str> = source
        .lines()
        .filter_map(|line| {
            let line = line.strip_prefix("export ").unwrap_or(line);
            let line = line.strip_prefix("abstract ").unwrap_or(line);
            let rest = line.strip_prefix("class ")?;
            rest.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_').next()
        })
        .collect();
    if declared.is_empty() {
        return (
            Outcome::Skipped("interfaces only, no runtime value to check".to_owned()),
            String::new(),
        );
    }
    let absent: Vec<&str> = declared
        .iter()
        .copied()
        .filter(|name| {
            if name.starts_with("XR") {
                !xr.contains(*name)
            } else {
                !core.contains(*name)
            }
        })
        .collect();
    if absent.is_empty() {
        (
            Outcome::Runs,
            "declaration: the classes exist, their members are not checked".to_owned(),
        )
    } else {
        (
            Outcome::Fails(format!("no export named {}", absent.join(", "))),
            "declaration".to_owned(),
        )
    }
}

/// `init` is the reference config: a fence that is the same text (the specification's section 20) is loaded the same way.
fn judge(file: &str, source: &str, init: &str, core: &BTreeSet<String>, xr: &BTreeSet<String>) -> (Outcome, String) {
    let skipped = |reason: &str| (Outcome::Skipped(reason.to_owned()), String::new());
    match file.rsplit('.').next().unwrap_or_default() {
        "js" => judge_js(source, file == "init.js" || source == init, core),
        "ts" => judge_ts(source, core, xr),
        "txt" => skipped("a diagram, not code"),
        "json" => skipped("an extension manifest, not code"),
        "c" => skipped("a C ABI sketch, not JavaScript"),
        other => skipped(&format!("not code (.{other})")),
    }
}

fn collect() -> Vec<Row> {
    let (core, xr) = exports();
    let dir = spec_dir();
    let mut names: Vec<(String, PathBuf)> = fs::read_dir(dir.join("snippets"))
        .expect("tests/spec/snippets exists")
        .map(|entry| entry.expect("a directory entry").path())
        .map(|path| (path.file_name().expect("a name").to_string_lossy().into_owned(), path))
        .collect();
    names.sort();
    let init = fs::read_to_string(dir.join("init.js.txt")).expect("the reference config reads");
    let mut rows = Vec::new();
    // The reference file first, then the fences in the specification's order.
    let mut files = vec![("init.js.txt".to_owned(), dir.join("init.js.txt"))];
    files.extend(names);
    for (name, path) in files {
        let file = name
            .strip_suffix(".txt")
            .expect("a verbatim copy is stored as .txt")
            .to_owned();
        let source = fs::read_to_string(&path).expect("the snippet reads");
        let (outcome, note) = judge(&file, &source, &init, &core, &xr);
        rows.push(Row { file, outcome, note });
    }
    rows
}

fn cell(row: &Row) -> String {
    match &row.outcome {
        Outcome::Runs => "runs".to_owned(),
        Outcome::Fails(error) => format!("fails: `{error}`"),
        Outcome::Skipped(reason) => format!("skipped: {reason}"),
    }
}

/// The table as prettier lays it out: every column as wide as its widest cell.
fn table(rows: &[Row]) -> String {
    let mut lines = vec![vec!["File".to_owned(), "Result".to_owned(), "Note".to_owned()]];
    for row in rows {
        let note = if row.note.is_empty() {
            String::new()
        } else {
            clean_note(&row.note)
        };
        lines.push(vec![format!("`{}`", row.file), cell(row), note]);
    }
    let widths: Vec<usize> = (0..3)
        .map(|column| lines.iter().map(|line| line[column].len()).max().unwrap_or(3).max(3))
        .collect();
    let render = |cells: Vec<String>| {
        let padded: Vec<String> = cells
            .iter()
            .zip(&widths)
            .map(|(text, width)| {
                let width = *width;
                format!("{text:<width$}")
            })
            .collect();
        format!("| {} |\n", padded.join(" | "))
    };
    let mut out = render(lines[0].clone());
    out.push_str(&render(widths.iter().map(|width| "-".repeat(*width)).collect()));
    for line in &lines[1..] {
        out.push_str(&render(line.clone()));
    }
    out
}

/// A note is prose with the error text of a later step inside it: keep it one table-safe line.
fn clean_note(note: &str) -> String {
    match note.split_once(": ") {
        Some((lead, error)) => format!("{lead}: `{}`", error.replace('`', "'")),
        None => note.to_owned(),
    }
}

fn document(rows: &[Row]) -> String {
    let count = |want: fn(&Outcome) -> bool| rows.iter().filter(|row| want(&row.outcome)).count();
    let runs = count(|outcome| matches!(outcome, Outcome::Runs));
    let fails = count(|outcome| matches!(outcome, Outcome::Fails(_)));
    let skipped = count(|outcome| matches!(outcome, Outcome::Skipped(_)));
    let mut out = String::new();
    out.push_str("# Spec conformance of the JS shell\n\n");
    out.push_str("Generated by spec_conformance.rs. Regenerate with `UPDATE_SPEC=1 cargo test -p gnoblin-js --test spec_conformance`; do not edit by hand.\n\n");
    out.push_str("Each row is one file of the owner's specification archive (design/js-research/owner-spec.md), loaded verbatim into a freshly booted shell over the fake compositor of `tests/common`: a screen, a seat with a mouse, a keyboard and a touchpad, and three windows (Firefox focused, a terminal and Nautilus).\n\n");
    out.push_str("A fence is a fragment, so it runs behind a prelude that declares what it assumes (the specification's section 19: `workspace`, `client`, `display`, `camera`, the workspaces, profiles, layouts and cameras the earlier fences made). Everything the library exports is in scope, as for a script that imported it all. A fixture whose class the shell lacks throws that class's name, which is what a row then shows. A fence about the XR extension loads after the shell has seen the XR program's session; one that makes workspaces of its own loads before it, so its XR guard is false.\n\n");
    out.push_str("`runs` means the file evaluated without throwing. `fails` names the first error: a missing class, property or method; where the engine does not name it (`not a function`), the line of the fence follows after `@`. `skipped` is a fragment that is not code. A `.ts` fence is a declaration: it runs when every class it declares exists, and its members are not checked. The note column gives the first error past a missing import (the import is declared as a class that throws, so the rest of the file shows), and says when a guard keeps a body from running.\n\n");
    out.push_str(&format!(
        "{} files: {runs} run, {fails} fail, {skipped} skipped.\n\n",
        rows.len()
    ));
    out.push_str(&table(rows));
    out
}

/// A table with its whitespace and dash runs folded, so that a formatter's column padding does not count.
fn normalise(text: &str) -> String {
    text.lines()
        .map(|line| {
            if line.starts_with('|') {
                line.split('|')
                    .map(|cell| {
                        let cell = cell.trim();
                        if !cell.is_empty() && cell.chars().all(|ch| ch == '-') {
                            "-"
                        } else {
                            cell
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("|")
            } else {
                line.trim().to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

#[test]
fn the_shell_against_the_owners_own_examples() {
    let rows = collect();
    let snippets = rows.len() - 1;
    assert_eq!(rows[0].file, "init.js", "the reference config is the first row");
    assert_eq!(
        snippets, SNIPPET_COUNT,
        "the archive holds {SNIPPET_COUNT} fences; tests/spec/snippets holds {snippets}"
    );
    // The harness itself: the fences that are code must have been judged, not skipped wholesale.
    let judged = rows
        .iter()
        .filter(|row| !matches!(row.outcome, Outcome::Skipped(_)))
        .count();
    assert!(judged > 100, "only {judged} fences were judged; the harness is broken");
    let expected = document(&rows);
    let path = table_path();
    if std::env::var_os("UPDATE_SPEC").is_some() {
        fs::write(&path, &expected).expect("the table is written");
        // The instance a test runs on is not the checkout: the table is printed for `--nocapture` to bring back.
        println!("---BEGIN-SPEC-TABLE---\n{expected}---END-SPEC-TABLE---");
        return;
    }
    let current = fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        normalise(&current),
        normalise(&expected),
        "design/js-spec-conformance.md is stale: run `UPDATE_SPEC=1 cargo test -p gnoblin-js --test spec_conformance`"
    );
}
