#![allow(unsafe_code)]
//! The render pass, the pipelines, the samplers and the descriptor pools.
//!
//! One render pass draws into the target and keeps what is already there (`LOAD`), because a
//! frame repaints only its damage. Five pipelines share a layout and a vertex stage: `solid`,
//! `surface`, `shadow`, `inset_shadow` and `border`, which are the GLES renderer's passes of the same names. All blend
//! premultiplied colour (`ONE`, `ONE_MINUS_SRC_ALPHA`, for colour and for alpha), which is
//! what the software renderer's compositing and every GLES pass agree on.
//!
//! A shell's own shader pass is another kind of pipeline, one per source (`ShaderPipelines`): the same vertex stage, state and
//! layout, with a fragment module compiled from the validated WGSL when a frame first draws the source.
//!
//! # The pipeline cache
//!
//! The driver's cache of compiled pipelines is kept in a file under `$XDG_CACHE_HOME/gnoblin/vulkan/`, named by the device's
//! cache UUID, so a restart does not compile every pipeline again. The file is the driver's blob behind a header of ours
//! (a magic, a format version, the blob's length and its CRC32), and the blob's own header is checked against the device's
//! vendor, device and UUID. A file that fails any check is deleted and ignored. It is written to a temporary file and renamed,
//! when the device is dropped and after every [`PIPELINES_PER_SAVE`] pipelines the shader worker builds. It is never larger
//! than [`MAX_CACHE_BYTES`].
//!
//! # Descriptor sets
//!
//! A frame's own sets (the uniform set, a backdrop's, a transient texture's) come from the frame slot's arena
//! ([`DescriptorArena`]), which is reset when the slot's frame has finished. A cached texture owns one set for as long as it
//! lives ([`TextureSets`], pools that free individual sets), so a steady frame writes no texture descriptors.
//!
//! # Bindings
//!
//! | Set | Binding | What                                              | Stage    |
//! | --- | ------- | ------------------------------------------------- | -------- |
//! | 0   | 0       | the pass's uniform block, dynamic offset          | fragment |
//! | 0   | 1       | a shader pass's own block, dynamic offset         | fragment |
//! | 1   | 0       | the layer's texture                               | fragment |
//! | 1   | 1       | its sampler                                       | fragment |
//! | 2   | 0       | the backdrop texture, a shader pass that reads it | fragment |
//! | 2   | 1       | its sampler                                       | fragment |
//!
//! These are the `@group` and `@binding` numbers the WGSL preamble and the user block declare, which `naga` carries into
//! the SPIR-V as descriptor sets, so the layout here is the WGSL's. Every pipeline has the one layout. A draw binds both
//! offsets of set 0 (a shipped pass never reads the second block) and binds set 2 only when its pass reads the backdrop.
//!
//! # The vertex input
//!
//! Binding 0 is the unit quad, four vertices of a strip. Binding 1 is per instance: the
//! rectangle, the viewport and the optional projection, 22 floats at the locations the
//! vertex stage declares (see `plan.rs`).

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use ash::vk;

use super::context::{CacheIdentity, TARGET_FORMAT, VkContext};
use super::error::VulkanError;
use super::shader::{StageKind, compile, compile_user};
use crate::log;
use gnoblin_render::ShaderKey;
use gnoblin_render::ShaderPass;
use gnoblin_render_frame::redraw::RedrawSignal;
use gnoblin_render_frame::shader::uniforms::USER_BLOCK_BYTES;
use gnoblin_render_frame::shader::{SHADER_CACHE_ENTRIES, ShaderCache};

// The shipped pipelines are made when the renderer starts; a shell's are built later on a worker thread.

/// The WGSL of each module of the two pipelines.
#[derive(Clone, Copy, Debug)]
pub(super) struct ShaderSources<'a> {
    /// The vertex stage, shared by every pass.
    pub vertex: &'a str,
    /// The solid-colour fragment stage (also used for the background).
    pub solid: &'a str,
    /// The textured fragment stage for client surfaces and bitmaps.
    pub surface: &'a str,
    /// The drop shadow's fragment stage.
    pub shadow: &'a str,
    /// The inset shadow's fragment stage.
    pub inset_shadow: &'a str,
    /// The border's fragment stage.
    pub border: &'a str,
}

impl ShaderSources<'static> {
    /// The sources the GLES renderer draws with.
    pub(super) fn shared() -> Self {
        use gnoblin_render_frame::shader::sources::{BORDER, INSET_SHADOW, SHADOW, SOLID, SURFACE, VERTEX};
        Self {
            vertex: VERTEX,
            solid: SOLID,
            surface: SURFACE,
            shadow: SHADOW,
            inset_shadow: INSET_SHADOW,
            border: BORDER,
        }
    }
}

/// The SPIR-V of the three modules.
///
/// Compiling does not need a device, so it is done before one is created: a pass that does not
/// compile is a startup error on a machine with no Vulkan at all, and the test of it needs none.
pub(super) struct Compiled {
    vertex: Vec<u32>,
    solid: Vec<u32>,
    surface: Vec<u32>,
    shadow: Vec<u32>,
    inset_shadow: Vec<u32>,
    border: Vec<u32>,
}

impl Compiled {
    /// Compiles `sources`.
    ///
    /// # Errors
    ///
    /// [`VulkanError::Shader`] naming the first pass that does not compile.
    pub(super) fn new(sources: &ShaderSources<'_>) -> Result<Self, VulkanError> {
        Ok(Self {
            vertex: compile("vertex", sources.vertex, StageKind::Vertex)?,
            solid: compile("solid", sources.solid, StageKind::Fragment)?,
            surface: compile("surface", sources.surface, StageKind::Fragment)?,
            shadow: compile("shadow", sources.shadow, StageKind::Fragment)?,
            inset_shadow: compile("inset-shadow", sources.inset_shadow, StageKind::Fragment)?,
            border: compile("border", sources.border, StageKind::Fragment)?,
        })
    }
}

/// The size of one uniform block, in bytes: the preamble's `Gnoblin` struct.
pub(super) const UNIFORM_BYTES: usize = 112;

/// The floats one quad's vertex data holds: see the vertex input above.
pub(super) const INSTANCE_FLOATS: usize = 22;

/// How many sets one descriptor pool holds.
const POOL_SETS: u32 = 256;

