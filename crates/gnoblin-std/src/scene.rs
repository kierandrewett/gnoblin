//! The scene: nodes, views and cameras a shell makes in the compositor's tree.
//!
//! A [`SceneNode`] is a tree node the library made for the shell. `scene.root()` is the node scene content hangs from
//! (`root.add(&camera)`); a node that is in no parent is not drawn. A [`View`] draws a client without moving it, a [`Camera3D`] projects
//! its children in three dimensions, and a [`Camera2D`] is the flat camera of a viewport: its position is the node's `x` and `y`, its
//! zoom the node's scale and its roll the rotation about the view axis, so a [`Transition`](crate::Transition) can move, zoom and roll
//! it. A write goes through the checks of `data/js/gnoblin-std/scene/` (a number is finite, a projection is one of three names, a zoom is
//! above zero) before it reaches the tree, and through the staging of a transition when one is being staged.
//!
//! Not built: showing a workspace in a view (the TypeScript `WorkspaceMirror`), and the XR camera. A view shows a client.
#![allow(unsafe_code)]

use gnoblin_core::NodeId;
use gnoblin_std_abi as abi;

use crate::animation::PropValue;
use crate::api::Std;
use crate::client::Client;
use crate::error::{Error, ErrorKind, Result};

/// How a pointer that hits a node is handled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputMode {
    /// The pointer hits the node and, for a view of a client, goes on into the client.
    Forward,
    /// The pointer hits the node and stops there: the shell is sent the node's pointer events.
    Capture,
    /// The pointer misses the node, and its children are still tested.
    None,
}

impl InputMode {
    fn text(self) -> &'static str {
        match self {
            InputMode::Forward => "forward",
            InputMode::Capture => "capture",
            InputMode::None => "none",
        }
    }
}

/// How a camera maps its scene into an image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Projection {
    /// Flat: the default.
    Orthographic,
    /// Projects with the field of view.
    Perspective,
    /// Two eyes side by side.
    Stereo,
}

impl Projection {
    fn text(self) -> &'static str {
        match self {
            Projection::Orthographic => "orthographic",
            Projection::Perspective => "perspective",
            Projection::Stereo => "stereo",
        }
    }
}

/// A node of the scene the library made. Cheap to clone and compare (by node id).
#[derive(Clone)]
pub struct SceneNode {
    std: Std,
    id: NodeId,
}

impl std::fmt::Debug for SceneNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SceneNode").field(&self.id.0).finish()
    }
}

impl PartialEq for SceneNode {
    fn eq(&self, other: &SceneNode) -> bool {
        self.id == other.id
    }
}

impl Eq for SceneNode {}

impl SceneNode {
    fn make(std: &Std, kind: u32) -> Result<SceneNode> {
        let mut id = 0;
        // SAFETY: `scene_new` writes one node id through the pointer it is given, a live local.
        std.check(unsafe { (std.api().scene_new)(std.ctx(), kind, &mut id) })?;
        Ok(SceneNode {
            std: std.clone(),
            id: NodeId(id),
        })
    }

    /// A plain node, in no parent.
    ///
    /// # Errors
    ///
    /// The compositor cannot make the node.
    pub fn new(std: &Std) -> Result<SceneNode> {
        SceneNode::make(std, abi::STD_SCENE_NODE)
    }

    /// The node's id in the compositor's tree.
    pub fn id(&self) -> NodeId {
        self.id
    }

    fn set(&self, key: &str, value: impl Into<PropValue>) -> Result<()> {
        let value = value.into();
        let raw = value.raw();
        // SAFETY: the key and the value (and its text, borrowed from `value`) are read for the call.
        self.std
            .check(unsafe { (self.std.api().scene_set)(self.std.ctx(), self.id.0, key.as_ptr(), key.len(), &raw) })
            .map(|_| ())
    }

    fn get(&self, key: &str) -> Result<Option<PropValue>> {
        let mut raw = PropValue::Number(0.0).raw();
        // SAFETY: the key is read for the call; `scene_get` fills the struct up to the `size` the caller set, and `raw` is a live local.
        self.std.check(unsafe {
            (self.std.api().scene_get)(self.std.ctx(), self.id.0, key.as_ptr(), key.len(), &mut raw)
        })?;
        Ok(PropValue::from_raw(&raw))
    }

    fn number(&self, key: &str) -> Result<f64> {
        match self.get(key)? {
            Some(PropValue::Number(value)) => Ok(value),
            other => Err(Error::new(
                ErrorKind::Type,
                format!("`{key}` is {other:?}, not a number"),
            )),
        }
    }

