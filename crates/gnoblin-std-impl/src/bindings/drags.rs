//! The pointer bindings the library runs itself: move, resize, tile swap and split resize.
//!
//! The rules are `steps.rs`'s state machines, which read and write nothing; this adapter is what makes one a binding of the library's
//! own (`binding_builtin`): it reads the window the press landed on from the compositor's tree and the library's client records,
//! gives the machine each step, and writes what the machine answers back to them (the window's rectangle, the rectangle it floats at,
//! its snap, the order of its workspace, the ratio of a layout's divider). A shell that matches its own drags writes the same answers
//! to its own objects (`drag_step`).

use std::rc::Rc;

use gnoblin_std_abi::{
    STD_DRAG_CANCEL, STD_DRAG_CLIENT_DIALOG, STD_DRAG_CLIENT_FLOATING, STD_DRAG_CLIENT_FULLSCREEN,
    STD_DRAG_CLIENT_HIDDEN, STD_DRAG_CLIENT_MAXIMISED, STD_DRAG_CLIENT_MINIMISED, STD_DRAG_CLIENT_ON_WORKSPACE,
    STD_DRAG_CLIENT_TILED, STD_DRAG_END, STD_DRAG_START, STD_DRAG_UPDATE,
};

use gnoblin_core::Rect;

use super::steps::{ClientView, DragAnswer, DragInput, DragRule, Sides, SplitView, Tile};
use super::{DragCtx, DragHandler, with_state};
use crate::nodes;
use crate::state::{NodeId, State, Std};

/// The `STD_DRAG_CLIENT_*` flags of a client, as the library knows it.
fn flags_of(st: &State, client: NodeId) -> u32 {
    let rec = st.rec.get(&client);
    let mut flags = 0;
    let mut set = |on: bool, flag: u32| {
        if on {
            flags |= flag;
        }
    };
    set(nodes::flag(&st.g, client, "fullscreen"), STD_DRAG_CLIENT_FULLSCREEN);
    set(nodes::flag(&st.g, client, "maximised"), STD_DRAG_CLIENT_MAXIMISED);
    set(nodes::flag(&st.g, client, "tiled"), STD_DRAG_CLIENT_TILED);
    set(rec.is_some_and(|rec| rec.floating), STD_DRAG_CLIENT_FLOATING);
    set(
        nodes::node_ref(&st.g, client, "parent_window").is_some(),
        STD_DRAG_CLIENT_DIALOG,
    );
    set(nodes::flag(&st.g, client, "minimised"), STD_DRAG_CLIENT_MINIMISED);
    set(!nodes::visible(&st.g, client), STD_DRAG_CLIENT_HIDDEN);
    set(
        rec.is_some_and(|rec| rec.workspace.is_some()),
        STD_DRAG_CLIENT_ON_WORKSPACE,
    );
    flags
}

/// What the library knows of a client for a step.
fn view_of(std: &Std, client: NodeId) -> ClientView {
    let st = std.st.borrow();
    let rec = st.rec.get(&client);
    ClientView {
        flags: flags_of(&st, client),
        bounds: nodes::rect_of(&st.g, client),
        inset: st.inset_of(client),
        min: rec.and_then(|rec| rec.min),
        max: rec.and_then(|rec| rec.max),
    }
}

/// The tiles of a workspace, in its order.
fn tiles_of(std: &Std, workspace: u64) -> Vec<Tile> {
    let st = std.st.borrow();
    let Some(ws) = st.ws.get(&workspace) else {
        return Vec::new();
    };
    ws.clients
        .iter()
        .map(|client| Tile {
            id: *client,
            flags: flags_of(&st, *client),
            rect: nodes::rect_of(&st.g, *client),
        })
        .collect()
}

fn layout_of(std: &Std, workspace: u64) -> Option<Rc<crate::layout::LayoutObj>> {
    std.st.borrow().ws.get(&workspace).map(|ws| Rc::clone(&ws.layout))
}

/// What a drag in flight keeps from its start, which the rule is given back with every step.
struct HeldDrag {
    /// The pressed window, and the workspace it was on.
    client: NodeId,
    workspace: Option<u64>,
    /// The window's rectangle when the drag started.
    start: Rect,
    /// A resize: the sides it moves.
    sides: Sides,
    /// A split resize: the divider the layout found, and the token that names its dividers to the layout.
    split: Option<SplitView>,
}

/// One of the library's drags as a binding: a rule, and what the drag in flight kept.
pub(super) struct RuleDrag {
    rule: DragRule,
    held: Option<HeldDrag>,
}