/// The pipelines, their layout and the pass and samplers they use.
///
/// Every handle starts null and is filled in as it is made, so a creation that fails half way
/// is cleaned up by `Drop` and never leaks what it made before the failure.
pub(super) struct Pipelines {
    context: Rc<VkContext>,
    pub(super) render_pass: vk::RenderPass,
    pub(super) uniform_layout: vk::DescriptorSetLayout,
    pub(super) texture_layout: vk::DescriptorSetLayout,
    pub(super) layout: vk::PipelineLayout,
    /// The vertex stage every pipeline shares, kept for the pipelines made at run time.
    vertex: vk::ShaderModule,
    pub(super) solid: vk::Pipeline,
    pub(super) surface: vk::Pipeline,
    pub(super) shadow: vk::Pipeline,
    pub(super) inset_shadow: vk::Pipeline,
    pub(super) border: vk::Pipeline,
    pub(super) linear: vk::Sampler,
    pub(super) nearest: vk::Sampler,
    /// The driver's cache of compiled pipelines, null when it could not be made.
    cache: vk::PipelineCache,
    /// Where the cache is kept between runs, when it is.
    cache_file: Option<PathBuf>,
    /// Whether the cache was read from the file this run.
    cache_loaded: bool,
    /// How many pipelines have been built since the device was made.
    built: Arc<AtomicU64>,
    /// The sets of the textures the texture cache keeps, which live as long as their image.
    pub(super) long_sets: Rc<TextureSets>,
}

impl Pipelines {
    /// Creates everything a frame draws with, from modules compiled by [`Compiled::new`].
    ///
    /// # Errors
    ///
    /// [`VulkanError::Shader`] when the driver refuses a module, and [`VulkanError::Driver`]
    /// for any other refused creation.
    pub(super) fn new(
        context: &Rc<VkContext>,
        compiled: &Compiled,
        location: &PipelineCacheLocation,
    ) -> Result<Self, VulkanError> {
        let mut made = Self {
            context: Rc::clone(context),
            render_pass: vk::RenderPass::null(),
            uniform_layout: vk::DescriptorSetLayout::null(),
            texture_layout: vk::DescriptorSetLayout::null(),
            layout: vk::PipelineLayout::null(),
            vertex: vk::ShaderModule::null(),
            solid: vk::Pipeline::null(),
            surface: vk::Pipeline::null(),
            shadow: vk::Pipeline::null(),
            inset_shadow: vk::Pipeline::null(),
            border: vk::Pipeline::null(),
            linear: vk::Sampler::null(),
            nearest: vk::Sampler::null(),
            cache: vk::PipelineCache::null(),
            cache_file: location.file(&context.cache_identity),
            cache_loaded: false,
            built: Arc::new(AtomicU64::new(0)),
            long_sets: Rc::new(TextureSets::new(context)),
        };
        made.vertex = module(context, "vertex", &compiled.vertex)?;
        let modules = ShaderModules::new(context, compiled)?;
        made.create(&modules)?;
        Ok(made)
    }

    fn create(&mut self, modules: &ShaderModules) -> Result<(), VulkanError> {
        let device = &self.context.device;
        let attachments = [vk::AttachmentDescription::default()
            .format(TARGET_FORMAT)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::LOAD)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .final_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)];
        let references = [vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)];
        let subpasses = [vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&references)];
        let transfer_and_colour = vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT | vk::PipelineStageFlags::TRANSFER;
        let dependencies = [
            vk::SubpassDependency::default()
                .src_subpass(vk::SUBPASS_EXTERNAL)
                .dst_subpass(0)
                .src_stage_mask(transfer_and_colour)
                .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
                .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE | vk::AccessFlags::TRANSFER_WRITE)
                .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE),
            vk::SubpassDependency::default()
                .src_subpass(0)
                .dst_subpass(vk::SUBPASS_EXTERNAL)
                .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
                .dst_stage_mask(transfer_and_colour)
                .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                .dst_access_mask(vk::AccessFlags::TRANSFER_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE),
        ];
        let pass_info = vk::RenderPassCreateInfo::default()
            .attachments(&attachments)
            .subpasses(&subpasses)
            .dependencies(&dependencies);
        // SAFETY: every structure above outlives the call, and the references name attachment 0.
        self.render_pass = unsafe { device.create_render_pass(&pass_info, None) }
            .map_err(|result| VulkanError::driver("vkCreateRenderPass", result))?;
        // The driver's cache of compiled pipelines, kept in a file between runs so a source the shader cache evicted, or the
        // next start of the compositor, is not compiled by the driver from nothing. A cache that cannot be read or made
        // costs the cache and not the renderer.
        let initial = self
            .cache_file
            .as_deref()
            .and_then(|path| load_cache(path, &self.context.cache_identity));
        self.cache_loaded = initial.is_some();
        let data = initial.unwrap_or_default();
        // SAFETY: `data` is a blob this device's driver wrote, whose header was checked against the device, or empty.
        self.cache =
            unsafe { device.create_pipeline_cache(&vk::PipelineCacheCreateInfo::default().initial_data(&data), None) }
                .unwrap_or_else(|_| vk::PipelineCache::null());

        // Group 0 holds two dynamic uniform blocks: the shared one every pass reads, and the one a shell's shader pass adds
        // (`user_block.wgsl`). Every draw gives an offset for both, so a shipped pass points the second at a block it never reads.
        let uniform_bindings = [0, 1].map(|binding| {
            vk::DescriptorSetLayoutBinding::default()
                .binding(binding)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT)
        });
        let texture_bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
        ];
        // SAFETY: the bindings outlive the calls.
        unsafe {
            self.uniform_layout = device
                .create_descriptor_set_layout(
                    &vk::DescriptorSetLayoutCreateInfo::default().bindings(&uniform_bindings),
                    None,
                )
                .map_err(|result| VulkanError::driver("vkCreateDescriptorSetLayout", result))?;
            self.texture_layout = device
                .create_descriptor_set_layout(
                    &vk::DescriptorSetLayoutCreateInfo::default().bindings(&texture_bindings),
                    None,
                )
                .map_err(|result| VulkanError::driver("vkCreateDescriptorSetLayout", result))?;
        }
        // Group 2, the backdrop, has the layout of group 1: a texture and its sampler.
        let set_layouts = [self.uniform_layout, self.texture_layout, self.texture_layout];
        // SAFETY: both layouts were created just above.
        self.layout = unsafe {
            device.create_pipeline_layout(&vk::PipelineLayoutCreateInfo::default().set_layouts(&set_layouts), None)
        }
        .map_err(|result| VulkanError::driver("vkCreatePipelineLayout", result))?;

        self.linear = sampler(device, vk::Filter::LINEAR)?;
        self.nearest = sampler(device, vk::Filter::NEAREST)?;

        let made = self.graphics(&modules.fragments)?;
        self.solid = made[0];
        self.surface = made[1];
        self.shadow = made[2];
        self.inset_shadow = made[3];
        self.border = made[4];
        Ok(())
    }

    /// One pipeline for each of `fragments`, over the shared vertex stage, the shared layout and this render pass.
    ///
    /// The pipelines differ only in their fragment module, which is why a shell's shader pass can be made with the same call
    /// at run time.
    ///
    /// # Errors
    ///
    /// [`VulkanError::Driver`] when the driver refuses creation; nothing is leaked.
    pub(super) fn graphics(&self, fragments: &[vk::ShaderModule]) -> Result<Vec<vk::Pipeline>, VulkanError> {
        self.graphics_context().build(fragments)
    }

    /// The handles a pipeline is built from, which a worker thread can hold.
    pub(super) fn graphics_context(&self) -> GraphicsContext {
        GraphicsContext {
            device: self.context.device.clone(),
            vertex: self.vertex,
            layout: self.layout,
            render_pass: self.render_pass,
            cache: self.cache,
            cache_file: self.cache_file.clone(),
            built: Arc::clone(&self.built),
        }
    }

    /// Writes the driver's cache to its file, when it is kept. The caller is the only thread that uses the cache: the
    /// shader worker has been joined, or this is the worker.
    pub(super) fn save_cache(&self) {
        self.graphics_context().save_cache();
    }

    /// Whether the driver's cache was read from a file this run.
    pub(super) fn cache_loaded(&self) -> bool {
        self.cache_loaded
    }

    /// How many pipelines have been built since the device was made.
    pub(super) fn built(&self) -> u64 {
        self.built.load(Ordering::Relaxed)
    }
}

