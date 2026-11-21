//! The classes of `display.toml`: output profiles, displays, viewports and the manager that keeps the lists of them.
//!
//! `data/js/gnoblin-std/display/*` and `display_manager/*` are the reference. The model and the reconciling were ported before the object
//! protocol existed (`displays.rs`, and the std table's `profile_*` and `display_*` functions in `display_api.rs`, which the TypeScript
//! library still calls), so these classes and those functions see one set of profiles and displays. What this module adds is the objects: a
//! name for each of them that a shell holds, the checks of what a shell writes (with the words the TypeScript checks used), the state of a
//! viewport, and the signals the library tells the manager.
//!
//! A profile, a display and a viewport are names for what the library keeps. Asking again for the same one gives the same object. The
//! library makes a display for every output no display of the shell presents, and makes its object, and its viewport's, when a shell asks.
//!
//! The library starts the desktop when the manager is made or a display or a profile is changed (`events::open`): a display is where its
//! output is, so the outputs, the desks and the default workspace come with the first call, as they come with `open` of the table.

mod display;
mod manager;
mod profile;
mod viewport;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gnoblin_core::LogLevel;
use gnoblin_core_abi::GnoblinCtx;
use gnoblin_object::server::Builder;
use gnoblin_object::{Fail, Obj, Res, Val};
use gnoblin_std_abi as abi;

use crate::displays::Hid;
use crate::error::Res as StdRes;
use crate::object::with_lib;
use crate::scene::CameraState;
use crate::state::{NodeId, Notice, State, Std};

thread_local! {
    /// The object of each profile a shell was given: one for each, so asking for a profile again gives the one object.
    static PROFILES: RefCell<HashMap<(GnoblinCtx, Hid), Obj>> = RefCell::new(HashMap::new());
    /// The object of each display a shell was given.
    static DISPLAYS: RefCell<HashMap<(GnoblinCtx, Hid), Obj>> = RefCell::new(HashMap::new());
    /// The object of each viewport a shell was given.
    static VIEWPORTS: RefCell<HashMap<(GnoblinCtx, u64), Obj>> = RefCell::new(HashMap::new());
    /// The manager, once a shell has one.
    static MANAGERS: RefCell<HashMap<GnoblinCtx, Obj>> = RefCell::new(HashMap::new());
    /// The outputs each display presented when its object was last told, to tell when they change.
    static TOLD: RefCell<HashMap<(GnoblinCtx, Hid), Vec<NodeId>>> = RefCell::new(HashMap::new());
}

/// Runs `work` over the library's state as its desktop, for a change.
pub(super) fn edit<R>(ctx: GnoblinCtx, work: impl FnOnce(&mut State) -> StdRes<R>) -> Res<R> {
    crate::events::open(ctx).map_err(Fail::from)?;
    crate::table::mutate(ctx, work).map_err(Fail::from)
}

/// Runs `work` over the library's state as its desktop, to read.
pub(super) fn look<R>(ctx: GnoblinCtx, work: impl FnOnce(&State) -> StdRes<R>) -> Res<R> {
    crate::events::open(ctx).map_err(Fail::from)?;
    crate::table::read(ctx, work).map_err(Fail::from)
}

/// The manager of the generation `ctx`, if a shell has one.
fn manager_of(ctx: GnoblinCtx) -> Option<Obj> {
    MANAGERS.with(|held| held.borrow().get(&ctx).copied())
}

/// Tells the objects of the library what a notice says happened, from the next turn: the lists changed or a display presents other
/// outputs, the compositor answered a request, the library has a sentence for the shell's diagnostics. It runs outside any borrow of the
/// state, as the delivery of the notice to the table's listeners does.
pub(crate) fn announce(std: &Std, notice: &Notice) {
    let ctx = std.st.borrow().ctx;
    match notice.kind {
        abi::STD_EVENT_DISPLAYS => {
            changed_outputs(std, ctx);
            if let Some(manager) = manager_of(ctx) {
                let _ = with_lib(|lib| lib.with_cx(ctx, manager, |cx| cx.emit("changed", Vec::new())));
            }
        }
        abi::STD_EVENT_APPLIED => {
            if let Some(manager) = manager_of(ctx) {
                let fields = vec![Val::Uint(notice.member), Val::Text(notice.text.clone())];
                let _ = with_lib(|lib| lib.with_cx(ctx, manager, |cx| cx.emit("applied", fields)));
            }
        }
        abi::STD_EVENT_NOTE => {
            // A shell of the TypeScript library hears the sentence through the table; one of the classes hears it here and in the log.
            if let Some(manager) = manager_of(ctx) {
                std.g().log_at(LogLevel::Warn, &notice.text);
                let fields = vec![Val::Text(notice.text.clone())];
                let _ = with_lib(|lib| lib.with_cx(ctx, manager, |cx| cx.emit("noted", fields)));
            }
        }
        _ => {}
    }
}

