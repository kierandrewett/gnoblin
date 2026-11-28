//! XR: what a shell sees of a headset, and the room of panels the library builds for it (`xr.toml`).
//!
//! The XR program (`gnoblin-xr`) is a separate process; what the shell sees of it is nodes. Its session is the seat named `xr`, its head
//! the camera named `head` it writes, its hands `controller` nodes. The library follows them on a turn of its own:
//!
//! - while the seat exists there is a session (`gnoblin.extensions.xr`). The listeners hear it begin, and the stock room is built: a
//!   node named `room` under the tree's root with the `head` camera and one `view` for each window, placed on a ring (`room.rs`);
//! - every frame of the session the controllers are read, the room follows the windows and the focus, and the drags of the room layout's
//!   bindings run (`drags.rs`);
//! - when the seat goes the session ends: the drags are cancelled and give their claims back, the stock room goes, and the listeners
//!   hear it. The windows were never moved.
//!
//! The library watches the seats from the moment a shell opens its object protocol (`object::object_api`), as the TypeScript library
//! started its XR runtime when the package was loaded, so the stock shell builds the room without a line of its own. Until the
//! program's seat comes (or a shell asks for `gnoblin.extensions`) that watch is all there is (`watch`); then the library follows XR for
//! the generation (`start`). The turn is posted when a seat, a controller, a window or the focus changes, every frame (`FRAME_MS`) while
//! a session runs, and when a chord's settle time is up.
//!
//! `data/js/gnoblin-std/extensions/xr/` and `data/js/gnoblin-std/host/xr_bridge.ts` were the reference. One rule differs: a dialog is a
//! window of its own and gets a panel of its own, anchored to its parent's by `parent_window`; nothing assumes a window lies inside
//! another window's node.
#![allow(unsafe_code)]

mod classes;
mod controller;
mod drags;
mod layout;
mod ring;
mod room;
mod trigger;

pub(crate) use classes::register;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

use gnoblin_core::Timers;
use gnoblin_core::host::{Host, LogLevel, NodeId, Subscription};
use gnoblin_core_abi::{self as abi, GnoblinCtx, GnoblinNodes};
use gnoblin_object::server::{Cx, ShellImpl};
use gnoblin_object::{Closure, Fail, Obj, Res, Val};

use self::controller::ControllerData;
use self::drags::DragRun;
use self::room::RoomData;
use crate::animation::staging::Prop;
use crate::math::transform::PoseMatrix;
use crate::math::vector::Point3;
use crate::object::with_lib;
use crate::scene::Kind;
use crate::scene::registry::{NodeHandle, SceneWorld};

/// How often a running session asks for a turn, in milliseconds.
const FRAME_MS: f64 = 16.0;

/// The longest settle time a chord may have, in milliseconds. The program holds a press back from the window for 150 ms
/// (`xr_input::CLAIM_HOLD`) to wait for a claim; the claim is made after the settle time and needs about as long again to travel shell,
/// compositor and program, so a settle time past 60 would claim after the hold.
pub(crate) const MAX_CHORD_SETTLE_MS: f64 = 60.0;

/// The properties of a controller the program writes. `claimed` is the shell's own, so a write of it asks for no turn.
const CONTROLLER_KEYS: [&str; 13] = [
    "hand",
    "tracked",
    "x",
    "y",
    "z",
    "rotation",
    "trigger",
    "grip",
    "thumbstick_x",
    "thumbstick_y",
    "thumbstick_click",
    "primary",
    "secondary",
];

/// The properties of a window the room follows.
const WINDOW_KEYS: [&str; 10] = [
    "width",
    "height",
    "x",
    "y",
    "minimised",
    "secure",
    "parent_window",
    "title",
    "app_id",
    "visible",
];

thread_local! {
    /// The XR of each generation of this thread.
    static WORLDS: RefCell<HashMap<GnoblinCtx, Rc<XrWorld>>> = RefCell::new(HashMap::new());
    /// The generations whose seats the library watches for the program's, by their handle on the core.
    static WATCHED: RefCell<HashMap<GnoblinCtx, Host>> = RefCell::new(HashMap::new());
}

