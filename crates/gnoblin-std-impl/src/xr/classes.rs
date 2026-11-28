//! The classes of `xr.toml`, over the runtime of `mod.rs`: the extension and its session, the controllers and the head, the room, its
//! camera and viewport, the spatial layout and the bindings of its drags.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gnoblin_core::host::NodeId;
use gnoblin_object::server::{Builder, Cx, Inits};
use gnoblin_object::{ErrorCode, Fail, Obj, Res, Val};

use super::controller::{ClaimOwner, ControllerData, inputs_of};
use super::layout::{
    BindingCollection, BindingData, BindingHandler, DragTracking, LayoutData, NativeDrag, SPACING, Spacing, Triggers,
};
use super::ring::{self, RingParams};
use super::room::{self, DEFAULT_METRES, RoomData};
use super::trigger::{self, BUTTONS};
use super::{HeldListener, SessionData, XrWorld, handout_or_none, world_of};
use crate::animation::staging::Prop;
use crate::math::{pose_val, vec3_val};
use crate::scene::Kind;
use crate::scene::node::{camera_members, node_class_with};

/// The places of `extension_status`, counted from one.
const READY: u32 = 1;
const UNLOADED: u32 = 2;

/// The places of `xr_composition`.
const PROJECTION: u32 = 3;

fn enumeration(value: u32) -> Val {
    Val::Enum { class: 0, value }
}

fn record(fields: Vec<Val>) -> Val {
    Val::Record { class: 0, fields }
}

/// The state of an `xr_session`.
pub(crate) struct SessionHandle(pub(crate) Rc<SessionData>);
/// The state of an `xr_tracked_pose`: none, as the head is the `head` camera of the generation.
pub(crate) struct PoseHandle;
/// The state of an `xr_extension`: the session it serves.
pub(crate) struct ExtHandle(pub(crate) Rc<SessionData>);
/// The state of an `xr_controller`.
pub(crate) struct ControllerHandle(pub(crate) Rc<ControllerData>);
/// The state of a `spatial_layout`.
pub(crate) struct LayoutHandle(pub(crate) Rc<LayoutData>);
/// The state of an `xr_bindings`.
pub(crate) struct CollectionHandle(pub(crate) Rc<BindingCollection>);
/// The state of an `xr_drag_binding` and the classes that extend it.
pub(crate) struct BindingHandle(pub(crate) Rc<BindingData>);
/// The state of an `xr_viewport`.
pub(crate) struct ViewportHandle(pub(crate) Rc<ViewportData>);

/// A viewport: the room the headset shows and the camera it is seen through.
pub(crate) struct ViewportData {
    pub(crate) content: (u64, Obj),
    pub(crate) camera: Cell<(u64, Obj)>,
    pub(crate) composition: Cell<u32>,
}

// ---- helpers --------------------------------------------------------------------------------------------------------------------

/// The scene node an object of the scene holds, checked to be of `kind` when one is named.
fn scene_node(world: &XrWorld, obj: Obj, kind: Option<Kind>, what: &str) -> Res<u64> {
    let found = world
        .scene
        .entries
        .borrow()
        .iter()
        .find(|(_, entry)| entry.obj == obj)
        .map(|(node, entry)| (*node, entry.kind));
    match found {
        Some((node, held)) if kind.is_none_or(|kind| kind == held) => Ok(node),
        _ => Err(Fail::type_error(what.to_owned())),
    }
}

fn layout_of(cx: &Cx<'_>, obj: Obj) -> Res<Rc<LayoutData>> {
    cx.with_state(obj, |held: &mut LayoutHandle| Rc::clone(&held.0))
}

fn binding_of(cx: &Cx<'_>, obj: Obj) -> Res<Rc<BindingData>> {
    cx.with_state(obj, |held: &mut BindingHandle| Rc::clone(&held.0))
}

fn viewport_of(cx: &Cx<'_>, obj: Obj) -> Res<Rc<ViewportData>> {
    cx.with_state(obj, |held: &mut ViewportHandle| Rc::clone(&held.0))
}

/// The room a scene node object is, if the library set one up on it.
fn room_of(world: &XrWorld, node: u64) -> Res<Rc<RoomData>> {
    world
        .rooms
        .borrow()
        .get(&node)
        .cloned()
        .ok_or_else(|| Fail::stale(format!("XRRoom#{node}: the room is gone")))
}

/// A number above 0, or a refusal that names it.
fn above_zero(owner: &str, name: &str, value: f64) -> Res<f64> {
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(Fail::type_error(format!(
            "{owner}: {name} must be above 0, got {value}"
        )))
    }
}

/// A list of objects, each handed to the shell.
fn objects(cx: &Cx<'_>, list: impl IntoIterator<Item = Obj>) -> Res<Val> {
    list.into_iter()
        .map(|obj| cx.handout(obj))
        .collect::<Res<Vec<_>>>()
        .map(Val::List)
}

/// The script's spelling of a property, for a refusal.
fn camel(name: &str) -> String {
    super::camel(name)
}

// ---- the camera -----------------------------------------------------------------------------------------------------------------

/// Makes `node` the stereo head camera: named `head`, `metres` a unit, and `near` and `far` given in metres.
///
/// # Errors
///
/// A value is not a finite number, `metres` is not above 0, or a write is refused.
pub(crate) fn camera_setup(world: &XrWorld, node: u64, metres: f64, near: Option<f64>, far: Option<f64>) -> Res<()> {
    let metres = above_zero("XRCamera3D", "metres", metres)?;
    world.put(node, "name", Prop::Text("head".to_owned()))?;
    world.put(node, "projection", Prop::Text("stereo".to_owned()))?;
    if metres != DEFAULT_METRES {
        world.put(node, "metres", Prop::Number(metres))?;
    }
    if let Some(near) = near {
        world.put(node, "near", Prop::Number(near / metres))?;
    }
    if let Some(far) = far {
        world.put(node, "far", Prop::Number(far / metres))?;
    }
    Ok(())
}

fn reference_space(value: &str) -> Res<()> {
    if value == "local" {
        Ok(())
    } else {
        Err(Fail::type_error(format!(
            "XRCamera3D: referenceSpace \"{value}\" is not tracked; only \"local\" is"
        )))
    }
}

