//! How much of the owner's specification the JS shell runs today (design/js-research/owner-spec.md).
//!
//! The owner's reference `init.js` and 128 fences live verbatim in `tests/spec/` as `.txt` files.
//! `expected.toml` records each file's accepted status and SHA-256, making behavior changes and input edits explicit.
//! Snippet fixtures are inventoried above `PRELUDE`; the reference `init.js` runs without that prelude.
//! `UPDATE_SPEC=1` updates the report and permits only intentional progress (`fails` -> `runs`).

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use gnoblin_js::JsShell;
use gnoblin_tree_client::tree_mirror::MirrorValue;
use sha2::{Digest, Sha256};

mod common;
use common::{FakeCompositor, ROOT, timed_config};

/// The number of code fences the specification's archive holds, besides `init.js`.
const SNIPPET_COUNT: usize = 128;

/// The longest error text a table row keeps.
const ERROR_WIDTH: usize = 200;

/// Per-fence prelude fixture inventory (fence numbers; `128` is the final full-init fence):
/// `workspace`: 004, 019, 036, 044, 049, 051-052, 054, 059, 078, 080-081, 085-086, 100-102, 104, 111, 128.
/// `desktop`: 018, 023-024, 027, 034-036, 092, 105, 108-109, 128. `development`: 003, 009, 011, 018-019,
/// 031-033, 036, 078, 085, 095, 105, 107, 112, 128. `web`: 018, 031, 036, 038, 106, 123, 128.
/// `gaming`: 017, 036, 095. `dev`: 020-021. `work`: 017, 048. `media`: 017.
/// `client`: 001, 004-007, 009, 012, 014, 019-020, 038, 044, 048-051, 054, 064, 069, 072, 074, 079, 082-084,
/// 094, 097-098, 102, 121, 124, 128. `firefox`: 006, 009, 013, 099, 125, 128. `firefoxClient`: 026.
/// `terminal`: 125. `display`: 021, 032, 036, 044, 053, 059, 078, 080-081, 106, 110-111, 123, 128.
/// `camera2D`: 059, 080, 085, 128. `camera3D`: 059, 077, 080-081, 083-084, 128.
/// `camera`: 002, 022-025, 031, 033-035, 059-061, 077, 080-081, 085, 107-111, 114, 121, 128.
/// `arcLayout`: 076, 128. `spatialRoom`: 026, 061. `room`: 004, 064-065, 114, 128.
/// `background`: 015, 085, 126. `panel`: 022, 026, 064, 079. `initialTransform`: 124.
/// `runThrowSimulation`: 071. `ultrawide`: 028, 031, 034-035. `side`: 028, 031, 034-035, 128.
/// `mainProfile`, `secondProfile`: 108-109. `leftDisplay`, `rightDisplay`: 031, 033, 107.
/// `xr`: 001-002, 061-062, 064-071, 088-091, 093, 114-118, 128.
/// `spatial`: 059, 065-068, 079-081, 115-117.
/// The style fixture is injected by `__STYLE__` for fences 010-017. Base module exports are global, not fixtures.
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

impl Outcome {
    fn status(&self) -> &'static str {
        match self {
            Self::Runs => "runs",
            Self::Fails(_) => "fails",
            Self::Skipped(_) => "skipped",
        }
    }

    fn first_error(&self) -> &str {
        match self {
            Self::Fails(error) => error,
            _ => "",
        }
    }
}

struct Row {
    file: String,
    outcome: Outcome,
    note: String,
}

fn status_mismatch(file: &str, expected: &str, actual: &Outcome) -> String {
    format!(
        "{file}: expected {expected}, actual {} (first error: {})",
        actual.status(),
        if actual.first_error().is_empty() {
            "(none)"
        } else {
            actual.first_error()
        }
    )
}

/// During explicit regeneration, only a failure becoming a pass is automatic progress.
fn upgraded_status(expected: &str, actual: &str) -> Result<&'static str, String> {
    match (expected, actual) {
        ("runs", "runs") => Ok("runs"),
        ("fails", "fails") => Ok("fails"),
        ("skipped", "skipped") => Ok("skipped"),
        ("fails", "runs") => Ok("runs"),
        _ => Err(format!(
            "UPDATE_SPEC refuses status change {expected} -> {actual}; new failures and runs -> fails downgrades require a hand edit of tests/spec/expected.toml with a reason comment"
        )),
    }
}