/// The depth and stencil state of a pipeline drawn into a render pass that has a depth-stencil attachment.
#[derive(Clone, Copy, Debug)]
pub(super) struct DepthState {
    /// Whether a fragment is tested against the depth buffer.
    pub test: bool,
    /// Whether a fragment that passes writes its depth.
    pub write: bool,
    /// How the depth test compares.
    pub compare: vk::CompareOp,
    /// The stencil operation on a pass of a test that the stencil equals the dynamic reference, or `None` for no stencil test.
    pub stencil: Option<vk::StencilOp>,
}

/// What a pipeline differs in from the shipped ones: the render pass it draws in, whether it blends and writes colour, and the
/// depth and stencil state.
#[derive(Clone, Copy, Debug)]
pub(super) struct PipelineVariant {
    pub render_pass: vk::RenderPass,
    pub blend: bool,
    pub write_colour: bool,
    /// `Some` when `render_pass` has a depth-stencil attachment.
    pub depth: Option<DepthState>,
}

impl PipelineVariant {
    /// The shipped state: premultiplied blending into `render_pass`, which has only colour.
    pub(super) fn colour(render_pass: vk::RenderPass) -> Self {
        Self {
            render_pass,
            blend: true,
            write_colour: true,
            depth: None,
        }
    }
}

/// What a graphics pipeline is built from: the device, the shared vertex stage, the layout, the render pass and the driver's
/// cache. All are handles, so the worker thread that builds a shell's pipeline holds a copy. The cache is externally
/// synchronised and only the worker uses it once the renderer has started.
#[derive(Clone)]
pub(super) struct GraphicsContext {
    device: ash::Device,
    vertex: vk::ShaderModule,
    layout: vk::PipelineLayout,
    render_pass: vk::RenderPass,
    cache: vk::PipelineCache,
    /// Where the cache is written, when it is kept between runs.
    cache_file: Option<PathBuf>,
    /// How many pipelines have been built: the shipped ones and the shell's.
    built: Arc<AtomicU64>,
}

impl GraphicsContext {
    /// One pipeline for each of `fragments`; see [`Pipelines::graphics`].
    pub(super) fn build(&self, fragments: &[vk::ShaderModule]) -> Result<Vec<vk::Pipeline>, VulkanError> {
        self.build_with(fragments, &PipelineVariant::colour(self.render_pass))
    }

    /// One pipeline for each of `fragments` in the state `variant` says: another render pass, no blending, a depth or stencil
    /// test. The vertex stage, the layout and the vertex input are the shared ones.
    pub(super) fn build_with(
        &self,
        fragments: &[vk::ShaderModule],
        variant: &PipelineVariant,
    ) -> Result<Vec<vk::Pipeline>, VulkanError> {
        let device = &self.device;
        let entry = c"gnoblin_entry";
        let stage_sets: Vec<[vk::PipelineShaderStageCreateInfo; 2]> = fragments
            .iter()
            .map(|fragment| {
                [
                    vk::PipelineShaderStageCreateInfo::default()
                        .stage(vk::ShaderStageFlags::VERTEX)
                        .module(self.vertex)
                        .name(entry),
                    vk::PipelineShaderStageCreateInfo::default()
                        .stage(vk::ShaderStageFlags::FRAGMENT)
                        .module(*fragment)
                        .name(entry),
                ]
            })
            .collect();
        let float = std::mem::size_of::<f32>() as u32;
        let bindings = [
            vk::VertexInputBindingDescription::default()
                .binding(0)
                .stride(2 * float)
                .input_rate(vk::VertexInputRate::VERTEX),
            vk::VertexInputBindingDescription::default()
                .binding(1)
                .stride(INSTANCE_FLOATS as u32 * float)
                .input_rate(vk::VertexInputRate::INSTANCE),
        ];
        let attribute = |location: u32, binding: u32, format: vk::Format, floats: u32| {
            vk::VertexInputAttributeDescription::default()
                .location(location)
                .binding(binding)
                .format(format)
                .offset(floats * float)
        };
        let attributes = [
            attribute(0, 0, vk::Format::R32G32_SFLOAT, 0),
            // The rectangle, the viewport, then the projection's two columns, its translation
            // column and its parameters.
            attribute(1, 1, vk::Format::R32G32B32A32_SFLOAT, 0),
            attribute(2, 1, vk::Format::R32G32_SFLOAT, 4),
            attribute(3, 1, vk::Format::R32G32B32A32_SFLOAT, 6),
            attribute(4, 1, vk::Format::R32G32B32A32_SFLOAT, 10),
            attribute(5, 1, vk::Format::R32G32B32A32_SFLOAT, 14),
            attribute(6, 1, vk::Format::R32G32B32A32_SFLOAT, 18),
        ];
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&bindings)
            .vertex_attribute_descriptions(&attributes);
        let assembly =
            vk::PipelineInputAssemblyStateCreateInfo::default().topology(vk::PrimitiveTopology::TRIANGLE_STRIP);
        let viewport = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let raster = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .line_width(1.0);
        let multisample =
            vk::PipelineMultisampleStateCreateInfo::default().rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let mask = if variant.write_colour {
            vk::ColorComponentFlags::RGBA
        } else {
            vk::ColorComponentFlags::empty()
        };
        let blend_attachments = [vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(variant.blend)
            .src_color_blend_factor(vk::BlendFactor::ONE)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(mask)];
        let blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_attachments);
        let mut dynamic_states = vec![vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let stencil_state = variant.depth.map(|depth| {
            let stencil = depth.stencil.map(|operation| {
                vk::StencilOpState::default()
                    .fail_op(vk::StencilOp::KEEP)
                    .pass_op(operation)
                    .depth_fail_op(vk::StencilOp::KEEP)
                    .compare_op(vk::CompareOp::EQUAL)
                    .compare_mask(0xff)
                    .write_mask(0xff)
            });
            vk::PipelineDepthStencilStateCreateInfo::default()
                .depth_test_enable(depth.test)
                .depth_write_enable(depth.write)
                .depth_compare_op(depth.compare)
                .stencil_test_enable(stencil.is_some())
                .front(stencil.unwrap_or_default())
                .back(stencil.unwrap_or_default())
        });
        if variant.depth.is_some_and(|depth| depth.stencil.is_some()) {
            dynamic_states.push(vk::DynamicState::STENCIL_REFERENCE);
        }
        let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
        let infos: Vec<vk::GraphicsPipelineCreateInfo> = stage_sets
            .iter()
            .map(|stages| {
                let info = vk::GraphicsPipelineCreateInfo::default()
                    .stages(stages)
                    .vertex_input_state(&vertex_input)
                    .input_assembly_state(&assembly)
                    .viewport_state(&viewport)
                    .rasterization_state(&raster)
                    .multisample_state(&multisample)
                    .color_blend_state(&blend)
                    .dynamic_state(&dynamic)
                    .layout(self.layout)
                    .render_pass(variant.render_pass)
                    .subpass(0);
                match &stencil_state {
                    Some(state) => info.depth_stencil_state(state),
                    None => info,
                }
            })
            .collect();
        // SAFETY: every state structure above outlives the call and matches the render pass.
        let made = unsafe { device.create_graphics_pipelines(self.cache, &infos, None) };
        if let Ok(pipelines) = &made {
            self.built.fetch_add(pipelines.len() as u64, Ordering::Relaxed);
        }
        match made {
            Ok(pipelines) => Ok(pipelines),
            Err((partial, result)) => {
                // SAFETY: the pipelines the call did create are not in use.
                for pipeline in partial.into_iter().filter(|pipeline| *pipeline != vk::Pipeline::null()) {
                    unsafe { device.destroy_pipeline(pipeline, None) };
                }
                Err(VulkanError::driver("vkCreateGraphicsPipelines", result))
            }
        }
    }
}

