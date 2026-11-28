//! The scene of a generation: the nodes the shell made and the objects that stand for them, and what the library hears of the tree.
//!
//! A node is made by the constructor of a class (`scene_node`, `view`, `camera3d`, `camera2d`) and held by an object that stays until
//! `destroy`, until its parent object ends, or until the generation does. The registry maps the tree's node numbers to those objects,
//! so a tree walk (`parent`, `children`, `find`) answers objects, and it keeps what the tree has no property for: a node's metres a
//! unit, the name a shell gave it and how a view fits a workspace.
//!
//! It also listens to the tree: a node of the scene told of a pointer edge announces the `pointer` signal of its object, and a
//! window or an output that changes asks for a turn of the scene, which is when mirrors follow their workspaces and bindings run
//! again.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gnoblin_core::host::{Host, LogLevel, NodeId};
use gnoblin_core_abi::GnoblinCtx;
use gnoblin_object::server::Cx;
use gnoblin_object::{Fail, Obj, Res, Val};

use super::bindings::SceneBindings;
use super::mirror::Mirror;
use super::{Kind, SceneState};
use crate::animation::staging::Prop;
use crate::math::transform::PoseMatrix;
use crate::math::vector::Point3;
use crate::object::with_lib;
use crate::ui::CanvasEvent;

thread_local! {
    /// The scene of each generation of this thread: a shell has one thread, and a test may run several generations in turn.
    static WORLDS: RefCell<HashMap<GnoblinCtx, Rc<SceneWorld>>> = RefCell::new(HashMap::new());
}

/// The properties of a window that a mirror or a binding may follow.
const WINDOW_PROPERTIES: [&str; 18] = [
    "x",
    "y",
    "width",
    "height",
    "opacity",
    "visible",
    "z",
    "rotation",
    "scale",
    "title",
    "app_id",
    "focused",
    "fullscreen",
    "maximised",
    "minimised",
    "tiled",
    "decoration",
    "parent_window",
];

/// The properties of an output that a mirror or a binding may follow.
const SCREEN_PROPERTIES: [&str; 8] = ["x", "y", "width", "height", "mode", "scale", "enabled", "primary"];

/// What the library keeps of one node besides what the tree holds.
#[derive(Clone, Debug)]
pub(crate) struct Entry {
    /// The object that holds the node.
    pub(crate) obj: Obj,
    pub(crate) kind: Kind,
    /// The id a shell gave, which `id` reads back.
    pub(crate) given: Option<String>,
    /// Metres in one unit of the node's space.
    pub(crate) metres: f64,
    /// How a workspace picture fits the view (a place in `content_fit`, from one): stretch until the shell says.
    pub(crate) fit: u32,
    /// The tree's own root, which is the compositor's.
    pub(crate) root: bool,
}

/// The scene of one generation.
pub(crate) struct SceneWorld {
    pub(crate) ctx: GnoblinCtx,
    pub(crate) g: Host,
    /// What the checked writes of nodes keep (the kinds the library made).
    pub(crate) state: RefCell<SceneState>,
    pub(crate) entries: RefCell<HashMap<u64, Entry>>,
    /// Metres in one scene unit.
    pub(crate) metres: Cell<f64>,
    /// The generation is ending: the objects that go no longer take their nodes with them.
    pub(crate) ending: Cell<bool>,
    pub(crate) mirrors: RefCell<HashMap<u64, Mirror>>,
    pub(crate) engine: RefCell<SceneBindings>,
    /// A turn of the scene is posted and has not run.
    pub(crate) pending: Cell<bool>,
}

/// The scene of the generation `ctx`, if it has one.
pub(crate) fn find(ctx: GnoblinCtx) -> Option<Rc<SceneWorld>> {
    WORLDS.with(|worlds| worlds.borrow().get(&ctx).cloned())
}

fn forget(ctx: GnoblinCtx) {
    WORLDS.with(|worlds| worlds.borrow_mut().remove(&ctx));
}

/// The scene of the generation `ctx`, made on the first use: it attaches the generation to the tree and listens for what it needs.
///
/// # Errors
///
/// The core refuses the ABI, the tree cannot be attached, or a subscription is refused.
pub(crate) fn world_for(ctx: GnoblinCtx) -> Res<Rc<SceneWorld>> {
    if let Some(world) = find(ctx) {
        return Ok(world);
    }
    let g = gnoblin_core::private::gnoblin_for(ctx).map_err(core_fail)?;
    g.attach().map_err(core_fail)?;
    let world = Rc::new(SceneWorld {
        ctx,
        g,
        state: RefCell::new(SceneState::default()),
        entries: RefCell::new(HashMap::new()),
        metres: Cell::new(1.0),
        ending: Cell::new(false),
        mirrors: RefCell::new(HashMap::new()),
        engine: RefCell::new(SceneBindings::default()),
        pending: Cell::new(false),
    });
    WORLDS.with(|worlds| worlds.borrow_mut().insert(ctx, Rc::clone(&world)));
    if let Err(fail) = subscribe(&world) {
        forget(ctx);
        return Err(fail);
    }
    Ok(world)
}

