//! The canvas tree: making and ending canvases, putting them under one another and under nodes of the compositor, and what a
//! generation keeps of the toolkit.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gnoblin_core::{LogLevel, Node, NodeId, Value};
use gnoblin_core_abi::{self as abi, GnoblinCtx};
use gnoblin_object::server::{Cx, Library};
use gnoblin_object::{ErrorCode, Fail, Obj, Res, Val};

use super::widget::{CanvasHandle, Canvases, Cid, Ids, ModelIds, Ui, Widget, WidgetIds, WidgetKind};
use crate::object::with_lib;

thread_local! {
    /// The toolkit of each generation of this thread: a shell has one thread, and a test may run several generations in turn.
    static UIS: RefCell<HashMap<GnoblinCtx, Rc<Ui>>> = RefCell::new(HashMap::new());
}

/// The toolkit of the generation `ctx`, if it has one.
pub(crate) fn find(ctx: GnoblinCtx) -> Option<Rc<Ui>> {
    UIS.with(|uis| uis.borrow().get(&ctx).cloned())
}

/// Every toolkit of this thread, for a test that changes what they measure with.
#[cfg(any(test, feature = "testing"))]
pub(crate) fn all() -> Vec<Rc<Ui>> {
    UIS.with(|uis| uis.borrow().values().cloned().collect())
}

/// Takes a generation's toolkit away, once it has ended.
pub(crate) fn forget(ctx: GnoblinCtx) {
    UIS.with(|uis| uis.borrow_mut().remove(&ctx));
}

/// The toolkit of the generation `ctx`, made on the first use: it attaches the generation to the tree and subscribes to what it needs
/// of the compositor.
///
/// # Errors
///
/// The core refuses the ABI, the tree cannot be attached, or a subscription is refused.
pub(crate) fn ui_for(ctx: GnoblinCtx) -> Res<Rc<Ui>> {
    if let Some(ui) = find(ctx) {
        return Ok(ui);
    }
    let g = gnoblin_core::private::gnoblin_for(ctx).map_err(core_fail)?;
    g.attach().map_err(core_fail)?;
    let ui = Rc::new(Ui {
        ctx,
        g,
        w: RefCell::new(Canvases::default()),
        pending: Cell::new(false),
        timer: RefCell::new(None),
        core_timers: RefCell::new(None),
        epoch: Cell::new(0),
        frame_hooks: RefCell::new(Vec::new()),
        request_hooks: RefCell::new(Vec::new()),
        next_hook: Cell::new(0),
        ids: RefCell::new(None),
        frames: RefCell::new(crate::frame::FrameWorld::default()),
        menus: RefCell::new(crate::menus::State::default()),
        shell: RefCell::new(crate::shell::ShellWorld::default()),
        lock: RefCell::new(crate::lock::LockWorld::default()),
        policy: RefCell::new(crate::policy::PolicyWorld::default()),
    });
    UIS.with(|uis| uis.borrow_mut().insert(ctx, Rc::clone(&ui)));
    if let Err(fail) = super::input::subscribe(&ui) {
        forget(ctx);
        return Err(fail);
    }
    Ok(ui)
}

pub(crate) fn core_fail(error: gnoblin_core::CoreError) -> Fail {
    Fail::new(error.kind().code(), error.to_string())
}

impl Ui {
    /// Reads a canvas: `None` when the toolkit has no such canvas (its object ended).
    pub(crate) fn read<R>(&self, cid: Cid, read: impl FnOnce(&Widget) -> R) -> Option<R> {
        self.w.borrow().get(cid).map(read)
    }

    /// Changes a canvas without asking for a turn: for what the update pass itself does.
    pub(crate) fn tweak<R>(&self, cid: Cid, change: impl FnOnce(&mut Widget) -> R) -> Option<R> {
        self.w.borrow_mut().get_mut(cid).map(change)
    }

    /// Changes a canvas and asks for a turn, for what a shell did.
    pub(crate) fn edit<R>(self: &Rc<Self>, cid: Cid, change: impl FnOnce(&mut Widget) -> R) -> Res<R> {
        let done = self
            .tweak(cid, change)
            .ok_or_else(|| Fail::stale("the canvas is gone"))?;
        self.schedule();
        Ok(done)
    }

    /// A handle on the node `id`.
    pub(crate) fn node(&self, id: u64) -> Node {
        self.g.node(NodeId(id))
    }