    fn triple(&self, key: &str) -> Result<[f64; 3]> {
        match self.get(key)? {
            Some(PropValue::Triple(value)) => Ok(value),
            other => Err(Error::new(
                ErrorKind::Type,
                format!("`{key}` is {other:?}, not three numbers"),
            )),
        }
    }

    /// Sets the display name.
    ///
    /// # Errors
    ///
    /// The node was destroyed.
    pub fn set_name(&self, name: &str) -> Result<()> {
        self.set("name", name)
    }

    /// Sets the position in the parent's units.
    ///
    /// # Errors
    ///
    /// A coordinate is not finite, or the node was destroyed.
    pub fn set_position(&self, position: [f64; 3]) -> Result<()> {
        self.set("position", position)
    }

    /// The position in the parent's units.
    ///
    /// # Errors
    ///
    /// The node was destroyed.
    pub fn position(&self) -> Result<[f64; 3]> {
        Ok([self.number("x")?, self.number("y")?, self.number("z")?])
    }

    /// Sets the angles about X, Y and Z, in radians.
    ///
    /// # Errors
    ///
    /// An angle is not finite, or the node was destroyed.
    pub fn set_rotation(&self, angles: [f64; 3]) -> Result<()> {
        self.set("rotation", angles)
    }

    /// The angles about X, Y and Z, in radians.
    ///
    /// # Errors
    ///
    /// The node was destroyed.
    pub fn rotation(&self) -> Result<[f64; 3]> {
        self.triple("rotation")
    }

    /// Sets one scale for every axis.
    ///
    /// # Errors
    ///
    /// The scale is not finite, or the node was destroyed.
    pub fn set_scale(&self, scale: f64) -> Result<()> {
        self.set("scale", scale)
    }

    /// Sets the scale along each axis.
    ///
    /// # Errors
    ///
    /// A scale is not finite, or the node was destroyed.
    pub fn set_scale_axes(&self, scale: [f64; 3]) -> Result<()> {
        self.set("scale", scale)
    }

    /// The scale along each axis.
    ///
    /// # Errors
    ///
    /// The node was destroyed.
    pub fn scale(&self) -> Result<[f64; 3]> {
        self.triple("scale")
    }

    /// Sets whether the node is visible.
    ///
    /// # Errors
    ///
    /// The node was destroyed.
    pub fn set_visible(&self, visible: bool) -> Result<()> {
        self.set("visible", visible)
    }

    /// Sets the opacity, from 0 (transparent) to 1 (opaque).
    ///
    /// # Errors
    ///
    /// The opacity is not finite, or the node was destroyed.
    pub fn set_opacity(&self, opacity: f64) -> Result<()> {
        self.set("opacity", opacity)
    }

    /// The opacity.
    ///
    /// # Errors
    ///
    /// The node was destroyed.
    pub fn opacity(&self) -> Result<f64> {
        self.number("opacity")
    }

    /// Sets how a pointer that reaches the node is handled.
    ///
    /// # Errors
    ///
    /// The node was destroyed.
    pub fn set_input(&self, mode: InputMode) -> Result<()> {
        self.set("input", mode.text())
    }

    /// How a pointer that reaches the node is handled.
    ///
    /// # Errors
    ///
    /// The node was destroyed.
    pub fn input(&self) -> Result<InputMode> {
        Ok(match self.get("input")? {
            Some(PropValue::Text(mode)) if mode == "capture" => InputMode::Capture,
            Some(PropValue::Text(mode)) if mode == "none" => InputMode::None,
            _ => InputMode::Forward,
        })
    }

    /// Puts `child` last under this node. A child already in the tree moves.
    ///
    /// # Errors
    ///
    /// Either node is gone, or the insert would make a node its own ancestor.
    pub fn add(&self, child: &SceneNode) -> Result<()> {
        self.add_at(child, None)
    }

    /// Puts `child` under this node at `index` (`None` is last). A child already in the tree moves.
    ///
    /// # Errors
    ///
    /// As for [`SceneNode::add`].
    pub fn add_at(&self, child: &SceneNode, index: Option<usize>) -> Result<()> {
        let at = index.map_or(-1, |index| i64::try_from(index).unwrap_or(i64::MAX));
        // SAFETY: `scene_insert` takes the context, two node ids and an index.
        self.std
            .check(unsafe { (self.std.api().scene_insert)(self.std.ctx(), self.id.0, child.id.0, at) })
            .map(|_| ())
    }