fn register_camera(builder: &mut Builder) {
    let camera = node_class_with(builder, "xr_camera3d", Kind::Camera3D, |h, cx, inits| {
        let world = world_of(cx)?;
        if let Some(space) = inits.take("reference_space") {
            reference_space(space.text()?)?;
        }
        let metres = inits.take("metres").map(|value| value.double()).transpose()?;
        let near = inits.take("near").map(|value| value.double()).transpose()?;
        let far = inits.take("far").map(|value| value.double()).transpose()?;
        camera_setup(&world, h.node, metres.unwrap_or(DEFAULT_METRES), near, far)?;
        if let Some(tracking) = inits.take("tracking") {
            let obj = tracking.object()?;
            cx.hold(obj)?;
            world.tracking.borrow_mut().insert(h.node, obj);
        }
        Ok(())
    });
    let _ = camera_members(camera)
        .get("tracking", |h, cx| {
            let world = world_of(cx)?;
            let held = world.tracking.borrow().get(&h.node).copied().unwrap_or(0);
            handout_or_none(cx, held)
        })
        .set_ref("tracking", |h, cx, value| {
            let world = world_of(cx)?;
            let obj = if value.is_none() { None } else { Some(value.object()?) };
            if let Some(obj) = obj {
                cx.hold(obj)?;
            }
            let old = match obj {
                Some(obj) => world.tracking.borrow_mut().insert(h.node, obj),
                None => world.tracking.borrow_mut().remove(&h.node),
            };
            if let Some(old) = old {
                cx.unhold(old);
            }
            Ok(())
        })
        .get("reference_space", |_, _| Ok(Val::Text("local".to_owned())))
        .set_ref("reference_space", |_, _, value| reference_space(value.text()?));
}

// ---- the room and the viewport --------------------------------------------------------------------------------------------------

fn register_room(builder: &mut Builder) {
    let room = node_class_with(builder, "xr_room", Kind::Node, |h, cx, inits| {
        let world = world_of(cx)?;
        let layout = match inits.take("layout") {
            Some(value) => {
                let obj = value.object()?;
                Some((obj, layout_of(cx, obj)?))
            }
            None => None,
        };
        let accept = match inits.take("accept") {
            Some(value) => Some(Rc::clone(value.closure()?)),
            None => None,
        };
        room::setup(&world, cx, h.node, cx.me(), layout, accept).map(drop)
    });
    let _ = room
        .get("head", |h, cx| {
            let room = room_of(&*world_of(cx)?, h.node)?;
            cx.handout(room.head.get().1)
        })
        .get("layout", |h, cx| {
            let room = room_of(&*world_of(cx)?, h.node)?;
            let held = room.layout.borrow().as_ref().map_or(0, |(obj, _)| *obj);
            handout_or_none(cx, held)
        })
        .set_ref("layout", |h, cx, value| {
            let room = room_of(&*world_of(cx)?, h.node)?;
            let layout = if value.is_none() {
                None
            } else {
                let obj = value.object()?;
                let layout = layout_of(cx, obj)?;
                cx.hold(obj)?;
                Some((obj, layout))
            };
            if let Some((old, _)) = room.layout.replace(layout) {
                cx.unhold(old);
            }
            Ok(())
        })
        .get("accept", |h, cx| {
            let room = room_of(&*world_of(cx)?, h.node)?;
            Ok(room.accept.borrow().clone().map_or(Val::None, Val::Closure))
        })
        .set_ref("accept", |h, cx, value| {
            let room = room_of(&*world_of(cx)?, h.node)?;
            let accept = if value.is_none() {
                None
            } else {
                Some(Rc::clone(value.closure()?))
            };
            *room.accept.borrow_mut() = accept;
            Ok(())
        })
        .get("unit", |h, cx| {
            let world = world_of(cx)?;
            let room = room_of(&world, h.node)?;
            Ok(Val::Double(room::unit(&world, &room)))
        })
        .get("views", |h, cx| {
            let room = room_of(&*world_of(cx)?, h.node)?;
            let panels: Vec<Obj> = room.panels.borrow().iter().map(|panel| panel.obj).collect();
            objects(cx, panels)
        })
        .method_ref("view_of", |h, cx, args| {
            let room = room_of(&*world_of(cx)?, h.node)?;
            let client = crate::object::client::client_node(cx.ctx(), "view_of", &args[0])?;
            handout_or_none(cx, room.panel_of(client).map_or(0, |panel| panel.obj))
        })
        .method_ref("pin", |h, cx, args| {
            let world = world_of(cx)?;
            let room = room_of(&world, h.node)?;
            room.pin(scene_node(
                &world,
                args[0].object()?,
                Some(Kind::View),
                "XRRoom.pin: expected a View",
            )?);
            Ok(Val::None)
        })
        .method_ref("unpin", |h, cx, args| {
            let world = world_of(cx)?;
            let room = room_of(&world, h.node)?;
            room.unpin(scene_node(
                &world,
                args[0].object()?,
                Some(Kind::View),
                "XRRoom.unpin: expected a View",
            )?);
            world.schedule();
            Ok(Val::None)
        })
        .method_ref("is_pinned", |h, cx, args| {
            let world = world_of(cx)?;
            let room = room_of(&world, h.node)?;
            let view = scene_node(
                &world,
                args[0].object()?,
                Some(Kind::View),
                "XRRoom.isPinned: expected a View",
            )?;
            Ok(Val::Bool(room.is_pinned(view)))
        })
        .method_ref("pin_to_head", |h, cx, args| {
            let world = world_of(cx)?;
            let room = room_of(&world, h.node)?;
            let client = crate::object::client::client_node(cx.ctx(), "pin_to_head", &args[0])?;
            let options = args.get(1).filter(|value| !value.is_none());
            let field = |at: usize, default: Option<f64>| -> Res<Option<f64>> {
                match options
                    .map(|options| options.record())
                    .transpose()?
                    .and_then(|fields| fields.get(at))
                {
                    None | Some(Val::None) => Ok(default),
                    Some(value) => value.double().map(Some),
                }
            };
            let offset = [
                field(0, Some(0.0))?.unwrap_or_default(),
                field(1, Some(0.0))?.unwrap_or_default(),
                field(2, Some(-1.2))?.unwrap_or(-1.2),
            ];
            let panel = room::pin_to_head(&world, cx, &room, client, offset, field(3, None)?)?;
            cx.handout(panel.obj)
        });

    let _ = builder
        .class::<ViewportHandle>("xr_viewport")
        .ctor(|cx, inits| {
            let world = world_of(cx)?;
            let content = inits
                .take("content")
                .ok_or_else(|| Fail::type_error("XRViewport: content must be the room node"))?
                .object()?;
            let content_node = scene_node(&world, content, None, "XRViewport: content must be the room node")?;
            let camera = inits
                .take("camera")
                .ok_or_else(|| Fail::type_error("XRViewport: camera must be a camera"))?
                .object()?;
            let camera_node = scene_node(
                &world,
                camera,
                Some(Kind::Camera3D),
                "XRViewport: camera must be a camera",
            )?;
            let composition = match inits.take("composition") {
                Some(value) => value.enum_value()?,
                None => 1,
            };
            Ok(ViewportHandle(Rc::new(ViewportData {
                content: (content_node, content),
                camera: Cell::new((camera_node, camera)),
                composition: Cell::new(composition),
            })))
        })
        .get("content", |h, cx| cx.handout(h.0.content.1))
        .set_ref("content", |_, _, _| {
            Err(Fail::type_error(
                "XRViewport.content: the room is fixed when the viewport is made",
            ))
        })
        .get("camera", |h, cx| cx.handout(h.0.camera.get().1))
        .set_ref("camera", |h, cx, value| {
            let world = world_of(cx)?;
            let obj = value.object()?;
            let node = scene_node(&world, obj, Some(Kind::Camera3D), "XRViewport.camera: must be a camera")?;
            h.0.camera.set((node, obj));
            Ok(())
        })
        .get("composition", |h, _| Ok(enumeration(h.0.composition.get())))
        .set_ref("composition", |h, _, value| {
            h.0.composition.set(value.enum_value()?);
            Ok(())
        });
}