/// A listener of the sessions.
#[derive(Clone)]
pub(crate) struct HeldListener {
    pub(crate) obj: Obj,
    pub(crate) shell: Rc<ShellImpl>,
}

/// One session: the seat the program made.
pub(crate) struct SessionData {
    /// The session's object, held by the runtime while the session runs.
    pub(crate) obj: Cell<Obj>,
    pub(crate) seat: u64,
    pub(crate) alive: Cell<bool>,
    /// The chord settle time, in milliseconds.
    pub(crate) settle: Cell<f64>,
    /// The head's object.
    pub(crate) head: Cell<Obj>,
    /// The viewport a shell assigned, held.
    pub(crate) viewport: Cell<Obj>,
    /// The room the headset shows: the stock one, or the room of the viewport a shell assigned.
    pub(crate) room: Cell<Obj>,
    pub(crate) controllers: RefCell<Vec<Rc<ControllerData>>>,
    /// The core's input context of the seat, held by one reference, once looked up.
    pub(crate) input: Cell<Obj>,
}

/// What the stock shell builds for a session.
#[derive(Default)]
pub(crate) struct Options {
    /// No room of the library's.
    pub(crate) no_room: bool,
    /// The stock room's layout, held.
    pub(crate) layout: Option<Obj>,
    pub(crate) accept: Option<Closure>,
}

/// What runs from turn to turn.
#[derive(Default)]
pub(crate) struct Runtime {
    pub(crate) options: Options,
    pub(crate) listeners: Vec<HeldListener>,
    pub(crate) session: Option<Rc<SessionData>>,
    /// The extension's object while there is a session, held.
    pub(crate) ext: Obj,
    /// The room the library built.
    pub(crate) stock: Obj,
    pub(crate) runs: Vec<DragRun>,
    /// Controllers whose chord found no drag: they start none until their buttons are let go.
    pub(crate) declined: BTreeSet<u64>,
    /// The chord each controller holds and since when.
    pub(crate) since: HashMap<u64, (Vec<usize>, f64)>,
}

/// The XR of one generation.
pub(crate) struct XrWorld {
    pub(crate) ctx: GnoblinCtx,
    pub(crate) g: Host,
    pub(crate) scene: Rc<SceneWorld>,
    pub(crate) rt: RefCell<Runtime>,
    /// The rooms the library set up, by node.
    pub(crate) rooms: RefCell<HashMap<u64, Rc<RoomData>>>,
    /// The head each `xr_camera3d` follows, by node, held.
    pub(crate) tracking: RefCell<HashMap<u64, Obj>>,
    /// The object every turn runs as: the `extensions` service.
    anchor: Cell<Obj>,
    pending: Cell<bool>,
    timer: RefCell<Option<Subscription>>,
    clock: RefCell<Option<Rc<Timers>>>,
    /// The generation ended.
    ended: Cell<bool>,
}

/// The XR of the generation `ctx`, if the library follows one.
pub(crate) fn find(ctx: GnoblinCtx) -> Option<Rc<XrWorld>> {
    WORLDS.with(|worlds| worlds.borrow().get(&ctx).cloned())
}

fn core_fail(error: gnoblin_core::host::CoreError) -> Fail {
    Fail::new(error.kind().code(), error.to_string())
}

/// Watches the seats of the generation `ctx` for the program's, once, and starts following XR when it is there. Until it comes the
/// library holds three subscriptions and no state of the room's, so a shell with no XR program pays next to nothing for it.
///
/// # Errors
///
/// The core refuses the attach or a subscription.
pub(crate) fn watch(ctx: GnoblinCtx) -> Res<()> {
    let g = watch_seats(ctx)?;
    if find(ctx).is_none() && xr_seat(&g).is_some() {
        start(ctx)?;
    }
    Ok(())
}

