//! What the library keeps for one shell generation, and how a shell's callbacks are reached.
//!
//! A generation that calls `open` gets one [`Std`], kept in a thread-local map keyed by the core's context handle: a shell has one
//! thread, and everything here runs on it. The [`State`] inside is the object model of the desktop: the clients, outputs and
//! workspaces, the collections over them, the desks that say which output shows which workspace, and the notices that are waiting
//! to be told to the shell's listeners.
//!
//! ## The rule about callbacks
//!
//! The library never runs a shell callback while it holds a borrow of its state and never inside a table call. A change queues a
//! [`Notice`]; [`Std::drain`] delivers the queue, outside any borrow, at the end of a callback the library itself received from the
//! core and at the start of the next turn (the posted pass in `turn.rs`). A shell's listener may therefore call any table function.
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use gnoblin_core::{Gnoblin, Rect};
use gnoblin_core_abi::{self as abi, GnoblinCtx, GnoblinDropFn, GnoblinStr};
use gnoblin_std_abi::{GnoblinStdEvent, GnoblinStdEventFn, GnoblinStdRect};

use crate::collection::Coll;
use crate::desks::{Desk, Policy, Slide};
use crate::error::{Fail, Res};
use crate::layout::LayoutObj;
use crate::workspace::Workspace;

/// A library-made id: a workspace, a collection or a registration. Never zero, never reused in the process.
pub(crate) type Id = u64;

/// A node of the compositor's tree, by id.
pub(crate) type NodeId = u64;

thread_local! {
    static NEXT_ID: Cell<u64> = const { Cell::new(0) };
    static OPEN: RefCell<HashMap<GnoblinCtx, Rc<Std>>> = RefCell::new(HashMap::new());
    static RETIRED: RefCell<HashSet<GnoblinCtx>> = RefCell::new(HashSet::new());
}

/// The next id.
pub(crate) fn next_id() -> Id {
    NEXT_ID.with(|next| {
        let id = next.get() + 1;
        next.set(id);
        id
    })
}

/// What the library knows about one window.
#[derive(Debug, Default)]
pub(crate) struct ClientRec {
    /// The window was in the tree when the shell opened the library: a layout is not offered it as an arrival.
    pub(crate) preexisting: bool,
    /// The workspace the client is on.
    pub(crate) workspace: Option<Id>,
    /// The shell marked the client floating: a layout leaves it where it is.
    pub(crate) floating: bool,
    /// The client was offered to a layout as an arrival, so leaving the workspace is told to the layout.
    pub(crate) offered: bool,
    /// A dialog that was centred in its parent already.
    pub(crate) centred: bool,
    /// What this library last wrote to the window's `visible`: a client it has not hidden is shown.
    pub(crate) written_visible: Option<bool>,
    /// Whether the last placement wanted the client shown.
    pub(crate) shown_by_layout: Option<bool>,
    /// The window this one is a dialog of, as last read: the window is gone from reads when it closes, and its parent gets the
    /// keyboard back.
    pub(crate) parent: Option<NodeId>,
    /// Where the window was when it last changed, so a dialog follows its parent by the distance it moved.
    pub(crate) last_pos: (f64, f64),
    /// What the last layout pass wrote to the window's rectangle, and what the window held before it did: a write the compositor
    /// never took leaves the window at `before`, and the pass that finds it there lays the workspace out again.
    pub(crate) expected: Option<Rect>,
    pub(crate) before: Option<Rect>,
    /// How many passes in a row found the expected rectangle missing.
    pub(crate) lost: u8,
    /// The geometry changed since the listeners were last told.
    pub(crate) geometry_pending: bool,
    /// The room the client's frame takes round its content (`client_set_state`).
    pub(crate) inset: crate::floating::Inset,
    /// The least and the most the client may be resized to, when the shell said.
    pub(crate) min: Option<(f64, f64)>,
    pub(crate) max: Option<(f64, f64)>,
    /// The rectangle the client floats at, when the shell kept one.
    pub(crate) float_rect: Option<Rect>,
    /// Where the client is snapped, and the rectangle it floated at before.
    pub(crate) snap: Option<crate::client_state::Snap>,
}

/// A change that is waiting to be told to the shell's listeners.
#[derive(Debug, Clone)]
pub(crate) struct Notice {
    /// One of the `STD_EVENT_*` kinds.
    pub(crate) kind: u32,
    /// The member the notice is about.
    pub(crate) member: u64,
    /// The collections that hold the member: a listener of one of them hears the notice.
    pub(crate) holders: Vec<Id>,
    /// A listener of exactly this registration hears it (a replay), and nobody else.
    pub(crate) only: Option<Id>,
    pub(crate) text: String,
    pub(crate) flag: u32,
    pub(crate) rect: Rect,
}

impl Notice {
    pub(crate) fn new(kind: u32, member: u64, holders: Vec<Id>) -> Notice {
        Notice {
            kind,
            member,
            holders,
            only: None,
            text: String::new(),
            flag: 0,
            rect: Rect::default(),
        }
    }
}