/// Names the viewport's room and camera for the program: the room `room` (mounted under the tree's root when it hangs from nothing), the
/// camera `head` under it, which becomes the room's head (the room's own steps aside, and goes when nothing hangs from it). With
/// `projection` composition every panel of the room draws through the head camera.
///
/// # Errors
///
/// A node is gone, or a write is refused.
pub(crate) fn apply(world: &XrWorld, cx: &Cx<'_>, viewport: &ViewportData) -> Res<()> {
    let (content, content_obj) = viewport.content;
    let (camera, camera_obj) = viewport.camera.get();
    if !world.scene.alive(content) || !world.scene.alive(camera) {
        return Err(Fail::stale("XRViewport: its room or its camera was destroyed"));
    }
    if world.g.node(NodeId(content)).parent().ok().flatten().is_none() {
        let root = world.scene.root()?;
        world.scene.insert(content, root, None)?;
    }
    world.put(content, "name", Prop::Text("room".to_owned()))?;
    let room = world.rooms.borrow().get(&content).cloned();
    if let Some(room) = &room {
        let (old, old_obj) = room.head.get();
        if old != camera {
            room.head.set((camera, camera_obj));
            if world.scene.children_of(old).is_empty() {
                world.destroy(old_obj);
            } else {
                world.put(old, "name", Prop::Text(String::new()))?;
            }
        }
    }
    world.put(camera, "name", Prop::Text("head".to_owned()))?;
    if world.scene.parent_of(camera).map(|(parent, _)| parent) != Some(content) {
        world.scene.insert(camera, content, Some(0))?;
        cx.set_parent(camera_obj, content_obj)?;
    }
    let projection = viewport.composition.get() == PROJECTION;
    if let Some(room) = &room {
        room.projection.set(projection);
    }
    let through = if projection { camera } else { 0 };
    for (child, _) in world.scene.children_of(content) {
        if world.scene.entry(child).is_some_and(|entry| entry.kind == Kind::View) {
            world.put(child, "camera", Prop::Node(through))?;
        }
    }
    world.schedule();
    Ok(())
}

/// Makes `viewport` (the object `obj`) what the session shows: a room other than the stock one replaces it.
///
/// # Errors
///
/// The session ended, or [`apply`] fails.
pub(crate) fn assign(
    world: &XrWorld,
    cx: &Cx<'_>,
    session: &SessionData,
    obj: Obj,
    viewport: &ViewportData,
) -> Res<()> {
    if !session.alive.get() {
        return Err(Fail::refused(
            ErrorCode::InvalidState,
            "XRSession.viewport: the session has ended",
        ));
    }
    let stock = world.rt.borrow().stock;
    if stock != 0 && viewport.content.1 != stock {
        // A script's own room replaces the stock one.
        world.drop_stock();
    }
    apply(world, cx, viewport)?;
    if world.rooms.borrow().contains_key(&viewport.content.0) {
        session.room.set(viewport.content.1);
    }
    cx.hold(obj)?;
    let old = session.viewport.replace(obj);
    if old != 0 {
        cx.unhold(old);
    }
    Ok(())
}

/// Shows the stock room `room` through its own head camera.
///
/// # Errors
///
/// As [`assign`].
pub(crate) fn assign_stock(world: &XrWorld, cx: &Cx<'_>, session: &SessionData, room: &RoomData) -> Res<()> {
    let viewport = Rc::new(ViewportData {
        content: (room.node, room.obj),
        camera: Cell::new(room.head.get()),
        composition: Cell::new(1),
    });
    let obj = cx.spawn("xr_viewport", ViewportHandle(Rc::clone(&viewport)))?;
    assign(world, cx, session, obj, &viewport)
}

// ---- the spatial layout and its bindings ------------------------------------------------------------------------------------------

/// The `spatial_triggers` record.
fn triggers_val(triggers: &Triggers) -> Val {
    record(
        triggers
            .each()
            .iter()
            .map(|(_, text)| Val::Text((*text).to_owned()))
            .collect(),
    )
}

