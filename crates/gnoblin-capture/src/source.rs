//! Where a frame's pixels come from.
//!
//! Output capture and window capture are the same path: a session names a
//! [`Target`], and the compositor answers three questions about it — how big it is,
//! what changed, and copy it here. Keeping that behind a trait is what lets the
//! rules in [`crate::table`] be tested without a device, a Wayland socket or a
//! scene, and it is the seam a PipeWire producer plugs into when it lands
//! ([`crate::lib`] states why it is not here yet).

use gnoblin_types::Rect;

use crate::format::Format;
use crate::session::Target;

/// What a copy did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Copied {
    /// The region reached the destination.
    Copied,
    /// Nothing was written: the source has no pixels for that region, which is not
    /// the client's fault and does not end its session.
    Nothing,
}

/// The pixels of whatever the compositor can be asked to export.
///
/// Every method takes the target rather than being a per-target object, because a
/// session outlives a single answer: a capture of a window asks again on every
/// frame, which is what makes the copy follow the window instead of a rectangle
/// remembered from when the session opened.
pub trait CaptureSource {
    /// What `target` covers now, in the target's own pixels, or [`None`] when it is
    /// gone.
    ///
    /// A window's bounds are its own size at its own origin, so a window that moves
    /// on the desktop keeps the same bounds: where it is drawn is the source's
    /// business, and copying it must not depend on the client having a stable place.
    fn bounds(&self, target: Target) -> Option<Rect>;

    /// Copy `region` of `target` into the destination the frame named.
    ///
    /// The destination is the client's buffer, which the compositor holds for the
    /// duration of the call: a frame takes one buffer from the client and copies
    /// into it exactly once, which is the whole of "single use".
    fn copy(&mut self, target: Target, region: Rect, format: Format) -> Copied;
}