/// A sampler that clamps to the edge and does not mip.
fn sampler(device: &ash::Device, filter: vk::Filter) -> Result<vk::Sampler, VulkanError> {
    let info = vk::SamplerCreateInfo::default()
        .mag_filter(filter)
        .min_filter(filter)
        .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
        .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .max_lod(0.0)
        .border_color(vk::BorderColor::FLOAT_TRANSPARENT_BLACK);
    // SAFETY: `info` is a valid description.
    unsafe { device.create_sampler(&info, None) }.map_err(|result| VulkanError::driver("vkCreateSampler", result))
}

impl Drop for Pipelines {
    fn drop(&mut self) {
        if self.cache != vk::PipelineCache::null() {
            self.save_cache();
        }
        let device = &self.context.device;
        // SAFETY: the renderer waited for its last frame, and every handle is null or was made on
        // this device. Destroying a null handle is a no-op.
        unsafe {
            device.destroy_pipeline(self.solid, None);
            device.destroy_pipeline(self.surface, None);
            device.destroy_pipeline(self.shadow, None);
            device.destroy_pipeline(self.inset_shadow, None);
            device.destroy_pipeline(self.border, None);
            device.destroy_shader_module(self.vertex, None);
            device.destroy_pipeline_layout(self.layout, None);
            device.destroy_descriptor_set_layout(self.uniform_layout, None);
            device.destroy_descriptor_set_layout(self.texture_layout, None);
            device.destroy_sampler(self.linear, None);
            device.destroy_sampler(self.nearest, None);
            device.destroy_render_pass(self.render_pass, None);
            device.destroy_pipeline_cache(self.cache, None);
        }
    }
}

/// The fragment modules of the shipped passes, destroyed when the pipelines have been made from them.
struct ShaderModules {
    context: Rc<VkContext>,
    /// Solid, surface, shadow, inset shadow and border, in the order [`Pipelines::create`] makes pipelines from them.
    fragments: [vk::ShaderModule; 5],
}

impl ShaderModules {
    fn new(context: &Rc<VkContext>, compiled: &Compiled) -> Result<Self, VulkanError> {
        let mut made = Self {
            context: Rc::clone(context),
            fragments: [vk::ShaderModule::null(); 5],
        };
        for (slot, (name, words)) in made.fragments.iter_mut().zip([
            ("solid", &compiled.solid),
            ("surface", &compiled.surface),
            ("shadow", &compiled.shadow),
            ("inset-shadow", &compiled.inset_shadow),
            ("border", &compiled.border),
        ]) {
            *slot = module(context, name, words)?;
        }
        Ok(made)
    }
}

impl Drop for ShaderModules {
    fn drop(&mut self) {
        // SAFETY: pipelines keep what they need of a module, so it may be destroyed once they
        // are made; a null handle is a no-op.
        unsafe {
            for fragment in self.fragments {
                self.context.device.destroy_shader_module(fragment, None);
            }
        }
    }
}

/// One shader module from SPIR-V words.
pub(super) fn module(context: &VkContext, pass: &'static str, words: &[u32]) -> Result<vk::ShaderModule, VulkanError> {
    let info = vk::ShaderModuleCreateInfo::default().code(words);
    // SAFETY: `words` is a SPIR-V module `naga` wrote and validated.
    unsafe { context.device.create_shader_module(&info, None) }.map_err(|result| VulkanError::Shader {
        pass,
        message: format!("vkCreateShaderModule failed with {result:?}"),
    })
}

/// Descriptor pools that are reset together at the start of every frame.
///
/// A pool holds a fixed number of sets, so a frame with more textured draws than that moves to
/// a second pool instead of failing; the pools are kept, so a steady session allocates none.
pub(super) struct DescriptorArena {
    context: Rc<VkContext>,
    pools: Vec<vk::DescriptorPool>,
    current: usize,
}

impl DescriptorArena {
    pub(super) fn new(context: &Rc<VkContext>) -> Self {
        Self {
            context: Rc::clone(context),
            pools: Vec::new(),
            current: 0,
        }
    }

