//! The dmabuf formats the renderer can import and export, and what each is as a Vulkan texture.
//!
//! A DRM fourcc names the byte layout of a pixel in memory. On a little-endian machine `ARGB8888` is blue, green, red, alpha
//! (what `B8G8R8A8_UNORM` reads, the `wl_shm` layout) and `ABGR8888` is red, green, blue, alpha (`R8G8B8A8_UNORM`, the layout
//! of the renderer's own target). The `X` variants carry padding in the fourth byte, which the texture's view reads as one.
//!
//! # Modifiers
//!
//! Only `DRM_FORMAT_MOD_LINEAR` is offered. A device without `VK_EXT_image_drm_format_modifier` (lavapipe) cannot sample a
//! tiled buffer at all, and the linear path (design decision 12) maps the buffer and copies its rows into a texture, so it
//! needs no extension. `DRM_FORMAT_MOD_INVALID`, an implicit modifier, is never offered. Tiled and multi-plane modifiers need
//! a device that imports them as images; that import is not here, and a buffer with another modifier is refused by
//! [`DmabufError::Modifier`](super::dmabuf::DmabufError).
//!
//! # Failure modes
//!
//! 1. **A format is read with the wrong channel order.** The table maps each fourcc to the layout its bytes have, and the
//!    tests draw each:  `formats::tests::every_offered_format_has_a_layout`.
//! 2. **An implicit or tiled modifier is accepted and drawn as garbage.** Only linear is offered and checked on import.

use ash::vk;
use smithay::backend::allocator::{Format, Fourcc, Modifier};

use super::context::{SHM_FORMAT, TARGET_FORMAT};

/// How a fourcc's pixels sit in memory, as a texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Layout {
    /// The Vulkan format whose bytes are the fourcc's.
    pub format: vk::Format,
    /// Whether the fourth byte is padding, read as one.
    pub opaque: bool,
}

/// What the renderer imports and exports: four 32-bit layouts, which are what clients, `wl_shm` and the screen copy use.
const OFFERED: [(Fourcc, Layout); 4] = [
    (
        Fourcc::Argb8888,
        Layout {
            format: SHM_FORMAT,
            opaque: false,
        },
    ),
    (
        Fourcc::Xrgb8888,
        Layout {
            format: SHM_FORMAT,
            opaque: true,
        },
    ),
    (
        Fourcc::Abgr8888,
        Layout {
            format: TARGET_FORMAT,
            opaque: false,
        },
    ),
    (
        Fourcc::Xbgr8888,
        Layout {
            format: TARGET_FORMAT,
            opaque: true,
        },
    ),
];

/// The layout of `fourcc`, when the renderer offers it.
pub(super) fn layout(fourcc: Fourcc) -> Option<Layout> {
    OFFERED
        .iter()
        .find(|(offered, _)| *offered == fourcc)
        .map(|(_, layout)| *layout)
}

/// Whether the byte order of `fourcc` is blue first, so a copy to or from the renderer's red-first pixels swaps channels.
pub(super) fn blue_first(fourcc: Fourcc) -> bool {
    matches!(fourcc, Fourcc::Argb8888 | Fourcc::Xrgb8888)
}

/// The format and modifier pairs a client may import, and a capture may be written to.
#[must_use]
pub fn dmabuf_formats() -> Vec<Format> {
    OFFERED
        .iter()
        .map(|(code, _)| Format {
            code: *code,
            modifier: Modifier::Linear,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_offered_format_has_a_layout() {
        for format in dmabuf_formats() {
            assert_eq!(format.modifier, Modifier::Linear, "only linear is offered");
            assert!(layout(format.code).is_some(), "{:?}", format.code);
        }
        assert_eq!(layout(Fourcc::Argb8888).map(|layout| layout.opaque), Some(false));
        assert_eq!(layout(Fourcc::Xrgb8888).map(|layout| layout.opaque), Some(true));
        assert_eq!(
            layout(Fourcc::Abgr8888).map(|layout| layout.format),
            Some(TARGET_FORMAT)
        );
        assert_eq!(layout(Fourcc::Argb8888).map(|layout| layout.format), Some(SHM_FORMAT));
        assert!(layout(Fourcc::Nv12).is_none(), "a multi-plane format is not offered");
        assert!(blue_first(Fourcc::Xrgb8888) && !blue_first(Fourcc::Abgr8888));
    }
}
