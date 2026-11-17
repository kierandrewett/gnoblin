//! Animation, styling and the scene of the native std against the TypeScript std (M7.6).
//!
//! The same scripted scenario runs twice. Once the TypeScript std (`data/js/gnoblin-std`) runs it in this process, through `JsShell`, over
//! the fake compositor; once the native shell of `fixtures/shell-anim` runs it in the launcher, over the same fake behind the tree link
//! of the core. Each step is one call a shell makes (a transition, a style assignment, a camera) and the writes it sends are printed one to
//! a line: the timed writes with their duration, delay, curve and `tween` node, the plain ones with their value, the effects a style
//! became, the nodes a scene made. The two lists of a step must be the same writes (node ids are renamed by the order the nodes were
//! made, since each shell numbers its own), and so must what the steps logged.
//!
//! The fake compositor records timed writes and does not play them, so the clock is virtual twice over: the compositor's word that a
//! track ended (`track_ended`) is sent by the scenario where the step says so, and each timed write prints the eased progress it has at
//! fixed times after it was sent, worked out from the curve, duration and delay it carries.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use gnoblin_js::{JsShell, JsShellConfig};
use gnoblin_shell_host::supervisor::ManualClock;
use gnoblin_testkit::compositor::{FakeCompositor, ROOT, text};
use gnoblin_tree_client::tree_mirror::{MirrorValue, TxOp};

#[path = "../../gnoblin-core-impl/tests/support/mod.rs"]
mod support;
use support::*;

/// What the compositor says after a step.
#[derive(Clone, Copy)]
enum Then {
    Nothing,
    /// The tracks of the step's writes ended at their targets.
    Completed,
    /// The tracks of the writes since the last held transition ran back to their start.
    RanBack,
}

/// The common preamble of the TypeScript shell.
const SOURCE: &str = "import \"gnoblin-std\";\n\
    import { MasterStackLayout } from \"gnoblin-std\";\n\
    import { Camera3D, View } from \"gnoblin-std/scene\";\n\
    import { Camera2D } from \"gnoblin-std/display\";\n\
    const g = globalThis as any;\n\
    g.g = g;\n\
    g.log = [];\n\
    g.c = (n: number) => gnoblin.clients.get(String(n));\n\
    g.err = (f: () => void) => { try { f(); return \"accepted\"; } catch (e) { return (e as Error).message; } };\n\
    Object.assign(g, { MasterStackLayout, Camera3D, View, Camera2D });\n";

const STAGED: &str = "g.held = gnoblin.requestTransition(() => { g.c(41).x = 100; g.c(41).rotation = [0, 0.5, 0]; }, \
    { duration: 100, delay: 50, easing: \"linear\", autoplay: false });\n\
    g.held.onFinished((r) => g.log.push(\"staged finished cancelled=\" + r.cancelled));";