    /// Makes every set the arena handed out invalid and starts over.
    pub(super) fn reset(&mut self) -> Result<(), vk::Result> {
        for pool in &self.pools {
            // SAFETY: the previous frame's fence was waited, so no set is in use.
            unsafe {
                self.context
                    .device
                    .reset_descriptor_pool(*pool, vk::DescriptorPoolResetFlags::empty())
            }?;
        }
        self.current = 0;
        Ok(())
    }

    /// A set of `layout`, valid until the next [`reset`](Self::reset).
    pub(super) fn allocate(&mut self, layout: vk::DescriptorSetLayout) -> Result<vk::DescriptorSet, vk::Result> {
        let layouts = [layout];
        loop {
            if let Some(pool) = self.pools.get(self.current) {
                let info = vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(*pool)
                    .set_layouts(&layouts);
                // SAFETY: the pool and layout belong to this device.
                match unsafe { self.context.device.allocate_descriptor_sets(&info) } {
                    Ok(sets) => return Ok(sets[0]),
                    Err(vk::Result::ERROR_OUT_OF_POOL_MEMORY | vk::Result::ERROR_FRAGMENTED_POOL) => {
                        self.current += 1;
                        continue;
                    }
                    Err(result) => return Err(result),
                }
            }
            // The uniform set holds two dynamic blocks.
            let sizes = [
                vk::DescriptorPoolSize::default()
                    .ty(vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC)
                    .descriptor_count(POOL_SETS * 2),
                vk::DescriptorPoolSize::default()
                    .ty(vk::DescriptorType::SAMPLED_IMAGE)
                    .descriptor_count(POOL_SETS),
                vk::DescriptorPoolSize::default()
                    .ty(vk::DescriptorType::SAMPLER)
                    .descriptor_count(POOL_SETS),
            ];
            let info = vk::DescriptorPoolCreateInfo::default()
                .max_sets(POOL_SETS)
                .pool_sizes(&sizes);
            // SAFETY: `info` is a valid description.
            let pool = unsafe { self.context.device.create_descriptor_pool(&info, None) }?;
            self.pools.push(pool);
        }
    }
}

impl Drop for DescriptorArena {
    fn drop(&mut self) {
        for pool in &self.pools {
            // SAFETY: the renderer is idle; destroying a pool frees its sets.
            unsafe { self.context.device.destroy_descriptor_pool(*pool, None) };
        }
    }
}

/// The size of the shared uniform block a descriptor binds, in bytes: room for the largest block any pass reads, the 128
/// bytes of the view pass; the layers' own is 112.
pub(super) const BLOCK_RANGE: u64 = 128;

/// The size of a shell's shader pass's own block a descriptor binds, in bytes: `GnoblinUser` in `user_block.wgsl`.
pub(super) const USER_BLOCK_RANGE: u64 = USER_BLOCK_BYTES as u64;

/// A shell's shader pass as a pipeline.
#[derive(Clone, Copy, Debug)]
pub(super) struct UserPipeline {
    /// The pipeline: the shared vertex stage and the shell's fragment stage.
    pub pipeline: vk::Pipeline,
    /// Whether the fragment stage reads the backdrop, so a draw binds group 2.
    pub backdrop: bool,
}

/// The code of a source the validator or compiler refuses: what GLES reports for the same.
pub(super) const COMPILE_FAILED: &str = "compile-failed";

/// The code of a pipeline the driver refused to make.
pub(super) const PIPELINE_FAILED: &str = "pipeline-failed";

/// The code of a pass whose pipeline is being built on the worker: the node is drawn without it until the build is done.
pub(super) const COMPILING: &str = "compiling";

/// The code of a pass left out because the frame already draws as many distinct sources as the cache holds.
pub(super) const CACHE_FULL: &str = "cache-full";

/// Why a source could not be made into a pipeline: a code for the node's `skipped` list, and the words for the log.
struct PipelineRefusal {
    code: &'static str,
    reason: String,
}

/// A source for the worker to build.
struct BuildRequest {
    key: ShaderKey,
    source: std::sync::Arc<str>,
    /// A test's stand-in for a slow driver: how long the worker waits before it builds.
    delay: Duration,
    /// A test's stand-in for a driver that refuses the pipeline.
    fail: Option<vk::Result>,
}

/// What the worker built.
struct BuildResult {
    key: ShaderKey,
    outcome: Result<UserPipeline, PipelineRefusal>,
}

/// The thread that builds pipelines, and the two ends of its queues.
struct Worker {
    requests: Sender<BuildRequest>,
    results: Receiver<BuildResult>,
    thread: JoinHandle<()>,
}

/// Compiles `source` and makes its pipeline. This runs on the worker thread, and uses only handles of the device.
fn build(ctx: &GraphicsContext, source: &str, fail: Option<vk::Result>) -> Result<UserPipeline, PipelineRefusal> {
    let compiled = compile_user(source).map_err(|reason| PipelineRefusal {
        code: COMPILE_FAILED,
        reason,
    })?;
    let driver = |reason: String| PipelineRefusal {
        code: PIPELINE_FAILED,
        reason,
    };
    if let Some(result) = fail {
        return Err(driver(format!("vkCreateGraphicsPipelines failed with {result:?}")));
    }
    let info = vk::ShaderModuleCreateInfo::default().code(&compiled.words);
    // SAFETY: `words` is a SPIR-V module `naga` wrote and validated; the device outlives the worker (it is joined first).
    let fragment = unsafe { ctx.device.create_shader_module(&info, None) }
        .map_err(|result| driver(format!("vkCreateShaderModule failed with {result:?}")))?;
    let made = ctx.build(&[fragment]);
    // SAFETY: a pipeline keeps what it needs of a module, so the module is destroyed once the pipeline is made.
    unsafe { ctx.device.destroy_shader_module(fragment, None) };
    let pipeline = made.map_err(|error| driver(error.to_string()))?[0];
    Ok(UserPipeline {
        pipeline,
        backdrop: compiled.backdrop,
    })
}

