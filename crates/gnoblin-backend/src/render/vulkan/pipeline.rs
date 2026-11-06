#![allow(unsafe_code)]
//! The render pass, the pipelines, the samplers and the descriptor pools.
//!
//! One render pass draws into the target and keeps what is already there (`LOAD`), because a
//! frame repaints only its damage. Two pipelines share a layout and a vertex stage: `solid`
//! and `surface`, which are the GLES renderer's passes of the same names. Both blend
//! premultiplied colour (`ONE`, `ONE_MINUS_SRC_ALPHA`, for colour and for alpha), which is
//! what the software renderer's compositing and every GLES pass agree on.
//!
//! # Bindings
//!
//! | Set | Binding | What                                   | Stage    |
//! | --- | ------- | -------------------------------------- | -------- |
//! | 0   | 0       | the pass's uniform block, dynamic offset | fragment |
//! | 1   | 0       | the layer's texture                    | fragment |
//! | 1   | 1       | its sampler                            | fragment |
//!
//! These are the `@group` and `@binding` numbers the WGSL preamble declares, which `naga`
//! carries into the SPIR-V as descriptor sets, so the layout here is the preamble's.
//!
//! # The vertex input
//!
//! Binding 0 is the unit quad, four vertices of a strip. Binding 1 is per instance: the
//! rectangle, the viewport and the optional projection, 22 floats at the locations the
//! vertex stage declares (see `plan.rs`).

use std::rc::Rc;

use ash::vk;

use super::context::{TARGET_FORMAT, VkContext};
use super::error::VulkanError;
use super::shader::{StageKind, compile};

/// The WGSL of each module of the two pipelines.
#[derive(Clone, Copy, Debug)]
pub(super) struct ShaderSources<'a> {
    pub vertex: &'a str,
    pub solid: &'a str,
    pub surface: &'a str,
}

impl ShaderSources<'static> {
    /// The sources the GLES renderer draws with.
    pub(super) fn shared() -> Self {
        use crate::render::shader::sources::{SOLID, SURFACE, VERTEX};
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
    pub(super) solid: vk::Pipeline,
    pub(super) surface: vk::Pipeline,
    pub(super) linear: vk::Sampler,
    pub(super) nearest: vk::Sampler,
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
            solid: vk::Pipeline::null(),
            surface: vk::Pipeline::null(),
            linear: vk::Sampler::null(),
            nearest: vk::Sampler::null(),
        };
        let modules = ShaderModules::new(context, &compiled.vertex, &compiled.solid, &compiled.surface)?;
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

        let uniform_bindings = [vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT)];
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
        let set_layouts = [self.uniform_layout, self.texture_layout];
        // SAFETY: both layouts were created just above.
        self.layout = unsafe {
            device.create_pipeline_layout(&vk::PipelineLayoutCreateInfo::default().set_layouts(&set_layouts), None)
        }
        .map_err(|result| VulkanError::driver("vkCreatePipelineLayout", result))?;

        self.linear = sampler(device, vk::Filter::LINEAR)?;
        self.nearest = sampler(device, vk::Filter::NEAREST)?;

        let entry = c"gnoblin_entry";
        let stages = |fragment: vk::ShaderModule| {
            [
                vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::VERTEX)
                    .module(modules.vertex)
                    .name(entry),
                vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::FRAGMENT)
                    .module(fragment)
                    .name(entry),
            ]
        };
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
        let solid_stages = stages(modules.solid);
        let surface_stages = stages(modules.surface);
        // Both pipelines differ only in their fragment module.
        macro_rules! pipeline {
            ($stages:expr) => {
                vk::GraphicsPipelineCreateInfo::default()
                    .stages($stages)
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
            };
        }
        let infos = [pipeline!(&solid_stages), pipeline!(&surface_stages)];
        // SAFETY: every state structure above outlives the call and matches the render pass.
        let made = unsafe { device.create_graphics_pipelines(vk::PipelineCache::null(), &infos, None) };
        match made {
            Ok(pipelines) => {
                self.solid = pipelines[0];
                self.surface = pipelines[1];
                Ok(())
            }
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
            device.destroy_pipeline_layout(self.layout, None);
            device.destroy_descriptor_set_layout(self.uniform_layout, None);
            device.destroy_descriptor_set_layout(self.texture_layout, None);
            device.destroy_sampler(self.linear, None);
            device.destroy_sampler(self.nearest, None);
            device.destroy_render_pass(self.render_pass, None);
        }
    }
}

/// The three shader modules, destroyed when the pipelines have been made from them.
struct ShaderModules {
    context: Rc<VkContext>,
    vertex: vk::ShaderModule,
    solid: vk::ShaderModule,
    surface: vk::ShaderModule,
}

impl ShaderModules {
    fn new(context: &Rc<VkContext>, vertex: &[u32], solid: &[u32], surface: &[u32]) -> Result<Self, VulkanError> {
        let mut made = Self {
            context: Rc::clone(context),
            vertex: vk::ShaderModule::null(),
            solid: vk::ShaderModule::null(),
            surface: vk::ShaderModule::null(),
        };
        made.vertex = module(context, "vertex", vertex)?;
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
            self.context.device.destroy_shader_module(self.vertex, None);
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
            let sizes = [
                vk::DescriptorPoolSize::default()
                    .ty(vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC)
                    .descriptor_count(POOL_SETS),
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

/// The size of the uniform block the descriptor binds, in bytes.
pub(super) const BLOCK_RANGE: u64 = UNIFORM_BYTES as u64;
