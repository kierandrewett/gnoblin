//! The pure translation from planned layers to OpenXR composition layers.
//!
//! [`xr_scene::plan`] says where each View sits. This crate says what to submit for it once its picture is in a
//! swapchain: the layer kind, the pose and size, the eye visibility, the sub-image, the colour scale for opacity, and
//! which layers fit in the runtime's budget. It has no OpenXR types, so every rule here runs under `cargo test`
//! with no runtime. `xr-session` turns a [`SubmittedLayer`] into the `openxr` structs.
//!
//! The rules:
//!
//! - Layers are submitted in plan order. Nothing here reorders them, and nothing edits a pose or a size.
//! - A View with no ready swapchain is not submitted and is reported [`NoticeKind::NoSwapchain`]. It takes no budget.
//! - A quad is shown to both eyes. A projection layer carries the two eyes its stereo camera states (the planner
//!   copies them from the tree: they are not measured). The two eyes read the left and right halves of the View's one
//!   image, which is how the compositor draws a View whose camera is stereo: left eye on the left.
//! - Opacity below 1 becomes a colour scale `[o, o, o, o]` on the layer (`XR_KHR_composition_layer_color_scale_bias`) when
//!   the runtime offers it. The captured pictures are premultiplied, as `wl_shm` ARGB is, and OpenXR's default
//!   blend treats a layer as premultiplied, so scaling all four channels is the right fade. Without the extension the
//!   opacity cannot be shown: the layer is submitted at full strength and reported [`NoticeKind::OpacityIgnored`].
//! - The budget is `max_layer_count - reserved`. World layers keep it before head-locked ones, so the head-locked
//!   layers beyond it are dropped first; if the world layers alone exceed it, those last in plan order are dropped. Each
//!   dropped layer is reported [`NoticeKind::LayerBudget`]. A frame that went over the budget would be refused whole by
//!   `xrEndFrame`.

use xr_scene::{Eye, Fov, Layer, NodeId, Placement, Pose, Projected, Space};

/// The runtime's handle to one swapchain. Opaque to this crate: the runtime that created it maps it back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SwapchainId(pub u64);

/// A swapchain that holds a View's picture and is ready to be submitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwapchainRef {
    pub id: SwapchainId,
    pub width: u32,
    pub height: u32,
}

/// What the runtime allows and offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The runtime's `maxLayerCount`.
    pub max_layer_count: u32,
    /// Layers the extension keeps for itself (an environment, say), taken off the budget.
    pub reserved: u32,
    /// Whether `XR_KHR_composition_layer_color_scale_bias` is enabled on the session.
    pub color_scale_bias: bool,
}

impl Limits {
    /// How many planned layers may be submitted.
    pub const fn budget(&self) -> usize {
        self.max_layer_count.saturating_sub(self.reserved) as usize
    }
}

/// Which eyes see a quad.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EyeVisibility {
    Both,
    Left,
    Right,
}

/// A sub-rectangle of a swapchain image, in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// An `XrCompositionLayerQuad`.
#[derive(Debug, Clone, PartialEq)]
pub struct QuadLayer {
    /// The View this layer draws.
    pub view: NodeId,
    pub swapchain: SwapchainId,
    /// `Space::World` is the local reference space, `Space::View` the head-locked one.
    pub space: Space,
    /// Position in metres and orientation, in `space`.
    pub pose: Pose,
    /// Width and height in metres.
    pub size: [f64; 2],
    pub eye_visibility: EyeVisibility,
    pub rect: ImageRect,
    /// The colour scale for opacity, with a zero bias, if the layer is not opaque.
    pub color_scale: Option<[f32; 4]>,
}

/// One eye of a projection layer: its view and the part of the image it reads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectionEye {
    pub pose: Pose,
    pub fov: Fov,
    pub rect: ImageRect,
}

/// An `XrCompositionLayerProjection`, left eye first.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectionLayer {
    pub view: NodeId,
    pub swapchain: SwapchainId,
    pub space: Space,
    pub eyes: [ProjectionEye; 2],
    pub color_scale: Option<[f32; 4]>,
}

/// A layer ready for `xrEndFrame`.
#[derive(Debug, Clone, PartialEq)]
pub enum SubmittedLayer {
    Quad(QuadLayer),
    Projection(ProjectionLayer),
}

/// Why a View was not submitted as planned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    /// The layer did not fit the budget and was dropped.
    LayerBudget,
    /// The View has no ready swapchain yet, so it was not submitted.
    NoSwapchain,
    /// The opacity is below 1 and the runtime lacks the colour scale extension, so the layer is submitted opaque.
    OpacityIgnored,
}

impl NoticeKind {
    /// The name the extension reports, in the form the design uses (`layer-budget`).
    pub const fn name(self) -> &'static str {
        match self {
            Self::LayerBudget => "layer-budget",
            Self::NoSwapchain => "no-swapchain",
            Self::OpacityIgnored => "opacity-ignored",
        }
    }
}

