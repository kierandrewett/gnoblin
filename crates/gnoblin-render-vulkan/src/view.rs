#![allow(unsafe_code)]
//! The whole-output view pass: the magnifier and the colour matrix, drawn over a copy of the finished output.
//!
//! The pass has its own uniform block of 128 bytes, laid out as `GnoblinView` in `wgsl/view.wgsl`: eight `vec4`s, so the
//! std140 layout and WGSL agree on the offsets without padding. It is never the 112-byte layer block, so a layer pass cannot
//! reach it and this pass cannot change what a layer pass reads. `gnoblin-render-gles`'s `view.rs` packs the same layout; this
//! one also sets `flags.y`, which says the source is top-down (a Vulkan image is, a GL texture is not).
//!
//! # What a frame does
//!
//! The pass runs only when the frame has a view or a matrix that changes a pixel ([`is_active`]); an identity view (a zoom of
//! one) and an identity matrix are skipped, so a frame with neither draws exactly as it did before this pass existed. When it
//! runs the whole output is repainted, because the filter reads and rewrites every pixel: a pixel not repainted would be
//! filtered again, and one left unfiltered after the filter was turned off would stay wrong. So the frame is widened to the
//! whole output whenever the filter is on now or was on the frame before ([`Vulkan`]'s `filtered`).
//!
//! The finished target (the cursor included, as GLES draws it) is copied into a scratch image the size of the output, and the
//! view pipeline draws one quad over the whole target sampling that copy, with blending off: each pixel is the copy's pixel at
//! its source position, not a blend over the pixel already there.

use ash::vk;
use gnoblin_render_frame::frame::{ColourMatrix, View};
use gnoblin_types::Size;

use super::Vulkan;
use super::pipeline::INSTANCE_FLOATS;
use super::plan::{DrawBatch, UNIFORM_STRIDE};

/// The bytes of the view block: eight `vec4`s.
pub(super) const VIEW_BYTES: usize = 128;

/// The uniform block of the view pass, packed as the shader reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ViewUniforms {
    /// The view's centre x, centre y, zoom and an unused slot.
    centre: [f32; 4],
    /// The output's width and height in physical pixels, then two unused slots.
    size: [f32; 4],
    /// The matrix's four rows, one per output channel, each with its four weights.
    rows: [[f32; 4]; 4],
    /// The matrix's offset for each output channel.
    offsets: [f32; 4],
    /// `x` is one when the matrix applies, `y` is one when the source is top-down.
    flags: [f32; 4],
}

impl ViewUniforms {
    /// The block for a view and matrix over an output of `size` physical pixels, read from a top-down source.
    ///
    /// A missing view centres on the output at zoom one, which maps every pixel to itself; a missing matrix sets the flag to
    /// zero, so the shader never runs the colour stage.
    pub(super) fn new(view: Option<View>, matrix: Option<ColourMatrix>, size: Size) -> Self {
        let (centre_x, centre_y, zoom) = match view {
            Some(view) => (view.centre_x, view.centre_y, view.zoom),
            None => (0.0, 0.0, 1.0),
        };
        let mut rows = [[0.0f32; 4]; 4];
        let mut offsets = [0.0f32; 4];
        let mut flag = 0.0f32;
        if let Some(matrix) = matrix {
            flag = 1.0;
            for (channel, row) in matrix.rows.iter().enumerate() {
                rows[channel] = [row[0], row[1], row[2], row[3]];
                offsets[channel] = row[4];
            }
        }
        Self {
            centre: [centre_x, centre_y, zoom, 0.0],
            size: [size.width as f32, size.height as f32, 0.0, 0.0],
            rows,
            offsets,
            flags: [flag, 1.0, 0.0, 0.0],
        }
    }

    /// The block's bytes, in the order the shader reads them.
    pub(super) fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(VIEW_BYTES);
        for vector in [
            self.centre,
            self.size,
            self.rows[0],
            self.rows[1],
            self.rows[2],
            self.rows[3],
            self.offsets,
            self.flags,
        ] {
            for value in vector {
                out.extend_from_slice(&value.to_ne_bytes());
            }
        }
        out
    }
}

/// Whether a frame's view or colour matrix changes any pixel, so the pass has work to do.
///
/// A view with a zoom of one maps every pixel to itself whatever its centre, so it is idle. A matrix is idle when it equals
/// the identity.
pub(super) fn is_active(view: Option<View>, matrix: Option<ColourMatrix>) -> bool {
    let magnified = view.is_some_and(|view| view.zoom != 1.0);
    let tinted = matrix.is_some_and(|matrix| matrix.rows != ColourMatrix::IDENTITY.rows);
    magnified || tinted
}

/// A pass that draws one quad over a whole target: where its block and its quad are in the frame's buffers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct WholePass {
    /// The pass's block, as an offset into the frame's uniform buffer.
    pub uniform_offset: u32,
    /// The quad, as an index into the frame's instances.
    pub instance: u32,
}

/// Appends a whole-target pass's `block` and its quad (the whole of a target of `size`) to the frame's buffers.
pub(super) fn push_block(batch: &mut DrawBatch, block: &[u8], size: Size) -> WholePass {
    let offset = batch.uniforms.len().next_multiple_of(UNIFORM_STRIDE);
    batch.uniforms.resize(offset + UNIFORM_STRIDE, 0);
    batch.uniforms[offset..offset + block.len()].copy_from_slice(block);
    let instance = batch.instances.len() / INSTANCE_FLOATS;
    batch.instances.extend_from_slice(&[
        0.0,
        0.0,
        size.width as f32,
        size.height as f32,
        size.width as f32,
        size.height as f32,
    ]);
    batch.instances.extend([0.0; INSTANCE_FLOATS - 6]);
    WholePass {
        uniform_offset: offset as u32,
        instance: instance as u32,
    }
}

