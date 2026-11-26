#![allow(unsafe_code)]
//! Layers placed in three dimensions, and the depth-stencil pass they and a View's clipping share.
//!
//! A layer that carries a [`Model`](gnoblin_render_frame::frame::Model) is a quad whose corners a matrix places in the
//! output (or in a View's picture) before the perspective divide. The shared vertex stage already takes the matrix as three
//! columns per quad (`quad.wgsl`, the `params.x` flag), so a placed quad is one more instance of the same draw. What it adds
//! is state: a depth buffer for the layers of an output, and a stencil for the clipping ancestors of a View.
//!
//! # The pass
//!
//! One render pass has a colour attachment that is kept (`LOAD`, as the plain pass does) and a depth-stencil attachment that
//! is cleared at the start of the pass and not kept: depth `1`, stencil `0`. Its pipelines are made on first use, one per
//! pass kind (`solid`, `surface`) and [`Class`]:
//!
//! | Class         | Depth test       | Stencil                                            | Colour |
//! | ------------- | ---------------- | -------------------------------------------------- | ------ |
//! | `Flat`        | off              | off                                                | drawn  |
//! | `Opaque`      | `<`, writes      | off                                                | drawn  |
//! | `Transparent` | `<=`, no write   | off                                                | drawn  |
//! | `Test`        | off              | passes where the stencil equals the reference      | drawn  |
//! | `Push`        | off              | as `Test`, and raises the stencil by one           | none   |
//! | `Pop`         | off              | as `Test`, and lowers the stencil by one           | none   |
//!
//! The reference is a dynamic state of the pipeline, so one pipeline serves every depth of a clip chain. A chain deeper than
//! the eight bit stencil counts is left unclipped past its limit ([`MAX_CLIP_DEPTH`]), as the GLES renderer leaves it.
//!
//! # Order in an output
//!
//! Opaque layers are drawn front to back, then transparent ones back to front, so a pair that can overlap is depth tested
//! right whatever order the frame lists them in ([`placements`]). They are drawn after the flat layers of the frame and before
//! the overlay and the cursor, which is where the GLES renderer draws them. A View's picture is drawn in painter's order
//! (class `Flat` or `Test`): its layers are listed back to front already and the depth buffer plays no part.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use ash::vk;
use gnoblin_render::Mat4;
use gnoblin_render_frame::frame::{Layer, Model, OutputFrame};
use gnoblin_render_frame::shader::sources::{SOLID, SURFACE};
use gnoblin_render_frame::shader::uniforms::Uniforms;
use gnoblin_types::{Rect, Scale, Size};

use super::context::{TARGET_FORMAT, VkContext};
use super::error::VulkanError;
use super::memory::VkImage;
use super::pipeline::{Depth, Pipelines, Variant, module};
use super::plan::{Draws, FramePlan, PassKind, TextureSizes, quad_of, slot_of, texture_mapping};
use super::shader::{StageKind, compile};

/// The deepest chain of clipping ancestors a View draws: the stencil buffer is eight bits.
pub(super) const MAX_CLIP_DEPTH: u8 = 254;

/// How a placed quad takes part in the depth and stencil tests: which pipeline draws it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(super) enum Class {
    /// No test: painter's order.
    #[default]
    Flat,
    /// Tested and written: an opaque layer, drawn front to back.
    Opaque,
    /// Tested and not written: a transparent layer, drawn back to front after the opaque ones.
    Transparent,
    /// Drawn only where the stencil holds the reference: inside every clip of the chain.
    Test,
    /// A clip's rectangle: raises the stencil where the chain before it covers, and draws no colour.
    Push,
    /// The rectangle of the clip that ends: lowers the stencil where the chain covers, and draws no colour.
    Pop,
}

/// A draw's class and, for a stencil class, the reference the stencil is compared with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct DrawVariant {
    pub class: Class,
    pub reference: u8,
}

impl DrawVariant {
    /// A layer drawn inside the clips of a chain `depth` deep.
    pub(super) fn inside(depth: u8) -> Self {
        if depth == 0 {
            Self::default()
        } else {
            Self {
                class: Class::Test,
                reference: depth,
            }
        }
    }

    /// The rectangle of the clip that makes the chain `depth` deep.
    pub(super) fn push(depth: u8) -> Self {
        Self {
            class: Class::Push,
            reference: depth - 1,
        }
    }

