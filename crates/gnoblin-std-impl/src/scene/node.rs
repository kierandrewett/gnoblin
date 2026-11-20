//! The classes of nodes: `scene_node`, and `camera3d` and `camera2d`, which add to it. Every one holds a [`NodeHandle`] and reads and
//! writes the node through the checked path of the scene (`SceneState`), so a write is what the TypeScript scene wrote.

use gnoblin_object::server::{Builder, ClassBuilder, Cx};
use gnoblin_object::{Fail, Res, Val};

use super::bindings::bindings_val;
use super::registry::{NodeHandle, World, world_of};
use super::{Kind, camera};
use crate::animation::staging::Prop;
use crate::math::spatial::{self, Given, Parsed};
use crate::math::transform::Pose;
use crate::math::vector::Vec3;
use crate::math::{given_of, pose_val, triple_val, vec3_of};

type Class<'a> = ClassBuilder<'a, NodeHandle>;

/// The places of `input_mode`, counted from one.
const FORWARD: u32 = 1;
const CAPTURE: u32 = 2;
const NONE: u32 = 3;

/// The places of `projection`, counted from one.
const PROJECTIONS: [&str; 3] = ["orthographic", "perspective", "stereo"];

pub(super) fn enumeration(value: u32) -> Val {
    Val::Enum { class: 0, value }
}

impl NodeHandle {
    /// How an error path names the node.
    pub(super) fn label(&self) -> String {
        if self.root {
            format!("SceneRoot#{}", self.node)
        } else {
            self.world.label(self.node)
        }
    }

    /// A property of the node: the staged value while a transition stages, the tree's otherwise.
    pub(super) fn read(&self, key: &str) -> Res<Option<Prop>> {
        self.world
            .state
            .borrow()
            .get(&self.world.g, self.node, key)
            .map_err(Fail::from)
    }

    /// A write of one property, checked.
    pub(super) fn put(&self, key: &str, prop: Prop) -> Res<()> {
        self.world
            .state
            .borrow_mut()
            .set(&self.world.g, self.node, key, prop)
            .map_err(Fail::from)?;
        self.touched();
        Ok(())
    }

    /// A property changed: what follows it gets a turn, and so does the library's own, which lets go of what a transition played.
    pub(super) fn touched(&self) {
        if self.world.follows() {
            self.world.schedule();
        }
        if let Some(std) = crate::state::find(self.world.ctx) {
            crate::turn::schedule(&std);
        }
    }

    pub(super) fn number(&self, key: &str) -> Res<f64> {
        Ok(match self.read(key)? {
            Some(Prop::Number(value)) => value,
            _ => 0.0,
        })
    }

    fn triple(&self, key: &str, default: [f64; 3]) -> Res<[f64; 3]> {
        Ok(match self.read(key)? {
            Some(Prop::Triple(value)) => value,
            _ => default,
        })
    }

    fn flag(&self, key: &str, default: bool) -> Res<bool> {
        Ok(match self.read(key)? {
            Some(Prop::Bool(value)) => value,
            _ => default,
        })
    }

    fn text(&self, key: &str) -> Res<String> {
        Ok(match self.read(key)? {
            Some(Prop::Text(value)) => value,
            _ => String::new(),
        })
    }

    /// The node's own placement.
    fn pose(&self) -> Res<Pose> {
        self.world.pose_at(self.node)
    }

    /// Writes the parts of a placement that were given.
    fn place(&self, parts: &Parsed) -> Res<()> {
        if let Some(position) = parts.position {
            self.put("position", Prop::Triple(position))?;
        }
        if let Some(rotation) = parts.rotation {
            self.put("rotation", Prop::Triple(rotation))?;
        }
        if let Some(scale) = parts.scale {
            self.put("scale", Prop::Triple(scale))?;
        }
        Ok(())
    }

    /// The placement in the space of the topmost scene ancestor.
    fn world_pose(&self) -> Res<Pose> {
        let mut pose = self.pose()?;
        let mut at = self.node;
        while let Some((parent, _)) = self.world.parent_of(at) {
            pose = self.world.pose_at(parent)?.multiply(&pose);
            at = parent;
        }
        Ok(pose)
    }

    /// The object of the node, as a value for the shell to keep.
    fn handout(&self, cx: &Cx<'_>, node: u64) -> Res<Val> {
        match self.world.entry(node) {
            Some(entry) => cx.handout(entry.obj),
            None => Ok(Val::None),
        }
    }
}

/// The node and the root flag of the object `obj`.
pub(super) fn node_of(cx: &Cx<'_>, obj: u64) -> Res<(u64, bool)> {
    cx.with_state(obj, |handle: &mut NodeHandle| (handle.node, handle.root))
}

fn mode_of(value: &Val) -> Res<&'static str> {
    Ok(match value.enum_value()? {
        FORWARD => "forward",
        CAPTURE => "capture",
        NONE => "none",
        _ => return Err(Fail::type_error("not a value of `input_mode`")),
    })
}

