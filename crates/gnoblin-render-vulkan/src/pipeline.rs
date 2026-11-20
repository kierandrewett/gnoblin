#![allow(unsafe_code)]
//! The render pass, the pipelines, the samplers and the descriptor pools.
//!
//! One render pass draws into the target and keeps what is already there (`LOAD`), because a
//! frame repaints only its damage. Two pipelines share a layout and a vertex stage: `solid`
//! and `surface`, which are the GLES renderer's passes of the same names. Both blend
//! premultiplied colour (`ONE`, `ONE_MINUS_SRC_ALPHA`, for colour and for alpha), which is
//! what the software renderer's compositing and every GLES pass agree on.
//!
//! A shell's own shader pass is a third kind of pipeline, one per source (`ShaderPipelines`): the same vertex stage, state and
//! layout, with a fragment module compiled from the validated WGSL when a frame first draws the source.
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

use std::rc::Rc;
use std::time::Instant;

use ash::vk;

use super::context::{TARGET_FORMAT, VkContext};
use super::error::VulkanError;
use super::shader::{StageKind, compile, compile_user};
use crate::log;
use gnoblin_render::ShaderPass;
use gnoblin_render_frame::shader::ShaderCache;
use gnoblin_render_frame::shader::uniforms::USER_BLOCK_BYTES;

/// The WGSL of each module of the two pipelines.
#[derive(Clone, Copy, Debug)]
pub(super) struct ShaderSources<'a> {
    /// The vertex stage, shared by every pass.
    pub vertex: &'a str,
    /// The solid-colour fragment stage (also used for the background).
    pub solid: &'a str,
    /// The textured fragment stage for client surfaces and bitmaps.
    pub surface: &'a str,
}

impl ShaderSources<'static> {
    /// The sources the GLES renderer draws with.
    pub(super) fn shared() -> Self {
        use gnoblin_render_frame::shader::sources::{SOLID, SURFACE, VERTEX};
        Self {
            vertex: VERTEX,
            solid: SOLID,
            surface: SURFACE,
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
        })
    }
}

/// The size of one uniform block, in bytes: the preamble's `Gnoblin` struct.
pub(super) const UNIFORM_BYTES: usize = 96;

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
    pub(super) linear: vk::Sampler,
    pub(super) nearest: vk::Sampler,
    /// The driver's cache of compiled pipelines, null when it could not be made.
    cache: vk::PipelineCache,
}