    /// The rectangle of the clip that ends a chain `depth` deep.
    pub(super) fn pop(depth: u8) -> Self {
        Self {
            class: Class::Pop,
            reference: depth,
        }
    }

    /// Whether the pipeline reads a stencil reference.
    pub(super) fn stenciled(self) -> bool {
        matches!(self.class, Class::Test | Class::Push | Class::Pop)
    }
}

/// Where a quad sits when a matrix places it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Placed {
    /// Columns 0, 1 and 3 of the clip matrix, applied to the unit quad point `(x, y, 0, 1)`.
    pub columns: [[f32; 4]; 3],
    /// The quad's depth at its centre, which orders the layers of an output; unused where nothing sorts.
    pub depth: f32,
    pub variant: DrawVariant,
    /// The part of the target the quad's clip space spreads over, for a View's eye. `None` is the whole target.
    pub viewport: Option<Rect>,
}

/// The two shipped fragment stages a placed quad can use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Base {
    Solid,
    Surface,
}

/// The whole-target passes that replace what they draw over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Replace {
    /// The magnifier and the colour matrix.
    View,
    /// The supersampled target averaged down.
    Downsample,
}

/// The depth-stencil render pass and the pipelines made for it and for the whole-target passes.
pub(super) struct Extras {
    context: Rc<VkContext>,
    pub(super) render_pass: vk::RenderPass,
    pub(super) format: vk::Format,
    placed: RefCell<HashMap<(Base, Class), vk::Pipeline>>,
    replace: RefCell<HashMap<u8, vk::Pipeline>>,
}