/// The worker's loop: build what is asked for, send it back and ask for a frame, until the renderer closes the queue.
fn work(
    ctx: &GraphicsContext,
    requests: &Receiver<BuildRequest>,
    results: &Sender<BuildResult>,
    listeners: &Mutex<Vec<RedrawSignal>>,
) {
    let mut since_saved = 0_u32;
    while let Ok(request) = requests.recv() {
        if !request.delay.is_zero() {
            std::thread::sleep(request.delay);
        }
        let outcome = build(ctx, &request.source, request.fail);
        let built = outcome.is_ok();
        if let Err(failed) = results.send(BuildResult {
            key: request.key,
            outcome,
        }) {
            // The renderer stopped listening: nobody will destroy what was built.
            if let Ok(made) = failed.0.outcome {
                // SAFETY: nothing recorded a draw with a pipeline nobody has seen.
                unsafe { ctx.device.destroy_pipeline(made.pipeline, None) };
            }
            return;
        }
        // The cache is the worker's to read while it runs, so this is where it is written out: after a run of builds, not
        // for each one.
        since_saved += u32::from(built);
        if since_saved >= PIPELINES_PER_SAVE {
            since_saved = 0;
            ctx.save_cache();
        }
        for signal in listeners.lock().unwrap_or_else(PoisonError::into_inner).iter() {
            signal.request();
        }
    }
}

/// The pipelines of the shell's shader passes: one per source, built on a worker thread the first frame that draws it.
///
/// The renderer builds the shipped pipelines when it starts, but a shell's source is only known when it writes it. Compiling it
/// and making its pipeline takes milliseconds to tens of milliseconds, so the draw never waits: the frame that first meets a
/// source asks the worker for it and draws the node without the pass (reported as `compiling`), and when the worker is done it
/// asks for another frame ([`RedrawSignal`]) and the next draw finds the pipeline in the shared [`ShaderCache`] (64 entries,
/// least recently used). A source that does not compile is kept too, as its code, so a broken shader costs one compile.
///
/// A frame that draws more distinct sources than the cache holds does not evict what it has used: the extras are left out as
/// `cache-full`, so the same 64 stay built and nothing is made again every frame.
///
/// # When a pipeline is destroyed
///
/// A frame is submitted and not waited for, so a pipeline is in use from its recording until the frame's timeline value is
/// reached. Pipelines the cache evicts are taken by [`ShaderPipelines::take_retired`] and destroyed through the retire queue
/// once the last frame that may have drawn with them has finished. A lost device, or dropping the device, closes the worker's
/// queue and joins it before anything is destroyed, so the worker never touches a device that is gone, then
/// [`ShaderPipelines::drain`] destroys what was built (the cache and the results nobody read).
pub(super) struct ShaderPipelines {
    context: Rc<VkContext>,
    cache: ShaderCache<Result<UserPipeline, &'static str>>,
    retired: Vec<vk::Pipeline>,
    /// The sources the worker has been asked for and has not answered.
    pending: Vec<ShaderKey>,
    /// The distinct sources this frame has asked about.
    touched: Vec<ShaderKey>,
    worker: Option<Worker>,
    /// The signal of every output on the device: the worker asks each for another frame, as any of them may draw the node.
    listeners: Arc<Mutex<Vec<RedrawSignal>>>,
    /// What the worker builds with: handles of the device, copied from the shipped pipelines.
    graphics: GraphicsContext,
}

impl ShaderPipelines {
    pub(super) fn new(context: &Rc<VkContext>, pipelines: &Pipelines) -> Self {
        Self {
            context: Rc::clone(context),
            cache: ShaderCache::new(),
            retired: Vec::new(),
            pending: Vec::new(),
            touched: Vec::new(),
            worker: None,
            listeners: Arc::new(Mutex::new(Vec::new())),
            graphics: pipelines.graphics_context(),
        }
    }

    /// Makes `signal` one the worker asks for another frame on, when a pipeline it built is ready.
    pub(super) fn listen(&self, signal: RedrawSignal) {
        self.listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(signal);
    }

    /// Starts a frame: takes what the worker has finished into the cache, and forgets what the last frame asked about.
    pub(super) fn begin_frame(&mut self) {
        self.touched.clear();
        let Some(worker) = &self.worker else { return };
        while let Ok(done) = worker.results.try_recv() {
            self.pending.retain(|key| *key != done.key);
            let outcome = match done.outcome {
                Ok(made) => Ok(made),
                Err(refusal) => {
                    log::warn!(
                        area = "render",
                        key = done.key.get(),
                        code = refusal.code,
                        reason = refusal.reason.as_str(),
                        "a shader pass was refused; its node is drawn without it"
                    );
                    Err(refusal.code)
                }
            };
            if let Some((_, Ok(evicted))) = self.cache.insert(done.key, outcome) {
                self.retired.push(evicted.pipeline);
            }
        }
    }

    /// The pipeline for `pass`'s source, or the code of why this frame cannot draw it.
    ///
    /// A source the cache has not seen is sent to the worker and answers [`COMPILING`] until it is built. A refusal is kept in
    /// the cache, so a broken source is not compiled again. The detail (the shell's line and column, the compiler's or the
    /// driver's words) goes to the log once; the code is what the node's `skipped` list shows.
    ///
    /// # Errors
    ///
    /// [`COMPILING`], [`CACHE_FULL`], [`COMPILE_FAILED`] or [`PIPELINE_FAILED`].
    pub(super) fn resolve(&mut self, pass: &ShaderPass) -> Result<UserPipeline, &'static str> {
        let key = pass.key();
        if !self.touched.contains(&key) {
            if self.touched.len() >= SHADER_CACHE_ENTRIES {
                return Err(CACHE_FULL);
            }
            self.touched.push(key);
        }
        if let Some(held) = self.cache.get(key) {
            return *held;
        }
        if self.pending.contains(&key) {
            return Err(COMPILING);
        }
        let request = BuildRequest {
            key,
            source: std::sync::Arc::clone(pass.shared_source()),
            #[cfg(test)]
            delay: self.context.faults.build_delay.take().unwrap_or(Duration::ZERO),
            #[cfg(not(test))]
            delay: Duration::ZERO,
            #[cfg(test)]
            fail: self.context.faults.pipeline.take(),
            #[cfg(not(test))]
            fail: None,
        };
        if self.worker.is_none() {
            self.worker = self.start();
        }
        match &self.worker {
            Some(worker) if worker.requests.send(request).is_ok() => {
                self.pending.push(key);
                Err(COMPILING)
            }
            _ => Err(PIPELINE_FAILED),
        }
    }

    /// Starts the worker thread, or `None` when the system will not give one.
    fn start(&self) -> Option<Worker> {
        let (requests, request_queue) = std::sync::mpsc::channel();
        let (result_queue, results) = std::sync::mpsc::channel();
        let graphics = self.graphics.clone();
        let listeners = Arc::clone(&self.listeners);
        let thread = std::thread::Builder::new()
            .name("gnoblin-shader".to_owned())
            .spawn(move || work(&graphics, &request_queue, &result_queue, &listeners))
            .map_err(|error| log::warn!(area = "render", %error, "the shader worker could not start"))
            .ok()?;
        Some(Worker {
            requests,
            results,
            thread,
        })
    }

    /// How many sources the cache holds, refused ones included.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.cache.len()
    }

    /// How many sources the worker has not answered.
    #[cfg(test)]
    pub(super) fn pending(&self) -> usize {
        self.pending.len()
    }

    /// Whether the cache holds a source, without counting it as a use.
    #[cfg(test)]
    pub(super) fn contains(&self, key: gnoblin_render::ShaderKey) -> bool {
        self.cache.contains(key)
    }

    /// Takes the pipelines the cache evicted, for the caller to retire with the value of the last frame submitted: a frame in
    /// flight may still draw with one.
    pub(super) fn take_retired(&mut self) -> Vec<vk::Pipeline> {
        std::mem::take(&mut self.retired)
    }

    /// Destroys every pipeline, for a renderer that lost its device or is going away. Stops the worker and waits for it and
    /// for the queue first.
    pub(super) fn drain(&mut self) {
        if let Some(Worker {
            requests,
            results,
            thread,
        }) = self.worker.take()
        {
            // Closing the queue ends the worker after the build it is in, and the join is what makes the device safe to
            // destroy: nothing of the worker's runs after it.
            drop(requests);
            let _ = thread.join();
            for done in results.try_iter() {
                if let Ok(made) = done.outcome {
                    // SAFETY: nothing recorded a draw with a pipeline nobody had read.
                    unsafe { self.context.device.destroy_pipeline(made.pipeline, None) };
                }
            }
        }
        self.pending.clear();
        self.context.wait_idle();
        for pipeline in self.retired.drain(..) {
            // SAFETY: the queue is idle, so no command buffer uses the pipeline.
            unsafe { self.context.device.destroy_pipeline(pipeline, None) };
        }
        for (_, held) in self.cache.drain() {
            if let Ok(held) = held {
                // SAFETY: the queue is idle, so no command buffer uses the pipeline.
                unsafe { self.context.device.destroy_pipeline(held.pipeline, None) };
            }
        }
    }
}

