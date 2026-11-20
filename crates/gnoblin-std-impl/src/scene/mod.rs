//! The scene: nodes, views and cameras a shell makes in the compositor's tree, with the checks of the TypeScript scene.
//!
//! `data/js/gnoblin-std/scene/` is the reference. A scene node is a tree node the library made for the shell: the library remembers
//! what kind it made (a plain node, a view, a camera, a flat camera), so a key means what it means there (a flat camera's `zoom` is the
//! node's scale), and checks every write before it reaches the tree: a number is finite, a projection is one of the three names, a
//! zoom is above zero. A write goes through the staging of a transition when one is being staged, so a camera can be moved, zoomed and
//! rolled by a transition.
//!
//! The pieces:
//!
//! - this module: the kinds and the checked reads and writes of a node, which the std table's scene functions and the classes share;
//! - `camera`: what a camera takes of its own;
//! - `registry`: the generation's scene (its nodes and their objects, what it hears of the tree);
//! - `node`, `view`: the classes of `scene.toml` (`scene_node`, `camera3d`, `camera2d` and `view`), and `scene`, the service;
//! - `mirror`: the picture of a workspace a view keeps;
//! - `bindings`: properties of a node kept in step with what a function reads;
//! - `table`: the scene functions of the std table, for the TypeScript library that still serves them.

pub(crate) mod bindings;
pub(crate) mod camera;
mod classes;
pub(crate) mod mirror;
mod node;
pub(crate) mod registry;
pub(crate) mod table;
mod view;

use std::collections::HashMap;

use gnoblin_core::{Gnoblin, NodeId};
use gnoblin_object::server::Builder;

use crate::animation::engine::read_prop;
use crate::animation::staging::{self, Prop};
use crate::error::{Fail, Res};
use crate::state::State;

/// What kind of scene node the library made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A plain node.
    Node,
    /// A node that draws a client without moving it.
    View,
    /// A camera that projects its children in three dimensions.
    Camera3D,
    /// A flat camera: a plain node whose position, zoom and roll say where a viewport looks.
    Camera2D,
}

impl Kind {
    /// The tree class the node is made as.
    pub(crate) fn class(self) -> &'static str {
        match self {
            Kind::Node | Kind::Camera2D => "node",
            Kind::View => "view",
            Kind::Camera3D => "camera",
        }
    }

    /// The name an error path gives the node.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Kind::Node => "Node",
            Kind::View => "View",
            Kind::Camera3D => "Camera3D",
            Kind::Camera2D => "Camera2D",
        }
    }
}

/// What the library keeps of the scene nodes it made.
#[derive(Debug, Default)]
pub(crate) struct SceneState {
    pub(crate) kinds: HashMap<u64, Kind>,
    /// A camera's orthographic height, which the tree has no property for.
    pub(crate) sizes: HashMap<u64, f64>,
}

/// The staged value of a property while a transition is staging, else the tree's.
pub(crate) fn read_through(g: &Gnoblin, node: u64, key: &str) -> Option<Prop> {
    staging::peek(node, key).or_else(|| read_prop(g, node, key))
}

/// A write of one property: skipped when the node already holds the value, held when a transition is staging, else made at once.
pub(crate) fn write(g: &Gnoblin, node: u64, label: &str, key: &str, value: Prop) -> Res<()> {
    if read_through(g, node, key).as_ref() == Some(&value) {
        return Ok(());
    }
    if staging::capture(node, key, &value) {
        return Ok(());
    }
    staging::released(node, key);
    g.node(NodeId(node))
        .set(key, value.to_value())
        .map_err(|error| with_context(error, &format!("{label}.{key}: the shell refused the write")))
}

/// A failure of the core, with what the library was doing in front of its text.
fn with_context(error: gnoblin_core::CoreError, context: &str) -> Fail {
    let fail = Fail::from(error);
    Fail::new(fail.code, format!("{context} ({})", fail.message))
}