impl Extras {
    /// The render pass for `format`, which the device has chosen.
    ///
    /// # Errors
    ///
    /// [`VulkanError::Driver`] when the driver refuses the pass.
    pub(super) fn new(context: &Rc<VkContext>, format: vk::Format) -> Result<Self, VulkanError> {
        let attachments = [
            vk::AttachmentDescription::default()
                .format(TARGET_FORMAT)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::LOAD)
                .store_op(vk::AttachmentStoreOp::STORE)
                .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                .initial_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .final_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL),
            vk::AttachmentDescription::default()
                .format(format)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::DONT_CARE)
                .stencil_load_op(vk::AttachmentLoadOp::CLEAR)
                .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .final_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL),
        ];
        let colour = [vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)];
        let depth = vk::AttachmentReference::default()
            .attachment(1)
            .layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);
        let subpasses = [vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&colour)
            .depth_stencil_attachment(&depth)];
        let transfer_and_colour = vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT | vk::PipelineStageFlags::TRANSFER;
        let tests = vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS | vk::PipelineStageFlags::LATE_FRAGMENT_TESTS;
        let dependencies = [
            vk::SubpassDependency::default()
                .src_subpass(vk::SUBPASS_EXTERNAL)
                .dst_subpass(0)
                .src_stage_mask(transfer_and_colour | tests)
                .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT | tests)
                .src_access_mask(
                    vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                        | vk::AccessFlags::TRANSFER_WRITE
                        | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
                )
                .dst_access_mask(
                    vk::AccessFlags::COLOR_ATTACHMENT_READ
                        | vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                        | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_READ
                        | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
                ),
            vk::SubpassDependency::default()
                .src_subpass(0)
                .dst_subpass(vk::SUBPASS_EXTERNAL)
                .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT | tests)
                .dst_stage_mask(transfer_and_colour | tests)
                .src_access_mask(
                    vk::AccessFlags::COLOR_ATTACHMENT_WRITE | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
                )
                .dst_access_mask(vk::AccessFlags::TRANSFER_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE),
        ];
        let info = vk::RenderPassCreateInfo::default()
            .attachments(&attachments)
            .subpasses(&subpasses)
            .dependencies(&dependencies);
        // SAFETY: every structure above outlives the call, and the references name attachments 0 and 1.
        let render_pass = unsafe { context.device.create_render_pass(&info, None) }
            .map_err(|result| VulkanError::driver("vkCreateRenderPass", result))?;
        Ok(Self {
            context: Rc::clone(context),
            render_pass,
            format,
            placed: RefCell::new(HashMap::new()),
            replace: RefCell::new(HashMap::new()),
        })
    }

    /// The pipeline of `base` in `class`, made when first asked for, or null when the stage does not compile or the driver
    /// refuses the pipeline: that is logged once and the draws that need it are left out.
    pub(super) fn placed(&self, pipelines: &Pipelines, base: Base, class: Class) -> vk::Pipeline {
        if let Some(pipeline) = self.placed.borrow().get(&(base, class)) {
            return *pipeline;
        }
        let pipeline = match self.placed_pipeline(pipelines, base, class) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                crate::log::warn!(area = "render", ?base, ?class, %error, "a pipeline for a placed layer could not be made");
                vk::Pipeline::null()
            }
        };
        self.placed.borrow_mut().insert((base, class), pipeline);
        pipeline
    }

    fn placed_pipeline(&self, pipelines: &Pipelines, base: Base, class: Class) -> Result<vk::Pipeline, VulkanError> {
        let (name, source) = match base {
            Base::Solid => ("solid", SOLID),
            Base::Surface => ("surface", SURFACE),
        };
        let depth = match class {
            Class::Flat => Depth {
                test: false,
                write: false,
                compare: vk::CompareOp::ALWAYS,
                stencil: None,
            },
            Class::Opaque => Depth {
                test: true,
                write: true,
                compare: vk::CompareOp::LESS,
                stencil: None,
            },
            Class::Transparent => Depth {
                test: true,
                write: false,
                compare: vk::CompareOp::LESS_OR_EQUAL,
                stencil: None,
            },
            Class::Test => Depth {
                test: false,
                write: false,
                compare: vk::CompareOp::ALWAYS,
                stencil: Some(vk::StencilOp::KEEP),
            },
            Class::Push => Depth {
                test: false,
                write: false,
                compare: vk::CompareOp::ALWAYS,
                stencil: Some(vk::StencilOp::INCREMENT_AND_CLAMP),
            },
            Class::Pop => Depth {
                test: false,
                write: false,
                compare: vk::CompareOp::ALWAYS,
                stencil: Some(vk::StencilOp::DECREMENT_AND_CLAMP),
            },
        };
        let variant = Variant {
            render_pass: self.render_pass,
            blend: true,
            write_colour: !matches!(class, Class::Push | Class::Pop),
            depth: Some(depth),
        };
        self.build(pipelines, name, source, &variant)
    }

    /// The pipeline of a whole-target pass that replaces what it draws over (no blending), in the plain colour pass.
    ///
    /// # Errors
    ///
    /// As [`Extras::placed`].
    pub(super) fn replacing(&self, pipelines: &Pipelines, which: Replace) -> Result<vk::Pipeline, VulkanError> {
        let key = which as u8;
        if let Some(pipeline) = self.replace.borrow().get(&key) {
            return Ok(*pipeline);
        }
        use gnoblin_render_frame::shader::sources::{DOWNSAMPLE, VIEW};
        let (name, source) = match which {
            Replace::View => ("view", VIEW),
            Replace::Downsample => ("downsample", DOWNSAMPLE),
        };
        let variant = Variant {
            blend: false,
            ..Variant::colour(pipelines.render_pass)
        };
        let pipeline = self.build(pipelines, name, source, &variant)?;
        self.replace.borrow_mut().insert(key, pipeline);
        Ok(pipeline)
    }

    fn build(
        &self,
        pipelines: &Pipelines,
        name: &'static str,
        source: &str,
        variant: &Variant,
    ) -> Result<vk::Pipeline, VulkanError> {
        let words = compile(name, source, StageKind::Fragment)?;
        let fragment = module(&self.context, name, &words)?;
        let made = pipelines.graphics_context().build_with(&[fragment], variant);
        // SAFETY: a pipeline keeps what it needs of its modules, so the module may go once the call has returned.
        unsafe { self.context.device.destroy_shader_module(fragment, None) };
        made.map(|pipelines| pipelines[0])
    }

    /// The names of the pipelines made so far, which a test reads to prove one is made once and only when drawn with.
    #[cfg(test)]
    pub(super) fn made(&self) -> usize {
        self.placed.borrow().len() + self.replace.borrow().len()
    }
}

