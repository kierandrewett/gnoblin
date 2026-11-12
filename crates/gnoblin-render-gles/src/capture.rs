//! Capture into, and out of, a client's GPU buffer.
//!
//! Three things the compositor does with a dmabuf a client handed it, none of which is drawing a frame:
//!
//! - [`Gles::copy_frame_into_dmabuf`]: a capture client (`ext-image-copy-capture`, `zwlr_screencopy`) allocated a
//!   buffer on the GPU and asked for the last frame. The frame is blitted from the renderer's target into the
//!   buffer on the GPU; no pixel crosses to the CPU.
//! - [`Gles::copy_image_into_dmabuf`]: the same for a picture the CPU holds (a View's), which is uploaded.
//! - [`Gles::read_dmabuf`]: a View of a window that shows a dmabuf needs its pixels on the CPU, so the imported
//!   texture is read back.
//!
//! # Orientation
//!
//! Two row orders meet here, and every function below is a statement about which is which.
//!
//! - The renderer's own target is drawn top-down into a framebuffer whose GL row 0 is the *bottom* of the picture
//!   (a `glReadPixels` returns the bottom row first and [`Gles::capture`] reverses them).
//! - An imported dmabuf is a texture whose GL row 0 is the buffer's first *memory* row. A client allocates its
//!   buffers top-first, so memory row 0 is the top of the picture, unless the buffer carries the `y-invert` flag,
//!   which says the first memory row is the bottom.
//!
//! So a frame copied into an ordinary buffer is a blit with its rows reversed, and into a `y-invert` buffer one
//! with the rows kept; a readback of an ordinary buffer is already top-first, and of a `y-invert` buffer is
//! reversed. [`gl_rows`] is the one place that decides it, and the tests state each case on a texture of the
//! renderer's own, because only the claim about a *dmabuf's* row 0 needs a GPU: `tests/dmabuf_gpu.rs` states it
//! on the real device.
//!
//! # Failure modes
//!
//! 1. **A frame lands upside down.** Both buffer orders are written and read back:
//!    `tests::a_frame_copied_into_a_buffer_is_top_first` and
//!    `tests::a_frame_copied_into_an_inverted_buffer_is_bottom_first`.
//! 2. **A region lands at the wrong place or runs past the buffer.** The source and the destination are clipped
//!    together, so a pixel keeps the offset it had: `tests::a_region_is_copied_from_where_it_starts`.
//! 3. **The cursor shows in a capture that asked for none.** The pixels the cursor covers are put back after the
//!    blit: `tests::a_capture_without_the_cursor_puts_back_what_it_covers`.
//! 4. **A buffer with no alpha reads back transparent.** The padding byte is made opaque:
//!    `tests::a_buffer_without_alpha_reads_back_opaque`.

use smithay::backend::allocator::Buffer as _;
use smithay::backend::allocator::Fourcc;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::renderer::gles::GlesTexture;

use gnoblin_render_frame::error::RenderError;
use gnoblin_render_frame::image::Image;
use gnoblin_types::{Point, Rect, Size};

use crate::gl::Texture;
use crate::{Gles, flip_rows, log};

/// Where a copy reads from and writes to, in the pixels of each side as a picture is described, top-left first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Plan {
    /// The pixels read, in the source's.
    pub(crate) src: Rect,
    /// Where they land, in the buffer's.
    pub(crate) dst: Rect,
}

/// The part of the copy that fits both sides.
///
/// Buffer pixel `(x, y)` of `region` is read from source pixel `(from.x + x, from.y + y)`. A region that overlaps
/// the buffer only partly writes the overlap, and one that reaches past the source stops at its edge, and both are
/// clipped together so a pixel keeps the offset it had. `None` when nothing is left.
pub(crate) fn plan(region: Rect, from: Point, buffer: Size, source: Size) -> Option<Plan> {
    let written = region.intersection(Rect::from_size(buffer))?;
    let wanted = Rect::new(
        from.x.saturating_add(written.x),
        from.y.saturating_add(written.y),
        written.width,
        written.height,
    );
    let src = wanted.intersection(Rect::from_size(source))?;
    let dst = Rect::new(
        src.x.saturating_sub(from.x),
        src.y.saturating_sub(from.y),
        src.width,
        src.height,
    );
    Some(Plan { src, dst })
}

/// `rect`, stated in a picture's top-left-first rows, as the rows of a buffer's storage, and whether a copy from a
/// frame must reverse its rows to fill them.
///
/// An ordinary buffer's storage is top-first, so the rows are the picture's and the copy reverses (a frame's own
/// row 0 is its bottom). A `y-invert` buffer's storage is bottom-first, so the rows are mirrored in the buffer's
/// height and the copy keeps them.
pub(crate) fn gl_rows(rect: Rect, buffer_height: u32, inverted: bool) -> (Rect, bool) {
    if inverted {
        let top = i64::from(buffer_height) - i64::from(rect.y) - i64::from(rect.height);
        (
            Rect::new(rect.x, i32::try_from(top).unwrap_or(0), rect.width, rect.height),
            false,
        )
    } else {
        (rect, true)
    }
}

