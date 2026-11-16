//! A stage of a generation, and what taking it back means.
//!
//! A reload stages a generation by running the user's entry against the live tree and the shared [`HostState`], and only
//! then decides whether it takes over. Everything the entry does at its top level lands in that shared state before the
//! decision, so everything it touches that a failed stage must not keep is behind one mark, taken here: the pending
//! writes, the canvases and measurements of the UI toolkit, the nodes of the XR module, the `apply` calls of the outputs
//! module, the password checks of the lock screen and the seat claims. A process the entry spawns is held back, not
//! started, until the generation takes over (`Processes::begin_stage`).

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use gnoblin_tree_client::tree_mirror::{NodeId, TxBuilder};

use super::HostState;
use crate::ui::UiMark;

/// What [`HostState::begin_stage`] kept of the state, to put back if the stage fails.
pub(crate) struct StageMark {
    tick: TxBuilder,
    ui: UiMark,
    xr: HashSet<NodeId>,
    outputs: HashSet<u64>,
    checks: HashMap<u64, Duration>,
}

impl HostState {
    /// Marks the start of a stage. The entry that runs now writes into the live state, so a top-level write such as
    /// `client.x = 5` lands in the pending writes before it is known that the stage succeeds, and a canvas it makes is
    /// owned before the tree has it. The seat claims it makes are kept apart, and are sent only when the generation takes
    /// over, after the old one's have been released; the programs it spawns are not started until then.
    pub(crate) fn begin_stage(&mut self) -> StageMark {
        crate::input::begin_staging(self);
        self.api.processes.begin_stage();
        StageMark {
            tick: self.tick.clone(),
            ui: self.ui.mark(),
            xr: self.xr.mark(),
            outputs: self.outputs.mark(),
            checks: self.lock.mark(),
        }
    }

    /// Ends a stage that failed: everything it touched goes back to what it was at [`begin_stage`](Self::begin_stage),
    /// the claims it kept are dropped and the programs it spawned never start. The running generation was never touched.
    pub(crate) fn abandon_stage(&mut self, mark: StageMark) {
        crate::input::abandon_staging(self);
        self.api.processes.abandon_stage();
        self.tick = mark.tick;
        self.ui.rollback(mark.ui);
        self.xr.rollback(mark.xr);
        self.outputs.rollback(mark.outputs);
        self.lock.rollback(mark.checks);
    }
}

#[cfg(test)]
mod tests {
    use gnoblin_tree_client::tree_mirror::{SnapshotNode, TreeMirror};

    use super::*;

    fn snapshot(id: NodeId, parent: NodeId, class: &str) -> SnapshotNode {
        SnapshotNode {
            id,
            parent,
            index: 0,
            class: class.to_owned(),
            title: String::new(),
            app_id: String::new(),
            props: Vec::new(),
        }
    }

    /// A root (1), a screen (2) and a lock (3) under it.
    fn state() -> HostState {
        let nodes = [snapshot(1, 0, "node"), snapshot(2, 1, "screen"), snapshot(3, 1, "lock")];
        HostState::new(TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror"))
    }

    #[test]
    fn a_failed_stage_leaves_no_canvas_node_measurement_apply_or_check_behind() {
        let mut state = state();
        let before = crate::ui::create(&mut state).expect("a canvas the running generation owns");

        let mark = state.begin_stage();
        let canvas = crate::ui::create(&mut state).expect("a canvas the stage makes");
        let measure = crate::ui::measure(&mut state, canvas, "sans", 10.0, "Hello").expect("a token");
        let xr = crate::xr::create(&mut state, "Node").expect("an XR node the stage makes");
        let apply = crate::output_apply::apply(&mut state, 2.0, "key=value".to_owned()).expect("a token");
        let check = crate::lock::authenticate(&mut state, "hunter2".to_owned()).expect("a token");
        assert!(
            state.tick.node(&state.mirror, canvas).is_some(),
            "the stage's writes are pending"
        );

        state.abandon_stage(mark);
        assert!(
            state.tick.node(&state.mirror, canvas).is_none(),
            "the pending writes are back"
        );
        assert!(state.tick.node(&state.mirror, xr).is_none());
        assert!(
            !state.ui.accept_answer(measure, b"", ""),
            "the measurement is not waited on"
        );
        assert!(!state.outputs.accept_answer(apply, ""), "the apply is not waited on");
        assert!(!state.lock.accept_answer(check, b"", ""), "the check is not waited on");
        assert!(!state.ui.owns(canvas), "the canvas is not the toolkit's");
        assert!(state.ui.owns(before), "the running generation's canvas still is");
        assert!(!state.xr.owns(xr), "the XR node is not the module's");
    }
}