/// The scene for an object call, from the generation the call is for.
pub(crate) fn world_of(cx: &Cx<'_>) -> Res<Rc<SceneWorld>> {
    world_for(cx.ctx())
}

fn core_fail(error: gnoblin_core::host::CoreError) -> Fail {
    Fail::new(error.kind().code(), error.to_string())
}

/// Subscribes the scene to what it needs of the compositor.
fn subscribe(world: &Rc<SceneWorld>) -> Res<()> {
    let ctx = world.ctx;
    let g = &world.g;
    g.on_shutdown(move || shutdown(ctx)).map_err(core_fail)?;
    g.on_signal_named("tree", "event", move |event| {
        if let (Some(world), Some(raw)) = (find(ctx), CanvasEvent::read(event)) {
            world.on_event(&raw);
        }
    })
    .map_err(core_fail)?;
    // What a mirror or a binding may depend on: windows and outputs, when they come, go and change.
    for (class, properties) in [
        ("window", WINDOW_PROPERTIES.as_slice()),
        ("screen", SCREEN_PROPERTIES.as_slice()),
    ] {
        g.on_node_created(class, move |_node| ask(ctx)).map_err(core_fail)?;
        g.on_node_destroyed(class, move |_node| ask(ctx)).map_err(core_fail)?;
        for property in properties {
            // A property the compositor's schema does not have is not one to follow.
            let _ = g.on_class_property_changed(class, property, move |_node, _value| ask(ctx));
        }
    }
    Ok(())
}

/// Something a mirror or a binding may follow changed.
fn ask(ctx: GnoblinCtx) {
    if let Some(world) = find(ctx)
        && world.follows()
    {
        world.schedule();
    }
}

/// The generation ended: the nodes the shell made that have no parent of the library's go, with everything under them.
fn shutdown(ctx: GnoblinCtx) {
    let Some(world) = find(ctx) else {
        return;
    };
    let tops: Vec<Obj> = {
        let entries = world.entries.borrow();
        // The tree's root is the compositor's, and what hangs from it hangs from nothing the shell made.
        let made = |node: u64| entries.get(&node).is_some_and(|entry| !entry.root);
        entries
            .iter()
            .filter(|(node, entry)| {
                !entry.root
                    && world
                        .g
                        .node(NodeId(**node))
                        .parent()
                        .ok()
                        .flatten()
                        .is_none_or(|parent| !made(parent.id().0))
            })
            .map(|(_, entry)| entry.obj)
            .collect()
    };
    for obj in tops {
        let _ = with_lib(|lib| lib.destroy(ctx, obj));
    }
    world.ending.set(true);
    forget(ctx);
}

/// The handle an object of the scene holds: the generation's scene and the node. When the object ends, the node goes with it, unless
/// it is the tree's own root.
pub(crate) struct NodeHandle {
    pub(crate) world: Rc<SceneWorld>,
    pub(crate) node: u64,
    /// The tree's root, which is the compositor's: the object that holds it ends without taking it.
    pub(crate) root: bool,
}

impl Drop for NodeHandle {
    fn drop(&mut self) {
        self.world.object_ended(self.node, self.root);
    }
}

impl SceneWorld {
    /// Says a sentence the shell's author must read: a notice, once per site (see `LogLevel::Notice`).
    pub(crate) fn warn(&self, message: &str) {
        #[cfg(any(test, feature = "testing"))]
        crate::ui::testing::note(message);
        self.g.log_at(LogLevel::Notice, &format!("gnoblin-std: {message}"));
    }

    /// Whether something here follows the windows and the outputs: a mirror, or a binding.
    pub(crate) fn follows(&self) -> bool {
        !self.mirrors.borrow().is_empty() || self.engine.borrow().active()
    }

    /// Asks for a turn of the scene, once however many changes there are.
    pub(crate) fn schedule(self: &Rc<Self>) {
        if self.pending.replace(true) {
            return;
        }
        let ctx = self.ctx;
        if self.g.poster().post(move |_g| turn(ctx)).is_err() {
            self.pending.set(false);
        }
    }

    /// The entry of a node the shell made.
    pub(crate) fn entry(&self, node: u64) -> Option<Entry> {
        self.entries.borrow().get(&node).cloned()
    }

    /// Changes what is kept of a node.
    pub(crate) fn tweak<R>(&self, node: u64, change: impl FnOnce(&mut Entry) -> R) -> Option<R> {
        self.entries.borrow_mut().get_mut(&node).map(change)
    }