impl Drop for Extras {
    fn drop(&mut self) {
        let device = &self.context.device;
        // SAFETY: the device waited for the queue before anything below it was dropped, and every handle was made on it.
        unsafe {
            for pipeline in self.placed.borrow().values().chain(self.replace.borrow().values()) {
                device.destroy_pipeline(*pipeline, None);
            }
            device.destroy_render_pass(self.render_pass, None);
        }
    }
}

/// A depth-stencil image and the framebuffer over it and a colour image: what a pass of [`Extras::render_pass`] draws into.
pub(super) struct DepthTarget {
    context: Rc<VkContext>,
    pub(super) depth: VkImage,
    pub(super) framebuffer: vk::Framebuffer,
}

impl DepthTarget {
    /// A depth-stencil image the size of `colour`, and a framebuffer over `colour` and it.
    ///
    /// # Errors
    ///
    /// The driver's result.
    pub(super) fn over(context: &Rc<VkContext>, extras: &Extras, colour: &VkImage) -> Result<Self, vk::Result> {
        let depth = VkImage::depth_stencil(context, colour.width(), colour.height(), extras.format)?;
        let views = [colour.view(), depth.view()];
        let info = vk::FramebufferCreateInfo::default()
            .render_pass(extras.render_pass)
            .attachments(&views)
            .width(colour.width())
            .height(colour.height())
            .layers(1);
        // SAFETY: both views are of images that have the attachment usage and the pass's formats.
        let framebuffer = unsafe { context.device.create_framebuffer(&info, None) }?;
        Ok(Self {
            context: Rc::clone(context),
            depth,
            framebuffer,
        })
    }
}

impl Drop for DepthTarget {
    fn drop(&mut self) {
        // SAFETY: the owner waited for the last frame that drew into it, or retired it until that frame had finished.
        unsafe { self.context.device.destroy_framebuffer(self.framebuffer, None) };
    }
}

