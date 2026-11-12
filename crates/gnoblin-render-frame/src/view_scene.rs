//! A View's source subtree described for a renderer that draws it on the GPU.
//!
//! A View node draws another node's subtree through a camera into a picture. The CPU renderer
//! (`gnoblin_render_software::render_rig`) does that per pixel and hands back an [`Image`]. A GPU renderer does not
//! need the pixels on the CPU at all: it draws the same subtree into a texture and the View node, drawn in the
//! scene, samples that texture. [`ViewScene`] is the description it draws from. It names no renderer API, and like an
//! [`OutputFrame`](crate::frame::OutputFrame) it is a value the composer builds without a device.
//!
//! # What a scene holds
//!
//! One [`ViewEye`] per camera: a mono camera has one eye that fills the picture, a stereo rig has two, the left eye in
//! the left half and the right eye in the right half (an odd width gives the spare column to the right eye, as the
//! CPU rig does). A blank rig has no eye and draws the picture's opaque black. An eye is an ordered list of
//! [`ViewOp`]s, bottom first:
//!
//! - a layer to draw. Its [`Model`](crate::frame::Model) is the node's plane through the camera, so the matrix already
//!   ends in the eye's pixel space and the depth is the camera's clip depth between `near` and `far`. The layers are
//!   drawn in list order with no depth test: that is the painter's order the CPU rig draws in, where a later sibling is
//!   on top of an earlier one whatever its depth.
//! - the start and the end of a clipping ancestor, which keeps what is drawn between them to the ancestor's rectangle
//!   as it projects, nested clips cutting each other.
//!
//! # Revisions
//!
//! A scene carries the `revision` the composer drew it at. A renderer that has already drawn a scene of the same `id`
//! and `revision` at the same size keeps its texture and draws nothing, which is what makes an unchanged View free.
//!
//! # The capture variant
//!
//! A View's capture never shows a secure node's pixels. When the source holds a secure node the scene carries a second
//! list of eyes, the same subtree with each secure node replaced the way an output capture replaces a secure layer.
//! [`ViewScene::eyes`] answers the one a reader wants.
//!
//! # Failure modes
//!
//! 1. **A View too big to draw.** [`size_fits`] states the one cap both paths use, so no View asks a driver or the
//!    CPU for a picture the machine cannot hold.

use std::sync::Arc;

use gnoblin_types::{Rect, Size};

use crate::frame::{Layer, Model};

/// The longest side of a View's picture, in pixels. A side past this is a View no renderer draws.
pub const MAX_SIDE: u32 = 8192;

/// The most pixels a View's picture may hold: a 7680 by 4320 picture and a little more.
pub const MAX_PIXELS: u64 = 33_554_432;

/// Whether a View of `size` can be drawn: both sides are at least one pixel, neither is past [`MAX_SIDE`] and the area
/// is at most [`MAX_PIXELS`].
#[must_use]
pub fn size_fits(size: Size) -> bool {
    size.width > 0
        && size.height > 0
        && size.width <= MAX_SIDE
        && size.height <= MAX_SIDE
        && u64::from(size.width) * u64::from(size.height) <= MAX_PIXELS
}

/// A clipping ancestor's rectangle, placed like a layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipPlane {
    /// The rectangle's size in its own plane, whose origin is its top-left corner.
    pub dst: Rect,
    /// How the plane sits in the eye's pixel space.
    pub model: Model,
}

/// One step of an eye's drawing.
#[derive(Clone, Debug)]
pub enum ViewOp {
    /// Draw a layer above what is drawn.
    Draw(Layer),
    /// Keep every following draw to this plane's rectangle, until the matching [`ViewOp::PopClip`].
    PushClip(ClipPlane),
    /// End the clip the matching [`ViewOp::PushClip`] began.
    PopClip(ClipPlane),
}