/// Tells each display object whose outputs changed since it was last told.
fn changed_outputs(std: &Std, ctx: GnoblinCtx) {
    let now: Vec<(Hid, Vec<NodeId>)> = {
        let st = std.st.borrow();
        st.disp
            .display_list
            .iter()
            .map(|hid| (*hid, st.display_outputs(*hid)))
            .collect()
    };
    let changed: Vec<Obj> = TOLD.with(|told| {
        let mut told = told.borrow_mut();
        let mut changed = Vec::new();
        for (hid, outputs) in now {
            if told.get(&(ctx, hid)) != Some(&outputs) {
                told.insert((ctx, hid), outputs);
                if let Some(obj) = DISPLAYS.with(|held| held.borrow().get(&(ctx, hid)).copied()) {
                    changed.push(obj);
                }
            }
        }
        changed
    });
    for obj in changed {
        let _ = with_lib(|lib| lib.with_cx(ctx, obj, |cx| cx.changed("outputs")));
    }
}

/// The sentence a camera on a viewport earns, when it asks for what the library cannot draw.
const CAMERA3D_NOTICE: &str = "not applied: camera3d (needs a projected workspace)";
const CAMERA2D_NOTICE: &str = "not applied: camera2d zoom, position and rotation (windows sit under the tree root: there is no per-display root to transform)";
const COLOR_NOTICE: &str =
    "not applied: viewport.clear_color (the compositor has no background colour behind a workspace)";

/// Called each turn after the layout pass: says what the cameras and colours of the displays' viewports ask of a renderer that has none.
/// Each sentence is told once for the generation.
pub(crate) fn frame(std: &Rc<Std>) {
    let (ctx, viewports): (GnoblinCtx, Vec<(Option<Obj>, bool)>) = {
        let st = std.st.borrow();
        if st.disp.viewports.is_empty() {
            return;
        }
        let viewports = st
            .disp
            .display_list
            .iter()
            .filter_map(|hid| st.disp.displays.get(hid).and_then(|rec| rec.viewport))
            .filter_map(|vid| st.disp.viewports.get(&vid))
            .map(|rec| (rec.camera, rec.clear_color != "#000000"))
            .collect();
        (st.ctx, viewports)
    };
    let mut told = Vec::new();
    for (camera, colored) in viewports {
        match camera.and_then(|obj| crate::scene::camera_state(ctx, obj)) {
            Some(CameraState::Projected) => told.push(CAMERA3D_NOTICE),
            Some(CameraState::Flat { identity: false }) => told.push(CAMERA2D_NOTICE),
            _ => {}
        }
        if colored {
            told.push(COLOR_NOTICE);
        }
    }
    let mut st = std.st.borrow_mut();
    for text in told {
        st.note(text.to_owned());
    }
}

/// The generation ended: the objects it held are gone.
pub(crate) fn retired(ctx: GnoblinCtx) {
    PROFILES.with(|held| held.borrow_mut().retain(|(owner, _), _| *owner != ctx));
    DISPLAYS.with(|held| held.borrow_mut().retain(|(owner, _), _| *owner != ctx));
    VIEWPORTS.with(|held| held.borrow_mut().retain(|(owner, _), _| *owner != ctx));
    MANAGERS.with(|held| held.borrow_mut().remove(&ctx));
    TOLD.with(|held| held.borrow_mut().retain(|(owner, _), _| *owner != ctx));
}

/// Implements the classes of the display module.
pub(crate) fn register(builder: &mut Builder) {
    profile::register(builder);
    viewport::register(builder);
    display::register(builder);
    manager::register(builder);
}