    /// Writes a line to the generation's log.
    pub(crate) fn warn(&self, message: &str) {
        #[cfg(any(test, feature = "testing"))]
        super::testing::note(message);
        self.g.log_at(LogLevel::Warn, &format!("gnoblin-std: {message}"));
    }

    /// The ids of the object library's classes and members that the toolkit calls by name.
    pub(crate) fn ids(&self) -> Ids {
        if let Some(ids) = *self.ids.borrow() {
            return ids;
        }
        let ids = with_lib(resolve).unwrap_or_else(|fail| {
            self.warn(&format!("the toolkit's catalog is incomplete: {fail}"));
            Ids {
                widget: WidgetIds::default(),
                model: ModelIds::default(),
            }
        });
        *self.ids.borrow_mut() = Some(ids);
        ids
    }

    /// Makes a canvas of `kind`: its node, in this turn's transaction, and nothing else. The caller puts it in an object.
    pub(crate) fn make(&self, kind: WidgetKind) -> Res<Cid> {
        let node = self.g.create_node("canvas").map_err(core_fail)?.id().0;
        let mut w = self.w.borrow_mut();
        let cid = w.mint();
        w.widgets.insert(cid, Widget::new(kind, node));
        w.by_node.insert(node, cid);
        Ok(cid)
    }

    /// Makes a canvas of `kind` that lives in the object `obj`, the object the call is about. Returns its handle state.
    pub(crate) fn make_for(self: &Rc<Self>, obj: Obj, kind: WidgetKind) -> Res<CanvasHandle> {
        let cid = self.make(kind)?;
        self.tweak(cid, |widget| widget.obj = obj);
        self.schedule();
        Ok(CanvasHandle {
            ui: Rc::clone(self),
            cid,
        })
    }

    /// Makes a canvas of `kind` in an object of its own, owned by the object the call is about.
    pub(crate) fn spawn(self: &Rc<Self>, cx: &mut Cx<'_>, kind: WidgetKind, owner: Obj) -> Res<(Obj, Cid)> {
        self.spawn_named(cx, kind.class(), kind, owner)
    }

    /// Makes a canvas of `kind` in an object of the class `class`, which derives from the class of `kind` and is owned by `owner`.
    pub(crate) fn spawn_named(
        self: &Rc<Self>,
        cx: &mut Cx<'_>,
        class: &str,
        kind: WidgetKind,
        owner: Obj,
    ) -> Res<(Obj, Cid)> {
        let cid = self.make(kind)?;
        let handle = CanvasHandle {
            ui: Rc::clone(self),
            cid,
        };
        let obj = cx.spawn(class, handle)?;
        self.tweak(cid, |widget| widget.obj = obj);
        cx.set_parent(obj, owner)?;
        self.schedule();
        Ok((obj, cid))
    }

    /// Whether the canvas exists in the compositor's tree as far as the toolkit knows.
    pub(crate) fn alive(&self, cid: Cid) -> bool {
        self.read(cid, |widget| widget.alive).unwrap_or(false)
    }

    /// Puts `child` last under `parent`: the toolkit's tree and the compositor's.
    ///
    /// # Errors
    ///
    /// Either canvas is destroyed, `child` is `parent` or holds it, or the compositor refuses the insertion.
    pub(crate) fn link(&self, parent: Cid, child: Cid) -> Res<()> {
        let (parent_node, child_node) = {
            let mut w = self.w.borrow_mut();
            let alive = |w: &Canvases, cid: Cid| w.get(cid).is_some_and(|widget| widget.alive);
            if !alive(&w, parent) || !alive(&w, child) {
                return Err(Fail::stale("Canvas.add: the canvas is destroyed"));
            }
            if w.contains(child, parent) {
                return Err(Fail::type_error("a canvas cannot be its own ancestor"));
            }
            if let Some(old) = w.get(child).and_then(|widget| widget.parent)
                && let Some(old) = w.get_mut(old)
            {
                old.children.retain(|held| *held != child);
            }
            let nodes = (
                w.get(parent).map_or(0, |widget| widget.node),
                w.get(child).map_or(0, |widget| widget.node),
            );
            if let Some(widget) = w.get_mut(child) {
                widget.parent = Some(parent);
            }
            if let Some(widget) = w.get_mut(parent) {
                widget.children.push(child);
            }
            nodes
        };
        self.node(child_node)
            .insert_into(&self.node(parent_node), None)
            .map_err(|error| refused("add", &error))
    }