    /// How an error path names the node: its class and its number.
    pub(crate) fn label(&self, node: u64) -> String {
        match self.entry(node) {
            Some(entry) => format!("{}#{node}", entry.kind.label()),
            None => format!("Node#{node}"),
        }
    }

    /// Makes a node of `kind` in the tree. The caller puts it in an object with [`World::adopt`].
    pub(crate) fn make(&self, kind: Kind) -> Res<u64> {
        self.state.borrow_mut().make(&self.g, kind).map_err(Fail::from)
    }

    /// Records that `obj` holds `node`; `root` is the tree's own root.
    pub(crate) fn adopt(&self, node: u64, obj: Obj, kind: Kind, root: bool) {
        self.entries.borrow_mut().insert(
            node,
            Entry {
                obj,
                kind,
                given: None,
                metres: 1.0,
                fit: 3,
                root,
            },
        );
    }

    /// The tree's own root as a scene node: the library knows it as a plain node, so a shell can mount nodes under it.
    pub(crate) fn root(&self) -> Res<u64> {
        let root = self.g.root().map_err(core_fail)?.id().0;
        self.state.borrow_mut().kinds.entry(root).or_insert(Kind::Node);
        Ok(root)
    }

    /// The object that ended: its node goes, and what depends on it.
    pub(crate) fn object_ended(&self, node: u64, root: bool) {
        self.mirror_forget(node);
        self.engine
            .borrow_mut()
            .owner_ended(self.entry(node).map(|entry| entry.obj));
        self.entries.borrow_mut().remove(&node);
        self.state.borrow_mut().forget(node);
        if root || self.ending.get() {
            return;
        }
        // A node the compositor already took is not an error. The nodes under it go with it, and the objects of those that have one
        // end by themselves.
        let _ = self.g.node(NodeId(node)).destroy();
    }

    /// The placement of a node: its position, rotation and first scale.
    pub(crate) fn pose_at(&self, node: u64) -> Res<PoseMatrix> {
        let state = self.state.borrow();
        let read = |key: &str| state.get(&self.g, node, key).map_err(Fail::from);
        let number = |key: &str| -> Res<f64> {
            Ok(match read(key)? {
                Some(Prop::Number(value)) => value,
                _ => 0.0,
            })
        };
        let triple = |key: &str, default: [f64; 3]| -> Res<[f64; 3]> {
            Ok(match read(key)? {
                Some(Prop::Triple(value)) => value,
                _ => default,
            })
        };
        let position = Point3::new(number("x")?, number("y")?, number("z")?);
        Ok(PoseMatrix::new(
            position,
            triple("rotation", [0.0; 3])?,
            triple("scale", [1.0; 3])?[0],
        ))
    }

    /// The scene nodes the library made that sit directly under `node`, in tree order.
    pub(crate) fn children_of(&self, node: u64) -> Vec<(u64, Obj)> {
        let Ok(children) = self.g.node(NodeId(node)).children() else {
            return Vec::new();
        };
        let entries = self.entries.borrow();
        children
            .iter()
            .filter_map(|child| entries.get(&child.id().0).map(|entry| (child.id().0, entry.obj)))
            .collect()
    }

    /// The scene node the library made that `node` sits under, if there is one.
    pub(crate) fn parent_of(&self, node: u64) -> Option<(u64, Obj)> {
        let parent = self.g.node(NodeId(node)).parent().ok().flatten()?.id().0;
        self.entries.borrow().get(&parent).map(|entry| (parent, entry.obj))
    }

    /// Whether the node is in the tree.
    pub(crate) fn alive(&self, node: u64) -> bool {
        crate::nodes::alive(&self.g, node)
    }

    /// An event the compositor sent: a pointer edge on one of our nodes.
    fn on_event(self: &Rc<Self>, raw: &CanvasEvent) {
        let Some(entry) = self.entry(raw.node) else {
            return;
        };
        let Some(edge) = edge_of(&raw.kind) else {
            return;
        };
        let event = Val::Record {
            class: 0,
            fields: vec![
                Val::Enum { class: 0, value: edge },
                Val::Double(raw.x),
                Val::Double(raw.y),
                Val::Uint(raw.button),
            ],
        };
        let ctx = self.ctx;
        let _ = with_lib(|lib| lib.with_cx(ctx, entry.obj, |cx| cx.emit("pointer", vec![event])));
    }
}

/// The place in `pointer_edge` of what the compositor calls a pointer edge, counted from one.
fn edge_of(kind: &str) -> Option<u32> {
    Some(match kind {
        "enter" => 1,
        "motion" => 2,
        "leave" => 3,
        "press" => 4,
        "release" => 5,
        _ => return None,
    })
}

/// A turn of the scene: the mirrors follow their workspaces, and the bindings run again for what changed.
fn turn(ctx: GnoblinCtx) {
    let Some(world) = find(ctx) else {
        return;
    };
    world.pending.set(false);
    world.mirror_turn();
    world.binding_turn();
}