    /// The scene node this one is under, or `None` for one in no parent.
    ///
    /// # Errors
    ///
    /// The node was destroyed.
    pub fn parent(&self) -> Result<Option<SceneNode>> {
        Ok(self
            .std
            .gnoblin()
            .node(self.id)
            .parent()?
            .map(|parent| SceneNode::wrap(&self.std, parent.id())))
    }

    /// The children, in tree order.
    ///
    /// # Errors
    ///
    /// The node was destroyed.
    pub fn children(&self) -> Result<Vec<SceneNode>> {
        Ok(self
            .std
            .gnoblin()
            .node(self.id)
            .children()?
            .iter()
            .map(|child| SceneNode::wrap(&self.std, child.id()))
            .collect())
    }

    fn wrap(std: &Std, id: NodeId) -> SceneNode {
        SceneNode { std: std.clone(), id }
    }

    /// Removes the node and everything under it from the tree.
    ///
    /// # Errors
    ///
    /// The node is the scene's root, which is the compositor's and cannot be destroyed.
    pub fn destroy(&self) -> Result<()> {
        // SAFETY: `scene_destroy` takes the context and a node id.
        self.std
            .check(unsafe { (self.std.api().scene_destroy)(self.std.ctx(), self.id.0) })
            .map(|_| ())
    }
}

/// The scene a shell draws.
#[derive(Clone, Debug)]
pub struct Scene {
    root: SceneNode,
}

impl Scene {
    /// The scene of a generation.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn of(std: &Std) -> Result<Scene> {
        let mut id = 0;
        // SAFETY: `scene_root` writes one node id through the pointer it is given, a live local.
        std.check(unsafe { (std.api().scene_root)(std.ctx(), &mut id) })?;
        Ok(Scene {
            root: SceneNode::wrap(std, NodeId(id)),
        })
    }

    /// The root node: `scene.root().add(&room)` mounts a room.
    pub fn root(&self) -> &SceneNode {
        &self.root
    }
}

/// The initial values of a [`Camera3D`]; a field left `None` keeps the compositor's default.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Camera3DOptions {
    /// The camera's position.
    pub position: Option<[f64; 3]>,
    /// How the camera maps its scene into an image.
    pub projection: Option<Projection>,
    /// Metres per unit of the camera's space.
    pub metres: Option<f64>,
    /// The near clipping distance in camera-space units.
    pub near: Option<f64>,
    /// The far clipping distance in camera-space units.
    pub far: Option<f64>,
    /// The vertical field of view in degrees.
    pub fov: Option<f64>,
    /// The distance between a stereo camera's eyes, in metres.
    pub ipd: Option<f64>,
    /// The visible height in scene units of an orthographic camera.
    pub size: Option<f64>,
}

/// A scene node that projects its children. A [`View`] draws content through its camera.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Camera3D {
    node: SceneNode,
}

impl std::ops::Deref for Camera3D {
    type Target = SceneNode;

    fn deref(&self) -> &SceneNode {
        &self.node
    }
}

impl Camera3D {
    /// Makes a camera and writes `options`.
    ///
    /// # Errors
    ///
    /// An option is not valid (a number that is not finite, a size at or below zero); the camera is destroyed.
    pub fn new(std: &Std, options: Camera3DOptions) -> Result<Camera3D> {
        let camera = Camera3D {
            node: SceneNode::make(std, abi::STD_SCENE_CAMERA3D)?,
        };
        let written = (|| {
            if let Some(position) = options.position {
                camera.node.set_position(position)?;
            }
            if let Some(projection) = options.projection {
                camera.set_projection(projection)?;
            }
            if let Some(metres) = options.metres {
                camera.set_metres(metres)?;
            }
            if let Some(near) = options.near {
                camera.set_near(near)?;
            }
            if let Some(far) = options.far {
                camera.set_far(far)?;
            }
            if let Some(fov) = options.fov {
                camera.set_fov(fov)?;
            }
            if let Some(ipd) = options.ipd {
                camera.set_ipd(ipd)?;
            }
            if let Some(size) = options.size {
                camera.set_size(size)?;
            }
            Ok(())
        })();
        match written {
            Ok(()) => Ok(camera),
            Err(error) => {
                let _ = camera.node.destroy();
                Err(error)
            }
        }
    }

    /// Sets the projection mode.
    ///
    /// # Errors
    ///
    /// The node was destroyed.
    pub fn set_projection(&self, projection: Projection) -> Result<()> {
        self.node.set("projection", projection.text())
    }

