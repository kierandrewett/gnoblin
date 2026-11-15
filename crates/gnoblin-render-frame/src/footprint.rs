//! What a renderer holds in memory for one output, by what holds it.
//!
//! The compositor cannot ask a driver how much video memory a texture took, so each renderer states the bytes of
//! the objects it made from their size and format. The number is the nominal storage of the object (width times
//! height times the bytes per pixel of the format it asked for); a driver may pad or tile it, so it is a floor on
//! what the driver holds and an exact count of what this code asked for. That is what attributes growth: a number
//! that rises with the session names the allocator that grows, and the per-process `fdinfo` of the kernel's DRM
//! driver (`drm-memory-*`) is the cross-check on a real GPU.

use gnoblin_types::Size;

/// Bytes per pixel of the `RGBA8` colour format every renderer draws in.
pub const COLOUR_BYTES_PER_PIXEL: u64 = 4;

/// The bytes one renderer holds for one output.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Footprint {
    /// The output-sized colour target with its depth buffer, and the copy a supersampled output averages down into.
    pub target: u64,
    /// The effect intermediates: the blur scratch pair and the tint scratch.
    pub scratch: u64,
    /// The host-visible upload, uniform, vertex and readback buffers a renderer keeps between frames. They are
    /// grown to the largest frame seen, so this is the number that shows a ratchet.
    pub staging: u64,
    /// The pictures of the Views drawn on the GPU.
    pub views: u64,
}

impl Footprint {
    /// Everything this renderer holds.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.target
            .saturating_add(self.scratch)
            .saturating_add(self.staging)
            .saturating_add(self.views)
    }
}

/// The nominal bytes of `size` pixels at `bytes_per_pixel`.
#[must_use]
pub fn image_bytes(size: Size, bytes_per_pixel: u64) -> u64 {
    u64::from(size.width)
        .saturating_mul(u64::from(size.height))
        .saturating_mul(bytes_per_pixel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_is_width_times_height_times_the_pixel_size() {
        assert_eq!(
            image_bytes(Size::new(1920, 1080), COLOUR_BYTES_PER_PIXEL),
            1920 * 1080 * 4
        );
    }

    #[test]
    fn a_size_past_the_word_saturates_instead_of_wrapping() {
        assert_eq!(image_bytes(Size::new(u32::MAX, u32::MAX), u64::MAX), u64::MAX);
    }

    #[test]
    fn the_total_is_the_sum_of_what_holds_the_bytes() {
        let footprint = Footprint {
            target: 8,
            scratch: 16,
            staging: 32,
            views: 64,
        };
        assert_eq!(footprint.total(), 120);
        assert_eq!(Footprint::default().total(), 0);
    }
}