/// The triggers a `spatial_triggers` record holds; a field left out is the default's.
fn triggers_of(value: &Val) -> Res<Triggers> {
    let fields = value.record()?;
    let defaults = Triggers::default();
    let at = |index: usize, default: &str| -> Res<String> {
        match fields.get(index) {
            None | Some(Val::None) => Ok(default.to_owned()),
            Some(value) => value.text().map(str::to_owned),
        }
    };
    let triggers = Triggers {
        move_: at(0, &defaults.move_)?,
        resize: at(1, &defaults.resize)?,
        rotate: at(2, &defaults.rotate)?,
    };
    triggers.check()?;
    Ok(triggers)
}

/// Makes a binding object of `class`.
fn make_binding(cx: &Cx<'_>, class: &str, native: NativeDrag, id: &str, text: &str) -> Res<(Obj, Rc<BindingData>)> {
    let parsed = trigger::parse(text, "trigger")?;
    let data = Rc::new(BindingData {
        obj: Cell::new(0),
        id: RefCell::new(id.to_owned()),
        trigger: RefCell::new(text.to_owned()),
        parsed: RefCell::new(parsed),
        enabled: Cell::new(true),
        priority: Cell::new(0.0),
        handler: RefCell::new(None),
        native: RefCell::new(native),
    });
    let obj = cx.spawn(class, BindingHandle(Rc::clone(&data)))?;
    data.obj.set(obj);
    Ok((obj, data))
}

/// Puts the stock bindings of `triggers` in `collection`.
fn add_stock(cx: &Cx<'_>, collection: &BindingCollection, triggers: &Triggers) -> Res<()> {
    for (id, text) in triggers.each() {
        if text.is_empty() {
            continue;
        }
        let (class, native) = match id {
            "move" => (
                "view_move_binding",
                NativeDrag::Move {
                    tracking: DragTracking::Controller,
                    keep: false,
                },
            ),
            "resize" => (
                "view_resize_binding",
                NativeDrag::Resize {
                    pixels_per_metre: 1200.0,
                    min_width: 0.25,
                    min_height: 0.15,
                    edges: 0,
                },
            ),
            _ => ("view_rotate_binding", NativeDrag::Rotate),
        };
        let (obj, data) = make_binding(cx, class, native, id, text)?;
        collection.admits(&data)?;
        cx.hold(obj)?;
        collection.items.borrow_mut().push(data);
    }
    Ok(())
}

/// Takes the stock bindings out of `collection`.
fn remove_stock(cx: &Cx<'_>, collection: &BindingCollection) {
    let removed: Vec<Rc<BindingData>> = {
        let mut items = collection.items.borrow_mut();
        let (gone, keep): (Vec<_>, Vec<_>) = items
            .drain(..)
            .partition(|item| matches!(item.id.borrow().as_str(), "move" | "resize" | "rotate"));
        *items = keep;
        gone
    };
    for item in removed {
        cx.unhold(item.obj.get());
    }
}

/// A layout the library makes: the default ring and the stock bindings, unless told otherwise. The caller holds it.
///
/// # Errors
///
/// An object could not be made.
pub(crate) fn new_layout(
    world: &XrWorld,
    cx: &Cx<'_>,
    spacing: Spacing,
    triggers: Triggers,
) -> Res<(Obj, Rc<LayoutData>)> {
    let _ = world;
    let collection = Rc::new(BindingCollection::default());
    let collection_obj = cx.spawn("xr_bindings", CollectionHandle(Rc::clone(&collection)))?;
    let layout = Rc::new(LayoutData {
        obj: Cell::new(0),
        spacing: Cell::new(spacing),
        triggers: RefCell::new(triggers.clone()),
        bindings_obj: collection_obj,
        bindings: Rc::clone(&collection),
    });
    let obj = cx.spawn("spatial_layout", LayoutHandle(Rc::clone(&layout)))?;
    layout.obj.set(obj);
    cx.set_parent(collection_obj, obj)?;
    add_stock(cx, &collection, &triggers)?;
    Ok((obj, layout))
}

