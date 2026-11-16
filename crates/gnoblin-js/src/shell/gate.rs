//! Change gating: a frame that has nothing to do does no work.
//!
//! The runtime role takes a turn every 16 ms and each turn offers the shell a frame. A frame is not free: it calls into
//! script a dozen times, and the layout and style passes read every window, so it costs time proportional to the
//! windows (about 1.2 ms at 300) whether or not anything happened. Everything that can change what a frame does comes
//! from one of a few places, and a frame runs when one of them has something and not otherwise.
//!
//! # What wakes a frame
//!
//! 1. **The compositor sent something** on the tree link: a batch of changes, a reply, an event, a claimed chord, the
//!    end of an animation. Every `TreeIncoming` variant is a wake-up: the gate looks at whether anything arrived,
//!    never at what it was, so a new kind of message cannot be forgotten.
//! 2. **A link beside the tree has something**: a process ended, a session call finished or a signal came, the
//!    compositor offered a dialog, the idle state moved, or one of those connections was lost. The gate reads these
//!    links itself on a quiet turn and keeps what they said for the frame it then runs.
//! 3. **Script code ran outside a frame**: an `eval`, the console, loading or reloading a config, a generation swap, a
//!    link handed to the shell. Such code may have queued a listener, written a property or changed what a layout reads,
//!    and it cannot say which, so the next frame runs ([`JsShell::note_script_ran`]).
//! 4. **The script asked to be woken** (`wake` of the entry, `data/js/gnoblin-core/wake.ts`): a timer or a deadline that is
//!    due, a listener call still queued, a lock screen or an XR session that draws every frame, a clock widget whose
//!    minute is ending. Each part of the library that has such a need registers it, by name, where it keeps its state,
//!    so the list lives with what it is about and not here. A source that throws is reported and asks for nothing: one
//!    part's failure must not end the shell, which asks at the end of every frame.
//! 5. **The last frame did work.** One more frame runs after it, and after that one while each does work: what a frame
//!    changes late (the toolkit's pass comes after the layout pass) is seen by the frame after it, as it was when a
//!    frame ran every turn. A frame that had nothing to do is the last.
//!
//! # Failure modes first
//!
//! 1. **A change that does not wake the loop is never drawn.** That is the failure gating risks, and each wake-up
//!    above has a test that changes one thing and asserts the next frame ran. `tests/frame_gating.rs` holds the tables
//!    for the tree link (`every_kind_of_change_wakes_the_next_frame`, and a row for each kind of message it carries,
//!    which the compiler asks for) and a random order of changes after random rests. The links beside the tree are
//!    tested with their fakes (`host_api.rs`, `host_idle.rs`, `config_slots.rs`), the lock screen, an XR session and a
//!    window drag where they live (`lock.rs`, `xr.rs`, `frame_gating.rs`). What a part of the library has to register
//!    is `tests/wake_sources.rs`: every field of `HostContext` is classified, the sources the stock shell registers
//!    are the ones expected, and a file that reads a clock registers one or says why not. The tests below read this
//!    module's own source: a link handed to the shell that a quiet turn does not read, and a public way into script
//!    that does not mark the gate, fail.
//! 2. **A deadline is missed.** The script's clock is the shell's clock, so [`JsShell::next_wake`] says how long until a
//!    timer is due and the role never waits past it; a frame that is due runs on the turn after it is.
//! 3. **A budget cut a pass short.** The layout or toolkit pass that ran out its budget left work for the next frame, so
//!    the next frame is due (`deferred`).

use std::cell::Cell;
use std::time::Duration;

use gnoblin_tree_client::slot_link::SlotEvent;
use rquickjs::Function;
use serde_json::Value as Json;

use super::{JsShell, JsShellError};
use crate::host_idle::JsIdlePoll;

/// How many frames ran and how many turns were found quiet, for a test and for the numbers in the design document.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JsFrameStats {
    /// Frames that did their work.
    pub run: u64,
    /// Turns on which nothing was due, so the frame did nothing.
    pub idle: u64,
}

/// When the script next needs a frame by itself, on the script's clock.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum ScriptWake {
    /// Never: the script only wants a frame when something arrives.
    #[default]
    Never,
    /// At this reading of the script's clock, in milliseconds. A reading in the past is "now".
    At(f64),
}

/// What the links beside the tree had when a quiet turn read them.
#[derive(Default)]
pub(super) struct Backlog {
    pub(super) api: Vec<Json>,
    pub(super) dialogs: Vec<SlotEvent>,
    pub(super) idle: JsIdlePoll,
    pub(super) diagnostics: Vec<String>,
}

impl Backlog {
    /// Whether the quiet turns found anything. The fields are named one by one and not with `..`, so a link that is
    /// given a field of its own here cannot be left out of the question: the compiler stops at the new field.
    pub(super) fn is_empty(&self) -> bool {
        let Self {
            api,
            dialogs,
            idle: JsIdlePoll { reading, lost },
            diagnostics,
        } = self;
        api.is_empty() && dialogs.is_empty() && reading.is_none() && !lost && diagnostics.is_empty()
    }
}