/// A listener the shell registered: its callback and the closure behind `user`, freed exactly once when the last reference goes.
pub(crate) struct RegData {
    pub(crate) coll: Id,
    pub(crate) event: u32,
    cb: GnoblinStdEventFn,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
}

impl RegData {
    pub(crate) fn new(
        coll: Id,
        event: u32,
        cb: GnoblinStdEventFn,
        user: *mut c_void,
        drop_user: Option<GnoblinDropFn>,
    ) -> RegData {
        RegData {
            coll,
            event,
            cb,
            user,
            drop_user,
        }
    }

    /// Calls the shell's listener with `event`. A panic is contained: the shell's trampoline catches its own, and this catches
    /// whatever else would unwind through the library.
    fn call(&self, event: &GnoblinStdEvent) {
        // SAFETY: `cb` and `user` are what the shell passed to `collection_listen`; the library adopted `user` and frees it only
        // in `drop`, after the last reference to this registration, so it is live for the call. The callback runs on the shell
        // thread, the one this code runs on.
        let _ = catch_unwind(AssertUnwindSafe(|| unsafe { (self.cb)(self.user, event) }));
    }
}

impl Drop for RegData {
    fn drop(&mut self) {
        if let Some(drop_user) = self.drop_user.take() {
            // SAFETY: the shell gave `user` and its `drop_user` together; this is the one call, made when the last reference to
            // the registration is gone, so nothing can use `user` after it.
            let _ = catch_unwind(AssertUnwindSafe(|| unsafe { drop_user(self.user) }));
        }
    }
}

/// Counters, for `stats`.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Counters {
    pub(crate) layout_passes: u64,
    pub(crate) computes: u64,
}

/// The object model of one generation.
pub(crate) struct State {
    pub(crate) g: Gnoblin,
    pub(crate) ctx: GnoblinCtx,
    /// The first turn has run: windows that waited get a workspace and a layout starts placing arrivals.
    pub(crate) opened: bool,
    /// Every window, in the order it appeared.
    pub(crate) clients: Vec<NodeId>,
    pub(crate) rec: HashMap<NodeId, ClientRec>,
    /// Every connected output, in the order it appeared.
    pub(crate) outputs: Vec<NodeId>,
    /// The added workspaces, in order.
    pub(crate) workspaces: Vec<Id>,
    pub(crate) ws: HashMap<Id, Workspace>,
    pub(crate) colls: HashMap<Id, Coll>,
    /// The root collections: clients, outputs, workspaces.
    pub(crate) roots: [Id; 3],
    pub(crate) desks: Vec<Desk>,
    pub(crate) regs: HashMap<Id, Rc<RegData>>,
    pub(crate) notices: Vec<Notice>,
    /// Layouts that were replaced: dropped outside any borrow, because dropping one runs the shell's `drop_user`.
    pub(crate) graveyard: Vec<Rc<LayoutObj>>,
    pub(crate) slides: Vec<Slide>,
    /// Workspaces whose windows stay drawn until a slide away from them ends.
    pub(crate) holding: HashMap<Id, u32>,
    pub(crate) policy: Policy,
    /// The window that had the keyboard at the end of the last turn.
    pub(crate) last_focus: Option<NodeId>,
    pub(crate) counters: Counters,
    /// The transition engine: staged writes, tracks and what owns them.
    pub(crate) anim: crate::animation::engine::AnimState,
    /// Styles: what each scope sets and what was applied to each client.
    pub(crate) styles: crate::style::cascade::StyleState,
    /// The scene nodes the shell made.
    pub(crate) scene: crate::scene::SceneState,
    /// The built-in layouts that belong to no workspace (`standalone.rs`), by handle.
    pub(crate) builtins: HashMap<Id, Rc<LayoutObj>>,
}

/// One generation's library state and the flags that decide what runs when.
pub(crate) struct Std {
    pub(crate) st: RefCell<State>,
    /// The pointer and gesture bindings, their claims and the drags in flight (`bindings/`). Kept apart from the state: a binding's
    /// callback runs while a drag is being routed, and the state must be free for it.
    pub(crate) router: RefCell<crate::bindings::BindingRouter>,
    /// A turn is posted and has not run.
    pub(crate) scheduled: Cell<bool>,
    /// Notices are being delivered.
    draining: Cell<bool>,
}

impl Std {
    pub(crate) fn new(state: State) -> Std {
        Std {
            st: RefCell::new(state),
            router: RefCell::new(crate::bindings::BindingRouter::default()),
            scheduled: Cell::new(false),
            draining: Cell::new(false),
        }
    }

    /// The generation's handle on the core.
    pub(crate) fn g(&self) -> Gnoblin {
        self.st.borrow().g.clone()
    }

    /// Tells the shell's listeners what is queued, in order, until nothing is left. Runs outside any borrow of the state; a
    /// listener that changes something queues more, which this delivers too. Delivery is not re-entrant: a call made from inside
    /// a listener leaves its notices for the loop that is running.
    pub(crate) fn drain(&self) {
        if self.draining.replace(true) {
            return;
        }
        loop {
            let (notices, graveyard) = {
                let mut st = self.st.borrow_mut();
                (std::mem::take(&mut st.notices), std::mem::take(&mut st.graveyard))
            };
            let had_graveyard = !graveyard.is_empty();
            // Dropping a replaced layout runs the shell's `drop_user`, which may queue more notices: look again.
            drop(graveyard);
            if notices.is_empty() && !had_graveyard {
                break;
            }
            for notice in notices {
                self.deliver(&notice);
            }
        }
        self.draining.set(false);
    }

