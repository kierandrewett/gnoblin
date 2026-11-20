//! Compositor operations that more than one protocol crate needs.
//!
//! Each trait is implemented once, for `gnoblin-wayland`'s `State`. A protocol crate bounds its
//! functions by the traits it uses and by its own host trait, so its tests can run against a
//! small mock instead of a compositor.

use gnoblin_types::{OutputId, Rect, WindowId};
use smithay::output::Output;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

/// Which window a surface belongs to, and what a window's protocol state does to its role.
pub trait WindowHost {
    /// The policy identity of the window that owns `surface`, if it owns one.
    fn window_of(&self, surface: &WlSurface) -> Option<WindowId>;

    /// The rectangle the window occupies in the layout, if it has one.
    fn geometry_of(&self, window: WindowId) -> Option<Rect>;

    /// Re-reads the protocol state that decides a window's kind (a dialog hint, a parent) and
    /// tells the window manager when the kind changed.
    fn sync_window_role(&mut self, window: WindowId);
}

/// What every protocol that creates objects, sends events or reads another client's state asks first.
pub trait CoreHost {
    /// The display the session's globals and objects live on.
    fn display_handle(&self) -> &DisplayHandle;
}

/// What a protocol asks of the output layout: which output a Smithay output is, and where it is.
pub trait OutputHost {
    /// The identity the layout gave `output`, or `None` for an output it does not know.
    fn output_id_of(&self, output: &Output) -> Option<OutputId>;

    /// The rectangle `output` covers in the layout, in logical pixels.
    fn output_rect(&self, output: OutputId) -> Option<Rect>;
}