/// The steps: a name, the TypeScript that does what the fixture's chord does, and what the compositor says after it.
fn steps() -> Vec<(&'static str, &'static str, Then)> {
    vec![
        (
            "basic",
            "g.t1 = gnoblin.requestTransition(() => { g.c(40).opacity = 0.5; g.c(40).x = 300; g.c(41).opacity = 0.25; }, \
             { duration: 200, easing: \"ease-in-out\" });\n\
             g.t1.onFinished((r) => g.log.push(\"basic finished cancelled=\" + r.cancelled));",
            Then::Completed,
        ),
        ("staged", STAGED, Then::Nothing),
        ("seek", "g.held.progress = 0.5;", Then::Nothing),
        ("play", "g.held.play();", Then::Nothing),
        ("pause", "g.held.pause();", Then::Nothing),
        ("reverse", "g.held.reverse();", Then::RanBack),
        ("staged_again", STAGED, Then::Nothing),
        ("cancel", "g.held.cancel();", Then::Nothing),
        (
            "instant",
            "const t = gnoblin.requestTransition(() => { g.c(42).opacity = 0.75; g.c(42).x = 20; }, { duration: 0 });\n\
             t.onFinished((r) => g.log.push(\"instant finished cancelled=\" + r.cancelled));",
            Then::Nothing,
        ),
        (
            "layout",
            "g.ws = [...gnoblin.workspaces][0];\n\
             gnoblin.requestTransition(() => { g.ws.layout = new g.MasterStackLayout({ ratio: 0.6 }); }, { duration: 300 });",
            Then::Completed,
        ),
        (
            "style_global",
            "const s = gnoblin.clients.style;\n\
             s.shadows = [{ x: 0, y: 8, blur: 24, color: \"#00000066\" }, { blur: 4, color: \"#0004\" }];\n\
             s.borders = [{ width: 2, color: \"#ff8800\" }, { offset: 3 }];\n\
             s.backdropFilter = [{ blur: 12 }, { saturate: 1.2 }, { brightness: 1.1 }];\n\
             s.cornerRadius = 12;\n\
             s.opacity = 0.9;",
            Then::Nothing,
        ),
        (
            "style_selection",
            "gnoblin.clients.where({ appId: \"nautilus\" }).style.borders = [{ width: 3, color: \"#00ff00\" }];",
            Then::Nothing,
        ),
        (
            "style_client",
            "const s = g.c(40).style; s.opacity = 0.5; s.shadows = [];",
            Then::Nothing,
        ),
        (
            "style_unset",
            "g.c(40).style.unset(\"opacity\");\n\
             const cs = g.c(41).computedStyle;\n\
             g.log.push(\"computed 41 opacity=\" + cs.opacity + \" borders=\" + cs.borders.length + \" origin=\" + cs.origin.borders);",
            Then::Nothing,
        ),
        (
            "style_bad",
            "g.log.push(\"bad opacity: \" + g.err(() => { gnoblin.clients.style.opacity = 2; }));\n\
             g.log.push(\"bad border: \" + g.err(() => { g.c(42).style.borders = [{ width: -1 }]; }));",
            Then::Nothing,
        ),
        (
            "scene",
            "const cam = new g.Camera3D({ projection: \"perspective\", fov: 70, near: 0.2, position: { x: 1, y: 2, z: 3 } });\n\
             gnoblin.scene.root.add(cam);\n\
             const view = new g.View({ content: g.c(40), camera: cam, size: { width: 400, height: 300 } });\n\
             gnoblin.scene.root.add(view);\n\
             const flat = new g.Camera2D({ position: { x: 10, y: 20 }, zoom: 2, rotation: 0.5 });\n\
             gnoblin.scene.root.add(flat);\n\
             g.cam = cam; g.flat = flat;\n\
             g.log.push(\"camera size=\" + cam.size);\n\
             g.log.push(\"bad zoom: \" + g.err(() => { flat.zoom = 0; }));",
            Then::Nothing,
        ),
        (
            "scene_transition",
            "gnoblin.requestTransition(() => { g.flat.zoom = 4; g.flat.position = { x: 30, y: 40 }; g.flat.rotation = 1; g.cam.fov = 40; }, \
             { duration: 100 });\n\
             g.log.push(\"flat zoom=\" + g.flat.zoom + \" fov=\" + g.cam.fov);",
            Then::Completed,
        ),
    ]
}

// ---- the writes, as lines ---------------------------------------------------------------------------------------------------

/// The eased progress of a timed write `t_ms` after it was sent (`driver_anim.rs` has the same function for the native side).
fn eased(samples: &[f64], seconds: f64, delay: f64, t_ms: f64) -> f64 {
    let progress = ((t_ms / 1000.0 - delay) / seconds).clamp(0.0, 1.0);
    let at = progress * (samples.len() - 1) as f64;
    let low = at.floor() as usize;
    let high = (low + 1).min(samples.len() - 1);
    samples[low] + (samples[high] - samples[low]) * (at - at.floor())
}

/// One write as a line (`driver_anim.rs` prints the same for the native side).
fn op_line(op: &TxOp) -> Option<String> {
    match op {
        TxOp::New { id, class } => Some(format!("new id={id} class={class}")),
        TxOp::Set {
            node,
            key,
            value,
            seconds,
            delay,
            tween,
            samples,
        } => {
            let curve = samples.as_ref().map_or(String::new(), |samples| {
                samples
                    .iter()
                    .map(|sample| format!("{sample:.6}"))
                    .collect::<Vec<_>>()
                    .join(",")
            });
            let play = match (samples, seconds) {
                (Some(samples), Some(seconds)) if samples.len() >= 2 => [0.0, 25.0, 100.0, 200.0, 300.0]
                    .iter()
                    .map(|t| format!("{:.6}", eased(samples, *seconds, delay.unwrap_or(0.0), *t)))
                    .collect::<Vec<_>>()
                    .join(","),
                _ => String::new(),
            };
            Some(format!(
                "set node={node} key={key} value={value:?} seconds={seconds:?} delay={delay:?} tween={tween:?} samples={curve} play={play}"
            ))
        }
        TxOp::Insert { node, parent, index } => Some(format!("insert node={node} parent={parent} index={index}")),
        TxOp::Destroy { node } => Some(format!("destroy node={node}")),
        TxOp::Call { .. } => None,
    }
}

