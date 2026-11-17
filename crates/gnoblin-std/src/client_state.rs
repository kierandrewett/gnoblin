//! What a client keeps beyond its place in the tree: the room its frame takes, the sizes it allows itself, the rectangle it floats at,
//! and the zone of the work area it is snapped to.
//!
//! These are the TypeScript library's `client.frameInset`, `client.minSize` and `client.maxSize`, `client.floatingBounds` and
//! `gnoblin-std/snap`. The library keeps them (they outlive a shell's reload and every language's shell sees the same record), and the
//! moves, resizes and refits of the library and of the pointer bindings read them: a dragged window keeps its frame inside the work
//! area, a resize stops at the client's own limits, a window snapped to the left half is snapped again when the work area changes.
//!
//! ```ignore
//! // A frame provider tells the library how much room each window's frame takes.
//! client.set_frame_inset(FrameInset { top: 30.0, ..FrameInset::NONE })?;
//! client.set_min_size(Some((320.0, 200.0)))?;
//! ```
#![allow(unsafe_code)]

use gnoblin_core::Rect;
use gnoblin_std_abi::{self as abi, GnoblinStdClientState, GnoblinStdRect};

use crate::client::Client;
use crate::error::Result;

/// The room a window's frame takes on each side of its content, in logical pixels. Every side is 0 or more. The frame a layout places
/// is the content with this much added on each side.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameInset {
    /// Room above the content.
    pub top: f64,
    /// Room to the right of the content.
    pub right: f64,
    /// Room below the content.
    pub bottom: f64,
    /// Room to the left of the content.
    pub left: f64,
}

impl FrameInset {
    /// No room on any side: a window with no frame, and a fullscreen one.
    pub const NONE: FrameInset = FrameInset {
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
        left: 0.0,
    };

    /// The frame of a content rectangle: `content` with the inset added on each side.
    pub fn grow(&self, content: Rect) -> Rect {
        Rect::new(
            content.x - self.left,
            content.y - self.top,
            content.width + self.left + self.right,
            content.height + self.top + self.bottom,
        )
    }

    /// The content rectangle of a frame: `frame` less the inset on each side, never smaller than nothing.
    pub fn shrink(&self, frame: Rect) -> Rect {
        Rect::new(
            frame.x + self.left,
            frame.y + self.top,
            (frame.width - self.left - self.right).max(0.0),
            (frame.height - self.top - self.bottom).max(0.0),
        )
    }
}

/// The part of the work area a snapped window tiles to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapZone {
    /// The left half.
    Left,
    /// The right half.
    Right,
    /// All of it.
    Top,
}

impl SnapZone {
    fn raw(self) -> u32 {
        match self {
            SnapZone::Left => abi::STD_SNAP_LEFT,
            SnapZone::Right => abi::STD_SNAP_RIGHT,
            SnapZone::Top => abi::STD_SNAP_TOP,
        }
    }

    fn from_raw(raw: u32) -> Option<SnapZone> {
        match raw {
            abi::STD_SNAP_LEFT => Some(SnapZone::Left),
            abi::STD_SNAP_RIGHT => Some(SnapZone::Right),
            abi::STD_SNAP_TOP => Some(SnapZone::Top),
            _ => None,
        }
    }
}

fn rect_out(rect: Rect) -> GnoblinStdRect {
    GnoblinStdRect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }
}

fn rect_in(raw: &GnoblinStdRect) -> Rect {
    Rect::new(raw.x, raw.y, raw.width, raw.height)
}