/// What the gate knows between frames.
#[derive(Default)]
pub(super) struct FrameGate {
    /// Script ran outside a frame since the last one. A `Cell` because the console takes the shell by shared reference.
    external: Cell<bool>,
    /// The last frame did work, so the next looks again.
    echo: bool,
    /// A pass was cut short by its budget and left work for the next frame.
    pub(super) deferred: bool,
    wake: ScriptWake,
    /// What a quiet turn's reads of the links beside the tree found, kept for the frame they started.
    backlog: Backlog,
    /// How many transactions the shell has sent, so a frame can tell it sent one.
    pub(super) sent: u64,
    stats: JsFrameStats,
}

impl JsShell {
    /// How many frames ran and how many turns were found quiet since the shell started.
    pub const fn frame_stats(&self) -> JsFrameStats {
        self.gate.stats
    }

    /// Notes that script ran outside a frame, so the next frame is due. Everything that evaluates script on the shell's
    /// behalf between frames calls this.
    pub(super) fn note_script_ran(&self) {
        self.gate.external.set(true);
    }

    /// How long until the shell needs a frame without anything arriving: `Some(ZERO)` when one is due now, the time to
    /// the next timer or deadline otherwise, `None` when the shell only wants a frame when something arrives. The
    /// runtime role never waits longer than this.
    pub fn next_wake(&self) -> Option<Duration> {
        let gate = &self.gate;
        if gate.external.get() || gate.echo || gate.deferred || self.boot_ready.is_some() || !gate.backlog.is_empty() {
            return Some(Duration::ZERO);
        }
        match gate.wake {
            ScriptWake::Never => None,
            ScriptWake::At(at) => Some(Duration::from_secs_f64((at - self.now_ms()).max(0.0) / 1000.0)),
        }
    }

    /// The script's clock in milliseconds.
    fn now_ms(&self) -> f64 {
        self.state.borrow().input.now_ms()
    }

    /// Whether a quiet turn (nothing on the tree link) has a frame to run: script ran outside a frame, the last frame did
    /// work, a link beside the tree has something, or the script's own wake-up is due.
    pub(super) fn frame_due(&mut self) -> bool {
        let gate = &self.gate;
        if gate.external.get() || gate.echo || gate.deferred || self.boot_ready.is_some() {
            return true;
        }
        let mut diagnostics = Vec::new();
        let api = self.state.borrow_mut().api.poll();
        let dialogs = self.dialogs.poll(&mut diagnostics);
        let idle = self.idle.poll(&mut diagnostics);
        let backlog = &mut self.gate.backlog;
        backlog.api.extend(api);
        backlog.dialogs.extend(dialogs);
        backlog.idle.reading = idle.reading.or_else(|| backlog.idle.reading.take());
        backlog.idle.lost |= idle.lost;
        backlog.diagnostics.extend(diagnostics);
        if !backlog.is_empty() {
            return true;
        }
        matches!(self.gate.wake, ScriptWake::At(at) if at <= self.now_ms())
    }

    /// Records a quiet turn.
    pub(super) fn note_quiet_turn(&mut self) {
        self.gate.stats.idle = self.gate.stats.idle.saturating_add(1);
    }

    /// What the quiet turns' reads kept, handed to the frame that handles them.
    pub(super) fn take_backlog(&mut self) -> Backlog {
        std::mem::take(&mut self.gate.backlog)
    }

    /// Takes the "script ran outside a frame" mark, which the frame that handles it clears.
    pub(super) fn take_script_ran(&self) -> bool {
        self.gate.external.replace(false)
    }

