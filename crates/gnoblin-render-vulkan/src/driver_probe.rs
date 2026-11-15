#![allow(unsafe_code)]
//! A Vulkan program that shares nothing with the renderer but the shader compiler, to tell the renderer's memory
//! growth from the driver's (design/gpu-memory.md, "Vulkan on lavapipe").
//!
//! The renderer on lavapipe (Mesa 22.3.6, llvmpipe) grew the process by about 0.8 MiB a frame whatever the frame
//! held, and every allocation that was never freed was made by `libvulkan_lvp.so` on the driver's own queue thread
//! (heaptrack, no renderer frame in any stack). This module is the independent loop that settles whose it is: its own
//! instance, device, image, render pass, pipeline and command buffer, one draw a frame, and the renderer's own solid
//! pass for the shaders. It is the same loop with the one thing that matters changed, one run at a time:
//!
//! - **no draw** (the pass is begun, the pipeline bound, nothing drawn): the process does not grow;
//! - **the solid pass** (`gnoblin_render_frame::shader::sources`): the process grows as the renderer's does;
//! - **the solid pass with `gnoblin_corner(local)` replaced by `1.0`** (no rounded-corner coverage): it does not grow.
//!
//! So the leak is the driver's, triggered by one function of the shared fragment stage, and not a handle, a
//! descriptor set, a command buffer or a fence of the renderer. Nothing in this file is used by the renderer.

use ash::vk;

use super::shader::{StageKind, compile};
use super::tests::resident;
use gnoblin_render_frame::shader::sources::{SOLID, VERTEX};

/// What the loop draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Draw {
    /// Begins the pass and binds the pipeline, and draws nothing.
    Nothing,
    /// The renderer's solid pass: a 16 x 16 red square.
    Solid,
    /// The same pass with the coverage of a rounded corner replaced by `1.0`.
    SolidWithoutCorner,
}