impl RuleDrag {
    pub(super) fn new(rule: DragRule) -> RuleDrag {
        RuleDrag { rule, held: None }
    }

    /// Runs one step of the rule over what the library knows now.
    fn run(&self, std: &Rc<Std>, phase: u32, event: &DragCtx, tiles: &[Tile]) -> DragAnswer {
        let client = event.client.map(|client| view_of(std, client));
        let held = self.held.as_ref();
        self.rule.step(
            phase,
            &DragInput {
                event,
                client,
                start: held.map_or_else(
                    || client.map_or(Rect::new(0.0, 0.0, 0.0, 0.0), |view| view.bounds),
                    |held| held.start,
                ),
                sides: held.map_or(Sides::from_flags(0), |held| held.sides),
                split: held.and_then(|held| held.split),
                tiles,
            },
        )
    }

    /// Writes what the rule asked for.
    fn apply(&self, std: &Rc<Std>, event: &DragCtx, outcome: &DragAnswer) {
        let Some(held) = &self.held else {
            return;
        };
        let (client, workspace) = (held.client, held.workspace);
        with_state(std, |st| {
            if outcome.forget_snap {
                st.unsnap_client(client, true);
            }
            if let Some(bounds) = outcome.bounds {
                nodes::write_rect(&st.g, client, &bounds);
            }
            if let Some(floating) = outcome.floating
                && let Some(rec) = st.rec.get_mut(&client)
            {
                rec.float_rect = Some(floating);
            }
            if let (Some(zone), Some(area)) = (outcome.snap_zone, event.work_area) {
                st.snap_client(client, zone, &area, true);
            }
        });
        let Some(workspace) = workspace else {
            return;
        };
        if outcome.ratio.is_some() || outcome.reset_split {
            let token = held.split.map_or(0, |split| split.token);
            if let Some(layout) = layout_of(std, workspace) {
                if let Some(ratio) = outcome.ratio {
                    layout.set_split(token, ratio, outcome.cross_ratio.unwrap_or(f64::NAN));
                } else {
                    layout.reset_split(token);
                }
                with_state(std, |st| {
                    if let Some(ws) = st.ws.get_mut(&workspace) {
                        ws.dirty = true;
                    }
                });
            }
        }
    }
}

impl DragHandler for RuleDrag {
    fn start(&mut self, std: &Rc<Std>, event: &DragCtx) -> bool {
        let Some(client) = event.client else {
            return false;
        };
        let workspace = std.st.borrow().rec.get(&client).and_then(|rec| rec.workspace);
        // The layout answers outside the library's borrow of its state.
        let split = self
            .rule
            .wants_split()
            .then(|| {
                workspace.and_then(|workspace| {
                    layout_of(std, workspace)
                        .and_then(|layout| layout.find_split(client, event.position.x, event.position.y))
                })
            })
            .flatten();
        self.held = Some(HeldDrag {
            client,
            workspace,
            start: nodes::rect_of(&std.st.borrow().g, client),
            sides: Sides::from_flags(0),
            split: split.as_ref().map(SplitView::of),
        });
        let outcome = self.run(std, STD_DRAG_START, event, &[]);
        if !outcome.took {
            self.held = None;
            return false;
        }
        if let (Some(held), Some(sides)) = (&mut self.held, outcome.sides) {
            held.sides = sides;
        }
        self.apply(std, event, &outcome);
        true
    }

    fn update(&mut self, std: &Rc<Std>, event: &DragCtx) {
        let outcome = self.run(std, STD_DRAG_UPDATE, event, &[]);
        self.apply(std, event, &outcome);
    }

    fn end(&mut self, std: &Rc<Std>, event: &DragCtx) {
        let tiles = self
            .held
            .as_ref()
            .and_then(|held| held.workspace)
            .map(|workspace| tiles_of(std, workspace))
            .unwrap_or_default();
        let outcome = self.run(std, STD_DRAG_END, event, &tiles);
        self.apply(std, event, &outcome);
        // The release trades places with the tile it was dropped on, when the dragged window is still on the workspace it started on.
        if let (Some(held), Some(over)) = (&self.held, outcome.swap)
            && let Some(workspace) = held.workspace
        {
            let client = held.client;
            with_state(std, |st| {
                if st.rec.get(&client).and_then(|rec| rec.workspace) == Some(workspace) {
                    let _ = st.swap_clients(workspace, client, over);
                }
            });
        }
        self.held = None;
    }

    fn cancel(&mut self, std: &Rc<Std>, event: &DragCtx) {
        let outcome = self.run(std, STD_DRAG_CANCEL, event, &[]);
        self.apply(std, event, &outcome);
        self.held = None;
    }
}