/// A report about one View.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Notice {
    pub view: NodeId,
    pub kind: NoticeKind,
}

/// The result of [`compose`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Submission {
    /// The layers to submit, in plan order.
    pub layers: Vec<SubmittedLayer>,
    /// Views that were dropped or degraded, in plan order for each kind.
    pub notices: Vec<Notice>,
}

/// Turns the planned `layers` into the layers to submit.
///
/// `swapchain` finds the ready swapchain of a View.
pub fn compose(layers: &[Layer], swapchain: &dyn Fn(NodeId) -> Option<SwapchainRef>, limits: &Limits) -> Submission {
    let mut notices = Vec::new();
    let mut candidates: Vec<(SubmittedLayer, f64)> = Vec::with_capacity(layers.len());
    for layer in layers {
        let (Layer::Quad(placement) | Layer::Projection(Projected { placement, .. })) = layer;
        let Some(chain) = swapchain(placement.id) else {
            notices.push(Notice {
                view: placement.id,
                kind: NoticeKind::NoSwapchain,
            });
            continue;
        };
        let submitted = match layer {
            Layer::Quad(_) => quad(placement, chain),
            Layer::Projection(projected) => projection(placement, chain, &projected.eyes),
        };
        candidates.push((submitted, placement.opacity));
    }

    let kept = within_budget(&candidates, limits.budget());
    let mut submitted = Vec::with_capacity(candidates.len());
    for ((layer, opacity), keep) in candidates.into_iter().zip(kept) {
        if keep {
            submitted.push(fade(layer, opacity, limits, &mut notices));
        } else {
            notices.push(Notice {
                view: view_of(&layer),
                kind: NoticeKind::LayerBudget,
            });
        }
    }
    Submission {
        layers: submitted,
        notices,
    }
}

/// Which candidates fit in `budget`: the first world layers, then the first head-locked ones, in plan order.
fn within_budget(candidates: &[(SubmittedLayer, f64)], budget: usize) -> Vec<bool> {
    let mut left = budget;
    let mut keep = vec![false; candidates.len()];
    for want_space in [Space::World, Space::View] {
        for (index, (layer, _)) in candidates.iter().enumerate() {
            if space_of(layer) == want_space && left > 0 {
                keep[index] = true;
                left -= 1;
            }
        }
    }
    keep
}

fn space_of(layer: &SubmittedLayer) -> Space {
    match layer {
        SubmittedLayer::Quad(quad) => quad.space,
        SubmittedLayer::Projection(projection) => projection.space,
    }
}

fn view_of(layer: &SubmittedLayer) -> NodeId {
    match layer {
        SubmittedLayer::Quad(quad) => quad.view,
        SubmittedLayer::Projection(projection) => projection.view,
    }
}

fn fade(mut layer: SubmittedLayer, opacity: f64, limits: &Limits, notices: &mut Vec<Notice>) -> SubmittedLayer {
    if opacity >= 1.0 {
        return layer;
    }
    if !limits.color_scale_bias {
        notices.push(Notice {
            view: view_of(&layer),
            kind: NoticeKind::OpacityIgnored,
        });
        return layer;
    }
    #[allow(clippy::cast_possible_truncation)]
    let o = opacity.clamp(0.0, 1.0) as f32;
    let scale = Some([o; 4]);
    match &mut layer {
        SubmittedLayer::Quad(quad) => quad.color_scale = scale,
        SubmittedLayer::Projection(projection) => projection.color_scale = scale,
    }
    layer
}

fn full(chain: SwapchainRef) -> ImageRect {
    ImageRect {
        x: 0,
        y: 0,
        width: to_i32(chain.width),
        height: to_i32(chain.height),
    }
}

fn to_i32(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

fn quad(placement: &Placement, chain: SwapchainRef) -> SubmittedLayer {
    SubmittedLayer::Quad(QuadLayer {
        view: placement.id,
        swapchain: chain.id,
        space: placement.space,
        pose: placement.pose,
        size: placement.size,
        eye_visibility: EyeVisibility::Both,
        rect: full(chain),
        color_scale: None,
    })
}

fn projection(placement: &Placement, chain: SwapchainRef, eyes: &[Eye; 2]) -> SubmittedLayer {
    let width = to_i32(chain.width);
    let height = to_i32(chain.height);
    let half = width / 2;
    let left = ImageRect {
        x: 0,
        y: 0,
        width: half,
        height,
    };
    let right = ImageRect {
        x: half,
        y: 0,
        width: width - half,
        height,
    };
    let eye = |eye: &Eye, rect| ProjectionEye {
        pose: eye.pose,
        fov: eye.fov,
        rect,
    };
    SubmittedLayer::Projection(ProjectionLayer {
        view: placement.id,
        swapchain: chain.id,
        space: placement.space,
        eyes: [eye(&eyes[0], left), eye(&eyes[1], right)],
        color_scale: None,
    })
}