/// One camera's drawing, into a rectangle of the picture.
#[derive(Clone, Debug)]
pub struct ViewEye {
    /// The part of the picture this eye fills, top-left first, in pixels.
    pub viewport: Rect,
    /// What the eye draws, bottom first.
    pub ops: Vec<ViewOp>,
}

/// A View's source subtree, ready for a GPU renderer to draw into a texture of `size`.
#[derive(Clone, Debug)]
pub struct ViewScene {
    id: u64,
    revision: u64,
    size: Size,
    display: Vec<ViewEye>,
    capture: Option<Vec<ViewEye>>,
}

impl ViewScene {
    /// A scene for the View `id`, at `revision`.
    ///
    /// `capture` is the same subtree with each secure node replaced, or `None` when no node of the source is secure
    /// and a capture reads what the display shows.
    #[must_use]
    pub fn new(id: u64, revision: u64, size: Size, display: Vec<ViewEye>, capture: Option<Vec<ViewEye>>) -> Self {
        Self {
            id,
            revision,
            size,
            display,
            capture,
        }
    }

    /// The View node this scene draws, as a plain number.
    #[must_use]
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The count of the composer's draws of this View. A new draw is a new revision.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The picture's size in pixels.
    #[must_use]
    pub fn size(&self) -> Size {
        self.size
    }

    /// Whether the capture picture differs from the one the display shows, because the source holds a secure node.
    #[must_use]
    pub fn has_capture_variant(&self) -> bool {
        self.capture.is_some()
    }

    /// The eyes to draw: the capture variant when `capture` is set and the scene has one, the display's otherwise.
    #[must_use]
    pub fn eyes(&self, capture: bool) -> &[ViewEye] {
        match (&self.capture, capture) {
            (Some(eyes), true) => eyes,
            _ => &self.display,
        }
    }
}

/// The texture a layer samples: the picture of a View, drawn by the renderer from the [`ViewScene`] the frame carries.
///
/// The renderer finds the texture by the View's node. A frame that names a texture it holds no scene for draws the
/// layer as nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewTexture {
    /// The View node, as [`ViewScene::id`] states it.
    pub id: u64,
}

/// A scene as a frame holds it: shared, so a frame per output costs a pointer.
pub type SharedScene = Arc<ViewScene>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_inside_the_cap_fits_and_one_past_it_does_not() {
        assert!(size_fits(Size::new(1, 1)));
        assert!(size_fits(Size::new(3840, 2160)));
        assert!(size_fits(Size::new(7680, 4320)));
        assert!(!size_fits(Size::new(0, 10)), "a zero side has no pixels");
        assert!(!size_fits(Size::new(10, 0)));
        assert!(!size_fits(Size::new(MAX_SIDE + 1, 1)), "a side past the cap is refused");
        assert!(!size_fits(Size::new(1, MAX_SIDE + 1)));
        assert!(
            !size_fits(Size::new(MAX_SIDE, MAX_SIDE)),
            "the area cap refuses a square at the side cap"
        );
        assert!(!size_fits(Size::new(u32::MAX, u32::MAX)), "no overflow on the extremes");
    }

    #[test]
    fn the_capture_variant_is_read_only_when_it_exists() {
        let eye = |x| ViewEye {
            viewport: Rect::new(x, 0, 4, 4),
            ops: Vec::new(),
        };
        let plain = ViewScene::new(1, 1, Size::new(4, 4), vec![eye(0)], None);
        assert!(!plain.has_capture_variant());
        assert_eq!(
            plain.eyes(true)[0].viewport.x,
            0,
            "a capture reads the display without a variant"
        );
        let redacted = ViewScene::new(1, 2, Size::new(4, 4), vec![eye(0)], Some(vec![eye(2)]));
        assert!(redacted.has_capture_variant());
        assert_eq!(redacted.eyes(false)[0].viewport.x, 0);
        assert_eq!(redacted.eyes(true)[0].viewport.x, 2);
    }
}