impl Vulkan {
    /// Records a render pass that draws `pipeline` over all of `framebuffer`, a target of `size`, sampling `texture`.
    pub(super) fn whole_pass(
        &self,
        target: (vk::Framebuffer, Size),
        pipeline: vk::Pipeline,
        sets: (vk::DescriptorSet, vk::DescriptorSet),
        pass: WholePass,
    ) {
        let device = &self.device.context.device;
        let pipelines = &self.device.pipelines;
        let slot = self.slot();
        let cmd = slot.cmd;
        let (framebuffer, size) = target;
        let begin = vk::RenderPassBeginInfo::default()
            .render_pass(pipelines.render_pass)
            .framebuffer(framebuffer)
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D {
                    width: size.width,
                    height: size.height,
                },
            });
        let viewport = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: size.width as f32,
            height: size.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let scissor = vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: vk::Extent2D {
                width: size.width,
                height: size.height,
            },
        };
        // SAFETY: the pass, framebuffer, pipeline and sets are live and compatible, the instance and block are inside the
        // buffers `upload_batch` made room for, and the viewport and scissor are the target's own.
        unsafe {
            device.cmd_begin_render_pass(cmd, &begin, vk::SubpassContents::INLINE);
            device.cmd_set_viewport(cmd, 0, &[viewport]);
            device.cmd_set_scissor(cmd, 0, &[scissor]);
            device.cmd_bind_vertex_buffers(cmd, 0, &[self.device.quad.raw(), slot.instances.raw()], &[0, 0]);
            device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, pipeline);
            device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                pipelines.layout,
                0,
                &[sets.0],
                &[pass.uniform_offset, 0],
            );
            device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                pipelines.layout,
                1,
                &[sets.1],
                &[],
            );
            device.cmd_draw(cmd, 4, 1, 0, pass.instance);
            device.cmd_end_render_pass(cmd);
        }
    }

    /// Records the view pass over the target: copies the target into the scratch image and draws the magnified, tinted copy
    /// back over it.
    pub(super) fn record_view(
        &self,
        scratch: vk::Image,
        pipeline: vk::Pipeline,
        sets: (vk::DescriptorSet, vk::DescriptorSet),
        pass: WholePass,
    ) {
        let slot = self.slot();
        let target = self.target.image.raw();
        let size = self.physical;
        slot.barrier(
            target,
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            (
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            ),
            (vk::PipelineStageFlags::TRANSFER, vk::AccessFlags::TRANSFER_READ),
        );
        slot.barrier(
            scratch,
            vk::ImageLayout::UNDEFINED,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            (vk::PipelineStageFlags::FRAGMENT_SHADER, vk::AccessFlags::SHADER_READ),
            (vk::PipelineStageFlags::TRANSFER, vk::AccessFlags::TRANSFER_WRITE),
        );
        let layers = super::textures::colour_layers();
        let region = vk::ImageCopy::default()
            .src_subresource(layers)
            .dst_subresource(layers)
            .extent(vk::Extent3D {
                width: size.width,
                height: size.height,
                depth: 1,
            });
        // SAFETY: both images are the output's size and in the transfer layouts the barriers above put them in.
        unsafe {
            self.device.context.device.cmd_copy_image(
                slot.cmd,
                target,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                scratch,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[region],
            );
        }
        slot.barrier(
            scratch,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            (vk::PipelineStageFlags::TRANSFER, vk::AccessFlags::TRANSFER_WRITE),
            (vk::PipelineStageFlags::FRAGMENT_SHADER, vk::AccessFlags::SHADER_READ),
        );
        slot.barrier(
            target,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            (vk::PipelineStageFlags::TRANSFER, vk::AccessFlags::empty()),
            (
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            ),
        );
        self.whole_pass((self.target.framebuffer, size), pipeline, sets, pass);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn floats(bytes: &[u8]) -> Vec<f32> {
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|chunk| f32::from_ne_bytes(*chunk))
            .collect()
    }

    #[test]
    fn the_block_is_the_declared_size_and_flags_a_top_down_source() {
        let bytes = ViewUniforms::new(View::new(2.0, 10.0, 20.0), None, Size::new(64, 48)).bytes();
        assert_eq!(bytes.len(), VIEW_BYTES);
        let values = floats(&bytes);
        assert_eq!(values[..3], [10.0, 20.0, 2.0], "centre and zoom");
        assert_eq!(values[4..6], [64.0, 48.0], "the output's size");
        assert_eq!(values[28..30], [0.0, 1.0], "no matrix, and a top-down source");
    }

    #[test]
    fn an_identity_view_and_matrix_are_idle() {
        assert!(!is_active(View::new(1.0, 5.0, 5.0), Some(ColourMatrix::IDENTITY)));
        assert!(is_active(View::new(2.0, 5.0, 5.0), None));
        let mut rows = ColourMatrix::IDENTITY.rows;
        rows[0][4] = 0.1;
        assert!(is_active(None, ColourMatrix::new(rows)));
    }

    #[test]
    fn a_block_joins_the_frame_buffers_at_the_next_stride() {
        let mut batch = DrawBatch::default();
        batch.uniforms.resize(UNIFORM_STRIDE * 3, 0);
        batch.instances.resize(INSTANCE_FLOATS * 3, 0.0);
        let pass = push_block(&mut batch, &[7; 16], Size::new(8, 4));
        assert_eq!(pass.uniform_offset as usize, UNIFORM_STRIDE * 3);
        assert_eq!(pass.instance, 3);
        assert_eq!(batch.uniforms.len(), UNIFORM_STRIDE * 4);
        assert_eq!(batch.instances.len(), INSTANCE_FLOATS * 4);
    }
}
