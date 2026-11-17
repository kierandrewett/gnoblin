//! A scripted scenario for the std library's tests: the launcher loads a shell, plays the script the test wrote, and prints what the
//! windows look like after each step.
//!
//! The script is a file of lines (`GNOBLIN_TEST_SCRIPT` names it); the same lines are played by the test itself over the TypeScript
//! library, so the two worlds see one sequence of events and the test compares what each left in the tree.
//!
//! ```text
//! create ID TITLE             a window opens (it does not take the keyboard)
//! destroy ID                  a window closes
//! focus ID                    the keyboard moves to the window
//! chord NAME                  a claimed key chord is pressed, `Super+1`
//! drag CHORD X,Y [X,Y ...] end|cancel   a pointer chord: the press, the motion through the points, then how it ends
//! place ID X Y W H            the compositor moves and sizes a window
//! seat KEY VALUE              a property of the seat (`true`, `false` or a number)
//! window ID KEY VALUE         a property of a window (`true`, `false` or a number)
//! gesture KIND FINGERS PHASE DX DY SCALE ROTATION   a touchpad gesture step
//! pump                        the shell's turns run
//! step NAME ID...             prints the named windows
//! ```
#![allow(unsafe_code)]

use std::io::Write as _;
use std::path::Path;

use gnoblin_testkit::compositor::FakeCompositor;
use gnoblin_tree_client::tree_mirror::MirrorValue;

use crate::driver_examples::{FIRST, SEAT, drag, finish, place, press, pump, start};

macro_rules! out {
    ($($arg:tt)*) => {{
        let _ = writeln!(std::io::stdout().lock(), $($arg)*);
    }};
}

/// A value a script writes: a boolean or a number.
fn value_of(text: &str) -> MirrorValue {
    match text {
        "true" => MirrorValue::Bool(true),
        "false" => MirrorValue::Bool(false),
        number => MirrorValue::Double(number.parse().unwrap_or(f64::NAN)),
    }
}

fn point(text: &str) -> (f64, f64) {
    let (x, y) = text.split_once(',').unwrap_or((text, "0"));
    (x.parse().unwrap_or(0.0), y.parse().unwrap_or(0.0))
}

fn numbers(comp: &FakeCompositor, id: u64, key: &str, fallback: &str) -> String {
    match comp.value(id, key) {
        Some(MirrorValue::List(list)) => list.iter().map(f64::to_string).collect::<Vec<_>>().join(","),
        _ => fallback.to_owned(),
    }
}

/// What one window looks like: its rectangle and what a layout's placement writes besides it.
fn describe(comp: &FakeCompositor, id: u64) {
    if !comp.exists(id) {
        out!("window id={id} gone");
        return;
    }
    out!(
        "window id={id} x={} y={} w={} h={}",
        comp.number(id, "x"),
        comp.number(id, "y"),
        comp.number(id, "width"),
        comp.number(id, "height")
    );
    out!(
        "props id={id} tiled={} visible={} z={} rot={} scale={}",
        comp.flag(id, "tiled").unwrap_or(false),
        comp.flag(id, "visible").unwrap_or(true),
        comp.number(id, "z"),
        numbers(comp, id, "rotation", "0,0,0"),
        numbers(comp, id, "scale", "1,1,1"),
    );
}

pub(crate) fn run(shell: &Path) -> i32 {
    let Some(script) = std::env::var_os("GNOBLIN_TEST_SCRIPT").and_then(|path| std::fs::read_to_string(path).ok())
    else {
        out!("no script: set GNOBLIN_TEST_SCRIPT to a readable file");
        return 2;
    };
    let (comp, module) = match start(shell) {
        Ok(started) => started,
        Err(code) => return code,
    };
    let mut windows = vec![FIRST];
    pump(&module);
    for line in script.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        let id = |at: usize| words.get(at).and_then(|word| word.parse::<u64>().ok()).unwrap_or(0);
        let real = |at: usize| words.get(at).and_then(|word| word.parse::<f64>().ok()).unwrap_or(0.0);
        match words.as_slice() {
            [] => {}
            [comment, ..] if comment.starts_with('#') => {}
            ["create", _, title, ..] => {
                comp.create_window(id(1), title, "app");
                windows.push(id(1));
                pump(&module);
            }
            ["destroy", ..] => {
                comp.destroy(id(1));
                windows.retain(|window| *window != id(1));
                pump(&module);
            }
            ["focus", ..] => {
                for window in &windows {
                    comp.external_set(*window, "focused", MirrorValue::Bool(*window == id(1)));
                }
                pump(&module);
            }
            ["chord", chord] => press(&comp, &module, chord),
            ["drag", chord, from, rest @ ..] if !rest.is_empty() => {
                let (last, path) = rest.split_last().map_or(("end", &[][..]), |(last, path)| (*last, path));
                let path: Vec<(f64, f64)> = path.iter().map(|text| point(text)).collect();
                drag(&comp, &module, chord, point(from), &path, last);
            }
            ["place", ..] => place(&comp, &module, id(1), (real(2), real(3), real(4), real(5))),
            ["seat", key, text] => {
                comp.external_set(SEAT, key, value_of(text));
                pump(&module);
            }
            ["window", _, key, text] => {
                comp.external_set(id(1), key, value_of(text));
                pump(&module);
            }
            ["gesture", kind, fingers, phase, ..] => {
                let claim = comp.gesture_claim(kind, fingers.parse().unwrap_or(3));
                comp.gesture(
                    claim,
                    kind,
                    fingers.parse().unwrap_or(3),
                    phase,
                    (real(4), real(5)),
                    real(6),
                    real(7),
                );
                pump(&module);
            }
            ["pump"] => pump(&module),
            ["step", name, ids @ ..] => {
                out!("step={name}");
                for window in ids {
                    describe(&comp, window.parse().unwrap_or(0));
                }
            }
            other => out!("bad script line: {other:?}"),
        }
    }
    finish(&comp, &module);
    0
}