/// The members every node has.
fn node_members(class: Class<'_>) -> Class<'_> {
    class
        .get("id", |h, _| {
            Ok(Val::Text(
                h.world
                    .entry(h.node)
                    .and_then(|entry| entry.given)
                    .unwrap_or_else(|| h.node.to_string()),
            ))
        })
        .set("id", |h, _, value| {
            let id = value.text()?.to_owned();
            if id.is_empty() {
                return Err(Fail::type_error(format!(
                    "{}.id: must be a non-empty string",
                    h.label()
                )));
            }
            h.world.tweak(h.node, |entry| entry.given = Some(id.clone()));
            h.put("name", Prop::Text(id))
        })
        .get("name", |h, _| Ok(Val::Text(h.text("name")?)))
        .set("name", |h, _, value| {
            h.put("name", Prop::Text(value.text()?.to_owned()))
        })
        .get("x", |h, _| Ok(Val::Double(h.number("x")?)))
        .set("x", |h, _, value| h.put("x", Prop::Number(value.double()?)))
        .get("y", |h, _| Ok(Val::Double(h.number("y")?)))
        .set("y", |h, _, value| h.put("y", Prop::Number(value.double()?)))
        .get("z", |h, _| Ok(Val::Double(h.number("z")?)))
        .set("z", |h, _, value| h.put("z", Prop::Number(value.double()?)))
        .get("position", |h, _| {
            Ok(triple_val([h.number("x")?, h.number("y")?, h.number("z")?]))
        })
        .set("position", |h, _, value| {
            let place = format!("{}.position", h.label());
            let at = vec3_of(&value)?;
            // A flat camera looks at a point: its position is `x` and `y`, and its depth is not its to move.
            if h.world.entry(h.node).is_some_and(|entry| entry.kind == Kind::Camera2D) {
                let x = camera::finite(&place, "x", at.x).map_err(Fail::from)?;
                let y = camera::finite(&place, "y", at.y).map_err(Fail::from)?;
                h.put("x", Prop::Number(x))?;
                return h.put("y", Prop::Number(y));
            }
            let position = spatial::vector_checked(&place, at).map_err(Fail::from)?;
            h.put("position", Prop::Triple(position))
        })
        .get("rotation", |h, _| Ok(triple_val(h.triple("rotation", [0.0; 3])?)))
        .set("rotation", |h, _, value| {
            let place = format!("{}.rotation", h.label());
            let angles = spatial::rotation_from_angles(&place, vec3_of(&value)?).map_err(Fail::from)?;
            h.put("rotation", Prop::Triple(angles))
        })
        .get("scale", |h, _| Ok(triple_val(h.triple("scale", [1.0; 3])?)))
        .set("scale", |h, _, value| {
            let place = format!("{}.scale", h.label());
            let axes = spatial::scale_from_vector(&place, vec3_of(&value)?).map_err(Fail::from)?;
            h.put("scale", Prop::Triple(axes))
        })
        .get("transform", |h, _| Ok(pose_val(&h.pose()?)))
        .set("transform", |h, _, value| {
            let pose = crate::math::pose_of(&value)?;
            let given = Given {
                position: Some(pose.position),
                rotation: Some(Vec3::of(&pose.rotation())),
                uniform_scale: Some(pose.scale),
                ..Given::default()
            };
            let parts = spatial::transform_from(&format!("{}.transform", h.label()), &given).map_err(Fail::from)?;
            h.place(&parts)
        })
        .get("visible", |h, _| Ok(Val::Bool(h.flag("visible", true)?)))
        .set("visible", |h, _, value| h.put("visible", Prop::Bool(value.bool()?)))
        .get("opacity", |h, _| Ok(Val::Double(h.number("opacity")?)))
        .set("opacity", |h, _, value| h.put("opacity", Prop::Number(value.double()?)))
        .get("input", |h, _| {
            let mode = h.text("input")?;
            Ok(enumeration(match mode.as_str() {
                "capture" => CAPTURE,
                "none" => NONE,
                _ => FORWARD,
            }))
        })
        .set("input", |h, _, value| {
            h.put("input", Prop::Text(mode_of(&value)?.to_owned()))
        })
        .get("metres_per_unit", |h, _| {
            Ok(Val::Double(h.world.entry(h.node).map_or(1.0, |entry| entry.metres)))
        })
        .set("metres_per_unit", |h, _, value| {
            let metres = value.double()?;
            if !(metres.is_finite() && metres > 0.0) {
                return Err(Fail::type_error(format!(
                    "{}.metresPerUnit: must be a finite number above 0",
                    h.label()
                )));
            }
            h.world.tweak(h.node, |entry| entry.metres = metres);
            Ok(())
        })
        .get("node", |h, _| Ok(Val::Uint(h.node)))
        .get("alive", |h, _| Ok(Val::Bool(h.world.alive(h.node))))
        .get("parent", |h, cx| match h.world.parent_of(h.node) {
            Some((_, obj)) => cx.handout(obj),
            None => Ok(Val::None),
        })
        .get("children", |h, cx| {
            let mut list = Vec::new();
            for (node, _) in h.world.children_of(h.node) {
                list.push(h.handout(cx, node)?);
            }
            Ok(Val::List(list))
        })
        .get("world_transform", |h, _| Ok(pose_val(&h.world_pose()?)))
        .get("properties", |h, cx| bindings_val(cx, h))
        .method("add", |h, cx, args| {
            let child = args[0].object()?;
            let (child_node, _) = node_of(cx, child)?;
            if !h.world.alive(h.node) || !h.world.alive(child_node) {
                return Err(Fail::stale(format!("{}.add: the node was destroyed", h.label())));
            }
            let index = match args.get(1) {
                Some(index) if !index.is_none() => usize::try_from(index.int()?).ok(),
                _ => None,
            };
            h.world.insert(child_node, h.node, index).map_err(|fail| {
                Fail::new(
                    fail.code,
                    format!(
                        "{}.add: the shell refused to add {} ({})",
                        h.label(),
                        h.world.label(child_node),
                        fail.message
                    ),
                )
            })?;
            // The tree's root is the compositor's, and its objects stay until they end themselves.
            if !h.root {
                cx.set_parent(child, cx.me())?;
            }
            h.touched();
            cx.handout(child)
        })
        .method("find", |h, cx, args| {
            let wanted = args[0].text()?;
            match find_below(&h.world, h.node, wanted) {
                Some(node) => h.handout(cx, node),
                None => Ok(Val::None),
            }
        })
        .method("place", |h, _, args| {
            let parts = spatial::transform_from(&format!("{}.transform", h.label()), &given_of(&args[0])?)
                .map_err(Fail::from)?;
            h.place(&parts)?;
            Ok(Val::None)
        })
}

/// The first descendant of `node` the library made whose name is `name`, depth first.
pub(super) fn find_below(world: &World, node: u64, name: &str) -> Option<u64> {
    for (child, _) in world.children_of(node) {
        let named = matches!(
            super::read_through(&world.g, child, "name"),
            Some(Prop::Text(held)) if held == name
        );
        if named {
            return Some(child);
        }
        if let Some(deeper) = find_below(world, child, name) {
            return Some(deeper);
        }
    }
    None
}

/// A number the camera takes, written checked.
fn camera_number<'a>(class: Class<'a>, name: &'static str) -> Class<'a> {
    class
        .get(name, move |h, _| Ok(Val::Double(h.number(name)?)))
        .set(name, move |h, _, value| h.put(name, Prop::Number(value.double()?)))
}

/// Registers the class `name` that makes a node of `kind`, with the members of every node.
pub(super) fn node_class<'a>(builder: &'a mut Builder, name: &str, kind: Kind) -> Class<'a> {
    let made = builder.class::<NodeHandle>(name).ctor(move |cx, _inits| {
        let world = world_of(cx)?;
        let node = world.make(kind)?;
        world.adopt(node, cx.me(), kind, false);
        Ok(NodeHandle {
            world,
            node,
            root: false,
        })
    });
    node_members(made)
}

