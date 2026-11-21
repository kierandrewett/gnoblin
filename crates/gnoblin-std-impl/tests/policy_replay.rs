//! The pointer drags and the touchpad gestures of the policy classes, held to what the TypeScript std did (`data/js/gnoblin-std`), which the
//! recorded fixtures `tiling_*.json` keep: the nine that `tests/objects.rs` leaves out because their scripts drag with a pointer chord or make
//! a gesture.
//!
//! The script is played to `shell-policy` in the launcher, over the real core and the real `libgnoblin-std.so.1`. The shell binds the drags
//! each layout brings through `swap_binding`, `split_resize_binding`, `move_binding` and `resize_binding` of the policy catalog, which are
//! installed on the core's input context, and the probe bindings of its own that `shell-tiling` has. After each step the rectangles and the
//! placement properties a layout writes must be the fixture's, number for number.
//!
//! **Allowance of the replay**: a number is the fixture's within 1e-9 of its size, as in `tests/objects.rs`.

#[path = "../../gnoblin-core-impl/tests/support/mod.rs"]
mod support;
use support::*;

use std::fs;
use std::path::PathBuf;
use std::sync::LazyLock;

use gnoblin_testkit::oracle::{Fixtures, Value, json};

struct Staged {
    dir: PathBuf,
    shell: PathBuf,
}

fn build_stage() -> Result<Staged, String> {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("std-policy-stage");
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
            "gnoblin-fixture-shell-policy",
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
        &debug.join("libshell_policy.so"),
        &dir.join("shells/libshell_policy.so"),
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
fn the_tile_swap_through_the_classes_trades_places_as_the_typescript_swap_does() {
    replay("tiling_tile_swap");
}

#[test]
fn the_split_resize_through_the_classes_moves_the_divider_as_the_typescript_drag_does() {
    replay("tiling_split_resize_drag");
    replay("tiling_dwindle_drag");
}

#[test]
fn the_move_and_resize_through_the_classes_are_the_typescript_drags() {
    replay("tiling_pointer_move_resize");
    replay("tiling_resize_limits");
}

#[test]
fn a_drop_on_an_edge_snaps_the_window_through_the_classes_as_the_typescript_move_does() {
    replay("tiling_snap");
}

#[test]
fn the_matcher_chooses_between_bindings_of_one_chord_by_priority_and_decline() {
    replay("tiling_binding_priority");
}

#[test]
fn a_gesture_binding_hears_the_gesture_that_went_its_way_and_a_window_that_closes_cancels_it() {
    replay("tiling_directed_gesture");
    replay("tiling_gesture_cancel_on_close");
}
