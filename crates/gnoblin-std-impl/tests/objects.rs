//! The layouts of the native std through the object protocol, held to what the TypeScript std did (`data/js/gnoblin-std`), which the
//! recorded fixtures `tiling_*.json` keep.
//!
//! `tests/tiling.rs` plays each script to a shell over the std table (`shell-tiling`). This file plays the same scripts, taken from the
//! fixtures, to `shell-objects`, which makes its workspace, its layouts, their actions and a client's size limits through the classes of
//! `workspace.toml` (`gnoblin_std::objects`), over the real core and the real `libgnoblin-std.so.1`. After each step the rectangles and the
//! placement properties a layout writes must be the fixture's, number for number.
//!
//! **Allowances of the replay** (each is in the list below, with its reason):
//!
//! 1. A fixture whose script drags with a pointer chord or makes a gesture is not replayed: a drag or a gesture is a binding, and the
//!    binding classes are the policy and input lanes' (NpStdPolicy, NpCoreInput). The divider a split-resize drag moves is covered
//!    from the object protocol by `modular::native_objects` (`find_split`, `set_split`, `reset_split` of a workspace).
//! 2. The fixtures `examples_tiling_layouts`, `examples_floating_placement` and `anim_style_transitions` come from scenarios that need a
//!    fixture shell of their own (the layouts shell with a swap binding, the example 2 shell, the transitions shell); they are not
//!    replayed through the objects yet.
//! 3. A number is the fixture's within 1e-9 of its size: the carousel's depth differs from the TypeScript run's in the last binary digit
//!    (`-1559.3945806691095` and `-1559.3945806691097`), which the shell's decimal output and the fixture's JSON both carry as they were.

#[path = "../../gnoblin-core-impl/tests/support/mod.rs"]
mod support;
use support::*;

use std::fs;
use std::path::PathBuf;
use std::sync::LazyLock;

use gnoblin_testkit::oracle::{Fixtures, Value, json};

/// The fixtures a script of which needs a binding (allowance 1), by name.
const BINDING_FIXTURES: &[&str] = &[
    "tiling_binding_priority",
    "tiling_directed_gesture",
    "tiling_dwindle_drag",
    "tiling_gesture_cancel_on_close",
    "tiling_pointer_move_resize",
    "tiling_resize_limits",
    "tiling_snap",
    "tiling_split_resize_drag",
    "tiling_tile_swap",
];

struct Staged {
    dir: PathBuf,
    shell: PathBuf,
}

fn build_stage() -> Result<Staged, String> {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("std-objects-stage");
    let _ = fs::remove_dir_all(&base);
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
            "gnoblin-fixture-shell-objects",
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
    let shell = copy_to(
        &debug.join("libshell_objects.so"),
        &dir.join("shells/libshell_objects.so"),
    );
    Ok(Staged { dir, shell })
}

fn stage() -> &'static Staged {
    static STAGE: LazyLock<Result<Staged, String>> = LazyLock::new(build_stage);
    match &*STAGE {
        Ok(stage) => stage,
        Err(message) => panic!("cannot build the objects shell: {message}"),
    }
}

fn list(text: &str) -> Value {
    Value::Array(
        text.split(',')
            .filter_map(|part| part.parse::<f64>().ok())
            .map(|number| json!(number))
            .collect(),
    )
}

/// What the shell printed, as the fixture holds it: one object for each step with the windows it listed.
fn steps_of(stdout: &str) -> Value {
    let mut steps: Vec<Value> = Vec::new();
    for line in stdout.lines() {
        let field = |key: &str| {
            let marker = format!("{key}=");
            line.split_whitespace()
                .find_map(|word| word.strip_prefix(marker.as_str()))
                .unwrap_or("")
                .to_owned()
        };
        if let Some(name) = line.strip_prefix("step=") {
            steps.push(json!({ "step": name, "windows": [] }));
        } else if line.starts_with("window id=") {
            let Some(step) = steps.last_mut() else { continue };
            let rect = if line.ends_with("gone") {
                Value::Null
            } else {
                json!(["x", "y", "w", "h"].map(|key| field(key).parse::<f64>().unwrap_or(0.0)))
            };
            let window = json!({
                "id": field("id").parse::<u64>().unwrap_or(0),
                "rect": rect,
                "tiled": false,
                "visible": true,
                "z": 0.0,
                "rotation": [0.0, 0.0, 0.0],
                "scale": [1.0, 1.0, 1.0],
            });
            step["windows"].as_array_mut().expect("a list").push(window);
        } else if line.starts_with("props id=")
            && let Some(step) = steps.last_mut()
            && let Some(window) = step["windows"].as_array_mut().and_then(|windows| windows.last_mut())
        {
            window["tiled"] = json!(field("tiled") == "true");
            window["visible"] = json!(field("visible") == "true");
            window["z"] = json!(field("z").parse::<f64>().unwrap_or(0.0));
            window["rotation"] = list(&field("rot"));
            window["scale"] = list(&field("scale"));
        }
    }
    Value::Array(steps)
}