    /// The projection mode.
    ///
    /// # Errors
    ///
    /// The node was destroyed.
    pub fn projection(&self) -> Result<Projection> {
        Ok(match self.node.get("projection")? {
            Some(PropValue::Text(text)) if text == "perspective" => Projection::Perspective,
            Some(PropValue::Text(text)) if text == "stereo" => Projection::Stereo,
            _ => Projection::Orthographic,
        })
    }

    /// Sets the camera's metres per scene unit.
    ///
    /// # Errors
    ///
    /// The value is not finite, or the node was destroyed.
    pub fn set_metres(&self, metres: f64) -> Result<()> {
        self.node.set("metres", metres)
    }

    /// Sets the near clipping distance.
    ///
    /// # Errors
    ///
    /// As for [`Camera3D::set_metres`].
    pub fn set_near(&self, near: f64) -> Result<()> {
        self.node.set("near", near)
    }

    /// Sets the far clipping distance.
    ///
    /// # Errors
    ///
    /// As for [`Camera3D::set_metres`].
    pub fn set_far(&self, far: f64) -> Result<()> {
        self.node.set("far", far)
    }

    /// Sets the vertical field of view in degrees.
    ///
    /// # Errors
    ///
    /// As for [`Camera3D::set_metres`].
    pub fn set_fov(&self, fov: f64) -> Result<()> {
        self.node.set("fov", fov)
    }

    /// The vertical field of view in degrees.
    ///
    /// # Errors
    ///
    /// The node was destroyed.
    pub fn fov(&self) -> Result<f64> {
        self.node.number("fov")
    }

    /// Sets the distance between a stereo camera's eyes, in metres.
    ///
    /// # Errors
    ///
    /// As for [`Camera3D::set_metres`].
    pub fn set_ipd(&self, ipd: f64) -> Result<()> {
        self.node.set("ipd", ipd)
    }

    /// Sets the visible height of an orthographic camera in scene units, which the tree has no property for: the library keeps it.
    ///
    /// # Errors
    ///
    /// The size is not above zero.
    pub fn set_size(&self, size: f64) -> Result<()> {
        self.node.set("size", size)
    }

    /// The visible height of an orthographic camera; 1000 until set.
    ///
    /// # Errors
    ///
    /// The node was destroyed.
    pub fn size(&self) -> Result<f64> {
        self.node.number("size")
    }
}

/// The initial values of a [`Camera2D`]; a field left `None` keeps the default (no pan, zoom 1, no roll).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Camera2DOptions {
    /// Where the camera looks, in logical pixels of the content.
    pub center: Option<(f64, f64)>,
    /// Magnification: 1 shows the content at its size. Above 0.
    pub zoom: Option<f64>,
    /// The roll about the view axis, in radians.
    pub roll: Option<f64>,
}

/// The flat camera of a viewport: where it looks, how much it magnifies and how far it is rolled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Camera2D {
    node: SceneNode,
}

impl std::ops::Deref for Camera2D {
    type Target = SceneNode;

    fn deref(&self) -> &SceneNode {
        &self.node
    }
}

impl Camera2D {
    /// Makes a flat camera and writes `options`.
    ///
    /// # Errors
    ///
    /// An option is not valid (not finite, a zoom at or below zero); the camera is destroyed.
    pub fn new(std: &Std, options: Camera2DOptions) -> Result<Camera2D> {
        let camera = Camera2D {
            node: SceneNode::make(std, abi::STD_SCENE_CAMERA2D)?,
        };
        let written = (|| {
            if let Some((x, y)) = options.center {
                camera.set_center(x, y)?;
            }
            if let Some(zoom) = options.zoom {
                camera.set_zoom(zoom)?;
            }
            if let Some(roll) = options.roll {
                camera.set_roll(roll)?;
            }
            Ok(())
        })();
        match written {
            Ok(()) => Ok(camera),
            Err(error) => {
                let _ = camera.node.destroy();
                Err(error)
            }
        }
    }

    /// Moves the camera to look at `(x, y)`, in logical pixels of the content.
    ///
    /// # Errors
    ///
    /// A coordinate is not finite, or the node was destroyed.
    pub fn set_center(&self, x: f64, y: f64) -> Result<()> {
        self.node.set("x", x)?;
        self.node.set("y", y)
    }

    /// Where the camera looks.
    ///
    /// # Errors
    ///
    /// The node was destroyed.
    pub fn center(&self) -> Result<(f64, f64)> {
        Ok((self.node.number("x")?, self.node.number("y")?))
    }