    /// Takes `cid` out of its parent canvas's list, so the canvas it hangs under in the compositor is not a canvas.
    fn orphan(&self, cid: Cid) {
        let mut w = self.w.borrow_mut();
        if let Some(old) = w.get(cid).and_then(|widget| widget.parent)
            && let Some(old) = w.get_mut(old)
        {
            old.children.retain(|held| *held != cid);
        }
        if let Some(widget) = w.get_mut(cid) {
            widget.parent = None;
        }
    }

    /// Mounts `cid` as the last child of a node that is not a canvas.
    pub(crate) fn mount(&self, cid: Cid, parent: u64) -> Res<()> {
        let node = self
            .read(cid, |widget| widget.alive.then_some(widget.node))
            .flatten()
            .ok_or_else(|| Fail::stale("Canvas.mount: the canvas is destroyed"))?;
        self.orphan(cid);
        self.node(node)
            .insert_into(&self.node(parent), None)
            .map_err(|error| refused("mount", &error))
    }

    /// Mounts `cid` at the root, above every window: the windows that have no parent are given the root first, in the order they
    /// already have, because the tree stacks a parentless window after everything under the root.
    pub(crate) fn mount_top(&self, cid: Cid) -> Res<()> {
        let node = self
            .read(cid, |widget| widget.alive.then_some(widget.node))
            .flatten()
            .ok_or_else(|| Fail::stale("Canvas.mountTop: the canvas is destroyed"))?;
        self.orphan(cid);
        let root = self.g.root().map_err(core_fail)?;
        for window in self.g.windows().map_err(core_fail)? {
            let handle = self.g.node(window.id());
            if handle.parent().ok().flatten().is_none() {
                handle
                    .insert_into(&root, None)
                    .map_err(|error| refused("mountTop", &error))?;
            }
        }
        self.node(node)
            .insert_into(&root, None)
            .map_err(|error| refused("mountTop", &error))
    }

    /// The node that holds the seat's keyboard focus as the shell's writes leave it.
    pub(crate) fn seat_focus(&self) -> Option<u64> {
        let seat = crate::nodes::seat(&self.g)?;
        seat.get_node("focus")
            .ok()
            .flatten()
            .map(|node| node.id().0)
            .filter(|id| *id != 0)
    }

    /// Gives the seat's keyboard focus to `node`, or takes it away with zero.
    ///
    /// # Errors
    ///
    /// The tree has no seat, or the compositor refuses.
    pub(crate) fn set_focus(&self, node: u64) -> Res<()> {
        let seat = crate::nodes::seat(&self.g)
            .ok_or_else(|| Fail::refused(ErrorCode::Unavailable, "focus: the tree has no seat"))?;
        seat.set("focus", Value::Node(NodeId(node)))
            .map_err(|error| refused("focus", &error))
    }

    /// Whether `cid` holds the seat's keyboard focus.
    pub(crate) fn focused(&self, cid: Cid) -> bool {
        match self.read(cid, |widget| widget.alive.then_some(widget.node)).flatten() {
            Some(node) => self.seat_focus() == Some(node),
            None => false,
        }
    }

    /// Marks `cid` and everything under it gone, and says what ended: the canvases that were alive, parents first. The compositor's
    /// node is not written: a caller that ends a node does that.
    pub(crate) fn bury(&self, cid: Cid) -> Vec<Cid> {
        let mut w = self.w.borrow_mut();
        if !w.get(cid).is_some_and(|widget| widget.alive) {
            return Vec::new();
        }
        if let Some(old) = w.get(cid).and_then(|widget| widget.parent)
            && let Some(old) = w.get_mut(old)
        {
            old.children.retain(|held| *held != cid);
        }
        let subtree = w.subtree(cid);
        let mut ended = Vec::new();
        for one in subtree {
            let node = match w.get_mut(one) {
                Some(widget) if widget.alive => {
                    widget.alive = false;
                    widget.parent = None;
                    widget.children.clear();
                    widget.node
                }
                _ => continue,
            };
            w.by_node.remove(&node);
            ended.push(one);
        }
        if w.focus_seen.is_some_and(|seen| ended.contains(&seen)) {
            w.focus_seen = None;
        }
        ended
    }

    /// The object of `cid` ended: its canvas goes, and with it the canvases under it. The node is destroyed once, at the top of what
    /// goes, and the compositor takes the rest of the subtree with it.
    pub(crate) fn object_ended(&self, cid: Cid) {
        let was_alive = self.read(cid, |widget| widget.alive).unwrap_or(false);
        let node = was_alive.then(|| self.read(cid, |widget| widget.node)).flatten();
        if was_alive {
            self.bury(cid);
        }
        self.w.borrow_mut().widgets.remove(&cid);
        if let Some(node) = node {
            // A node the compositor already took, or one of a generation that ended, is not an error.
            let _ = self.node(node).destroy();
        }
    }