/// The bytes of `rect` of `image`, top row first, `rect.width * 4` bytes a row.
fn crop(image: &Image, rect: Rect) -> Option<Vec<u8>> {
    let size = image.size();
    if !rect.is_inside(Rect::from_size(size)) {
        return None;
    }
    let row = rect.width as usize * 4;
    let mut bytes = Vec::with_capacity(row * rect.height as usize);
    for y in 0..rect.height as usize {
        let start = (rect.y as usize + y) * image.stride() + rect.x as usize * 4;
        bytes.extend_from_slice(image.data().get(start..start + row)?);
    }
    Some(bytes)
}

/// Whether a format stores an alpha channel. A buffer without one reads back opaque.
fn has_alpha(code: Fourcc) -> bool {
    !matches!(
        code,
        Fourcc::Xrgb8888
            | Fourcc::Xbgr8888
            | Fourcc::Rgbx8888
            | Fourcc::Bgrx8888
            | Fourcc::Xrgb2101010
            | Fourcc::Xbgr2101010
            | Fourcc::Rgbx1010102
            | Fourcc::Bgrx1010102
            | Fourcc::Xrgb16161616f
            | Fourcc::Xbgr16161616f
    )
}

impl Gles {
    /// The texture of a client's buffer, imported or cached, and the handle the passes name it by.
    fn client_texture(&mut self, dmabuf: &Dmabuf) -> Result<(GlesTexture, Texture), RenderError> {
        let imported = self.dmabuf_texture(dmabuf).ok_or(RenderError::Failed)?;
        let handle = Texture::from_smithay(&imported).ok_or(RenderError::Unavailable)?;
        Ok((imported, handle))
    }

    /// Copies the frame last drawn into `dmabuf` on the GPU.
    ///
    /// Buffer pixel `(x, y)` of `region` is read from frame pixel `(from.x + x, from.y + y)`; see [`plan`]. The
    /// cursor is in the copy when `with_cursor` is set and is covered by what lay under it otherwise. The call
    /// returns when the copy is done, so the buffer can be read as soon as it returns.
    ///
    /// # Errors
    ///
    /// [`RenderError::Failed`] when the driver refuses the import or the copy or no pixel of `region` fits both
    /// the buffer and the frame, and [`RenderError::Unavailable`] when the buffer cannot be drawn into.
    pub fn copy_frame_into_dmabuf(
        &mut self,
        dmabuf: &Dmabuf,
        region: Rect,
        from: Point,
        with_cursor: bool,
    ) -> Result<(), RenderError> {
        let (_held, texture) = self.client_texture(dmabuf)?;
        self.frame_into_texture(texture, dmabuf.y_inverted(), region, from, with_cursor)
    }

    /// Writes `image`, a picture the CPU holds, into `dmabuf`, with the same meaning of `region` and `from` as
    /// [`Gles::copy_frame_into_dmabuf`].
    ///
    /// # Errors
    ///
    /// The same as [`Gles::copy_frame_into_dmabuf`].
    pub fn copy_image_into_dmabuf(
        &mut self,
        dmabuf: &Dmabuf,
        image: &Image,
        region: Rect,
        from: Point,
    ) -> Result<(), RenderError> {
        let (_held, texture) = self.client_texture(dmabuf)?;
        self.image_into_texture(texture, dmabuf.y_inverted(), image, region, from)
    }

    /// Reads `dmabuf` back as a picture, top row first, with a buffer that has no alpha made opaque.
    ///
    /// # Errors
    ///
    /// [`RenderError::Failed`] when the driver refuses the import or the read, and
    /// [`RenderError::Unavailable`] when the buffer cannot be read.
    pub fn read_dmabuf(&mut self, dmabuf: &Dmabuf) -> Result<Image, RenderError> {
        let (_held, texture) = self.client_texture(dmabuf)?;
        self.picture_of_texture(texture, dmabuf.y_inverted(), has_alpha(dmabuf.format().code))
    }

    /// [`Gles::copy_frame_into_dmabuf`] for a texture of the context, whose storage is top-first unless `inverted`.
    pub(crate) fn frame_into_texture(
        &mut self,
        texture: Texture,
        inverted: bool,
        region: Rect,
        from: Point,
        with_cursor: bool,
    ) -> Result<(), RenderError> {
        let (width, height) = texture.size();
        let buffer = Size::new(width.max(0) as u32, height.max(0) as u32);
        let plan = plan(region, from, buffer, self.physical).ok_or(RenderError::Failed)?;
        let (rows, reverse) = gl_rows(plan.dst, buffer.height, inverted);
        self.device.blit_into(&self.target, texture, plan.src, rows, reverse)?;
        if with_cursor {
            return Ok(());
        }
        // The cursor was drawn into the frame, over pixels `cursor_under` kept: put those back.
        let Some(cursor) = self.cursor_under_rect else {
            return Ok(());
        };
        let covered = Rect::new(
            cursor.loc.x,
            cursor.loc.y,
            cursor.size.w.max(0) as u32,
            cursor.size.h.max(0) as u32,
        );
        let Some(over) = covered.intersection(plan.src) else {
            return Ok(());
        };
        let stride = self.physical.width as usize * 4;
        let row = over.width as usize * 4;
        let mut bytes: Vec<u8> = Vec::with_capacity(row * over.height as usize);
        for y in over.y..over.y.saturating_add_unsigned(over.height) {
            let start = y as usize * stride + over.x as usize * 4;
            let under = self.cursor_under.get(start..start + row).ok_or(RenderError::Failed)?;
            bytes.extend_from_slice(under);
        }
        let landed = over.translate(-from.x, -from.y);
        self.write_rows(texture, landed, buffer.height, inverted, bytes)
    }