impl Client {
    /// The library's record of the client, as the table hands it over.
    fn raw_state(&self) -> Result<GnoblinStdClientState> {
        let mut raw = GnoblinStdClientState {
            size: size_of::<GnoblinStdClientState>() as u32,
            flags: 0,
            inset_top: 0.0,
            inset_right: 0.0,
            inset_bottom: 0.0,
            inset_left: 0.0,
            min_width: 0.0,
            min_height: 0.0,
            max_width: 0.0,
            max_height: 0.0,
            floating: rect_out(Rect::default()),
            snap_zone: 0,
            reserved: 0,
            snap_floating: rect_out(Rect::default()),
        };
        let std = self.std();
        // SAFETY: `client_state` fills the struct it is given up to the `size` the caller set; `raw` is a live local.
        std.check(unsafe { (std.api().client_state)(std.ctx(), self.window().id().0, &mut raw) })?;
        Ok(raw)
    }

    /// Writes `raw`'s inset, limits and floating rectangle.
    fn put_state(&self, raw: &GnoblinStdClientState) -> Result<()> {
        let std = self.std();
        // SAFETY: `client_set_state` reads the struct it is given up to its `size`; `raw` is a live borrow.
        std.check(unsafe { (std.api().client_set_state)(std.ctx(), self.window().id().0, raw) })
            .map(|_| ())
    }

    /// The room the client's frame takes round its content. [`FrameInset::NONE`] until a frame provider says otherwise.
    ///
    /// # Errors
    ///
    /// The window is not a client of the compositor, or the generation retired.
    pub fn frame_inset(&self) -> Result<FrameInset> {
        let raw = self.raw_state()?;
        Ok(FrameInset {
            top: raw.inset_top,
            right: raw.inset_right,
            bottom: raw.inset_bottom,
            left: raw.inset_left,
        })
    }

    /// Sets the room the client's frame takes. The client's workspace computes again: a layout places the frame, so the content
    /// moves in by what the frame takes.
    ///
    /// # Errors
    ///
    /// A side is negative or not finite, the window is not a client, or the generation retired.
    pub fn set_frame_inset(&self, inset: FrameInset) -> Result<()> {
        let mut raw = self.raw_state()?;
        raw.inset_top = inset.top;
        raw.inset_right = inset.right;
        raw.inset_bottom = inset.bottom;
        raw.inset_left = inset.left;
        self.put_state(&raw)
    }

    /// The least size, `(width, height)`, a resize leaves the client, when one was set.
    ///
    /// # Errors
    ///
    /// The window is not a client of the compositor, or the generation retired.
    pub fn min_size(&self) -> Result<Option<(f64, f64)>> {
        let raw = self.raw_state()?;
        Ok((raw.flags & abi::STD_CLIENT_HAS_MIN != 0).then_some((raw.min_width, raw.min_height)))
    }

    /// Sets, or with `None` clears, the least size a resize leaves the client.
    ///
    /// # Errors
    ///
    /// The size is not finite, the window is not a client, or the generation retired.
    pub fn set_min_size(&self, size: Option<(f64, f64)>) -> Result<()> {
        let mut raw = self.raw_state()?;
        raw.flags &= !abi::STD_CLIENT_HAS_MIN;
        (raw.min_width, raw.min_height) = size.unwrap_or((0.0, 0.0));
        if size.is_some() {
            raw.flags |= abi::STD_CLIENT_HAS_MIN;
        }
        self.put_state(&raw)
    }

    /// The most a resize makes the client, `(width, height)`, when a limit was set.
    ///
    /// # Errors
    ///
    /// The window is not a client of the compositor, or the generation retired.
    pub fn max_size(&self) -> Result<Option<(f64, f64)>> {
        let raw = self.raw_state()?;
        Ok((raw.flags & abi::STD_CLIENT_HAS_MAX != 0).then_some((raw.max_width, raw.max_height)))
    }

    /// Sets, or with `None` clears, the most a resize makes the client.
    ///
    /// # Errors
    ///
    /// The size is not finite, the window is not a client, or the generation retired.
    pub fn set_max_size(&self, size: Option<(f64, f64)>) -> Result<()> {
        let mut raw = self.raw_state()?;
        raw.flags &= !abi::STD_CLIENT_HAS_MAX;
        (raw.max_width, raw.max_height) = size.unwrap_or((0.0, 0.0));
        if size.is_some() {
            raw.flags |= abi::STD_CLIENT_HAS_MAX;
        }
        self.put_state(&raw)
    }