/// What a run printed: the writes of each step, in the order the steps ran, and what the steps logged.
struct Run {
    steps: Vec<(String, Vec<String>)>,
    log: Vec<String>,
}

/// Renames the ids of the nodes the run made to the order they were made in, and the digits of an error path (`Camera2D#12`) to `N`.
fn normalise(run: Run) -> Run {
    let mut made: BTreeMap<String, usize> = BTreeMap::new();
    for (_, lines) in &run.steps {
        for line in lines {
            if let Some(id) = line
                .strip_prefix("new id=")
                .and_then(|rest| rest.split_whitespace().next())
            {
                let next = made.len() + 1;
                made.entry(id.to_owned()).or_insert(next);
            }
        }
    }
    let rename = |line: &str| -> String {
        let mut out = String::new();
        let mut digits = String::new();
        let mut previous = ' ';
        let flush = |out: &mut String, digits: &mut String, previous: char| {
            if digits.is_empty() {
                return;
            }
            match made.get(digits.as_str()) {
                Some(index) if matches!(previous, '=' | '(') => out.push_str(&format!("#{index}")),
                _ => out.push_str(digits),
            }
            digits.clear();
        };
        for ch in line.chars() {
            if ch.is_ascii_digit() {
                if digits.is_empty() {
                    previous = out.chars().last().unwrap_or(' ');
                }
                digits.push(ch);
            } else {
                flush(&mut out, &mut digits, previous);
                out.push(ch);
            }
        }
        flush(&mut out, &mut digits, previous);
        out
    };
    let path_digits = |line: &str| -> String {
        let mut out = String::new();
        let mut after_hash = false;
        for ch in line.chars() {
            if ch.is_ascii_digit() && after_hash {
                continue;
            }
            after_hash = ch == '#' || (after_hash && ch.is_ascii_digit());
            out.push(ch);
            if ch == '#' {
                out.push('N');
            }
        }
        out
    };
    Run {
        steps: run
            .steps
            .into_iter()
            .map(|(name, lines)| (name, lines.iter().map(|line| rename(line)).collect()))
            .collect(),
        // The TypeScript query names the key `appId` where the native one names it `app_id`: the origin of a style layer shows it.
        log: run
            .log
            .iter()
            .map(|line| path_digits(&line.replace("\"appId\"", "\"app_id\"")))
            .collect(),
    }
}

/// The compositor's word that the timed writes sent since `from` ended, once for each property.
fn end_tracks(comp: &FakeCompositor, from: usize, completed: bool) {
    let mut ended = BTreeSet::new();
    for op in comp.ops_since(from) {
        if let TxOp::Set {
            node,
            key,
            seconds: Some(_),
            tween: Some(tween),
            ..
        } = op
            && ended.insert((node, key.clone()))
        {
            comp.track_ended(node, &key, completed, Some(tween));
        }
    }
}

// ---- the TypeScript std -----------------------------------------------------------------------------------------------------

/// The world of the launcher's scenarios: a 1000 by 800 screen, a seat and the window 40.
fn reference_world() -> FakeCompositor {
    let comp = FakeCompositor::new();
    let double = MirrorValue::Double;
    comp.add_node(
        20,
        ROOT,
        "screen",
        vec![
            ("screen_x", double(0.0)),
            ("screen_y", double(0.0)),
            ("screen_width", double(1000.0)),
            ("screen_height", double(800.0)),
            ("x", double(0.0)),
            ("y", double(0.0)),
            ("width", double(1000.0)),
            ("height", double(800.0)),
            ("primary", MirrorValue::Bool(true)),
        ],
    );
    comp.add_node(21, ROOT, "seat", vec![("name", text("seat0"))]);
    comp.add_window(40, "Terminal", "foot");
    comp
}

fn settle(shell: &mut JsShell) {
    for _ in 0..6 {
        let diagnostics = shell.frame().expect("a frame").diagnostics;
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }
}