fn register_layout(builder: &mut Builder) {
    let mut layout = builder.class::<LayoutHandle>("spatial_layout").ctor(|cx, inits| {
        let mut spacing = Spacing::default();
        for name in SPACING {
            if let Some(value) = inits.take(name) {
                spacing.set(name, value.double()?)?;
            }
        }
        let triggers = match inits.take("triggers") {
            Some(value) => triggers_of(&value)?,
            None => Triggers::default(),
        };
        let collection = Rc::new(BindingCollection::default());
        let collection_obj = cx.spawn("xr_bindings", CollectionHandle(Rc::clone(&collection)))?;
        cx.set_parent(collection_obj, cx.me())?;
        add_stock(cx, &collection, &triggers)?;
        Ok(LayoutHandle(Rc::new(LayoutData {
            obj: Cell::new(cx.me()),
            spacing: Cell::new(spacing),
            triggers: RefCell::new(triggers),
            bindings_obj: collection_obj,
            bindings: collection,
        })))
    });
    for name in SPACING {
        layout = layout
            .get(name, move |h, _| Ok(Val::Double(h.0.spacing.get().get(name))))
            .set_ref(name, move |h, cx, value| {
                let mut spacing = h.0.spacing.get();
                spacing.set(name, value.double()?).map_err(|fail| {
                    let message = fail.message.replace(name, &camel(name));
                    fail.with_message(message)
                })?;
                h.0.spacing.set(spacing);
                world_of(cx)?.schedule();
                Ok(())
            });
    }
    let _ = layout
        .get("triggers", |h, _| Ok(triggers_val(&h.0.triggers.borrow())))
        .set_ref("triggers", |h, cx, value| {
            let triggers = triggers_of(&value)?;
            remove_stock(cx, &h.0.bindings);
            add_stock(cx, &h.0.bindings, &triggers)?;
            *h.0.triggers.borrow_mut() = triggers;
            Ok(())
        })
        .get("ring", |h, _| Ok(h.0.spacing.get().ring().val()))
        .get("bindings", |h, cx| cx.handout(h.0.bindings_obj))
        .method_ref("arrange", |h, _, args| {
            arrange(&args[0], args.get(1), &h.0.spacing.get().ring())
        })
        .function("arrange_ring", |_, args| {
            let ring = match args.get(2) {
                None | Some(Val::None) => RingParams::default(),
                Some(value) => RingParams::of(value)?,
            };
            arrange(&args[0], args.get(1), &ring)
        });

    let _ = builder
        .class::<CollectionHandle>("xr_bindings")
        .ctor(|_, _| {
            Err(Fail::refused(
                ErrorCode::Unsupported,
                "a spatial layout makes its own bindings collection",
            ))
        })
        .get("length", |h, _| Ok(Val::Uint(h.0.items.borrow().len() as u64)))
        .get("list", |h, cx| {
            let list: Vec<Obj> = h.0.items.borrow().iter().map(|item| item.obj.get()).collect();
            objects(cx, list)
        })
        .method_ref("add", |h, cx, args| {
            let obj = args[0].object()?;
            let binding = binding_of(cx, obj)?;
            h.0.admits(&binding)?;
            cx.hold(obj)?;
            h.0.items.borrow_mut().push(binding);
            world_of(cx)?.schedule();
            Ok(Val::None)
        })
        .method_ref("remove", |h, cx, args| {
            let id = args[0].text()?;
            let removed = {
                let mut items = h.0.items.borrow_mut();
                items
                    .iter()
                    .position(|item| *item.id.borrow() == id)
                    .map(|at| items.remove(at))
            };
            let Some(removed) = removed else {
                return Ok(Val::Bool(false));
            };
            cx.unhold(removed.obj.get());
            world_of(cx)?.schedule();
            Ok(Val::Bool(true))
        })
        .method_ref("find", |h, cx, args| {
            let found = h.0.get(args[0].text()?).map_or(0, |item| item.obj.get());
            handout_or_none(cx, found)
        })
        .method_ref("has", |h, _, args| Ok(Val::Bool(h.0.get(args[0].text()?).is_some())))
        .method_ref("clear", |h, cx, _| {
            let items = std::mem::take(&mut *h.0.items.borrow_mut());
            for item in items {
                cx.unhold(item.obj.get());
            }
            world_of(cx)?.schedule();
            Ok(Val::None)
        });

    for (class, native) in [
        ("xr_drag_binding", NativeDrag::Plain),
        (
            "view_move_binding",
            NativeDrag::Move {
                tracking: DragTracking::Controller,
                keep: false,
            },
        ),
        (
            "view_resize_binding",
            NativeDrag::Resize {
                pixels_per_metre: 1200.0,
                min_width: 0.25,
                min_height: 0.15,
                edges: 0,
            },
        ),
        ("view_rotate_binding", NativeDrag::Rotate),
        ("view_scale_binding", NativeDrag::Scale { min: 0.2, max: 3.0 }),
    ] {
        register_binding(builder, class, native);
    }
}

/// `arrange` and `arrange_ring`: the `ring_view` records and the focus, placed on `ring`.
fn arrange(views: &Val, focus: Option<&Val>, ring: &RingParams) -> Res<Val> {
    let widths: Vec<f64> = views
        .list()?
        .iter()
        .map(|view| match view.record()?.first() {
            None | Some(Val::None) => Ok(0.0),
            Some(width) => width.double(),
        })
        .collect::<Res<_>>()?;
    let focus = match focus {
        None | Some(Val::None) => None,
        Some(value) => usize::try_from(value.int()?).ok(),
    };
    Ok(ring::arrange(&widths, focus, ring).val())
}

/// The options of a binding's own drag, from the constructor's inits.
fn native_of(native: &mut NativeDrag, inits: &mut Inits) -> Res<()> {
    match native {
        NativeDrag::Plain | NativeDrag::Rotate => {}
        NativeDrag::Move { tracking, keep } => {
            if let Some(value) = inits.take("tracking") {
                *tracking = tracking_of(&value)?;
            }
            if let Some(value) = inits.take("rotation") {
                *keep = value.enum_value()? == 2;
            }
            if let Some(value) = inits.take("distance") {
                distance(value.text()?)?;
            }
        }
        NativeDrag::Resize {
            pixels_per_metre,
            min_width,
            min_height,
            edges,
        } => {
            for (name, field) in [
                ("pixels_per_metre", &mut *pixels_per_metre),
                ("min_width", &mut *min_width),
                ("min_height", &mut *min_height),
            ] {
                if let Some(value) = inits.take(name) {
                    *field = above_zero("ViewResizeBinding", &camel(name), value.double()?)?;
                }
            }
            if let Some(value) = inits.take("edges") {
                *edges = value.enum_value()? - 1;
            }
        }
        NativeDrag::Scale { min, max } => {
            if let Some(value) = inits.take("min_scale") {
                *min = above_zero("ViewScaleBinding", "minScale", value.double()?)?;
            }
            if let Some(value) = inits.take("max_scale") {
                *max = above_zero("ViewScaleBinding", "maxScale", value.double()?)?;
            }
            scale_limits(*min, *max)?;
        }
    }
    Ok(())
}

fn tracking_of(value: &Val) -> Res<DragTracking> {
    Ok(if value.enum_value()? == 2 {
        DragTracking::Ray
    } else {
        DragTracking::Controller
    })
}

fn distance(value: &str) -> Res<()> {
    if value == "preserve" {
        Ok(())
    } else {
        Err(Fail::type_error(format!(
            "ViewMoveBinding: distance \"{value}\" must be \"preserve\""
        )))
    }
}

fn scale_limits(min: f64, max: f64) -> Res<()> {
    if min > max {
        return Err(Fail::type_error(format!(
            "ViewScaleBinding: maxScale must be above or equal to minScale ({max} is below {min})"
        )));
    }
    Ok(())
}

/// Sets the handler of a binding to the shell's object `obj`, or takes it away for none.
fn set_handler(cx: &Cx<'_>, binding: &BindingData, obj: Option<Obj>) -> Res<()> {
    let handler = match obj {
        Some(obj) => {
            let shell = cx.shell(obj)?;
            cx.hold(obj)?;
            Some(BindingHandler { obj, shell })
        }
        None => None,
    };
    if let Some(old) = binding.handler.replace(handler) {
        cx.unhold(old.obj);
    }
    Ok(())
}