/// Subscribes to the seats of the generation `ctx`, once: a seat that comes, goes or is renamed may start or end a session.
fn watch_seats(ctx: GnoblinCtx) -> Res<Host> {
    if let Some(g) = WATCHED.with(|watched| watched.borrow().get(&ctx).cloned()) {
        return Ok(g);
    }
    let g = gnoblin_core::private::gnoblin_for(ctx).map_err(core_fail)?;
    g.attach().map_err(core_fail)?;
    g.on_shutdown(move || {
        WATCHED.with(|watched| watched.borrow_mut().remove(&ctx));
    })
    .map_err(core_fail)?;
    g.on_node_created("seat", move |_| seat_changed(ctx))
        .map_err(core_fail)?;
    g.on_node_destroyed("seat", move |_| seat_changed(ctx))
        .map_err(core_fail)?;
    let _ = g.on_class_property_changed("seat", "name", move |_, _| seat_changed(ctx));
    WATCHED.with(|watched| watched.borrow_mut().insert(ctx, g.clone()));
    Ok(g)
}

/// A seat came, went or was renamed: a turn of a world that follows XR, or the start of one when the program's seat is there. The start
/// is posted, so the core's subscriptions are not changed from inside its own callback.
fn seat_changed(ctx: GnoblinCtx) {
    if let Some(world) = find(ctx) {
        world.schedule();
        return;
    }
    let Some(g) = WATCHED.with(|watched| watched.borrow().get(&ctx).cloned()) else {
        return;
    };
    if xr_seat(&g).is_some() {
        let _ = g.poster().post(move |g| {
            if let Err(fail) = watch(ctx) {
                g.log_at(
                    LogLevel::Warn,
                    &format!("gnoblin-std: xr: the session could not be followed: {}", fail.message),
                );
            }
        });
    }
}

/// Starts following XR for the generation `ctx`, once: the program's seat is there, or a shell asked for `gnoblin.extensions`.
///
/// # Errors
///
/// The core refuses the attach or a subscription.
pub(crate) fn start(ctx: GnoblinCtx) -> Res<Rc<XrWorld>> {
    if let Some(world) = find(ctx) {
        return Ok(world);
    }
    watch_seats(ctx)?;
    let scene = crate::scene::registry::world_for(ctx)?;
    let world = Rc::new(XrWorld {
        ctx,
        g: scene.g.clone(),
        scene,
        rt: RefCell::new(Runtime::default()),
        rooms: RefCell::new(HashMap::new()),
        tracking: RefCell::new(HashMap::new()),
        anchor: Cell::new(0),
        pending: Cell::new(false),
        timer: RefCell::new(None),
        clock: RefCell::new(None),
        ended: Cell::new(false),
    });
    WORLDS.with(|worlds| worlds.borrow_mut().insert(ctx, Rc::clone(&world)));
    if let Err(fail) = subscribe(&world) {
        WORLDS.with(|worlds| worlds.borrow_mut().remove(&ctx));
        return Err(fail);
    }
    if xr_seat(&world.g).is_some() {
        world.schedule();
    }
    Ok(world)
}

/// The XR of the generation a call is for.
pub(crate) fn world_of(cx: &Cx<'_>) -> Res<Rc<XrWorld>> {
    start(cx.ctx())
}

/// The nodes of the tree class `class`, published or the shell's, in the order they came: unparented ones (the program's seat and
/// controllers) too.
pub(crate) fn nodes_of(g: &Host, class: &str) -> Vec<u64> {
    let Ok(class) = g.class(class) else {
        return Vec::new();
    };
    let api = gnoblin_core::private::api_of(g);
    let mut nodes = GnoblinNodes {
        ptr: core::ptr::null(),
        len: 0,
    };
    // SAFETY: `class_nodes` fills `nodes`, a live local; the ids it lends are copied before the next call into the core.
    let code = unsafe { (api.class_nodes)(gnoblin_core::private::ctx_of(g), class.0, &raw mut nodes) };
    if code != abi::E_OK || nodes.ptr.is_null() {
        return Vec::new();
    }
    // SAFETY: the core lends `len` ids at `ptr` until the next call into it on this thread.
    unsafe { std::slice::from_raw_parts(nodes.ptr, nodes.len) }.to_vec()
}