/// A number that is finite, in a property of a node.
pub(crate) fn number(prop: &Prop, label: &str, key: &str) -> Res<f64> {
    match prop {
        Prop::Number(value) => camera::finite(label, key, *value),
        _ => Err(Fail::type_error(format!("{label}.{key}: must be a finite number"))),
    }
}

/// Three finite numbers, in a property of a node.
fn triple(prop: &Prop, label: &str, key: &str) -> Res<[f64; 3]> {
    match prop {
        Prop::Triple(list) => {
            for part in list {
                camera::finite(label, key, *part)?;
            }
            Ok(*list)
        }
        _ => Err(Fail::type_error(format!("{label}.{key}: must be three numbers"))),
    }
}

impl SceneState {
    /// Makes a scene node of `kind` in the tree.
    pub(crate) fn make(&mut self, g: &Gnoblin, kind: Kind) -> Res<u64> {
        let node = g
            .create_node(kind.class())
            .map_err(|error| with_context(error, &format!("the shell could not make a {}", kind.label())))?;
        let id = node.id().0;
        self.kinds.insert(id, kind);
        Ok(id)
    }

    /// Takes a node and everything under it out of the tree, and out of what the library keeps. A node already gone is no failure.
    pub(crate) fn destroy(&mut self, g: &Gnoblin, node: u64) -> Res<()> {
        let mut all = vec![node];
        let mut at = 0;
        while at < all.len() {
            if let Ok(children) = g.node(NodeId(all[at])).children() {
                all.extend(children.iter().map(|child| child.id().0));
            }
            at += 1;
        }
        g.node(NodeId(node)).destroy()?;
        for gone in all {
            self.kinds.remove(&gone);
            self.sizes.remove(&gone);
        }
        Ok(())
    }

    /// Forgets a node that has gone (its object ended).
    pub(crate) fn forget(&mut self, node: u64) {
        self.kinds.remove(&node);
        self.sizes.remove(&node);
    }

    /// The kind of a scene node the library made, and its label for error paths.
    pub(crate) fn kind_of(&self, node: u64) -> Res<(Kind, String)> {
        let kind = *self.kinds.get(&node).ok_or_else(|| {
            Fail::new(
                gnoblin_core_abi::E_NO_SUCH_NODE,
                format!("node {node} is not a scene node"),
            )
        })?;
        Ok((kind, format!("{}#{node}", kind.label())))
    }