fn expected_manifest() -> toml::Value {
    toml::from_str(&fs::read_to_string(expected_path()).expect("tests/spec/expected.toml reads"))
        .expect("tests/spec/expected.toml parses")
}

fn verify_input_hashes(files: &[(String, PathBuf)], manifest: &toml::Value) {
    let expected = manifest["sha256"].as_table().expect("expected.toml has [sha256]");
    let actual_names: BTreeSet<_> = files.iter().map(|(name, _)| name.as_str()).collect();
    let expected_names: BTreeSet<_> = expected.keys().map(String::as_str).collect();
    assert_eq!(
        expected_names, actual_names,
        "expected.toml [sha256] must cover exactly all input files"
    );
    for (name, path) in files {
        let bytes = fs::read(path).expect("spec input reads");
        let digest = sha256(&bytes);
        let recorded = expected[name].as_str().expect("each recorded SHA-256 is a string");
        assert_eq!(
            digest, recorded,
            "owner input {name} changed; preserve its bytes and update only with owner authorization"
        );
    }
}

fn spec_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("spec")
}

fn expected_path() -> PathBuf {
    spec_dir().join("expected.toml")
}

fn sha256(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(&mut output, "{byte:02x}").expect("writing a SHA-256 digest to String cannot fail");
    }
    output
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

/// An error without what moves when the library is edited: a library module's own `gnoblin/name:LINE:COL: ` position
/// and a generated object id (`View#4294967297`). A row that kept them would go stale on every edit of the stock
/// modules, not on a change of what the shell supports.
fn steady(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("gnoblin/") {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let position = tail.split_once(": ").and_then(|(head, after)| {
            let mut parts = head.rsplitn(3, ':');
            let (col, line) = (parts.next()?, parts.next()?);
            let is_number = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
            (is_number(col) && is_number(line)).then_some(after)
        });
        match position {
            Some(after) => rest = after,
            None => {
                out.push_str("gnoblin/");
                rest = &tail["gnoblin/".len()..];
            }
        }
    }
    out.push_str(rest);
    let mut folded = String::new();
    let mut chars = out.chars().peekable();
    while let Some(ch) = chars.next() {
        folded.push(ch);
        if ch == '#' {
            let mut digits = String::new();
            while let Some(digit) = chars.next_if(char::is_ascii_digit) {
                digits.push(digit);
            }
            folded.push_str(if digits.len() > 6 { "N" } else { &digits });
        }
    }
    folded
}