/// How many KiB the process grew over `frames` frames of the loop, or [`None`] on a machine without Vulkan.
pub(super) fn growth_kib(frames: usize, draw: Draw) -> Option<u64> {
    let fragment = match draw {
        Draw::SolidWithoutCorner => {
            let edited = SOLID.replace("let coverage = gnoblin_corner(local);", "let coverage = 1.0;");
            assert_ne!(
                edited, SOLID,
                "the solid pass still computes its coverage with `gnoblin_corner`"
            );
            compile("probe fragment", &edited, StageKind::Fragment)
        }
        _ => compile("probe fragment", SOLID, StageKind::Fragment),
    }
    .expect("the fragment stage compiles");
    let vertex = compile("probe vertex", VERTEX, StageKind::Vertex).expect("the vertex stage compiles");
    // SAFETY: every object is made on the device created here, used from this thread only, waited for before the next
    // frame and destroyed in reverse order of creation before the device and the instance.
    unsafe {
        let entry = ash::Entry::load().ok()?;
        let app = vk::ApplicationInfo::default().api_version(vk::API_VERSION_1_1);
        let instance = entry
            .create_instance(&vk::InstanceCreateInfo::default().application_info(&app), None)
            .ok()?;
        let physical = *instance.enumerate_physical_devices().ok()?.first()?;
        let family = instance
            .get_physical_device_queue_family_properties(physical)
            .iter()
            .position(|properties| properties.queue_flags.contains(vk::QueueFlags::GRAPHICS))?
            as u32;
        let priorities = [1.0];
        let queues = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(family)
            .queue_priorities(&priorities)];
        let device = instance
            .create_device(
                physical,
                &vk::DeviceCreateInfo::default().queue_create_infos(&queues),
                None,
            )
            .expect("a device");
        let queue = device.get_device_queue(family, 0);
        let memory_types = instance.get_physical_device_memory_properties(physical);
        let allocate = |needs: vk::MemoryRequirements, wanted: vk::MemoryPropertyFlags| {
            let index = (0..memory_types.memory_type_count)
                .find(|index| {
                    needs.memory_type_bits & (1 << index) != 0
                        && memory_types.memory_types[*index as usize]
                            .property_flags
                            .contains(wanted)
                })
                .expect("a memory type");
            device
                .allocate_memory(
                    &vk::MemoryAllocateInfo::default()
                        .allocation_size(needs.size)
                        .memory_type_index(index),
                    None,
                )
                .expect("memory")
        };

        // A 64 x 48 target, drawn into and never read.
        let image = device
            .create_image(
                &vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(vk::Format::R8G8B8A8_UNORM)
                    .extent(vk::Extent3D {
                        width: 64,
                        height: 48,
                        depth: 1,
                    })
                    .mip_levels(1)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::OPTIMAL)
                    .usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
                    .initial_layout(vk::ImageLayout::UNDEFINED),
                None,
            )
            .expect("an image");
        let image_memory = allocate(
            device.get_image_memory_requirements(image),
            vk::MemoryPropertyFlags::empty(),
        );
        device.bind_image_memory(image, image_memory, 0).expect("bound");
        let view = device
            .create_image_view(
                &vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(vk::Format::R8G8B8A8_UNORM)
                    .subresource_range(vk::ImageSubresourceRange {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        base_mip_level: 0,
                        level_count: 1,
                        base_array_layer: 0,
                        layer_count: 1,
                    }),
                None,
            )
            .expect("a view");
        let attachments = [vk::AttachmentDescription::default()
            .format(vk::Format::R8G8B8A8_UNORM)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::DONT_CARE)
            .store_op(vk::AttachmentStoreOp::STORE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)];
        let references = [vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)];
        let subpasses = [vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&references)];
        let pass = device
            .create_render_pass(
                &vk::RenderPassCreateInfo::default()
                    .attachments(&attachments)
                    .subpasses(&subpasses),
                None,
            )
            .expect("a pass");
        let views = [view];
        let framebuffer = device
            .create_framebuffer(
                &vk::FramebufferCreateInfo::default()
                    .render_pass(pass)
                    .attachments(&views)
                    .width(64)
                    .height(48)
                    .layers(1),
                None,
            )
            .expect("a framebuffer");

        // The renderer's set layouts: a dynamic uniform block, then a sampled image and a sampler.
        let block_binding = [vk::DescriptorSetLayoutBinding::default()
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
        let block_layout = device
            .create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&block_binding),
                None,
            )
            .expect("a layout");
        let texture_layout = device
            .create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&texture_bindings),
                None,
            )
            .expect("a layout");
        let set_layouts = [block_layout, texture_layout];
        let layout = device
            .create_pipeline_layout(&vk::PipelineLayoutCreateInfo::default().set_layouts(&set_layouts), None)
            .expect("a pipeline layout");

        // Host buffers: the unit quad, one instance of the renderer's vertex layout and the fragment's block.
        let host = vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
        let buffer = |floats: &[f32], usage: vk::BufferUsageFlags| {
            let size = (floats.len() * 4).max(256) as u64;
            let buffer = device
                .create_buffer(&vk::BufferCreateInfo::default().size(size).usage(usage), None)
                .expect("a buffer");
            let memory = allocate(device.get_buffer_memory_requirements(buffer), host);
            device.bind_buffer_memory(buffer, memory, 0).expect("bound");
            let bytes = device
                .map_memory(memory, 0, size, vk::MemoryMapFlags::empty())
                .expect("mapped")
                .cast::<f32>();
            std::ptr::copy_nonoverlapping(floats.as_ptr(), bytes, floats.len());
            (buffer, memory)
        };
        let (quad, quad_memory) = buffer(
            &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            vk::BufferUsageFlags::VERTEX_BUFFER,
        );
        // The rectangle, the viewport and the projection and parameters of `quad.wgsl` (zero: not projected).
        let mut one_instance = [0.0_f32; 22];
        one_instance[..6].copy_from_slice(&[0.0, 0.0, 16.0, 16.0, 64.0, 48.0]);
        let (instances, instances_memory) = buffer(&one_instance, vk::BufferUsageFlags::VERTEX_BUFFER);
        // The block of `preamble.wgsl`: rect, src, colour, tint, params, then the corner, the opacity and padding.
        let (block, block_memory) = buffer(
            &[
                0.0, 0.0, 16.0, 16.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0,
                0.0, 1.0, 0.0, 0.0,
            ],
            vk::BufferUsageFlags::UNIFORM_BUFFER,
        );
        let pool_sizes = [vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC)
            .descriptor_count(1)];
        let pool = device
            .create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .max_sets(1)
                    .pool_sizes(&pool_sizes),
                None,
            )
            .expect("a pool");
        let block_layouts = [block_layout];
        let set = device
            .allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(pool)
                    .set_layouts(&block_layouts),
            )
            .expect("a set")[0];
        let block_info = [vk::DescriptorBufferInfo::default().buffer(block).offset(0).range(96)];
        device.update_descriptor_sets(
            &[vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC)
                .buffer_info(&block_info)],
            &[],
        );

        let module = |words: &[u32]| {
            device
                .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(words), None)
                .expect("a module")
        };
        let (vertex_module, fragment_module) = (module(&vertex), module(&fragment));
        let name = c"gnoblin_entry";
        let stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vertex_module)
                .name(name),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(fragment_module)
                .name(name),
        ];
        let bindings = [
            vk::VertexInputBindingDescription::default()
                .binding(0)
                .stride(8)
                .input_rate(vk::VertexInputRate::VERTEX),
            vk::VertexInputBindingDescription::default()
                .binding(1)
                .stride(88)
                .input_rate(vk::VertexInputRate::INSTANCE),
        ];
        let attributes = [
            (0, 0, vk::Format::R32G32_SFLOAT, 0),
            (1, 1, vk::Format::R32G32B32A32_SFLOAT, 0),
            (2, 1, vk::Format::R32G32_SFLOAT, 16),
            (3, 1, vk::Format::R32G32B32A32_SFLOAT, 24),
            (4, 1, vk::Format::R32G32B32A32_SFLOAT, 40),
            (5, 1, vk::Format::R32G32B32A32_SFLOAT, 56),
            (6, 1, vk::Format::R32G32B32A32_SFLOAT, 72),
        ]
        .map(|(location, binding, format, offset)| {
            vk::VertexInputAttributeDescription::default()
                .location(location)
                .binding(binding)
                .format(format)
                .offset(offset)
        });
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&bindings)
            .vertex_attribute_descriptions(&attributes);
        let assembly =
            vk::PipelineInputAssemblyStateCreateInfo::default().topology(vk::PrimitiveTopology::TRIANGLE_STRIP);
        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
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
        let pipeline = device
            .create_graphics_pipelines(
                vk::PipelineCache::null(),
                &[vk::GraphicsPipelineCreateInfo::default()
                    .stages(&stages)
                    .vertex_input_state(&vertex_input)
                    .input_assembly_state(&assembly)
                    .viewport_state(&viewport_state)
                    .rasterization_state(&raster)
                    .multisample_state(&multisample)
                    .color_blend_state(&blend)
                    .dynamic_state(&dynamic)
                    .layout(layout)
                    .render_pass(pass)
                    .subpass(0)],
                None,
            )
            .map_err(|(_, result)| result)
            .expect("a pipeline")[0];

        let command_pool = device
            .create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )
            .expect("a command pool");
        let cmd = device
            .allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )
            .expect("a command buffer")[0];
        let fence = device
            .create_fence(&vk::FenceCreateInfo::default(), None)
            .expect("a fence");
        let area = vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: vk::Extent2D { width: 64, height: 48 },
        };
        let viewport = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: 64.0,
            height: 48.0,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let frame = |count: usize| {
            for _ in 0..count {
                device.reset_fences(&[fence]).expect("reset");
                device
                    .reset_command_buffer(cmd, vk::CommandBufferResetFlags::empty())
                    .expect("reset buffer");
                device
                    .begin_command_buffer(
                        cmd,
                        &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                    )
                    .expect("begin");
                device.cmd_begin_render_pass(
                    cmd,
                    &vk::RenderPassBeginInfo::default()
                        .render_pass(pass)
                        .framebuffer(framebuffer)
                        .render_area(area),
                    vk::SubpassContents::INLINE,
                );
                device.cmd_set_viewport(cmd, 0, &[viewport]);
                device.cmd_set_scissor(cmd, 0, &[area]);
                device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, pipeline);
                device.cmd_bind_vertex_buffers(cmd, 0, &[quad, instances], &[0, 0]);
                device.cmd_bind_descriptor_sets(cmd, vk::PipelineBindPoint::GRAPHICS, layout, 0, &[set], &[0]);
                if draw != Draw::Nothing {
                    device.cmd_draw(cmd, 4, 1, 0, 0);
                }
                device.cmd_end_render_pass(cmd);
                device.end_command_buffer(cmd).expect("end");
                let buffers = [cmd];
                device
                    .queue_submit(queue, &[vk::SubmitInfo::default().command_buffers(&buffers)], fence)
                    .expect("submit");
                device.wait_for_fences(&[fence], true, u64::MAX).expect("wait");
            }
        };
        frame(30);
        let settled = resident();
        frame(frames);
        let grown = resident().saturating_sub(settled) >> 10;

        device.device_wait_idle().expect("idle");
        device.destroy_fence(fence, None);
        device.destroy_command_pool(command_pool, None);
        device.destroy_pipeline(pipeline, None);
        device.destroy_descriptor_pool(pool, None);
        for (buffer, memory) in [
            (quad, quad_memory),
            (instances, instances_memory),
            (block, block_memory),
        ] {
            device.destroy_buffer(buffer, None);
            device.free_memory(memory, None);
        }
        device.destroy_shader_module(vertex_module, None);
        device.destroy_shader_module(fragment_module, None);
        device.destroy_pipeline_layout(layout, None);
        device.destroy_descriptor_set_layout(texture_layout, None);
        device.destroy_descriptor_set_layout(block_layout, None);
        device.destroy_framebuffer(framebuffer, None);
        device.destroy_render_pass(pass, None);
        device.destroy_image_view(view, None);
        device.free_memory(image_memory, None);
        device.destroy_image(image, None);
        device.destroy_device(None);
        instance.destroy_instance(None);
        Some(grown)
    }
}
