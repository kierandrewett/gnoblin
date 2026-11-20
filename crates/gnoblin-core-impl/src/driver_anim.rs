//! The animation scenario: the std shell of `crates/gnoblin-std-impl/fixtures/shell-anim`, driven over the fake compositor.
//!
//! The fixture's chords each run one step of a script (a transition, a style change, a scene write). This scenario opens two more
//! windows, presses the chords in order, plays the compositor's part where a step needs it (the word that a timed write's track ended,
//! as `track_ended` carries it), and prints every write the shell sent in each step as one `op` line. The test holds those lines to the
//! TypeScript library's for the same steps, over the same fake compositor.
//!
//! The fake compositor does not play animations: it records the timed writes. What an animated property would be at a time is worked out
//! from the write itself, the curve it carries and the duration and delay it names (`play=`), on a clock the scenario owns.
#![allow(unsafe_code)]

use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::Path;

use gnoblin_testkit::compositor::FakeCompositor;
use gnoblin_tree_client::tree_mirror::TxOp;

use gnoblin_tree_client::tree_mirror::MirrorValue;

use crate::driver_examples::{SEAT, finish, press, pump, start};
use crate::host::Module;

macro_rules! out {
    ($($arg:tt)*) => {{
        let _ = writeln!(std::io::stdout().lock(), $($arg)*);
    }};
}

/// What the compositor says after a step.
#[derive(Clone, Copy)]
enum Then {
    /// Nothing: the tracks play on.
    Nothing,
    /// The tracks of the step's writes ended at their targets.
    Completed,
    /// The tracks of the writes since the last held transition ran back to their start.
    RanBack,
}

/// The steps, as the fixture binds them: a name, the chord and what the compositor does after it.
const STEPS: [(&str, &str, Then); 18] = [
    ("basic", "Super+a", Then::Completed),
    ("staged", "Super+b", Then::Nothing),
    ("seek", "Super+c", Then::Nothing),
    ("play", "Super+d", Then::Nothing),
    ("pause", "Super+e", Then::Nothing),
    ("reverse", "Super+f", Then::RanBack),
    ("staged_again", "Super+g", Then::Nothing),
    ("cancel", "Super+h", Then::Nothing),
    ("instant", "Super+i", Then::Nothing),
    ("layout", "Super+j", Then::Completed),
    ("style_global", "Super+k", Then::Nothing),
    ("style_selection", "Super+l", Then::Nothing),
    ("style_client", "Super+m", Then::Nothing),
    ("style_unset", "Super+n", Then::Nothing),
    ("style_bad", "Super+o", Then::Nothing),
    ("scene", "Super+p", Then::Nothing),
    ("scene_transition", "Super+q", Then::Completed),
    ("style_shader", "Super+r", Then::Nothing),
];

/// The eased progress of a timed write `t_ms` after the write was sent: the curve's samples, linearly between them, from the end of the
/// delay to the end of the duration.
fn eased(samples: &[f64], seconds: f64, delay: f64, t_ms: f64) -> f64 {
    let progress = ((t_ms / 1000.0 - delay) / seconds).clamp(0.0, 1.0);
    let at = progress * (samples.len() - 1) as f64;
    let low = at.floor() as usize;
    let high = (low + 1).min(samples.len() - 1);
    samples[low] + (samples[high] - samples[low]) * (at - at.floor())
}

/// One write as a line: what the test compares across the two libraries.
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

/// The compositor's word that the timed writes sent since `from` ended: at their targets, or (`completed` false) run back.
fn end_tracks(comp: &FakeCompositor, module: &Module, from: usize, completed: bool) {
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
    pump(module);
}

/// Opens two windows, presses every chord of the fixture and prints the writes of each step.
pub(crate) fn std_anim(shell: &Path) -> i32 {
    let (comp, module) = match start(shell) {
        Ok(started) => started,
        Err(code) => return code,
    };
    // The compositor accepts a shader effect (`compositor.user-shaders` is on), as the seat says.
    comp.external_set(SEAT, "user_shaders", MirrorValue::Bool(true));
    pump(&module);
    comp.create_window(41, "Files", "nautilus");
    pump(&module);
    comp.create_window(42, "Editor", "code");
    pump(&module);
    pump(&module);
    let mut held_from = comp.sent_count();
    for (name, chord, then) in STEPS {
        let mark = comp.sent_count();
        press(&comp, &module, chord);
        if name.starts_with("staged") {
            held_from = mark;
        }
        match then {
            Then::Nothing => {}
            Then::Completed => end_tracks(&comp, &module, mark, true),
            Then::RanBack => end_tracks(&comp, &module, held_from, false),
        }
        out!("step={name}");
        for line in comp.ops_since(mark).iter().filter_map(op_line) {
            out!("op {line}");
        }
    }
    finish(&comp, &module);
    0
}
