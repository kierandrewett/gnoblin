//! Pure core of view pixel capture for gnoblin-xr: the SHM buffer layout for a panel, and the
//! one-capture-per-panel frame cycle. The Wayland plumbing (the `gnoblin_view_source_manager_v1`
//! and `ext_image_copy_capture` objects, the `wl_shm` pool) drives these types and holds no state of
//! its own. Nothing here touches a file descriptor or a socket.

use std::collections::BTreeSet;
use std::fmt;

/// The `wl_shm` formats a panel capture may use. Both are 32 bits per pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShmFormat {
    Argb8888,
    Xrgb8888,
}

impl ShmFormat {
    /// Bytes per pixel. Both supported formats are 4.
    pub const fn bytes_per_pixel(self) -> u32 {
        4
    }
}

/// The byte layout of one panel buffer inside a `wl_shm` pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShmLayout {
    pub width: u32,
    pub height: u32,
    /// Bytes per row. Rows are tightly packed, so this is `width * bytes_per_pixel`.
    pub stride: u32,
    /// Total bytes: `stride * height`. Fits in the `int32` that `wl_shm.create_pool` takes.
    pub size: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutError {
    /// A panel has no pixels.
    Empty,
    /// The buffer would not fit in a `wl_shm` pool (its size must fit in an `int32`).
    TooLarge,
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("panel has no pixels"),
            Self::TooLarge => f.write_str("panel buffer does not fit in a wl_shm pool"),
        }
    }
}

impl std::error::Error for LayoutError {}

/// The layout of a `width` by `height` buffer in `format`, with tightly packed rows.
pub fn shm_layout(width: u32, height: u32, format: ShmFormat) -> Result<ShmLayout, LayoutError> {
    if width == 0 || height == 0 {
        return Err(LayoutError::Empty);
    }
    let stride = width
        .checked_mul(format.bytes_per_pixel())
        .ok_or(LayoutError::TooLarge)?;
    let size = u64::from(stride) * u64::from(height);
    let size = u32::try_from(size)
        .ok()
        .filter(|&size| size <= i32::MAX as u32)
        .ok_or(LayoutError::TooLarge)?;
    Ok(ShmLayout {
        width,
        height,
        stride,
        size,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CycleError {
    /// The panel already has a capture in flight. A consumer keeps one per panel.
    AlreadyPending(u64),
    /// A `ready` arrived for a panel with no capture in flight.
    NotPending(u64),
}

impl fmt::Display for CycleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyPending(panel) => write!(f, "panel {panel} already has a capture pending"),
            Self::NotPending(panel) => write!(f, "panel {panel} has no capture pending"),
        }
    }
}

impl std::error::Error for CycleError {}

/// The capture cycle for the panels of one scene: at most one capture in flight per panel.
///
/// `request` starts a capture, and `ready` ends it. Gnoblin sends `ready` for every panel whose
/// pixels changed before the set's `done`, so the last `ready` of a panel is its content.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CaptureCycle {
    pending: BTreeSet<u64>,
}

impl CaptureCycle {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts a capture of `panel`. Fails if one is already in flight for it.
    pub fn request(&mut self, panel: u64) -> Result<(), CycleError> {
        if self.pending.insert(panel) {
            Ok(())
        } else {
            Err(CycleError::AlreadyPending(panel))
        }
    }

    /// Ends the capture of `panel`. Fails if none is in flight for it.
    pub fn ready(&mut self, panel: u64) -> Result<(), CycleError> {
        if self.pending.remove(&panel) {
            Ok(())
        } else {
            Err(CycleError::NotPending(panel))
        }
    }

    pub fn is_pending(&self, panel: u64) -> bool {
        self.pending.contains(&panel)
    }

    /// Panels with a capture in flight, in ascending id order.
    pub fn pending(&self) -> impl Iterator<Item = u64> + '_ {
        self.pending.iter().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panel_layout_is_tightly_packed_argb() {
        let layout = shm_layout(640, 480, ShmFormat::Argb8888).expect("a valid panel");
        assert_eq!(layout.stride, 640 * 4);
        assert_eq!(layout.size, 640 * 4 * 480);
    }

    #[test]
    fn xrgb_has_the_same_layout_as_argb() {
        assert_eq!(
            shm_layout(3, 2, ShmFormat::Xrgb8888),
            shm_layout(3, 2, ShmFormat::Argb8888)
        );
    }

    #[test]
    fn an_empty_panel_has_no_layout() {
        assert_eq!(shm_layout(0, 480, ShmFormat::Argb8888), Err(LayoutError::Empty));
        assert_eq!(shm_layout(640, 0, ShmFormat::Argb8888), Err(LayoutError::Empty));
    }

    #[test]
    fn a_buffer_larger_than_a_wl_shm_pool_is_refused() {
        // 2^31 bytes is one past i32::MAX, the largest size wl_shm.create_pool accepts.
        assert_eq!(shm_layout(1 << 29, 1, ShmFormat::Argb8888), Err(LayoutError::TooLarge));
        assert_eq!(shm_layout(u32::MAX, 1, ShmFormat::Argb8888), Err(LayoutError::TooLarge));
    }

    #[test]
    fn a_panel_keeps_one_capture_in_flight() {
        let mut cycle = CaptureCycle::new();
        cycle.request(7).expect("first request");
        assert_eq!(cycle.request(7), Err(CycleError::AlreadyPending(7)));
        assert!(cycle.is_pending(7));
    }

    #[test]
    fn panels_are_independent() {
        let mut cycle = CaptureCycle::new();
        cycle.request(1).expect("panel 1");
        cycle.request(2).expect("panel 2");
        assert_eq!(cycle.pending().collect::<Vec<_>>(), vec![1, 2]);
        cycle.ready(1).expect("panel 1 ready");
        assert_eq!(cycle.pending().collect::<Vec<_>>(), vec![2]);
    }

    #[test]
    fn ready_without_a_request_is_an_error() {
        let mut cycle = CaptureCycle::new();
        assert_eq!(cycle.ready(3), Err(CycleError::NotPending(3)));
    }

    #[test]
    fn a_panel_can_be_captured_again_after_ready() {
        let mut cycle = CaptureCycle::new();
        cycle.request(4).expect("first");
        cycle.ready(4).expect("ready");
        cycle.request(4).expect("second capture after ready");
    }

    #[test]
    fn the_largest_buffer_that_fits_is_accepted() {
        // i32::MAX / 4 pixels across, one row: 4 bytes each, so the size is just under i32::MAX.
        let width = (i32::MAX as u32) / 4;
        let layout = shm_layout(width, 1, ShmFormat::Argb8888).expect("fits in an int32");
        assert_eq!(layout.size, width * 4);
        assert!(layout.size <= i32::MAX as u32);
    }
}