impl Drop for ShaderPipelines {
    fn drop(&mut self) {
        self.drain();
    }
}

/// A pipeline the shader cache evicted, kept until the last frame that may draw with it has finished.
pub(super) struct RetiredPipeline {
    context: Rc<VkContext>,
    pipeline: vk::Pipeline,
}

impl RetiredPipeline {
    pub(super) fn new(context: &Rc<VkContext>, pipeline: vk::Pipeline) -> Self {
        Self {
            context: Rc::clone(context),
            pipeline,
        }
    }
}

impl Drop for RetiredPipeline {
    fn drop(&mut self) {
        // SAFETY: the retire queue held it until the timeline passed the last frame that drew with it.
        unsafe { self.context.device.destroy_pipeline(self.pipeline, None) };
    }
}

/// Descriptor sets that live as long as the texture they bind, from pools that free single sets.
///
/// A frame's own sets come from an arena that is reset wholesale, which is right for sets that last one frame. A cached
/// texture's last until its surface goes, so each is allocated once and freed with its texture, and a steady frame writes
/// none.
pub(super) struct TextureSets {
    context: Rc<VkContext>,
    /// Each pool, and how many of its sets are in use.
    pools: RefCell<Vec<(vk::DescriptorPool, u32)>>,
}

impl TextureSets {
    fn new(context: &Rc<VkContext>) -> Self {
        Self {
            context: Rc::clone(context),
            pools: RefCell::new(Vec::new()),
        }
    }

    /// A set of `layout`, which goes back to its pool when the [`LongSet`] is dropped.
    ///
    /// # Errors
    ///
    /// The driver's result.
    pub(super) fn allocate(self: &Rc<Self>, layout: vk::DescriptorSetLayout) -> Result<LongSet, vk::Result> {
        let layouts = [layout];
        let device = &self.context.device;
        let mut pools = self.pools.borrow_mut();
        loop {
            for (index, (pool, used)) in pools.iter_mut().enumerate() {
                if *used >= POOL_SETS {
                    continue;
                }
                let info = vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(*pool)
                    .set_layouts(&layouts);
                // SAFETY: the pool and layout belong to this device.
                match unsafe { device.allocate_descriptor_sets(&info) } {
                    Ok(sets) => {
                        *used += 1;
                        return Ok(LongSet {
                            owner: Rc::clone(self),
                            pool: index,
                            set: sets[0],
                        });
                    }
                    Err(vk::Result::ERROR_OUT_OF_POOL_MEMORY | vk::Result::ERROR_FRAGMENTED_POOL) => continue,
                    Err(result) => return Err(result),
                }
            }
            let sizes = [
                vk::DescriptorPoolSize::default()
                    .ty(vk::DescriptorType::SAMPLED_IMAGE)
                    .descriptor_count(POOL_SETS),
                vk::DescriptorPoolSize::default()
                    .ty(vk::DescriptorType::SAMPLER)
                    .descriptor_count(POOL_SETS),
            ];
            let info = vk::DescriptorPoolCreateInfo::default()
                .flags(vk::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET)
                .max_sets(POOL_SETS)
                .pool_sizes(&sizes);
            // SAFETY: `info` is a valid description.
            let pool = unsafe { device.create_descriptor_pool(&info, None) }?;
            pools.push((pool, 0));
        }
    }

    fn free(&self, pool: usize, set: vk::DescriptorSet) {
        let mut pools = self.pools.borrow_mut();
        if let Some((handle, used)) = pools.get_mut(pool) {
            // SAFETY: the texture the set belonged to has been retired, so no frame in flight uses it.
            let _ = unsafe { self.context.device.free_descriptor_sets(*handle, &[set]) };
            *used = used.saturating_sub(1);
        }
    }

    /// How many sets are in use.
    #[cfg(test)]
    pub(super) fn in_use(&self) -> u32 {
        self.pools.borrow().iter().map(|(_, used)| *used).sum()
    }
}

impl Drop for TextureSets {
    fn drop(&mut self) {
        for (pool, _) in self.pools.borrow().iter() {
            // SAFETY: every set is freed with the texture it belonged to, and the device is idle or the textures retired.
            unsafe { self.context.device.destroy_descriptor_pool(*pool, None) };
        }
    }
}

/// A descriptor set owned by a cached texture.
pub(super) struct LongSet {
    owner: Rc<TextureSets>,
    pool: usize,
    pub(super) set: vk::DescriptorSet,
}

impl Drop for LongSet {
    fn drop(&mut self) {
        self.owner.free(self.pool, self.set);
    }
}

/// How many pipelines the shader worker builds between writes of the cache file.
pub(super) const PIPELINES_PER_SAVE: u32 = 16;

/// The largest cache file that is written, in bytes. A cache that has grown past it is not kept.
pub(super) const MAX_CACHE_BYTES: usize = 32 << 20;