/// An error as a row keeps it: the root and the prelude's own location are noise, and a QuickJS "not a function" names
/// nothing, so the line of the snippet it points at follows.
fn tidy(error: &str, source: &str) -> String {
    let error = &steady(error);
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
    let mut files = vec![("init.js.txt".to_owned(), dir.join("init.js.txt"))];
    files.extend(names);
    verify_input_hashes(&files, &expected_manifest());
    let init = fs::read_to_string(dir.join("init.js.txt")).expect("the reference config reads");
    let mut rows = Vec::new();
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
    out.push_str("Generated by spec_conformance.rs. `expected.toml` is the acceptance baseline; `UPDATE_SPEC=1` regenerates this report and permits only `fails` -> `runs` status upgrades. A downgrade needs a reasoned hand edit to `expected.toml`.\n\n");
    out.push_str("Each row is one file of the owner's specification archive (design/js-research/owner-spec.md), loaded verbatim into a freshly booted shell over the fake compositor of `tests/common`: a screen, a seat with a mouse, a keyboard and a touchpad, and three windows (Firefox focused, a terminal and Nautilus).\n\n");
    out.push_str("A fence is a fragment, so it runs behind the documented prelude fixtures. The reference `init.js` runs without that prelude. A missing class in a lazy fixture remains a failure naming the missing class. A fence about the XR extension loads after the shell has seen the XR program's session; one that makes workspaces of its own loads before it, so its XR guard is false.\n\n");
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

fn check_expected_statuses(rows: &[Row], update: bool) {
    let mut manifest_text = fs::read_to_string(expected_path()).expect("expected.toml reads");
    let manifest = expected_manifest();
    let statuses = manifest["statuses"].as_table().expect("expected.toml has [statuses]");
    let expected_names: BTreeSet<_> = statuses.keys().map(String::as_str).collect();
    let actual_names: BTreeSet<_> = rows.iter().map(|row| row.file.as_str()).collect();
    if let Some(row) = rows.iter().find(|row| !expected_names.contains(row.file.as_str())) {
        panic!(
            "{}: expected no recorded status, actual {} (first error: {}); add a deliberate baseline status",
            row.file,
            row.outcome.status(),
            if row.outcome.first_error().is_empty() {
                "(none)"
            } else {
                row.outcome.first_error()
            }
        );
    }
    assert_eq!(
        expected_names, actual_names,
        "expected.toml [statuses] must cover exactly all report files"
    );

    for row in rows {
        let expected = statuses[&row.file].as_str().expect("each expected status is a string");
        let actual = row.outcome.status();
        if update {
            let accepted = upgraded_status(expected, actual).unwrap_or_else(|message| {
                panic!("{}; {}", status_mismatch(&row.file, expected, &row.outcome), message)
            });
            if accepted != expected {
                let old = format!("\"{}\" = \"{}\"", row.file, expected);
                let new = format!("\"{}\" = \"{}\"", row.file, accepted);
                let mut replaced = false;
                manifest_text = manifest_text
                    .lines()
                    .map(|line| {
                        if !replaced && line.trim_start().starts_with(&old) {
                            replaced = true;
                            line.replacen(&old, &new, 1)
                        } else {
                            line.to_owned()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(replaced, "expected.toml status line for {} was not found", row.file);
            }
        } else if expected != actual {
            panic!("{}", status_mismatch(&row.file, expected, &row.outcome));
        }
    }
    if update {
        fs::write(expected_path(), manifest_text).expect("upgraded expected statuses are written");
    }
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
    let judged = rows
        .iter()
        .filter(|row| !matches!(row.outcome, Outcome::Skipped(_)))
        .count();
    assert!(judged > 100, "only {judged} fences were judged; the harness is broken");
    let update = std::env::var_os("UPDATE_SPEC").is_some();
    check_expected_statuses(&rows, update);
    let expected = document(&rows);
    let path = table_path();
    if update {
        fs::write(&path, &expected).expect("the table is written");
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

#[test]
fn update_statuses_only_accept_fails_to_runs() {
    assert_eq!(upgraded_status("fails", "runs"), Ok("runs"));
    assert!(upgraded_status("runs", "fails").unwrap_err().contains("hand edit"));
    assert!(
        upgraded_status("skipped", "fails")
            .unwrap_err()
            .contains("new failures")
    );
    assert_eq!(upgraded_status("fails", "fails"), Ok("fails"));
    assert_eq!(upgraded_status("skipped", "skipped"), Ok("skipped"));
}

#[test]
fn missing_classes_in_fixtures_remain_visible() {
    for (fixture, class) in [("arcLayout", "ArcLayout")] {
        let error = run_js(&format!("globalThis.fixtureProbe = {fixture};"), false)
            .expect_err("the unavailable fixture's class must fail the snippet");
        assert!(
            error.contains(&format!("fixture {fixture}: gnoblin exports no class {class}")),
            "{fixture} fixture failure must name the missing class: {error}"
        );
    }
}

#[test]
#[ignore = "acceptance gate for the spec-native cutover: remove this ignore when the reference init.js runs; tracked in design/js-spec-native.md part 2"]
fn the_owners_reference_init_js_runs_unchanged() {
    let input = fs::read(spec_dir().join("init.js.txt")).expect("owner init.js reads as bytes");
    let manifest = expected_manifest();
    let expected_hash = manifest["sha256"]["init.js.txt"]
        .as_str()
        .expect("init.js checksum is recorded");
    assert_eq!(
        sha256(&input),
        expected_hash,
        "owner init.js must remain byte-for-byte unchanged"
    );
    let source = String::from_utf8(input).expect("owner init.js is UTF-8");
    let probe = "JSON.stringify({workspaces:[...gnoblin.workspaces].map(x=>x.id),displays:[...gnoblin.displays].map(x=>x.id),profiles:[...gnoblin.outputs.profiles].map(x=>x.match.connector),bindings:[...gnoblin.input.primary.bindings].map(x=>x.trigger)})";
    let state = boot(&[("init.js", source)], Some(probe), false)
        .expect("the owner's reference init.js evaluates without the snippet prelude")
        .expect("the acceptance probe returns state");
    for expected in [
        "desktop",
        "development",
        "web",
        "main-display",
        "side-display",
        "DP-1",
        "HDMI-A-1",
        "Super+Return",
        "Super+Q",
        "Super+1",
        "Super+2",
        "Super+3",
        "Super+Tab",
    ] {
        assert!(
            state.contains(expected),
            "reference init.js did not create {expected}: {state}"
        );
    }
}