    /// Ends a frame: says whether the next one is due because this one did work, and asks the script when it next needs
    /// a frame of its own.
    pub(super) fn plan_next_frame(&mut self, worked: bool) -> Result<(), JsShellError> {
        self.gate.stats.run = self.gate.stats.run.saturating_add(1);
        self.gate.echo = worked || std::mem::take(&mut self.gate.deferred);
        let at: f64 = self.call_entry(self.entry_budget, |entry| {
            entry.get::<_, Function<'_>>("wake")?.call(())
        })?;
        // Promise jobs the pump left (it runs at most `MAX_JOBS_PER_PUMP`) are script work no message announces.
        self.gate.wake = if self.generation.jobs_pending() {
            ScriptWake::At(0.0)
        } else if at.is_finite() {
            ScriptWake::At(at)
        } else {
            ScriptWake::Never
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    //! What keeps a link or an entry point from being added without the gate hearing of it. The gate's tests of behaviour
    //! are `tests/frame_gating.rs` and the ones beside the fakes of each link; these read the shell's own source, because
    //! a new way in is a new function, and a function nobody calls from the gate is the bug.

    use std::collections::BTreeSet;

    /// The text of every module of the shell.
    const SOURCES: &[(&str, &str)] = &[
        ("mod.rs", include_str!("mod.rs")),
        ("links.rs", include_str!("links.rs")),
        ("document.rs", include_str!("document.rs")),
        ("frame.rs", include_str!("frame.rs")),
        ("gate.rs", include_str!("gate.rs")),
        ("txgate.rs", include_str!("txgate.rs")),
    ];

    /// The functions of the shell's `impl` blocks that are public in some way, as `(name, body)`. The code is rustfmt's, so
    /// a function begins on a line indented four spaces and ends on the first line that is just `    }`.
    fn public_functions() -> Vec<(String, String)> {
        let mut found = Vec::new();
        for (_, text) in SOURCES {
            // Only the part before this module: it names the functions it looks for.
            let text = text.split("#[cfg(test)]").next().unwrap_or(text);
            let mut current: Option<(String, String)> = None;
            for line in text.lines() {
                if let Some((_, body)) = current.as_mut() {
                    body.push_str(line);
                    body.push('\n');
                    if line == "    }" {
                        found.extend(current.take());
                    }
                    continue;
                }
                let declared = [
                    "    pub fn ",
                    "    pub const fn ",
                    "    pub(crate) fn ",
                    "    pub(super) fn ",
                ]
                .iter()
                .find_map(|prefix| line.strip_prefix(prefix));
                if let Some(rest) = declared {
                    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                    current = Some((name, format!("{line}\n")));
                }
            }
        }
        found
    }

    /// How a link is handed to the shell, and what a quiet turn reads of it in `frame_due`. `None` is a link that only
    /// carries what the shell sends and has nothing arrive on it.
    const LINKS: &[(&str, Option<&str>)] = &[
        ("set_slot_link", Some("self.dialogs.poll(")),
        ("set_session_link", Some("api.poll()")),
        ("set_idle_connector", Some("self.idle.poll(")),
        ("set_config_link", None),
    ];

    #[test]
    fn every_link_handed_to_the_shell_is_read_by_a_quiet_turn() {
        let functions = public_functions();
        let handed: BTreeSet<&str> = functions
            .iter()
            .map(|(name, _)| name.as_str())
            .filter(|name| name.starts_with("set_") && (name.ends_with("_link") || name.ends_with("_connector")))
            .collect();
        let listed: BTreeSet<&str> = LINKS.iter().map(|(name, _)| *name).collect();
        assert_eq!(
            handed, listed,
            "a link is handed to the shell that LINKS does not know (or one is gone). A link something can arrive on must \
             be read by `frame_due`, or the arrival is seen only when something else wakes a frame: add it there, and its \
             row here"
        );
        let due = functions
            .iter()
            .find(|(name, _)| name == "frame_due")
            .map(|(_, body)| body.as_str())
            .expect("the gate has a frame_due");
        for (name, read) in LINKS {
            if let Some(read) = read {
                assert!(due.contains(read), "{name}: `frame_due` does not read it (`{read}`)");
            }
        }
    }

    /// Public functions that enter script and need not mark the gate, and why.
    const NO_MARK_NEEDED: &[(&str, &str)] = &[
        ("frame", "is the frame the mark asks for"),
        ("workspace_snapshot", "reads which window is where, and changes nothing"),
        (
            "declared_document",
            "reads the document a config declares, and changes nothing",
        ),
        (
            "call_entry",
            "the frames' own way into the entry; what calls it marks or is a frame",
        ),
        ("collect_reports", "part of a frame"),
        ("run_pending", "part of a frame"),
        ("run_host_api", "part of a frame"),
        ("run_dialogs", "part of a frame"),
        ("run_compositor_links", "part of a frame"),
        ("commit_document_changes", "part of a frame"),
        (
            "apply_document",
            "part of a frame, or of a call that marks (`config_committed`)",
        ),
        ("plan_next_frame", "ends a frame"),
        ("send_transaction", "sends what a frame or a retire wrote"),
    ];

    #[test]
    fn every_public_way_into_script_marks_the_gate() {
        let enters = ["call_entry(", ".call_budgeted(", ".eval("];
        let mut unmarked = Vec::new();
        let mut stale = Vec::new();
        let functions = public_functions();
        for (name, body) in &functions {
            let enters_script = enters.iter().any(|token| body.contains(token));
            let exempt = NO_MARK_NEEDED.iter().any(|(listed, _)| listed == name);
            if enters_script && !body.contains("note_script_ran()") && !exempt {
                unmarked.push(name.clone());
            }
        }
        for (listed, _) in NO_MARK_NEEDED {
            if !functions.iter().any(|(name, _)| name == listed) {
                stale.push(*listed);
            }
        }
        assert!(
            unmarked.is_empty(),
            "{unmarked:?} run script outside a frame without `note_script_ran()`: what the script did (a queued listener, a \
             write, a changed layout input) is then drawn only when something else wakes a frame. Mark it, or list it in \
             NO_MARK_NEEDED with the reason it changes nothing"
        );
        assert!(stale.is_empty(), "{stale:?} are in NO_MARK_NEEDED and no longer exist");
    }
}