/// Plays `script` to the objects shell in the launcher.
fn native(script: &str) -> Value {
    let stage = stage();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(script, &mut hasher);
    let file = stage
        .dir
        .join(format!("script-{:x}", std::hash::Hasher::finish(&hasher)));
    fs::write(&file, script).expect("the script is written");
    let (stdout, stderr, code) = run_with_env(
        &stage.dir,
        &["std_script"],
        &[&stage.shell],
        &[
            ("LD_LIBRARY_PATH", stage.dir.as_path()),
            ("GNOBLIN_TEST_SCRIPT", file.as_path()),
        ],
    );
    assert_eq!(code, 0, "{stdout}\n{stderr}");
    assert!(stderr.contains("objects shell is running"), "{stderr}");
    assert!(!stderr.contains("cannot"), "{stderr}");
    assert!(!stdout.contains("bad script line"), "{stdout}");
    steps_of(&stdout)
}

/// Whether two values are the same, numbers within 1e-9 of each other (allowance 3).
fn close(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap_or(f64::NAN), y.as_f64().unwrap_or(f64::NAN));
            (x - y).abs() <= 1e-9 * x.abs().max(y.abs()).max(1.0)
        }
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(x, y)| close(x, y)),
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(key, x)| y.get(key).is_some_and(|y| close(x, y)))
        }
        _ => a == b,
    }
}

fn replay(name: &str) {
    assert!(!BINDING_FIXTURES.contains(&name), "{name} needs a binding");
    let fixtures = Fixtures::new("crates/gnoblin-std-impl/tests/fixtures/reference");
    let fixture = fixtures.read_json(name);
    let script = fixture["script"].as_str().expect("the fixture's script");
    let got = native(script);
    let want = &fixture["steps"];
    let (got_steps, want_steps) = (got.as_array().expect("steps"), want.as_array().expect("steps"));
    assert_eq!(got_steps.len(), want_steps.len(), "as many steps as the fixture");
    for (got, want) in got_steps.iter().zip(want_steps) {
        let windows = |step: &Value| step["windows"].as_array().cloned().unwrap_or_default();
        assert!(
            close(&Value::Array(windows(got)), &Value::Array(windows(want))),
            "step {}: the object classes differ from the fixture\n native: {got}\n fixture: {want}",
            want["step"]
        );
    }
}

#[test]
fn the_arc_through_the_classes_is_the_arc_of_the_typescript_std() {
    replay("tiling_arc");
}

#[test]
fn the_carousel_through_the_classes_is_the_carousel_of_the_typescript_std() {
    replay("tiling_carousel");
}

#[test]
fn the_dwindle_through_the_classes_places_arrivals_as_the_typescript_dwindle_does() {
    replay("tiling_dwindle_arrivals");
}

#[test]
fn the_dwindles_actions_through_the_classes_change_the_tree_as_the_typescript_dwindle_does() {
    replay("tiling_dwindle_actions");
}

#[test]
fn the_dwindles_preselection_through_the_classes_is_the_typescript_dwindles() {
    replay("tiling_dwindle_preselect");
}

#[test]
fn the_dwindles_pointer_options_through_the_classes_choose_where_a_window_opens() {
    replay("tiling_dwindle_pointer_super_5");
    replay("tiling_dwindle_pointer_super_6");
}

#[test]
fn the_strip_through_the_classes_opens_after_the_focused_column_and_scrolls() {
    replay("tiling_strip_opens_after_focused");
    replay("tiling_strip_scroll");
}

#[test]
fn every_fixture_that_is_not_replayed_needs_a_binding() {
    let fixtures = Fixtures::new("crates/gnoblin-std-impl/tests/fixtures/reference");
    for name in BINDING_FIXTURES {
        let script = fixtures.read_json(name)["script"].as_str().unwrap_or("").to_owned();
        assert!(
            script
                .lines()
                .any(|line| line.starts_with("drag ") || line.starts_with("gesture ")),
            "{name} has no drag or gesture: replay it"
        );
    }
}