/// The magic that starts a cache file.
const CACHE_MAGIC: [u8; 4] = *b"GNVC";

/// The format of the file's own header, raised when it changes.
const CACHE_VERSION: u32 = 1;

/// The bytes of the file's header: magic, version, length of the driver's blob and its CRC32.
const CACHE_HEADER: usize = 20;

/// The bytes of the header the Vulkan specification puts first in every pipeline cache blob: its size, version, vendor,
/// device and the 16 bytes of the cache UUID.
const BLOB_HEADER: usize = 32;

/// Where the driver's pipeline cache is kept between runs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PipelineCacheLocation {
    /// `$XDG_CACHE_HOME/gnoblin/vulkan/`, or `~/.cache/gnoblin/vulkan/` without it, in a file named by the cache UUID of the
    /// device. With neither variable set, the cache is not kept.
    #[default]
    Default,
    /// Not kept: every run compiles its pipelines from nothing.
    Off,
    /// In this directory, which a test makes.
    Directory(PathBuf),
}

impl PipelineCacheLocation {
    /// The file for a device, or `None` when the cache is not kept.
    pub(super) fn file(&self, identity: &CacheIdentity) -> Option<PathBuf> {
        let directory = match self {
            Self::Off => return None,
            Self::Directory(directory) => directory.clone(),
            Self::Default => default_directory()?,
        };
        let uuid: String = identity.uuid.iter().map(|byte| format!("{byte:02x}")).collect();
        Some(directory.join(format!("{uuid}.bin")))
    }
}

/// The directory of [`PipelineCacheLocation::Default`].
fn default_directory() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".cache"))
        })?;
    Some(base.join("gnoblin").join("vulkan"))
}

/// The table the CRC32 is computed from.
static CRC_TABLE: LazyLock<[u32; 256]> = LazyLock::new(|| {
    let mut table = [0_u32; 256];
    for (index, slot) in table.iter_mut().enumerate() {
        let mut value = index as u32;
        for _ in 0..8 {
            value = if value & 1 == 1 {
                0xEDB8_8320 ^ (value >> 1)
            } else {
                value >> 1
            };
        }
        *slot = value;
    }
    table
});

/// The CRC32 (IEEE) of `bytes`.
fn crc32(bytes: &[u8]) -> u32 {
    !bytes.iter().fold(!0_u32, |crc, byte| {
        CRC_TABLE[((crc ^ u32::from(*byte)) & 0xFF) as usize] ^ (crc >> 8)
    })
}

/// The file for a driver's blob: our header, then the blob.
pub(super) fn encode_cache(blob: &[u8]) -> Vec<u8> {
    let mut file = Vec::with_capacity(CACHE_HEADER + blob.len());
    file.extend_from_slice(&CACHE_MAGIC);
    file.extend_from_slice(&CACHE_VERSION.to_le_bytes());
    file.extend_from_slice(&(blob.len() as u64).to_le_bytes());
    file.extend_from_slice(&crc32(blob).to_le_bytes());
    file.extend_from_slice(blob);
    file
}

/// The driver's blob in a cache file, when the file is ours, whole, intact and of this device.
///
/// # Errors
///
/// Why the file is refused.
pub(super) fn decode_cache<'a>(file: &'a [u8], identity: &CacheIdentity) -> Result<&'a [u8], &'static str> {
    if file.len() > MAX_CACHE_BYTES {
        return Err("it is larger than the cache may be");
    }
    let header = file.get(..CACHE_HEADER).ok_or("it is shorter than its header")?;
    if header[..4] != CACHE_MAGIC {
        return Err("it is not a Gnoblin cache");
    }
    let word = |from: usize| u32::from_le_bytes([header[from], header[from + 1], header[from + 2], header[from + 3]]);
    if word(4) != CACHE_VERSION {
        return Err("it is of another format version");
    }
    let length = u64::from_le_bytes([
        header[8], header[9], header[10], header[11], header[12], header[13], header[14], header[15],
    ]);
    let blob = &file[CACHE_HEADER..];
    if length != blob.len() as u64 {
        return Err("its length is not the one it says");
    }
    if crc32(blob) != word(16) {
        return Err("its checksum does not match");
    }
    let blob_word = |from: usize| u32::from_le_bytes([blob[from], blob[from + 1], blob[from + 2], blob[from + 3]]);
    if blob.len() < BLOB_HEADER || blob_word(4) != 1 {
        return Err("the driver's blob has no header of version one");
    }
    if blob_word(8) != identity.vendor || blob_word(12) != identity.device || blob[16..32] != identity.uuid {
        return Err("it was made by another device or driver");
    }
    Ok(blob)
}

/// The driver's blob kept in `path`, or `None` when there is no file or it is refused. A refused file is deleted, so the next
/// write replaces it.
fn load_cache(path: &Path, identity: &CacheIdentity) -> Option<Vec<u8>> {
    let file = match std::fs::read(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            log::warn!(area = "render", path = %path.display(), %error, "the pipeline cache file could not be read");
            return None;
        }
    };
    match decode_cache(&file, identity) {
        Ok(blob) => Some(blob.to_vec()),
        Err(reason) => {
            log::warn!(area = "render", path = %path.display(), reason, "the pipeline cache file is ignored and deleted");
            let _ = std::fs::remove_file(path);
            None
        }
    }
}

/// Writes `blob` to `path` through a temporary file and a rename, so a reader never sees half a file.
fn save_cache_file(path: &Path, blob: &[u8]) -> std::io::Result<()> {
    if blob.len() + CACHE_HEADER > MAX_CACHE_BYTES {
        return Err(std::io::Error::other("the cache is larger than the cache file may be"));
    }
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory)?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(".{}.tmp", std::process::id()));
    let temporary = PathBuf::from(temporary);
    let written = std::fs::write(&temporary, encode_cache(blob)).and_then(|()| std::fs::rename(&temporary, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written
}

impl GraphicsContext {
    /// Writes the driver's cache to its file, when it is kept. The cache is externally synchronised: this is called by the
    /// shader worker between builds, or by the thread that owns the device after the worker has been joined.
    pub(super) fn save_cache(&self) {
        let Some(path) = &self.cache_file else { return };
        if self.cache == vk::PipelineCache::null() {
            return;
        }
        // SAFETY: no other thread uses the cache while this runs, by the contract above.
        let Ok(blob) = (unsafe { self.device.get_pipeline_cache_data(self.cache) }) else {
            return;
        };
        if let Err(error) = save_cache_file(path, &blob) {
            log::warn!(area = "render", path = %path.display(), %error, "the pipeline cache could not be written");
        }
    }
}