/// The matrix from an eye's pixel space (top-left first, y down) to clip space.
pub(super) fn screen_matrix(eye: Size) -> Mat4 {
    let (w, h) = (f64::from(eye.width), f64::from(eye.height));
    Mat4 {
        m: [
            [2.0 / w, 0.0, 0.0, -1.0],
            [0.0, 2.0 / h, 0.0, -1.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    }
}

/// The matrix from an output's logical pixels, `scale` to the physical pixel, to clip space.
pub(super) fn output_matrix(physical: Size, scale: Scale) -> Mat4 {
    let s = scale.as_f64();
    let (w, h) = (f64::from(physical.width), f64::from(physical.height));
    Mat4 {
        m: [
            [2.0 * s / w, 0.0, 0.0, -1.0],
            [0.0, 2.0 * s / h, 0.0, -1.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    }
}

/// How a rectangle of size `dst`, placed by `model`, lands in the target `screen` maps to: the clip matrix's three columns
/// and the quad's depth at its centre.
pub(super) fn place(screen: &Mat4, model: &Model, dst: Rect) -> ([[f32; 4]; 3], f32) {
    let matrix = screen
        .mul(&model.matrix)
        .mul(&Mat4::scale(f64::from(dst.width), f64::from(dst.height), 1.0));
    let column = |point: [f64; 4]| {
        let v = matrix.apply(point);
        [v[0] as f32, v[1] as f32, v[2] as f32, v[3] as f32]
    };
    let columns = [
        column([1.0, 0.0, 0.0, 0.0]),
        column([0.0, 1.0, 0.0, 0.0]),
        column([0.0, 0.0, 0.0, 1.0]),
    ];
    let centre = matrix.apply([0.5, 0.5, 0.0, 1.0]);
    let depth = ((centre[2] + centre[3]) * 0.5 / centre[3]) as f32;
    (columns, depth)
}

/// The layer's model, `None` for a layer drawn flat.
pub(super) fn layer_model(layer: &Layer) -> Option<&Model> {
    match layer {
        Layer::Surface(surface) => surface.model.as_ref(),
        Layer::Solid(solid) => solid.model.as_ref(),
        Layer::Overlay(_) => None,
    }
}

/// The layer's rectangle in its own plane.
pub(super) fn layer_dst(layer: &Layer) -> Rect {
    match layer {
        Layer::Surface(surface) => surface.dst,
        Layer::Solid(solid) => solid.dst,
        Layer::Overlay(overlay) => overlay.dst(),
    }
}

/// The placed layers of an output in the order they are drawn, each with its index in the frame: opaque ones front to back,
/// then transparent ones back to front.
pub(super) fn placements(layers: &[Layer], physical: Size, scale: Scale) -> Vec<(usize, Placed)> {
    let screen = output_matrix(physical, scale);
    let mut opaque = Vec::new();
    let mut transparent = Vec::new();
    for (index, layer) in layers.iter().enumerate() {
        let Some(model) = layer_model(layer) else {
            continue;
        };
        let (columns, depth) = place(&screen, model, layer_dst(layer));
        let (class, bucket) = if model.opaque {
            (Class::Opaque, &mut opaque)
        } else {
            (Class::Transparent, &mut transparent)
        };
        bucket.push((
            index,
            Placed {
                columns,
                depth,
                variant: DrawVariant { class, reference: 0 },
                viewport: None,
            },
        ));
    }
    opaque.sort_by(|a, b| a.1.depth.total_cmp(&b.1.depth));
    transparent.sort_by(|a, b| b.1.depth.total_cmp(&a.1.depth));
    opaque.extend(transparent);
    opaque
}

/// Plans the placed layers of an output: for each damage rectangle, every layer a matrix places, in the order [`placements`]
/// gives, each drawn through the scissor of the rectangle. A surface whose texture is not there is left out.
pub(super) fn plan_models(
    frame: &OutputFrame,
    scale: Scale,
    physical: Size,
    sizes: &TextureSizes,
    damage: &[Rect],
    draws: &mut Draws,
    built: &mut FramePlan,
) {
    let order = placements(frame.layers(), physical, scale);
    let radius_scale = scale.as_f64() as f32;
    for scissor in damage {
        for (index, placed) in &order {
            let layer = &frame.layers()[*index];
            let quad = quad_of(layer_dst(layer), scale);
            if quad.is_empty() {
                continue;
            }
            match layer {
                Layer::Solid(solid) => {
                    let mut uniforms = Uniforms::solid(quad, solid.colour, solid.tint, 1.0);
                    uniforms.corner = solid.corner_radius * radius_scale;
                    draws.push_placed(built, PassKind::Solid, None, *scissor, quad, &uniforms, *placed);
                }
                Layer::Surface(surface) => {
                    let Some(Some(size)) = sizes.layers.get(*index) else {
                        continue;
                    };
                    let uniforms = Uniforms::surface(
                        quad,
                        texture_mapping(surface, *size),
                        surface.tint,
                        surface.opacity,
                        surface.corner_radius * radius_scale,
                    );
                    draws.push_placed(
                        built,
                        PassKind::Surface,
                        Some(slot_of(surface, *index)),
                        *scissor,
                        quad,
                        &uniforms,
                        *placed,
                    );
                }
                Layer::Overlay(_) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_render_frame::frame::Solid;
    use gnoblin_types::Color;

    fn solid(opaque: bool, z: f64) -> Layer {
        let mut solid = Solid::new(Rect::new(0, 0, 4, 4), Color::WHITE);
        solid.model = Some(Model {
            matrix: Mat4::translation(0.0, 0.0, z),
            opaque,
        });
        Layer::Solid(solid)
    }

    #[test]
    fn opaque_layers_go_front_to_back_then_transparent_ones_back_to_front() {
        let layers = [
            solid(false, 0.2),
            solid(true, 0.6),
            solid(false, 0.8),
            solid(true, 0.1),
            Layer::Solid(Solid::new(Rect::new(0, 0, 4, 4), Color::WHITE)),
        ];
        let order: Vec<usize> = placements(&layers, Size::new(16, 16), Scale::IDENTITY)
            .into_iter()
            .map(|(index, _)| index)
            .collect();
        assert_eq!(
            order,
            [3, 1, 2, 0],
            "a flat layer is not placed; the rest sort by depth within their kind"
        );
    }

    #[test]
    fn a_clip_chain_numbers_its_stencil_references_as_the_gles_renderer_does() {
        assert_eq!(DrawVariant::inside(0), DrawVariant::default());
        assert_eq!(DrawVariant::inside(2).reference, 2);
        assert_eq!(DrawVariant::push(2).reference, 1, "a push tests the chain before it");
        assert_eq!(DrawVariant::pop(2).reference, 2, "a pop tests the chain with it");
    }
}
