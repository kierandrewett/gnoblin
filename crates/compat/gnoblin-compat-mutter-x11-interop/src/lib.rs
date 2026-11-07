//! `mutter_x11_interop`: a Wayland dialog's parent when the parent is an X11 window.
//!
//! This is `gnoblin-compat-mutter-x11-interop` of `design/rust-compositor.md` (the
//! redirect map: "generic parent relation"). A portal-built dialog is a Wayland toplevel,
//! but the window it belongs to can be an X11 one — an X11 application's file chooser, an
//! X11 application's print dialog — and the protocol is how the portal says so: it names
//! a `wl_surface` and an X11 window id, and the compositor is meant to make the Wayland
//! window transient for the X11 one.
//!
//! # What is delivered, and what is not
//!
//! Delivered: the protocol, the identity resolution on both sides of the boundary and the
//! recorded relation, with the X11 side looked up in `gnoblin-xwayland`'s window table
//! through the `gnoblin_backend::x11::Windows` seam and the Wayland side through the
//! compositor's own window table.
//!
//! Not delivered, and stated rather than implied: **applying the relation to a window
//! that is already managed**. `gnoblin-wm` takes a window's transient at
//! `Event::WindowOpened` and has no event to change it afterwards, so a dialog that is
//! already mapped keeps its own stacking. The relation is recorded and the support matrix
//! says `partial`; the missing piece is a window-manager event, not code in this crate.
//!
//! # Failure modes first
//!
//! 1. **An X11 window this session never managed becomes a parent.** The lookup answers
//!    nothing and no relation is recorded, so a dialog cannot attach itself to another
//!    application's window by guessing an id:
//!    `an_unmanaged_x11_window_is_not_a_parent`.
//! 2. **A surface that is not a window becomes a child.** [`gnoblin_wayland::Surface::Gone`]
//!    is not a relation: `a_surface_that_is_not_a_window_records_nothing`.
//! 3. **Two dialogs, one parent, and the second overwrites the first.** The table is keyed
//!    by the *child*, and a parent can have any number of children:
//!    `one_parent_can_have_many_children`.
//! 4. **A destroyed dialog leaves a relation behind.** [`ParentRelations::forget`] removes
//!    it, and the dispatch calls it when the object's destructor arrives:
//!    `forgetting_a_child_removes_only_that_child`.

// `deny`, not `forbid`: `wayland-scanner` emits the protocol's C interface tables and
// there is no generation mode without them, so `src/protocol.rs` allows unsafe code in its
// generated module and nowhere else. `Cargo.toml` says the same thing.
#![deny(unsafe_code)]

/// The admission gate: the support matrix and the mode check for each call.
pub mod compat;
/// The global, its object and the dispatch that records a parent relation.
pub mod global;
/// The recorded parent relations between Wayland and X11 windows.
pub mod relations;

/// Admission types and the support matrix, re-exported from `compat`.
pub use compat::{Admission, Call, MATRIX, NAME};
/// The redirect a session is given, and the recorded parent relations.
pub use global::Interop;
/// The parent of each Wayland window that has one.
pub use relations::ParentRelations;

/// The protocol's own names, for a client that has to speak it.
pub mod names {
    /// The protocol module, re-exported for clients.
    pub use crate::protocol::mutter_x11_interop;
}

mod protocol;

gnoblin_log::logger!(log = "compat-x11");