    /// Calls every listener that hears `notice`.
    fn deliver(&self, notice: &Notice) {
        let heard: Vec<(Id, Rc<RegData>)> = {
            let st = self.st.borrow();
            let mut heard: Vec<(Id, Rc<RegData>)> = st
                .regs
                .iter()
                .filter(|(id, reg)| match notice.only {
                    Some(only) => only == **id,
                    None => reg.event == notice.kind && notice.holders.contains(&reg.coll),
                })
                .map(|(id, reg)| (*id, Rc::clone(reg)))
                .collect();
            heard.sort_by_key(|(id, _)| *id);
            heard
        };
        for (id, reg) in heard {
            // A listener an earlier listener removed hears nothing.
            if !self.st.borrow().regs.contains_key(&id) {
                continue;
            }
            let text = notice.text.as_bytes();
            let event = GnoblinStdEvent {
                size: size_of::<GnoblinStdEvent>() as u32,
                kind: notice.kind,
                flag: notice.flag,
                reserved: 0,
                collection: reg.coll,
                member: notice.member,
                text: GnoblinStr {
                    ptr: if text.is_empty() {
                        core::ptr::null()
                    } else {
                        text.as_ptr()
                    },
                    len: text.len(),
                },
                rect: GnoblinStdRect {
                    x: notice.rect.x,
                    y: notice.rect.y,
                    width: notice.rect.width,
                    height: notice.rect.height,
                },
            };
            reg.call(&event);
            if notice.kind == gnoblin_std_abi::STD_EVENT_TRANSITION_FINISHED {
                // A transition ends once: the listener has heard it and goes.
                let removed = self.st.borrow_mut().regs.remove(&id);
                drop(removed);
            }
        }
    }
}

/// The library state of the generation `ctx`.
///
/// # Errors
///
/// The generation retired (`E_RETIRED`), or never opened the library (`E_STALE`).
pub(crate) fn lookup(ctx: GnoblinCtx) -> Res<Rc<Std>> {
    if let Some(std) = OPEN.with(|open| open.borrow().get(&ctx).cloned()) {
        return Ok(std);
    }
    if RETIRED.with(|retired| retired.borrow().contains(&ctx)) {
        return Err(Fail::new(
            abi::E_RETIRED,
            "the generation has retired; its std objects are gone",
        ));
    }
    Err(Fail::stale(
        "libgnoblin-std is not open for this generation: call `open` first",
    ))
}

/// Records the state of a newly opened generation.
pub(crate) fn insert(ctx: GnoblinCtx, std: Rc<Std>) {
    OPEN.with(|open| open.borrow_mut().insert(ctx, std));
    RETIRED.with(|retired| retired.borrow_mut().remove(&ctx));
}

/// The state of `ctx` if it is open, without an error.
pub(crate) fn find(ctx: GnoblinCtx) -> Option<Rc<Std>> {
    OPEN.with(|open| open.borrow().get(&ctx).cloned())
}

/// The generation retired: forgets its state, and every registration's `drop_user` runs now, while the shell's code is mapped.
pub(crate) fn retire(ctx: GnoblinCtx) {
    let std = OPEN.with(|open| open.borrow_mut().remove(&ctx));
    RETIRED.with(|retired| retired.borrow_mut().insert(ctx));
    if let Some(std) = std {
        // Each binding's `drop_user` runs while the shell's code is still mapped.
        crate::bindings::retire(&std);
        let (regs, workspaces, graveyard) = {
            let mut st = std.st.borrow_mut();
            st.notices.clear();
            let regs: Vec<Rc<RegData>> = st.regs.drain().map(|(_, reg)| reg).collect();
            let workspaces: Vec<Workspace> = st.ws.drain().map(|(_, workspace)| workspace).collect();
            let graveyard: Vec<Rc<LayoutObj>> = std::mem::take(&mut st.graveyard);
            (regs, workspaces, graveyard)
        };
        // Each drop below runs the shell's `drop_user` for a listener or a layout, while the shell's code is still mapped.
        drop(regs);
        drop(workspaces);
        drop(graveyard);
    }
}

/// Forgets the state of a generation whose `open` failed. The generation is still live, so it is not marked retired.
pub(crate) fn remove(ctx: GnoblinCtx) {
    let std = OPEN.with(|open| open.borrow_mut().remove(&ctx));
    drop(std);
}

/// The library state that owns registration `reg`.
pub(crate) fn owner_of(reg: Id) -> Option<Rc<Std>> {
    OPEN.with(|open| {
        open.borrow()
            .values()
            .find(|std| std.st.borrow().regs.contains_key(&reg) || std.router.borrow().has(reg))
            .cloned()
    })
}