/// The camera the program writes the head pose onto: the one named `head`.
pub(crate) fn head_camera(world: &XrWorld) -> Option<u64> {
    nodes_of(&world.g, "camera")
        .into_iter()
        .find(|node| matches!(world.read(*node, "name"), Some(Prop::Text(name)) if name == "head"))
}

/// The seat of the program's session: the seat named `xr`.
fn xr_seat(g: &Host) -> Option<u64> {
    nodes_of(g, "seat").into_iter().find(|seat| {
        let node = g.node(NodeId(*seat));
        node.is_alive() && node.get_text("name").is_ok_and(|name| name == "xr")
    })
}

/// What a world follows besides the seats (`watch_seats`).
fn subscribe(world: &Rc<XrWorld>) -> Res<()> {
    let ctx = world.ctx;
    let g = &world.g;
    g.on_shutdown(move || shutdown(ctx)).map_err(core_fail)?;
    // The keyboard moved: the ring turns with the focus.
    let _ = g.on_class_property_changed("seat", "focus", move |_, _| ask(ctx));
    // A compositor older than the controller class has none to follow.
    let _ = g.on_node_created("controller", move |_| ask(ctx));
    let _ = g.on_node_destroyed("controller", move |_| ask(ctx));
    for key in CONTROLLER_KEYS {
        let _ = g.on_class_property_changed("controller", key, move |_, _| ask(ctx));
    }
    for key in ["layer_budget", "metres", "name"] {
        let _ = g.on_class_property_changed("camera", key, move |_, _| ask(ctx));
    }
    g.on_node_created("window", move |_| ask(ctx)).map_err(core_fail)?;
    g.on_node_destroyed("window", move |_| ask(ctx)).map_err(core_fail)?;
    for key in WINDOW_KEYS {
        let _ = g.on_class_property_changed("window", key, move |_, _| ask(ctx));
    }
    Ok(())
}

/// A turn, whatever runs.
fn wake(ctx: GnoblinCtx) {
    if let Some(world) = find(ctx) {
        world.schedule();
    }
}

/// Something the room or the drags follow changed: a turn, while there is a session.
fn ask(ctx: GnoblinCtx) {
    if let Some(world) = find(ctx)
        && world.rt.borrow().session.is_some()
    {
        world.schedule();
    }
}

/// The generation ends: every claim it made (its drags' and its scripts') is given back, and the writes leave at once, since a
/// generation that ends has no turn of its own to send them in. Nothing more is done.
fn shutdown(ctx: GnoblinCtx) {
    let Some(world) = find(ctx) else {
        return;
    };
    world.ended.set(true);
    world.rt.borrow_mut().runs.clear();
    let session = world.rt.borrow().session.clone();
    if let Some(session) = session {
        for controller in session.controllers.borrow().iter() {
            let _ = controller.release_all(&world.g);
        }
        let _ = world.g.flush(|_| {});
    }
    if let Some(timer) = world.timer.borrow_mut().take() {
        let _ = timer.unsubscribe();
    }
    WORLDS.with(|worlds| worlds.borrow_mut().remove(&ctx));
}

impl XrWorld {
    /// Writes a line to the generation's log.
    pub(crate) fn warn(&self, message: &str) {
        #[cfg(any(test, feature = "testing"))]
        crate::ui::testing::note(message);
        self.g.log_at(LogLevel::Warn, &format!("gnoblin-std: {message}"));
    }

    /// Asks for a turn, once however many changes there are.
    pub(crate) fn schedule(&self) {
        if self.ended.get() || self.pending.replace(true) {
            return;
        }
        let ctx = self.ctx;
        if self.g.poster().post(move |_g| turn(ctx)).is_err() {
            self.pending.set(false);
        }
    }

    /// The core's clock, in milliseconds: the one its timers run by, and so the one a test's clock moves.
    pub(crate) fn now(&self) -> f64 {
        let held = self.clock.borrow().clone();
        let timers = match held {
            Some(timers) => timers,
            None => match self.g.objects().ok().and_then(|objects| Timers::get(&objects).ok()) {
                Some(timers) => {
                    let timers = Rc::new(timers);
                    *self.clock.borrow_mut() = Some(Rc::clone(&timers));
                    timers
                }
                None => return crate::ui::update::now_ms(),
            },
        };
        timers.now().unwrap_or_else(|_| crate::ui::update::now_ms())
    }