fn register_binding(builder: &mut Builder, class: &'static str, native: NativeDrag) {
    let made = builder.class::<BindingHandle>(class).ctor(move |cx, inits| {
        let mut native = native.clone();
        native_of(&mut native, inits)?;
        let (default_id, default_trigger) = native.defaults();
        let id = match inits.take("id") {
            Some(value) => value.text()?.to_owned(),
            None => String::new(),
        };
        let text = match inits.take("trigger") {
            Some(value) => value.text()?.to_owned(),
            None => String::new(),
        };
        let id = if id.is_empty() { default_id.to_owned() } else { id };
        let text = if text.is_empty() {
            default_trigger.to_owned()
        } else {
            text
        };
        let what = format!("{}: trigger", native.label());
        let parsed = trigger::parse(&text, &what)?;
        let data = Rc::new(BindingData {
            obj: Cell::new(cx.me()),
            id: RefCell::new(id),
            trigger: RefCell::new(text),
            parsed: RefCell::new(parsed),
            enabled: Cell::new(true),
            priority: Cell::new(0.0),
            handler: RefCell::new(None),
            native: RefCell::new(native),
        });
        if let Some(value) = inits.take("enabled") {
            data.enabled.set(value.bool()?);
        }
        if let Some(value) = inits.take("priority") {
            data.priority.set(value.double()?);
        }
        if let Some(value) = inits.take("handler") {
            set_handler(cx, &data, Some(value.object()?))?;
        }
        Ok(BindingHandle(data))
    });
    let made = made
        .get("id", |h, _| Ok(Val::Text(h.0.id.borrow().clone())))
        .set_ref("id", |h, _, value| {
            let id = value.text()?;
            if id.is_empty() {
                return Err(Fail::type_error("a binding's id must not be empty"));
            }
            *h.0.id.borrow_mut() = id.to_owned();
            Ok(())
        })
        .get("trigger", |h, _| Ok(Val::Text(h.0.trigger.borrow().clone())))
        .set_ref("trigger", |h, _, value| h.0.set_trigger(value.text()?))
        .get("parsed", |h, _| Ok(h.0.parsed.borrow().val()))
        .get("enabled", |h, _| Ok(Val::Bool(h.0.enabled.get())))
        .set_ref("enabled", |h, cx, value| {
            h.0.enabled.set(value.bool()?);
            world_of(cx)?.schedule();
            Ok(())
        })
        .get("priority", |h, _| Ok(Val::Double(h.0.priority.get())))
        .set_ref("priority", |h, _, value| {
            h.0.priority.set(value.double()?);
            Ok(())
        })
        .get("handler", |h, cx| {
            let held = h.0.handler.borrow().as_ref().map_or(0, |handler| handler.obj);
            handout_or_none(cx, held)
        })
        .set_ref("handler", |h, cx, value| {
            let obj = if value.is_none() { None } else { Some(value.object()?) };
            set_handler(cx, &h.0, obj)
        })
        .method_ref("claims", |h, _, _| {
            Ok(Val::List(
                h.0.parsed.borrow().inputs().into_iter().map(Val::Text).collect(),
            ))
        })
        .function("parse_trigger", |_, args| {
            Ok(trigger::parse(args[0].text()?, "trigger")?.val())
        });
    match class {
        "view_move_binding" => {
            let _ = made
                .get("tracking", |h, _| {
                    Ok(enumeration(match &*h.0.native.borrow() {
                        NativeDrag::Move {
                            tracking: DragTracking::Ray,
                            ..
                        } => 2,
                        _ => 1,
                    }))
                })
                .set_ref("tracking", |h, _, value| {
                    if let NativeDrag::Move { tracking, .. } = &mut *h.0.native.borrow_mut() {
                        *tracking = tracking_of(&value)?;
                    }
                    Ok(())
                })
                .get("rotation", |h, _| {
                    Ok(enumeration(match &*h.0.native.borrow() {
                        NativeDrag::Move { keep: true, .. } => 2,
                        _ => 1,
                    }))
                })
                .set_ref("rotation", |h, _, value| {
                    let keep = value.enum_value()? == 2;
                    if let NativeDrag::Move { keep: held, .. } = &mut *h.0.native.borrow_mut() {
                        *held = keep;
                    }
                    Ok(())
                })
                .get("distance", |_, _| Ok(Val::Text("preserve".to_owned())))
                .set_ref("distance", |_, _, value| distance(value.text()?));
        }
        "view_resize_binding" => {
            let mut made = made;
            for (at, name) in ["pixels_per_metre", "min_width", "min_height"].into_iter().enumerate() {
                made = made
                    .get(name, move |h, _| {
                        Ok(Val::Double(match &*h.0.native.borrow() {
                            NativeDrag::Resize {
                                pixels_per_metre,
                                min_width,
                                min_height,
                                ..
                            } => [*pixels_per_metre, *min_width, *min_height][at],
                            _ => 0.0,
                        }))
                    })
                    .set_ref(name, move |h, _, value| {
                        let value = above_zero("ViewResizeBinding", &camel(name), value.double()?)?;
                        if let NativeDrag::Resize {
                            pixels_per_metre,
                            min_width,
                            min_height,
                            ..
                        } = &mut *h.0.native.borrow_mut()
                        {
                            *[pixels_per_metre, min_width, min_height][at] = value;
                        }
                        Ok(())
                    });
            }
            let _ = made
                .get("edges", |h, _| {
                    Ok(enumeration(match &*h.0.native.borrow() {
                        NativeDrag::Resize { edges, .. } => edges + 1,
                        _ => 1,
                    }))
                })
                .set_ref("edges", |h, _, value| {
                    let place = value.enum_value()?;
                    if let NativeDrag::Resize { edges, .. } = &mut *h.0.native.borrow_mut() {
                        *edges = place - 1;
                    }
                    Ok(())
                });
        }
        "view_scale_binding" => {
            let _ = made
                .get("min_scale", |h, _| {
                    Ok(Val::Double(match &*h.0.native.borrow() {
                        NativeDrag::Scale { min, .. } => *min,
                        _ => 0.0,
                    }))
                })
                .set_ref("min_scale", |h, _, value| {
                    let value = above_zero("ViewScaleBinding", "minScale", value.double()?)?;
                    if let NativeDrag::Scale { min, max } = &mut *h.0.native.borrow_mut() {
                        scale_limits(value, *max)?;
                        *min = value;
                    }
                    Ok(())
                })
                .get("max_scale", |h, _| {
                    Ok(Val::Double(match &*h.0.native.borrow() {
                        NativeDrag::Scale { max, .. } => *max,
                        _ => 0.0,
                    }))
                })
                .set_ref("max_scale", |h, _, value| {
                    let value = above_zero("ViewScaleBinding", "maxScale", value.double()?)?;
                    if let NativeDrag::Scale { min, max } = &mut *h.0.native.borrow_mut() {
                        scale_limits(*min, value)?;
                        *max = value;
                    }
                    Ok(())
                });
        }
        _ => {}
    }
}

