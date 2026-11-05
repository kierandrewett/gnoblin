//! Smithay protocol handlers, one module per protocol.
//!
//! This is the Wayland engine shown in `design/rust-compositor.md`: the first slice
//! that runs. It serves a real client over a real socket and drives the real
//! handlers, so what it proves is a round trip rather than a compile.
//!
//! # What runs
//!
//! - [`Server`] owns a calloop event loop, a Wayland display, a backend and the
//!   compositor state, and starts with a socket path of your choosing.
//! - [`gnoblin_backend::headless::Headless`] is the backend that makes the session
//!   runnable anywhere: no DRM, no GPU, one virtual output with a settable size.
//! - The globals are `wl_compositor`, `wl_subcompositor`, `wl_shm`, `wl_output`
//!   (with `zxdg_output_manager_v1`), `wl_seat` with a keyboard and a pointer,
//!   `xdg_shell`, `wp_viewporter` and `zxdg_decoration_manager_v1`.
//! - Window state is `gnoblin-wm`'s: every map, unmap, configure, focus change and
//!   close is an [`gnoblin_wm::Event`] and comes back as an
//!   [`gnoblin_wm::Effect`]. Output rectangles come from `gnoblin-output`'s layout
//!   engine, so the window manager and the `wl_output` globals agree on where a
//!   monitor is.
//!
//! # What is deliberately not here
//!
//! The session lock global, interactive move and resize, popup grabs, rendering and
//! the scene graph belong to later slices. Nothing here is a stub that silently
//! pretends to work: a popup grab is refused and explained, a decoration request is
//! answered with client-side decorations because no frames are drawn, and a frame is
//! presented as nothing because there is no renderer.
//!
//! # Failure modes first
//!
//! Each entry names the integration test in `tests/real_client.rs` unless it is a
//! unit test in the module named.
//!
//! 1. **A client crash leaves a mapped ghost window.** A client that disconnects
//!    without destroying its toplevel has its resources destroyed by the display, so
//!    `toplevel_destroyed` runs and the window leaves the window manager:
//!    `a_client_that_disconnects_leaves_no_ghost_window`.
//! 2. **A configure is sent but never acknowledged and the session stalls.** Nothing
//!    waits for an acknowledgement: the unacknowledging client keeps its own window
//!    unconfigured and every other client keeps working, including a second configure
//!    for the client that never acked the first:
//!    `a_configure_that_is_never_acknowledged_does_not_stall_the_session`.
//! 3. **A window maps outside every output.** Placement is the window manager's, and
//!    it places inside the work area; with no output connected at all the window is
//!    refused rather than placed at a made-up position, and the session keeps
//!    running: `a_window_with_no_output_to_land_on_is_not_managed` and the geometry
//!    assertion in `a_real_client_maps_a_window_and_gets_focus`.
//! 4. **An `xdg_popup` grab arrives without a parent.** A parentless popup has
//!    nothing that can ever dismiss it, so Smithay refuses the commit as a protocol
//!    error on that client and the session keeps serving everyone else; a grab on a
//!    tracked popup is handed to [`protocols::xdg_shell`], which logs it rather than
//!    half-implementing a pointer grab:
//!    `a_popup_grab_without_a_parent_does_not_stop_the_session` and
//!    `a_popup_grab_on_a_tracked_parent_keeps_the_popup`.
//! 5. **A protocol error from a client takes the whole compositor down.** A client
//!    that asks for a second role on one surface gets the error on its own
//!    connection, and another client still maps, is focused and is cleaned up:
//!    `a_protocol_error_from_a_client_does_not_stop_the_compositor`.
//!
//! # Determinism
//!
//! Window identities are handed out in creation order, the window manager is
//! deterministic, and the only clock the protocol side reads is the serial counter.
//! A test therefore asserts on window order and geometry rather than on timing.

mod observation;
mod protocols;
mod server;
mod state;

pub use observation::{ObservedWindow, SessionSnapshot};
pub use server::{Config, ControlHandle, LoopError, Server, StartupError};
pub use state::State;

// The dispatch glue: one call implements `Dispatch` and `GlobalDispatch` for every
// `Dispatch2` implementation below, which is why no protocol module carries its own
// delegate macro.
smithay::delegate_dispatch2!(State);