/// Runs the scenario over the TypeScript std.
fn reference() -> Run {
    let comp = reference_world();
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = std::fs::canonicalize(dir.path()).expect("the temp dir exists");
    std::fs::write(root.join("shell.ts"), SOURCE).expect("shell.ts is written");
    let mut config = JsShellConfig::new(&root);
    config.entry_budget = Duration::from_secs(5);
    config.frame_budget = Duration::from_secs(10);
    config.clock = std::sync::Arc::new(ManualClock::default());
    let mut shell = JsShell::new(Box::new(comp.clone()), config).expect("the shell boots");
    let diagnostics = shell.load_entry(Path::new("shell.ts")).expect("shell.ts loads");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    for _ in 0..6 {
        shell.frame().expect("a frame");
    }
    settle(&mut shell);
    comp.create_window(41, "Files", "nautilus");
    settle(&mut shell);
    comp.create_window(42, "Editor", "code");
    settle(&mut shell);
    settle(&mut shell);
    let mut held_from = comp.sent_count();
    let mut recorded = Vec::new();
    for (name, snippet, then) in steps() {
        let mark = comp.sent_count();
        shell
            .eval(&format!("{{\n{snippet}\n}}"))
            .unwrap_or_else(|error| panic!("step {name} failed in the TypeScript std: {error}"));
        settle(&mut shell);
        if name.starts_with("staged") {
            held_from = mark;
        }
        match then {
            Then::Nothing => {}
            Then::Completed => end_tracks(&comp, mark, true),
            Then::RanBack => end_tracks(&comp, held_from, false),
        }
        settle(&mut shell);
        recorded.push((
            name.to_owned(),
            comp.ops_since(mark).iter().filter_map(op_line).collect::<Vec<_>>(),
        ));
    }
    let log = shell
        .eval("g.log.join('\\n')")
        .expect("the log is read")
        .unwrap_or_default();
    Run {
        steps: recorded,
        log: log.lines().map(str::to_owned).collect(),
    }
}

// ---- the native std ---------------------------------------------------------------------------------------------------------

struct Stage {
    dir: PathBuf,
    shell: PathBuf,
}

fn build_stage() -> Result<Stage, String> {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("std-anim-stage");
    let _ = std::fs::remove_dir_all(&base);
    let target = base.join("target");
    cargo_build(
        &target,
        &[
            "-p",
            "gnoblin-core-impl",
            "-p",
            "gnoblin-std-impl",
            "-p",
            "gnoblin-fixture-launcher",
            "-p",
            "gnoblin-fixture-shell-anim",
            "--features",
            "gnoblin-core-impl/test-driver",
        ],
        &[],
    )?;
    let debug = target.join("debug");
    let dir = base.join("prefix");
    copy_to(&debug.join("libgnoblin_core_impl.so"), &dir.join(CORE_FILE));
    copy_to(&debug.join(LAUNCHER), &dir.join(LAUNCHER));
    copy_to(&debug.join("libgnoblin_std_impl.so"), &dir.join("libgnoblin-std.so.1"));
    let shell = copy_to(&debug.join("libshell_anim.so"), &dir.join("shells/libshell_anim.so"));
    Ok(Stage { dir, shell })
}

fn stage() -> &'static Stage {
    static STAGE: LazyLock<Result<Stage, String>> = LazyLock::new(build_stage);
    match &*STAGE {
        Ok(stage) => stage,
        Err(message) => panic!("cannot build the animation fixture: {message}"),
    }
}