// ---- the extension, the session, the head and the controllers ---------------------------------------------------------------------

/// What a use of an extension that is not there is told.
pub(crate) fn missing(name: &str) -> String {
    format!(
        "gnoblin.extensions.{name}: the {} extension is not available. The session did not declare it (set GNOBLIN_XR=1 and install \
         gnoblin-xr, see docs/xr.md) and no XR program has a session. Guard the use with `gnoblin.extensions.{name}`, which is null \
         without one, or use it after the session starts.",
        name.to_uppercase()
    )
}

fn register_extension(builder: &mut Builder) {
    let _ = builder
        .class::<()>("extensions")
        .ctor(|cx, _| {
            world_of(cx)?.set_anchor(cx.me());
            Ok(())
        })
        .get("xr", |_, cx| {
            let world = world_of(cx)?;
            let ext = world.rt.borrow().ext;
            handout_or_none(cx, ext)
        })
        .get("list", |_, cx| {
            let world = world_of(cx)?;
            let ready = world.rt.borrow().session.is_some();
            Ok(Val::List(vec![record(vec![
                Val::Text("xr".to_owned()),
                enumeration(if ready { READY } else { UNLOADED }),
            ])]))
        })
        .method("configure", |_, cx, args| {
            let name = args[0].text()?;
            if name != "xr" {
                return Err(Fail::type_error(format!(
                    "gnoblin.extensions.configure: unknown extension \"{name}\"; expected \"xr\""
                )));
            }
            let world = world_of(cx)?;
            let fields = args[1].record()?;
            let room = match fields.first() {
                None | Some(Val::None) => true,
                Some(value) => value.bool()?,
            };
            let layout = match fields.get(1) {
                None | Some(Val::None) => None,
                Some(value) => {
                    let obj = value.object()?;
                    layout_of(cx, obj)?;
                    cx.hold(obj)?;
                    Some(obj)
                }
            };
            let accept = match args.get(2) {
                None | Some(Val::None) => None,
                Some(value) => Some(Rc::clone(value.closure()?)),
            };
            let old = {
                let mut rt = world.rt.borrow_mut();
                rt.options.no_room = !room;
                rt.options.accept = accept;
                std::mem::replace(&mut rt.options.layout, layout)
            };
            if let Some(old) = old {
                cx.unhold(old);
            }
            Ok(Val::None)
        })
        .method("require", |_, cx, args| {
            let name = args[0].text()?;
            let world = world_of(cx)?;
            if name != "xr" || !world.available() {
                return Err(Fail::refused(ErrorCode::Unavailable, missing(name)));
            }
            Ok(Val::None)
        })
        .method("add_listener", |_, cx, args| {
            let world = world_of(cx)?;
            let obj = args[0].object()?;
            if world.rt.borrow().listeners.iter().any(|held| held.obj == obj) {
                return Ok(Val::None);
            }
            let shell = cx.shell(obj)?;
            cx.hold(obj)?;
            world.rt.borrow_mut().listeners.push(HeldListener { obj, shell });
            Ok(Val::None)
        })
        .method("remove_listener", |_, cx, args| {
            let world = world_of(cx)?;
            let obj = args[0].object()?;
            let removed = {
                let mut rt = world.rt.borrow_mut();
                rt.listeners
                    .iter()
                    .position(|held| held.obj == obj)
                    .map(|at| rt.listeners.remove(at))
            };
            if let Some(held) = removed {
                cx.unhold(held.obj);
            }
            Ok(Val::None)
        });

    let _ = builder
        .class::<ExtHandle>("xr_extension")
        .ctor(|_, _| {
            Err(Fail::refused(
                ErrorCode::Unsupported,
                "the library makes the extension when a session starts",
            ))
        })
        .get("available", |_, _| Ok(Val::Bool(true)))
        .get("sessions", |h, cx| {
            let session = &h.0;
            if session.alive.get() {
                objects(cx, [session.obj.get()])
            } else {
                Ok(Val::List(Vec::new()))
            }
        })
        .get("session", |h, cx| {
            let session = &h.0;
            handout_or_none(cx, if session.alive.get() { session.obj.get() } else { 0 })
        })
        .get("room", |h, cx| {
            let room = h.0.room.get();
            let live = h.0.alive.get() && room != 0 && cx.class_of(room).is_ok();
            handout_or_none(cx, if live { room } else { 0 })
        });

    let _ = builder
        .class::<SessionHandle>("xr_session")
        .ctor(|_, _| {
            Err(Fail::refused(
                ErrorCode::Unsupported,
                "the library makes a session when the program makes its seat",
            ))
        })
        .get("id", |h, _| Ok(Val::Text(h.0.seat.to_string())))
        .get("alive", |h, _| Ok(Val::Bool(h.0.alive.get())))
        .get("connected", |h, _| Ok(Val::Bool(h.0.alive.get())))
        .get("tracking", |h, cx| {
            let world = world_of(cx)?;
            let tracked =
                h.0.controllers
                    .borrow()
                    .iter()
                    .any(|controller| controller.tracked(&world.g));
            Ok(enumeration(if tracked { 1 } else { 2 }))
        })
        .get("input", |h, cx| {
            if !h.0.alive.get() {
                return Ok(Val::None);
            }
            let world = world_of(cx)?;
            let Some(obj) = world.input_context(&h.0) else {
                return Ok(Val::None);
            };
            let value = Val::Foreign {
                namespace: "gnoblin.model".to_owned(),
                obj,
            };
            cx.hand_out(&value)?;
            Ok(value)
        })
        .get("layer_budget", |h, cx| {
            let world = world_of(cx)?;
            let budget = super::head_camera(&world).map_or(0.0, |head| world.number(head, "layer_budget"));
            let _ = &h.0;
            Ok(Val::Uint(budget as u64))
        })
        .get("viewport", |h, cx| handout_or_none(cx, h.0.viewport.get()))
        .set_ref("viewport", |h, cx, value| {
            let world = world_of(cx)?;
            let obj = value.object()?;
            let viewport = viewport_of(cx, obj)?;
            assign(&world, cx, &h.0, obj, &viewport)
        })
        .get("head", |h, cx| cx.handout(h.0.head.get()))
        .get("controllers", |h, cx| {
            let list: Vec<Obj> =
                h.0.controllers
                    .borrow()
                    .iter()
                    .map(|controller| controller.obj.get())
                    .collect();
            objects(cx, list)
        })
        .get("chord_settle_ms", |h, _| Ok(Val::Double(h.0.settle.get())))
        .set_ref("chord_settle_ms", |h, _, value| {
            let ms = value.double()?;
            if ms.is_nan() {
                return Err(Fail::type_error(
                    "session.chordSettleMs must be a number of milliseconds, got NaN",
                ));
            }
            h.0.settle.set(ms.clamp(0.0, super::MAX_CHORD_SETTLE_MS));
            Ok(())
        })
        .method_ref("controller", |h, cx, args| {
            let id = args[0].text()?;
            let found =
                h.0.controllers
                    .borrow()
                    .iter()
                    .find(|controller| controller.node.to_string() == id)
                    .map_or(0, |controller| controller.obj.get());
            handout_or_none(cx, found)
        });

    let _ = builder
        .class::<PoseHandle>("xr_tracked_pose")
        .ctor(|_, _| {
            Err(Fail::refused(
                ErrorCode::Unsupported,
                "the library makes the head with the session",
            ))
        })
        .get("available", |_, cx| {
            Ok(Val::Bool(super::head_camera(&*world_of(cx)?).is_some()))
        })
        .get("pose", |_, cx| {
            let world = world_of(cx)?;
            let Some(head) = super::head_camera(&world) else {
                return Ok(Val::None);
            };
            // The program writes the pose with the scene's y mirrored and the turns about x and z reversed (`xr_scene::head`).
            let rotation = match world.read(head, "rotation") {
                Some(Prop::Triple(angles)) => angles,
                _ => [0.0; 3],
            };
            let pose = crate::math::transform::PoseMatrix::new(
                crate::math::vector::Point3::new(
                    world.number(head, "x"),
                    -world.number(head, "y"),
                    world.number(head, "z"),
                ),
                [-rotation[0], rotation[1], -rotation[2]],
                1.0,
            );
            Ok(pose_val(&pose))
        });

    let _ = builder
        .class::<ControllerHandle>("xr_controller")
        .ctor(|_, _| {
            Err(Fail::refused(
                ErrorCode::Unsupported,
                "the library makes a controller when the program makes its node",
            ))
        })
        .get("id", |h, _| Ok(Val::Text(h.0.node.to_string())))
        .get("alive", |h, cx| Ok(Val::Bool(h.0.alive(&world_of(cx)?.g))))
        .get("hand", |h, _| Ok(Val::Text(h.0.hand.clone())))
        .get("tracked", |h, cx| Ok(Val::Bool(h.0.tracked(&world_of(cx)?.g))))
        .get("pose", |h, cx| Ok(pose_val(&h.0.pose(&world_of(cx)?.g))))
        .get("ray", |h, cx| {
            let ray = h.0.ray(&world_of(cx)?.g);
            Ok(record(vec![vec3_val(ray.origin), vec3_val(ray.direction)]))
        })
        .get("trigger", |h, cx| {
            Ok(Val::Double(h.0.number(&world_of(cx)?.g, "trigger")))
        })
        .get("grip", |h, cx| Ok(Val::Double(h.0.number(&world_of(cx)?.g, "grip"))))
        .get("thumbstick_x", |h, cx| {
            Ok(Val::Double(h.0.number(&world_of(cx)?.g, "thumbstick_x")))
        })
        .get("thumbstick_y", |h, cx| {
            Ok(Val::Double(h.0.number(&world_of(cx)?.g, "thumbstick_y")))
        })
        .get("buttons", |h, _| {
            Ok(Val::List(
                h.0.down
                    .borrow()
                    .iter()
                    .map(|at| Val::Text(BUTTONS[*at].to_owned()))
                    .collect(),
            ))
        })
        .get("claimed", |h, cx| {
            Ok(Val::List(
                h.0.claimed(&world_of(cx)?.g).into_iter().map(Val::Text).collect(),
            ))
        })
        .method_ref("claim", |h, cx, args| {
            let target = h.0.target();
            let owner = args[0].text()?;
            if owner.is_empty() {
                return Err(Fail::type_error(format!(
                    "{target}: a claim needs an owner, got an empty string"
                )));
            }
            let names: Vec<String> = args[1]
                .list()?
                .iter()
                .map(|name| name.text().map(str::to_owned))
                .collect::<Res<_>>()?;
            let inputs = inputs_of(&target, &names)?;
            h.0.claim(&world_of(cx)?.g, ClaimOwner::Script(owner.to_owned()), inputs)?;
            Ok(Val::None)
        })
        .method_ref("release", |h, cx, args| {
            let owner = args[0].text()?;
            h.0.release(&world_of(cx)?.g, &ClaimOwner::Script(owner.to_owned()))?;
            Ok(Val::None)
        });
}

/// Implements the classes of `xr.toml`.
pub(crate) fn register(builder: &mut Builder) {
    register_camera(builder);
    register_room(builder);
    register_layout(builder);
    register_extension(builder);
}