    /// The rectangle the client floats at, when one was kept: the pointer bindings keep it after a move or a resize.
    ///
    /// # Errors
    ///
    /// The window is not a client of the compositor, or the generation retired.
    pub fn floating_bounds(&self) -> Result<Option<Rect>> {
        let raw = self.raw_state()?;
        Ok((raw.flags & abi::STD_CLIENT_HAS_FLOATING != 0).then(|| rect_in(&raw.floating)))
    }

    /// Keeps, or with `None` forgets, the rectangle the client floats at.
    ///
    /// # Errors
    ///
    /// The rectangle is not finite, the window is not a client, or the generation retired.
    pub fn set_floating_bounds(&self, bounds: Option<Rect>) -> Result<()> {
        let mut raw = self.raw_state()?;
        raw.flags &= !abi::STD_CLIENT_HAS_FLOATING;
        raw.floating = rect_out(bounds.unwrap_or_default());
        if bounds.is_some() {
            raw.flags |= abi::STD_CLIENT_HAS_FLOATING;
        }
        self.put_state(&raw)
    }

    /// The zone the client is snapped to, if it is.
    ///
    /// # Errors
    ///
    /// The window is not a client of the compositor, or the generation retired.
    pub fn snapped_zone(&self) -> Result<Option<SnapZone>> {
        Ok(SnapZone::from_raw(self.raw_state()?.snap_zone))
    }

    /// Tiles the client to `zone` of `area`, less its frame, when `edge_tiling` is on (the seat's `edge_tiling`): `true` when it
    /// snapped, `false` when it was refused and stayed. A maximised client floats first, so [`unsnap`](Client::unsnap) returns it to
    /// the rectangle it had before; a client that is snapped already keeps the rectangle it first floated at. The library snaps it again
    /// when the work area changes.
    ///
    /// # Errors
    ///
    /// The window is not a client of the compositor, or the generation retired.
    pub fn snap(&self, zone: SnapZone, area: Rect, edge_tiling: bool) -> Result<bool> {
        let std = self.std();
        let raw = rect_out(area);
        // SAFETY: `client_snap` reads the rectangle it is given for the call; `raw` is a live local.
        let snapped = std.check(unsafe {
            (std.api().client_snap)(
                std.ctx(),
                self.window().id().0,
                zone.raw(),
                &raw,
                u32::from(edge_tiling),
            )
        })?;
        Ok(snapped == 1)
    }

    /// Puts a snapped client back where it floated: `false` when it was not snapped.
    ///
    /// # Errors
    ///
    /// The window is not a client of the compositor, or the generation retired.
    pub fn unsnap(&self) -> Result<bool> {
        self.unsnap_with(false)
    }

    /// Ends the snapped state without moving the client, for a client the user is about to drag somewhere else: `false` when it was
    /// not snapped.
    ///
    /// # Errors
    ///
    /// The window is not a client of the compositor, or the generation retired.
    pub fn forget_snap(&self) -> Result<bool> {
        self.unsnap_with(true)
    }

    fn unsnap_with(&self, forget: bool) -> Result<bool> {
        let std = self.std();
        // SAFETY: `client_unsnap` takes the context, a window id and a flag.
        let was =
            std.check(unsafe { (std.api().client_unsnap)(std.ctx(), self.window().id().0, u32::from(forget)) })?;
        Ok(was == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_is_the_content_with_the_inset_round_it() {
        let inset = FrameInset {
            top: 30.0,
            right: 2.0,
            bottom: 3.0,
            left: 4.0,
        };
        let content = Rect::new(100.0, 100.0, 300.0, 200.0);
        let frame = inset.grow(content);
        assert_eq!(
            (frame.x, frame.y, frame.width, frame.height),
            (96.0, 70.0, 306.0, 233.0)
        );
        assert_eq!(inset.shrink(frame), content);
    }
}