    /// The object every turn runs as: the `extensions` service, made on the first turn.
    fn anchor(&self) -> Res<Obj> {
        let held = self.anchor.get();
        if held != 0 {
            return Ok(held);
        }
        let ctx = self.ctx;
        let obj = with_lib(|lib| {
            let class = lib.class_id("extensions")?;
            lib.create(ctx, class, Vec::new())
        })?;
        self.anchor.set(obj);
        Ok(obj)
    }

    /// Records the `extensions` service as the object a turn runs as.
    pub(crate) fn set_anchor(&self, obj: Obj) {
        if self.anchor.get() == 0 {
            self.anchor.set(obj);
        }
    }

    /// Runs `work` as a call about the anchor, from outside any call.
    fn run<R>(&self, work: impl FnOnce(&mut Cx<'_>) -> R) -> Res<R> {
        let anchor = self.anchor()?;
        with_lib(|lib| lib.with_cx(self.ctx, anchor, work))
    }

    // ---- the scene, as the room uses it ---------------------------------------------------------------------------------------

    /// A property of a node, as a transition's staging leaves it.
    pub(crate) fn read(&self, node: u64, key: &str) -> Option<Prop> {
        crate::scene::read_through(&self.g, node, key)
    }

    pub(crate) fn number(&self, node: u64, key: &str) -> f64 {
        match self.read(node, key) {
            Some(Prop::Number(value)) => value,
            _ => 0.0,
        }
    }

    /// Writes a property of a scene node the library made, checked; one that holds the value already is not written.
    pub(crate) fn put(&self, node: u64, key: &str, prop: Prop) -> Res<()> {
        self.scene
            .state
            .borrow_mut()
            .set(&self.g, node, key, prop)
            .map_err(Fail::from)
    }

    /// A node's own placement.
    pub(crate) fn pose(&self, node: u64) -> PoseMatrix {
        self.scene.pose_at(node).unwrap_or_else(|_| PoseMatrix::identity())
    }

    /// A node's placement composed with the scene nodes above it: in the room's placement frame for a panel.
    pub(crate) fn world_pose(&self, node: u64) -> PoseMatrix {
        let mut pose = self.pose(node);
        let mut at = node;
        while let Some((parent, _)) = self.scene.parent_of(at) {
            pose = self.pose(parent).multiply(&pose);
            at = parent;
        }
        pose
    }

    /// Writes a placement: the position, the rotation and one scale on every axis.
    pub(crate) fn place(&self, node: u64, pose: &PoseMatrix) -> Res<()> {
        let Point3 { x, y, z } = pose.position;
        self.put(node, "x", Prop::Number(x))?;
        self.put(node, "y", Prop::Number(y))?;
        self.put(node, "z", Prop::Number(z))?;
        self.put(node, "rotation", Prop::Triple(pose.rotation()))?;
        self.put(node, "scale", Prop::Triple([pose.scale; 3]))
    }

    /// Puts the node at the pose `target` in the room's placement frame: the node is in its parent's space, so the pose goes back
    /// through the parents.
    pub(crate) fn place_world(&self, node: u64, target: &PoseMatrix) -> Res<()> {
        let above = match self.scene.parent_of(node) {
            Some((parent, _)) => self.world_pose(parent),
            None => PoseMatrix::identity(),
        };
        self.place(node, &above.inverse().multiply(target))
    }

    /// Makes a scene node of `kind` held by an object of `class`, which the caller parents or hands out.
    pub(crate) fn spawn_node(&self, cx: &Cx<'_>, class: &str, kind: Kind) -> Res<(u64, Obj)> {
        let node = self.scene.make(kind)?;
        let obj = cx.spawn(
            class,
            NodeHandle {
                world: Rc::clone(&self.scene),
                node,
                root: false,
            },
        )?;
        self.scene.adopt(node, obj, kind, false);
        Ok((node, obj))
    }

    /// Ends an object and what it holds.
    pub(crate) fn destroy(&self, obj: Obj) {
        let ctx = self.ctx;
        let _ = with_lib(|lib| lib.destroy(ctx, obj));
    }

    /// The room an object is.
    pub(crate) fn room_by_obj(&self, obj: Obj) -> Option<Rc<RoomData>> {
        if obj == 0 {
            return None;
        }
        self.rooms.borrow().values().find(|room| room.obj == obj).cloned()
    }

    // ---- the runtime ----------------------------------------------------------------------------------------------------------

    /// Whether the session declared XR (`GNOBLIN_XR=1`), so a shell may use it before the headset's session has started.
    fn declared() -> bool {
        std::env::var("GNOBLIN_XR").is_ok_and(|value| value == "1")
    }

    /// Whether the extension can be used now: the session declared XR, or the program has a session.
    pub(crate) fn available(&self) -> bool {
        XrWorld::declared() || self.rt.borrow().session.is_some() || xr_seat(&self.g).is_some()
    }

    /// One turn: follows the seat, then keeps the room and the drags current.
    fn frame(&self, cx: &mut Cx<'_>) {
        // A room whose object ended is forgotten, and so is the layout it held.
        let gone: Vec<Rc<RoomData>> = {
            let mut rooms = self.rooms.borrow_mut();
            let ended: Vec<u64> = rooms
                .keys()
                .copied()
                .filter(|node| self.scene.entry(*node).is_none())
                .collect();
            ended.into_iter().filter_map(|node| rooms.remove(&node)).collect()
        };
        for room in gone {
            if let Some((layout, _)) = room.layout.borrow_mut().take() {
                cx.unhold(layout);
            }
        }
        let seat = xr_seat(&self.g);
        let current = self.rt.borrow().session.as_ref().map(|session| session.seat);
        if current.is_some() && current != seat {
            self.end(cx);
        }
        if let Some(seat) = seat
            && self.rt.borrow().session.is_none()
        {
            self.begin(cx, seat);
        }
        let Some(session) = self.rt.borrow().session.clone() else {
            return;
        };
        self.sync_controllers(cx, &session);
        match self.room_by_obj(session.room.get()) {
            Some(room) if self.scene.alive(room.node) => {
                room::reconcile(self, cx, &room);
                drags::run(self, cx, &session, &room);
            }
            _ => drags::cancel_all(self, cx),
        }
    }

    /// A session begins: its objects are made, the listeners hear it, and the stock room is built.
    fn begin(&self, cx: &mut Cx<'_>, seat: u64) {
        let session = Rc::new(SessionData {
            obj: Cell::new(0),
            seat,
            alive: Cell::new(true),
            settle: Cell::new(50.0),
            head: Cell::new(0),
            viewport: Cell::new(0),
            room: Cell::new(0),
            controllers: RefCell::new(Vec::new()),
            input: Cell::new(0),
        });
        let made = (|| -> Res<Obj> {
            let obj = cx.spawn("xr_session", classes::SessionHandle(Rc::clone(&session)))?;
            cx.hold(obj)?;
            session.obj.set(obj);
            let head = cx.spawn("xr_tracked_pose", classes::PoseHandle)?;
            cx.set_parent(head, obj)?;
            session.head.set(head);
            let ext = cx.spawn("xr_extension", classes::ExtHandle(Rc::clone(&session)))?;
            cx.hold(ext)?;
            Ok(ext)
        })();
        let ext = match made {
            Ok(ext) => ext,
            Err(fail) => {
                self.warn(&format!("xr: the session could not start: {}", fail.message));
                return;
            }
        };
        {
            let mut rt = self.rt.borrow_mut();
            rt.session = Some(Rc::clone(&session));
            rt.ext = ext;
        }
        self.tell(cx, "on_session_started", &session);
        self.build_stock(cx, &session);
    }

    /// The stock room, unless the shell said not to or a room is in the tree already (the shell's).
    fn build_stock(&self, cx: &mut Cx<'_>, session: &Rc<SessionData>) {
        let (no_room, layout, accept, built) = {
            let rt = self.rt.borrow();
            (
                rt.options.no_room,
                rt.options.layout,
                rt.options.accept.clone(),
                rt.stock != 0,
            )
        };
        if no_room || built || !session.alive.get() {
            return;
        }
        let taken = nodes_of(&self.g, "node")
            .into_iter()
            .any(|node| matches!(self.read(node, "name"), Some(Prop::Text(name)) if name == "room"));
        if taken {
            return;
        }
        let layout = match layout {
            Some(obj) => match cx.with_state(obj, |held: &mut classes::LayoutHandle| Rc::clone(&held.0)) {
                Ok(layout) => Some((obj, layout)),
                Err(_) => None,
            },
            None => None,
        };
        match room::build(self, cx, layout, accept) {
            Ok(room) => {
                self.rt.borrow_mut().stock = room.obj;
                if let Err(fail) = classes::assign_stock(self, cx, session, &room) {
                    self.warn(&format!("xr: the stock room could not be shown: {}", fail.message));
                }
            }
            Err(fail) => self.warn(&format!("xr: the stock room could not be built: {}", fail.message)),
        }
    }

    /// The session ends: its drags are cancelled, the stock room goes, and the listeners hear it.
    fn end(&self, cx: &mut Cx<'_>) {
        drags::cancel_all(self, cx);
        let Some(session) = self.rt.borrow_mut().session.take() else {
            return;
        };
        session.alive.set(false);
        self.drop_stock();
        let controllers = std::mem::take(&mut *session.controllers.borrow_mut());
        for held in controllers {
            cx.emit_on(
                session.obj.get(),
                "controller_removed",
                vec![cx.object_value(held.obj.get())],
            );
            cx.unhold(held.obj.get());
        }
        let ext = {
            let mut rt = self.rt.borrow_mut();
            rt.declined.clear();
            rt.since.clear();
            std::mem::replace(&mut rt.ext, 0)
        };
        if ext != 0 {
            cx.unhold(ext);
        }
        let viewport = session.viewport.replace(0);
        if viewport != 0 {
            cx.unhold(viewport);
        }
        let input = session.input.replace(0);
        if input != 0
            && let Ok(core) = crate::object::core_api(self.ctx)
        {
            let _ = core.release(input);
        }
        self.tell(cx, "on_session_ended", &session);
        cx.unhold(session.obj.get());
    }

    /// Takes the stock room down.
    pub(crate) fn drop_stock(&self) {
        let stock = std::mem::replace(&mut self.rt.borrow_mut().stock, 0);
        if stock == 0 {
            return;
        }
        if let Some(session) = self.rt.borrow().session.clone()
            && session.room.get() == stock
        {
            session.room.set(0);
        }
        self.destroy(stock);
    }

    /// Tells every listener `method` with the session. A listener that throws is reported, and the next is still told.
    fn tell(&self, cx: &mut Cx<'_>, method: &str, session: &SessionData) {
        let listeners = self.rt.borrow().listeners.clone();
        let Ok(member) = cx
            .class_named("xr_listener")
            .and_then(|class| cx.member_of(class, abi::MEMBER_METHOD, method))
        else {
            return;
        };
        for listener in listeners {
            if !listener.shell.has(member) {
                continue;
            }
            let Ok(value) = cx.handout(session.obj.get()) else {
                return;
            };
            if let Err(fail) = listener.shell.call(member, &[value]) {
                self.warn(&format!("XRListener.{} threw: {}", camel(method), fail.message));
            }
        }
    }

    /// Brings the session's controllers up to date with the controller nodes, and reads each once.
    fn sync_controllers(&self, cx: &mut Cx<'_>, session: &SessionData) {
        let alive: Vec<u64> = nodes_of(&self.g, "controller")
            .into_iter()
            .filter(|node| self.g.node(NodeId(*node)).is_alive())
            .collect();
        let gone: Vec<Rc<ControllerData>> = {
            let mut held = session.controllers.borrow_mut();
            let (keep, gone): (Vec<_>, Vec<_>) = held.drain(..).partition(|data| alive.contains(&data.node));
            *held = keep;
            gone
        };
        for data in gone {
            cx.emit_on(
                session.obj.get(),
                "controller_removed",
                vec![cx.object_value(data.obj.get())],
            );
            cx.unhold(data.obj.get());
        }
        for node in alive {
            if session.controllers.borrow().iter().any(|data| data.node == node) {
                continue;
            }
            let data = Rc::new(ControllerData::new(&self.g, node));
            let made = cx
                .spawn("xr_controller", classes::ControllerHandle(Rc::clone(&data)))
                .and_then(|obj| cx.hold(obj).map(|()| obj));
            match made {
                Ok(obj) => {
                    data.obj.set(obj);
                    session.controllers.borrow_mut().push(data);
                    cx.emit_on(session.obj.get(), "controller_added", vec![cx.object_value(obj)]);
                }
                Err(fail) => self.warn(&format!("xr: a controller could not be followed: {}", fail.message)),
            }
        }
        for data in session.controllers.borrow().iter() {
            data.update(&self.g);
        }
    }

    /// Plans the next turn: every frame while a session runs, and when a chord's settle time is up.
    fn plan(&self) {
        if let Some(old) = self.timer.borrow_mut().take() {
            let _ = old.unsubscribe();
        }
        if self.ended.get() {
            return;
        }
        let Some(session) = self.rt.borrow().session.clone() else {
            return;
        };
        let due = drags::settle_due(self, &session).map_or(FRAME_MS, |left| left.min(FRAME_MS));
        let ctx = self.ctx;
        let delay = due.ceil().max(1.0) as u32;
        match self.g.after(delay, move || wake(ctx)) {
            Ok(timer) => *self.timer.borrow_mut() = Some(timer),
            Err(error) => self.warn(&format!("xr: the frame timer could not be set: {error}")),
        }
    }

    /// The core's input context of the session's seat, held once looked up.
    pub(crate) fn input_context(&self, session: &SessionData) -> Option<Obj> {
        let held = session.input.get();
        if held != 0 {
            return Some(held);
        }
        let core = crate::object::core_api(self.ctx).ok()?;
        let found = (|| -> Res<Option<Obj>> {
            let gnoblin = core.class("gnoblin")?;
            let root = core.create(gnoblin, &[])?;
            let input = core
                .member(gnoblin, abi::MEMBER_PROPERTY, "input")
                .and_then(|member| core.get(root, member))
                .and_then(|value| value.object());
            let _ = core.release(root);
            let input = input?;
            let contexts = core
                .member(core.class_of(input)?, abi::MEMBER_PROPERTY, "contexts")
                .and_then(|member| core.get(input, member));
            let _ = core.release(input);
            let seat = core.member(core.class("input_context")?, abi::MEMBER_PROPERTY, "seat")?;
            let mut found = None;
            for item in contexts?.list()? {
                let obj = item.object()?;
                if found.is_none() && core.get(obj, seat).and_then(|value| value.uint()).ok() == Some(session.seat) {
                    found = Some(obj);
                } else {
                    let _ = core.release(obj);
                }
            }
            Ok(found)
        })();
        let obj = found.ok().flatten()?;
        session.input.set(obj);
        Some(obj)
    }
}

/// A method name as a script spells it: `on_session_started` is `onSessionStarted`.
pub(crate) fn camel(name: &str) -> String {
    let mut out = String::new();
    let mut upper = false;
    for ch in name.chars() {
        if ch == '_' {
            upper = true;
        } else if upper {
            out.extend(ch.to_uppercase());
            upper = false;
        } else {
            out.push(ch);
        }
    }
    out
}

/// The turn the library posted.
fn turn(ctx: GnoblinCtx) {
    let Some(world) = find(ctx) else {
        return;
    };
    world.pending.set(false);
    if let Err(fail) = world.run(|cx| world.frame(cx)) {
        world.warn(&format!("xr: the turn did not run: {}", fail.message));
    }
    world.plan();
}

/// The value that hands an object to the shell, or none for none.
pub(crate) fn handout_or_none(cx: &Cx<'_>, obj: Obj) -> Res<Val> {
    if obj == 0 { Ok(Val::None) } else { cx.handout(obj) }
}