impl Pipelines {
    /// Creates everything a frame draws with, from modules compiled by [`Compiled::new`].
    ///
    /// # Errors
    ///
    /// [`VulkanError::Shader`] when the driver refuses a module, and [`VulkanError::Driver`]
    /// for any other refused creation.
    pub(super) fn new(context: &Rc<VkContext>, compiled: &Compiled) -> Result<Self, VulkanError> {
        let mut made = Self {
            context: Rc::clone(context),
            render_pass: vk::RenderPass::null(),
            uniform_layout: vk::DescriptorSetLayout::null(),
            texture_layout: vk::DescriptorSetLayout::null(),
            layout: vk::PipelineLayout::null(),
            vertex: vk::ShaderModule::null(),
            solid: vk::Pipeline::null(),
            surface: vk::Pipeline::null(),
            linear: vk::Sampler::null(),
            nearest: vk::Sampler::null(),
            cache: vk::PipelineCache::null(),
        };
        made.vertex = module(context, "vertex", &compiled.vertex)?;
        let modules = ShaderModules::new(context, &compiled.solid, &compiled.surface)?;
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
        // An in-memory cache for every pipeline, so a source the shader cache evicted and a frame draws again is not
        // compiled by the driver from nothing. A driver that cannot make one costs the cache and not the renderer.
        // SAFETY: a plain cache with no initial data.
        self.cache = unsafe { device.create_pipeline_cache(&vk::PipelineCacheCreateInfo::default(), None) }
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

        let made = self.graphics(&[modules.solid, modules.surface])?;
        self.solid = made[0];
        self.surface = made[1];
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
        let device = &self.context.device;
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
        let blend_attachments = [vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::ONE)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(vk::ColorComponentFlags::RGBA)];
        let blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_attachments);
        let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
        let infos: Vec<vk::GraphicsPipelineCreateInfo> = stage_sets
            .iter()
            .map(|stages| {
                vk::GraphicsPipelineCreateInfo::default()
                    .stages(stages)
                    .vertex_input_state(&vertex_input)
                    .input_assembly_state(&assembly)
                    .viewport_state(&viewport)
                    .rasterization_state(&raster)
                    .multisample_state(&multisample)
                    .color_blend_state(&blend)
                    .dynamic_state(&dynamic)
                    .layout(self.layout)
                    .render_pass(self.render_pass)
                    .subpass(0)
            })
            .collect();
        // SAFETY: every state structure above outlives the call and matches the render pass.
        let made = unsafe { device.create_graphics_pipelines(self.cache, &infos, None) };
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
        let device = &self.context.device;
        // SAFETY: the renderer waited for its last frame, and every handle is null or was made on
        // this device. Destroying a null handle is a no-op.
        unsafe {
            device.destroy_pipeline(self.solid, None);
            device.destroy_pipeline(self.surface, None);
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

/// The two fragment modules of the shipped passes, destroyed when the pipelines have been made from them.
struct ShaderModules {
    context: Rc<VkContext>,
    solid: vk::ShaderModule,
    surface: vk::ShaderModule,
}

impl ShaderModules {
    fn new(context: &Rc<VkContext>, solid: &[u32], surface: &[u32]) -> Result<Self, VulkanError> {
        let mut made = Self {
            context: Rc::clone(context),
            solid: vk::ShaderModule::null(),
            surface: vk::ShaderModule::null(),
        };
        made.solid = module(context, "solid", solid)?;
        made.surface = module(context, "surface", surface)?;
        Ok(made)
    }
}

impl Drop for ShaderModules {
    fn drop(&mut self) {
        // SAFETY: pipelines keep what they need of a module, so it may be destroyed once they
        // are made; a null handle is a no-op.
        unsafe {
            self.context.device.destroy_shader_module(self.solid, None);
            self.context.device.destroy_shader_module(self.surface, None);
        }
    }
}

/// One shader module from SPIR-V words.
fn module(context: &VkContext, pass: &'static str, words: &[u32]) -> Result<vk::ShaderModule, VulkanError> {
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

/// The size of the shared uniform block a descriptor binds, in bytes.
pub(super) const BLOCK_RANGE: u64 = UNIFORM_BYTES as u64;

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

/// Why a source could not be made into a pipeline: a code for the node's `skipped` list, and the words for the log.
struct PipelineRefusal {
    code: &'static str,
    reason: String,
}

/// The pipelines of the shell's shader passes: one per source, made when a frame first draws it.
///
/// The renderer builds the shipped pipelines when it starts, but a shell's source is only known when it writes it, so a
/// pipeline is compiled and created at draw time and kept in the shared [`ShaderCache`] (64 entries, least recently used).
/// A source that does not compile is kept too, as its reason, so a broken shader costs one compile and not one a frame.
///
/// # When a pipeline is destroyed
///
/// A frame is recorded into one command buffer and the renderer waits for its fence before `draw` returns, so a pipeline is
/// in use only between recording and that wait. A pipeline the cache evicts while a frame is being recorded may already be
/// in that frame's commands (the frame draws more than 64 sources), so eviction does not destroy it: it is retired, and
/// [`ShaderPipelines::release_retired`] destroys it once the frame has finished. A lost device drains the lot
/// ([`ShaderPipelines::drain`]) after waiting for the queue, and dropping the renderer waits first.
pub(super) struct ShaderPipelines {
    context: Rc<VkContext>,
    cache: ShaderCache<Result<UserPipeline, &'static str>>,
    retired: Vec<vk::Pipeline>,
}

impl ShaderPipelines {
    pub(super) fn new(context: &Rc<VkContext>) -> Self {
        Self {
            context: Rc::clone(context),
            cache: ShaderCache::new(),
            retired: Vec::new(),
        }
    }

    /// The pipeline for `pass`'s source, made when the cache has none, or the code of why it cannot be drawn.
    ///
    /// A refusal that is the source's own (the validator refuses it, or it does not compile) is kept in the cache, so a broken
    /// source costs one compile and not one a frame. A refusal that is the driver's (it ran out of memory making the
    /// pipeline) is not: the next frame tries again, so a shader that failed for want of memory comes back by itself. The
    /// detail (the shell's line and column, the compiler's or the driver's words) goes to the log once; the code is what the
    /// node's `skipped` list shows.
    ///
    /// # Errors
    ///
    /// [`COMPILE_FAILED`] or [`PIPELINE_FAILED`].
    pub(super) fn resolve(&mut self, pipelines: &Pipelines, pass: &ShaderPass) -> Result<UserPipeline, &'static str> {
        let key = pass.key();
        if let Some(held) = self.cache.get(key) {
            return *held;
        }
        let started = Instant::now();
        let made = self.create(pipelines, pass.source());
        match &made {
            Ok(_) => log::debug!(
                area = "render",
                key = key.get(),
                micros = started.elapsed().as_micros() as u64,
                "a shader pipeline was made"
            ),
            Err(refusal) => log::warn!(
                area = "render",
                key = key.get(),
                code = refusal.code,
                reason = refusal.reason.as_str(),
                "a shader pass was refused; its node is drawn without it"
            ),
        }
        let outcome = made.map_err(|refusal| refusal.code);
        if matches!(outcome, Ok(_) | Err(COMPILE_FAILED))
            && let Some((_, Ok(evicted))) = self.cache.insert(key, outcome)
        {
            self.retired.push(evicted.pipeline);
        }
        outcome
    }

    fn create(&self, pipelines: &Pipelines, source: &str) -> Result<UserPipeline, PipelineRefusal> {
        let compiled = compile_user(source).map_err(|reason| PipelineRefusal {
            code: COMPILE_FAILED,
            reason,
        })?;
        let driver = |reason: String| PipelineRefusal {
            code: PIPELINE_FAILED,
            reason,
        };
        #[cfg(test)]
        if let Some(result) = self.context.faults.pipeline.take() {
            return Err(driver(format!("vkCreateGraphicsPipelines failed with {result:?}")));
        }
        let fragment = module(&self.context, "shader", &compiled.words).map_err(|error| driver(error.to_string()))?;
        let made = pipelines.graphics(&[fragment]);
        // SAFETY: a pipeline keeps what it needs of a module, so the module is destroyed once the pipeline is made.
        unsafe { self.context.device.destroy_shader_module(fragment, None) };
        let pipeline = made.map_err(|error| driver(error.to_string()))?[0];
        Ok(UserPipeline {
            pipeline,
            backdrop: compiled.backdrop,
        })
    }

    /// How many sources the cache holds, refused ones included.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.cache.len()
    }

    /// Whether the cache holds a source, without counting it as a use.
    #[cfg(test)]
    pub(super) fn contains(&self, key: gnoblin_render::ShaderKey) -> bool {
        self.cache.contains(key)
    }

    /// Destroys the pipelines the cache evicted. The caller has waited for the frame that may have used them.
    pub(super) fn release_retired(&mut self) {
        for pipeline in self.retired.drain(..) {
            // SAFETY: the frame that recorded a draw with it has finished.
            unsafe { self.context.device.destroy_pipeline(pipeline, None) };
        }
    }

    /// Destroys every pipeline, for a renderer that lost its device or is going away. Waits for the queue first.
    pub(super) fn drain(&mut self) {
        self.context.wait_idle();
        self.release_retired();
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