pub(super) fn register(builder: &mut Builder) {
    let _ = node_class(builder, "scene_node", Kind::Node);
    let camera = node_class(builder, "camera3d", Kind::Camera3D)
        .get("projection", |h, _| {
            let held = h.text("projection")?;
            let place = PROJECTIONS.iter().position(|name| *name == held).map_or(1, |at| at + 1);
            Ok(enumeration(place as u32))
        })
        .set("projection", |h, _, value| {
            let place = value.enum_value()? as usize;
            let name = place
                .checked_sub(1)
                .and_then(|at| PROJECTIONS.get(at))
                .ok_or_else(|| Fail::type_error("not a value of `projection`"))?;
            h.put("projection", Prop::Text((*name).to_owned()))
        })
        .get("layer_budget", |h, _| Ok(Val::Uint(h.number("layer_budget")? as u64)));
    let camera = camera_number(camera, "metres");
    let camera = camera_number(camera, "near");
    let camera = camera_number(camera, "far");
    let camera = camera_number(camera, "fov");
    let camera = camera_number(camera, "ipd");
    let _ = camera_number(camera, "size");
    let _ = node_class(builder, "camera2d", Kind::Camera2D)
        .get("zoom", |h, _| Ok(Val::Double(h.number("zoom")?)))
        .set("zoom", |h, _, value| h.put("zoom", Prop::Number(value.double()?)))
        .get("roll", |h, _| Ok(Val::Double(h.number("roll")?)))
        .set("roll", |h, _, value| h.put("roll", Prop::Number(value.double()?)))
        .get("identity", |h, _| {
            Ok(Val::Bool(
                h.number("x")? == 0.0 && h.number("y")? == 0.0 && h.number("zoom")? == 1.0 && h.number("roll")? == 0.0,
            ))
        });
}