    /// Destroys the canvas and everything under it the way a shell's `destroy` does: the nodes go and the canvases are no longer alive,
    /// and what they were stays readable until the shell lets go of their objects.
    pub(crate) fn kill(self: &Rc<Self>, cid: Cid) {
        let Some(node) = self.read(cid, |widget| widget.alive.then_some(widget.node)).flatten() else {
            return;
        };
        // A node the compositor already took, or one of a generation that ended, is not an error.
        let _ = self.node(node).destroy();
        self.node_gone(cid);
    }

    /// Ends the canvas and everything under it: the objects go, and with them the nodes.
    pub(crate) fn destroy(&self, cid: Cid) {
        let Some(obj) = self.read(cid, |widget| widget.obj) else {
            return;
        };
        let ctx = self.ctx;
        if with_lib(|lib| lib.destroy(ctx, obj)).is_err() {
            // The object is already gone: its canvas goes with it.
            self.object_ended(cid);
        }
    }

    /// Tells what `work` announces about the object `obj`: a signal, a changed property.
    pub(crate) fn announce(&self, obj: Obj, work: impl FnOnce(&mut Cx<'_>)) {
        let ctx = self.ctx;
        let _ = with_lib(|lib| lib.with_cx(ctx, obj, work));
    }

    /// Runs `work` as a call about `obj`, from outside a call: for what the library does when the compositor or a frame asks and it has
    /// objects to make. `None` when the generation holds no such object any more.
    pub(crate) fn with_cx<R>(&self, obj: Obj, work: impl FnOnce(&mut Cx<'_>) -> R) -> Option<R> {
        let ctx = self.ctx;
        with_lib(|lib| lib.with_cx(ctx, obj, work)).ok()
    }

    /// Hands the object `obj` to the shell as a value: one more reference, which the shell releases.
    pub(crate) fn handout(&self, obj: Obj) -> Val {
        let ctx = self.ctx;
        with_lib(|lib| lib.with_cx(ctx, obj, |cx| cx.handout(obj)))
            .and_then(|value| value)
            .unwrap_or(Val::None)
    }

    /// Asks for a turn of the toolkit: what changed is written from it, once, whatever number of changes were made.
    pub(crate) fn schedule(&self) {
        if self.pending.replace(true) {
            return;
        }
        let ctx = self.ctx;
        if self.g.poster().post(move |_g| super::update::turn(ctx)).is_err() {
            self.pending.set(false);
        }
    }
}

/// A refusal of the compositor as a failure.
pub(crate) fn refused(what: &str, error: &gnoblin_core::CoreError) -> Fail {
    Fail::new(abi::E_REFUSED, format!("Canvas.{what}: the host refused ({error})"))
}

/// The ids of the classes and members the toolkit calls by name.
fn resolve(lib: &Library) -> Res<Ids> {
    let widget = lib.class_id("widget")?;
    let member = |class: u32, name: &str| lib.member_id(class, abi::MEMBER_METHOD, name);
    let model = lib.class_id("entry_model")?;
    let property = |name: &str| lib.member_id(model, abi::MEMBER_PROPERTY, name);
    Ok(Ids {
        widget: WidgetIds {
            refresh: member(widget, "refresh")?,
            arrange: member(widget, "arrange")?,
            paint: member(widget, "paint")?,
            natural_width: member(widget, "natural_width")?,
            natural_height: member(widget, "natural_height")?,
            on_pointer: member(widget, "on_pointer")?,
            on_key: member(widget, "on_key")?,
        },
        model: ModelIds {
            text: property("text")?,
            caret: property("caret")?,
            anchor: property("anchor")?,
            insert: member(model, "insert")?,
            backspace: member(model, "backspace")?,
            delete: member(model, "delete")?,
            left: member(model, "left")?,
            right: member(model, "right")?,
            home: member(model, "home")?,
            end: member(model, "end")?,
            select_all: member(model, "select_all")?,
            before: member(model, "before")?,
        },
    })
}

/// The generation's toolkit for an object call, from the generation the call is for.
pub(crate) fn ui_of(cx: &Cx<'_>) -> Res<Rc<Ui>> {
    ui_for(cx.ctx())
}