    /// [`Gles::copy_image_into_dmabuf`] for a texture of the context.
    pub(crate) fn image_into_texture(
        &mut self,
        texture: Texture,
        inverted: bool,
        image: &Image,
        region: Rect,
        from: Point,
    ) -> Result<(), RenderError> {
        let (width, height) = texture.size();
        let buffer = Size::new(width.max(0) as u32, height.max(0) as u32);
        let plan = plan(region, from, buffer, image.size()).ok_or(RenderError::Failed)?;
        let bytes = crop(image, plan.src).ok_or(RenderError::Failed)?;
        self.write_rows(texture, plan.dst, buffer.height, inverted, bytes)
    }

    /// [`Gles::read_dmabuf`] for a texture of the context.
    pub(crate) fn picture_of_texture(
        &mut self,
        texture: Texture,
        inverted: bool,
        alpha: bool,
    ) -> Result<Image, RenderError> {
        let (width, height) = texture.size();
        let mut bytes = self.device.read_texture(texture)?;
        let stride = width.max(0) as usize * 4;
        // The rows came in storage order, which is top-first unless the buffer is flagged otherwise.
        if inverted {
            flip_rows(&mut bytes, stride);
        }
        if !alpha {
            for pixel in bytes.as_chunks_mut::<4>().0 {
                pixel[3] = u8::MAX;
            }
        }
        Image::new(Size::new(width.max(0) as u32, height.max(0) as u32), stride, bytes).ok_or_else(|| {
            log::warn!(area = "render", "a client's GPU buffer read back with the wrong length");
            RenderError::Failed
        })
    }

    /// Writes `bytes`, the top-first rows of `rect` of a buffer's picture, into the buffer's storage.
    fn write_rows(
        &mut self,
        texture: Texture,
        rect: Rect,
        buffer_height: u32,
        inverted: bool,
        mut bytes: Vec<u8>,
    ) -> Result<(), RenderError> {
        let (rows, top_first) = gl_rows(rect, buffer_height, inverted);
        // A top-first storage takes the rows as they are; a bottom-first one takes them reversed.
        if !top_first {
            flip_rows(&mut bytes, rect.width as usize * 4);
        }
        self.device.upload_into(texture, rows, &bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The size a test picture has.
    const SIZE: Size = Size::new(8, 6);

    #[test]
    fn a_region_is_planned_from_where_it_starts() {
        let planned = plan(Rect::new(0, 0, 4, 3), Point::new(2, 1), SIZE, SIZE).expect("it fits");
        assert_eq!(planned.src, Rect::new(2, 1, 4, 3), "read from the offset");
        assert_eq!(planned.dst, Rect::new(0, 0, 4, 3), "written where the region is");
    }

    #[test]
    fn a_region_past_the_source_is_clipped_with_its_destination() {
        let planned = plan(Rect::new(0, 0, 8, 6), Point::new(4, 3), SIZE, SIZE).expect("part fits");
        assert_eq!(planned.src, Rect::new(4, 3, 4, 3), "the source stops at its edge");
        assert_eq!(planned.dst, Rect::new(0, 0, 4, 3), "and so does the destination");
    }

    #[test]
    fn a_region_that_starts_off_the_source_keeps_its_offset() {
        let planned = plan(Rect::new(0, 0, 4, 4), Point::new(-2, -1), SIZE, SIZE).expect("part fits");
        assert_eq!(planned.src, Rect::new(0, 0, 2, 3));
        assert_eq!(planned.dst, Rect::new(2, 1, 2, 3), "a pixel lands at the offset it had");
    }

    #[test]
    fn a_region_with_nothing_in_common_plans_nothing() {
        assert_eq!(plan(Rect::new(0, 0, 4, 4), Point::new(100, 0), SIZE, SIZE), None);
        assert_eq!(plan(Rect::new(20, 0, 4, 4), Point::new(0, 0), SIZE, SIZE), None);
    }

    #[test]
    fn an_ordinary_buffer_reverses_and_an_inverted_one_mirrors() {
        let rect = Rect::new(1, 1, 2, 2);
        assert_eq!(gl_rows(rect, 6, false), (rect, true));
        assert_eq!(gl_rows(rect, 6, true), (Rect::new(1, 3, 2, 2), false));
    }

    #[test]
    fn only_a_format_with_alpha_keeps_it() {
        assert!(has_alpha(Fourcc::Argb8888));
        assert!(has_alpha(Fourcc::Abgr8888));
        assert!(!has_alpha(Fourcc::Xrgb8888));
        assert!(!has_alpha(Fourcc::Xbgr8888));
    }
}