    /// Sets the magnification, a scale factor above zero.
    ///
    /// # Errors
    ///
    /// The zoom is not finite or not above zero.
    pub fn set_zoom(&self, zoom: f64) -> Result<()> {
        self.node.set("zoom", zoom)
    }

    /// The magnification.
    ///
    /// # Errors
    ///
    /// The node was destroyed.
    pub fn zoom(&self) -> Result<f64> {
        self.node.number("zoom")
    }

    /// Sets the roll about the view axis, in radians.
    ///
    /// # Errors
    ///
    /// The roll is not finite, or the node was destroyed.
    pub fn set_roll(&self, roll: f64) -> Result<()> {
        self.node.set("roll", roll)
    }

    /// The roll about the view axis, in radians.
    ///
    /// # Errors
    ///
    /// The node was destroyed.
    pub fn roll(&self) -> Result<f64> {
        self.node.number("roll")
    }

    /// Whether this is the identity view: no pan, zoom 1, no roll.
    ///
    /// # Errors
    ///
    /// The node was destroyed.
    pub fn is_identity(&self) -> Result<bool> {
        Ok(self.center()? == (0.0, 0.0) && self.zoom()? == 1.0 && self.roll()? == 0.0)
    }
}

/// The initial values of a [`View`].
#[derive(Clone, Debug, Default)]
pub struct ViewOptions<'a> {
    /// The camera the content is drawn through; none is the flat default.
    pub camera: Option<&'a Camera3D>,
    /// The width of the picture in client pixels.
    pub width: Option<f64>,
    /// The height of the picture in client pixels.
    pub height: Option<f64>,
    /// The client the view shows.
    pub content: Option<&'a Client>,
}

/// A node that draws a client as a picture without moving it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    node: SceneNode,
}

impl std::ops::Deref for View {
    type Target = SceneNode;

    fn deref(&self) -> &SceneNode {
        &self.node
    }
}

impl View {
    /// Makes a view and writes `options`.
    ///
    /// # Errors
    ///
    /// An option is not valid; the view is destroyed.
    pub fn new(std: &Std, options: &ViewOptions<'_>) -> Result<View> {
        let view = View {
            node: SceneNode::make(std, abi::STD_SCENE_VIEW)?,
        };
        let written = (|| {
            if let Some(camera) = options.camera {
                view.set_camera(Some(camera))?;
            }
            if let Some(width) = options.width {
                view.node.set("width", width)?;
            }
            if let Some(height) = options.height {
                view.node.set("height", height)?;
            }
            if let Some(content) = options.content {
                view.set_content(Some(content))?;
            }
            Ok(())
        })();
        match written {
            Ok(()) => Ok(view),
            Err(error) => {
                let _ = view.node.destroy();
                Err(error)
            }
        }
    }

    /// Shows `client`, or nothing.
    ///
    /// # Errors
    ///
    /// The node was destroyed.
    pub fn set_content(&self, client: Option<&Client>) -> Result<()> {
        self.node
            .set("source", client.map_or(NodeId::NIL, |client| client.window().id()))
    }

    /// The client the view shows, or `None`.
    ///
    /// # Errors
    ///
    /// The node was destroyed.
    pub fn content(&self) -> Result<Option<NodeId>> {
        Ok(match self.node.get("source")? {
            Some(PropValue::Node(id)) if id != NodeId::NIL => Some(id),
            _ => None,
        })
    }

    /// Draws through `camera`, or flat with `None`.
    ///
    /// # Errors
    ///
    /// The node was destroyed.
    pub fn set_camera(&self, camera: Option<&Camera3D>) -> Result<()> {
        self.node
            .set("camera", camera.map_or(NodeId::NIL, |camera| camera.id()))
    }

    /// Sets the picture's size in client pixels.
    ///
    /// # Errors
    ///
    /// A dimension is not finite, or the node was destroyed.
    pub fn set_size(&self, width: f64, height: f64) -> Result<()> {
        self.node.set("width", width)?;
        self.node.set("height", height)
    }

    /// The picture's size in client pixels.
    ///
    /// # Errors
    ///
    /// The node was destroyed.
    pub fn size(&self) -> Result<(f64, f64)> {
        Ok((self.node.number("width")?, self.node.number("height")?))
    }
}

impl Std {
    /// The scene of this generation.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn scene(&self) -> Result<Scene> {
        Scene::of(self)
    }
}