/// Removes the `E_*: ` prefix the library puts on a failure's text, which the TypeScript errors do not have.
fn bare(line: &str) -> String {
    let mut out = String::new();
    for (at, word) in line.split(' ').enumerate() {
        if word.starts_with("E_") && word.ends_with(':') {
            continue;
        }
        if at > 0 && !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

/// Runs the scenario over the native shell in the launcher.
fn native() -> (Run, String) {
    let stage = stage();
    let (stdout, stderr, code) = run_with_env(
        &stage.dir,
        &["std_anim"],
        &[stage.shell.as_path()],
        &[("LD_LIBRARY_PATH", stage.dir.as_path())],
    );
    assert_eq!(code, 0, "{stdout}\n{stderr}");
    let mut steps: Vec<(String, Vec<String>)> = Vec::new();
    for line in stdout.lines() {
        if let Some(name) = line.strip_prefix("step=") {
            steps.push((name.to_owned(), Vec::new()));
        } else if let Some(op) = line.strip_prefix("op ")
            && let Some((_, lines)) = steps.last_mut()
        {
            lines.push(op.to_owned());
        }
    }
    let log = stderr
        .lines()
        .filter_map(|line| line.split_once("anim-log ").map(|(_, rest)| bare(rest)))
        .collect();
    (Run { steps, log }, format!("{stdout}\n{stderr}"))
}

// ---- the proof --------------------------------------------------------------------------------------------------------------

fn lines_of<'a>(run: &'a Run, name: &str) -> &'a [String] {
    &run.steps
        .iter()
        .find(|(step, _)| step == name)
        .unwrap_or_else(|| panic!("no step {name}"))
        .1
}

#[test]
fn transitions_styles_and_the_scene_write_what_the_typescript_std_writes() {
    let (native, transcript) = native();
    let reference = reference();
    let (native, reference) = (normalise(native), normalise(reference));
    for (label, run) in [("native", &native), ("typescript", &reference)] {
        for (name, lines) in &run.steps {
            println!("{label} {name}: {} writes", lines.len());
            for line in lines {
                println!("    {line}");
            }
        }
        println!("{label} log: {:?}", run.log);
    }
    let names = |run: &Run| run.steps.iter().map(|(name, _)| name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&native), names(&reference), "{transcript}");
    for ((name, native_lines), (_, reference_lines)) in native.steps.iter().zip(&reference.steps) {
        let (mut left, mut right) = (native_lines.clone(), reference_lines.clone());
        left.sort();
        right.sort();
        assert_eq!(
            left,
            right,
            "step {name}\nnative:\n{}\ntypescript:\n{}",
            native_lines.join("\n"),
            reference_lines.join("\n")
        );
    }
    assert_eq!(native.log, reference.log, "what the steps logged\n{transcript}");

    // The scenario is not vacuous: each kind of write the design names was made.
    let basic = lines_of(&native, "basic");
    assert_eq!(
        basic.iter().filter(|line| line.contains("seconds=Some(0.2)")).count(),
        3,
        "{basic:#?}"
    );
    assert!(
        basic
            .iter()
            .any(|line| line.starts_with("new ") && line.to_lowercase().ends_with("class=tween")),
        "the tracks of a transition join one tween node: {basic:#?}"
    );
    let curves: Vec<usize> = basic
        .iter()
        .filter_map(|line| line.split(' ').find_map(|word| word.strip_prefix("samples=")))
        .filter(|samples| !samples.is_empty())
        .map(|samples| samples.split(',').count())
        .collect();
    assert_eq!(
        curves,
        [33, 33, 33],
        "each timed write carries the easing curve sampled 33 times"
    );
    let staged = lines_of(&native, "staged");
    assert!(
        staged.iter().any(|line| line.contains("key=paused value=Bool(true)")),
        "a held transition holds its tween: {staged:#?}"
    );
    assert!(
        staged.iter().any(|line| line.contains("delay=Some(0.05)")),
        "{staged:#?}"
    );
    let instant = lines_of(&native, "instant");
    assert!(
        instant.iter().all(|line| line.contains("seconds=None")),
        "no duration is no animation: {instant:#?}"
    );
    let layout = lines_of(&native, "layout");
    assert!(
        layout.iter().filter(|line| line.contains("seconds=Some(0.3)")).count() >= 4,
        "a layout changed in a transition moves the windows with timed writes: {layout:#?}"
    );
    let style = lines_of(&native, "style_global");
    assert!(
        style
            .iter()
            .any(|line| line.contains("key=effects") && line.contains("kind: \"shadow\"")),
        "{style:#?}"
    );
    assert!(
        style
            .iter()
            .any(|line| line.contains("key=effects") && line.contains("kind: \"blur\"")),
        "{style:#?}"
    );
    assert!(
        style
            .iter()
            .any(|line| line.contains("key=corner_radius value=Double(12.0)")),
        "{style:#?}"
    );
    let scene = lines_of(&native, "scene");
    assert!(
        scene.iter().filter(|line| line.starts_with("new ")).count() == 3
            && scene.iter().filter(|line| line.starts_with("insert ")).count() == 3,
        "{scene:#?}"
    );
}