    /// Writes a property of a scene node, checked.
    pub(crate) fn set(&mut self, g: &Gnoblin, node: u64, key: &str, prop: Prop) -> Res<()> {
        let (kind, label) = self.kind_of(node)?;
        if !crate::nodes::alive(g, node) {
            return Err(Fail::new(
                gnoblin_core_abi::E_NO_SUCH_NODE,
                format!("{label}.{key}: the node was destroyed"),
            ));
        }
        if let Some(result) = camera::set(self, g, node, kind, &label, key, &prop) {
            return result;
        }
        match key {
            "name" => match prop {
                Prop::Text(_) => write(g, node, &label, key, prop),
                _ => Err(Fail::type_error(format!("{label}.name: must be a string"))),
            },
            "x" | "y" | "z" | "opacity" => {
                let value = number(&prop, &label, key)?;
                write(g, node, &label, key, Prop::Number(value))
            }
            "position" => {
                let [x, y, z] = triple(&prop, &label, key)?;
                for (axis, value) in [("x", x), ("y", y), ("z", z)] {
                    write(g, node, &label, axis, Prop::Number(value))?;
                }
                Ok(())
            }
            "rotation" => {
                let angles = triple(&prop, &label, key)?;
                write(g, node, &label, key, Prop::Triple(angles))
            }
            "scale" => match prop {
                Prop::Number(scale) => {
                    let scale = camera::finite(&label, key, scale)?;
                    write(g, node, &label, key, Prop::Triple([scale, scale, scale]))
                }
                other => {
                    let axes = triple(&other, &label, key)?;
                    write(g, node, &label, key, Prop::Triple(axes))
                }
            },
            "visible" => match prop {
                Prop::Bool(_) => write(g, node, &label, key, prop),
                _ => Err(Fail::type_error(format!("{label}.visible: must be a boolean"))),
            },
            "input" => match &prop {
                Prop::Text(mode) if matches!(mode.as_str(), "forward" | "capture" | "none") => {
                    if mode != "none" {
                        write(g, node, &label, "input_mode", Prop::Text(mode.clone()))?;
                    }
                    write(g, node, &label, "input", Prop::Bool(mode != "none"))
                }
                _ => Err(Fail::type_error(format!(
                    "{label}.input: must be \"forward\", \"capture\", \"none\" or a boolean"
                ))),
            },
            "width" | "height" | "fps" if kind == Kind::View => {
                let value = number(&prop, &label, key)?;
                write(g, node, &label, key, Prop::Number(value))
            }
            "source" if kind == Kind::View => match prop {
                Prop::Node(0) => write(g, node, &label, key, prop),
                // A window the core has not heard of yet (the shell's engine sees a window a moment before the core's tree does) is
                // taken on the engine's word that it is a client; a node the core knows must be a window.
                Prop::Node(id)
                    if match g.node(NodeId(id)).class_name() {
                        Ok(class) => class == "window",
                        Err(error) => error.kind() == gnoblin_core::ErrorKind::NoSuchNode,
                    } =>
                {
                    write(g, node, &label, key, prop)
                }
                _ => Err(Fail::type_error(format!("{label}.content: must be a client or null"))),
            },
            "camera" if kind == Kind::View => match prop {
                Prop::Node(0) => write(g, node, &label, key, prop),
                Prop::Node(id) if self.kinds.get(&id) == Some(&Kind::Camera3D) => write(g, node, &label, key, prop),
                _ => Err(Fail::type_error(format!("{label}.camera: must be a Camera3D or null"))),
            },
            _ => Err(Fail::no_such(format!(
                "{label}.{key}: not a property of a {}",
                kind.label()
            ))),
        }
    }

    /// Reads a property of a scene node: the staged value while a transition is staging, the library's own for what the tree lacks.
    pub(crate) fn get(&self, g: &Gnoblin, node: u64, key: &str) -> Res<Option<Prop>> {
        let (kind, _) = self.kind_of(node)?;
        if let Some(own) = camera::get(self, g, node, kind, key) {
            return Ok(Some(own));
        }
        if key == "input" {
            let on = matches!(read_through(g, node, "input"), Some(Prop::Bool(true)));
            let mode = match read_through(g, node, "input_mode") {
                Some(Prop::Text(mode)) if mode == "capture" => "capture",
                _ => "forward",
            };
            return Ok(Some(Prop::Text(if on { mode } else { "none" }.to_owned())));
        }
        Ok(read_through(g, node, key))
    }
}

impl State {
    /// Makes a scene node of `kind`.
    pub(crate) fn scene_new(&mut self, kind: Kind) -> Res<u64> {
        self.scene.make(&self.g, kind)
    }

    /// Destroys a scene node and everything under it.
    pub(crate) fn scene_destroy(&mut self, node: u64) -> Res<()> {
        self.scene.destroy(&self.g, node)
    }

    /// Writes a property of a scene node, checked.
    pub(crate) fn scene_set(&mut self, node: u64, key: &str, prop: Prop) -> Res<()> {
        self.scene.set(&self.g, node, key, prop)
    }

    /// Reads a property of a scene node.
    pub(crate) fn scene_get(&self, node: u64, key: &str) -> Res<Option<Prop>> {
        self.scene.get(&self.g, node, key)
    }
}

/// Implements the classes of the scene and of the math it uses.
pub(crate) fn register(builder: &mut Builder) {
    crate::math::register(builder);
    classes::register(builder);
}
