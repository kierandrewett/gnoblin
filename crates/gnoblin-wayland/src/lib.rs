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
//!   `xdg_shell`, `ext_session_lock_manager_v1`, `wp_viewporter` and
//!   `zxdg_decoration_manager_v1`.
//! - [`Server::with_xwayland`] adds X11: `xwayland-shell-v1`, the Xwayland keyboard grab
//!   manager, the data device Smithay's X11 window manager needs and the Xwayland process
//!   itself. The decisions are `gnoblin-xwayland`'s: this crate spawns, configures and
//!   performs, and asks `gnoblin_backend::x11` what each X11 event means. A restart after
//!   a crash is the session's policy, and a server that never becomes ready is a failure
//!   rather than a hang.
//! - Window state is `gnoblin-wm`'s: every map, unmap, configure, focus change and
//!   close is an [`gnoblin_wm::Event`] and comes back as an
//!   [`gnoblin_wm::Effect`]. Output rectangles come from `gnoblin-output`'s layout
//!   engine, so the window manager and the `wl_output` globals agree on where a
//!   monitor is.
//! - The frame path runs every iteration: the hook in `frame.rs` asks each output for its
//!   damage, and a client's committed `wl_shm` buffer reaches the output's pixels
//!   through the software renderer in `gnoblin-backend`. [`ControlHandle::screenshot`]
//!   reads those pixels back, which is what the pixel tests assert on.
//! - Session locking is `gnoblin-lock`'s: a lock request is an
//!   [`gnoblin_lock::Event`], the effects it returns are what `protocols::session_lock`
//!   performs, and a frame carrying the armed cover generation on every output is what
//!   lets the locker hear `locked`. While anything is covered the frame path describes
//!   the opaque cover instead of the desktop, input reaches the owner's lock surface or
//!   nothing, and
//!   [`State::capture`]/[`State::capture_allowed`]/[`State::selection_allowed`] say
//!   what a capture or clipboard path may do.
//!
//! # What is deliberately not here
//!
//! Interactive move and resize, popup grabs, drawing the lock surface itself and the
//! effects passes belong to later slices, and so does the capture and selection surface
//! the lock embargo guards. Nothing here is a stub that silently pretends to work: a
//! popup grab is refused and explained, a decoration request is answered with
//! client-side decorations, and a frame is presented as
//! [`gnoblin_backend::Present::Nothing`] because a headless output scans out nothing —
//! the pixels are still composed and readable.
//!
//! # Failure modes first
//!
//! Each entry names the integration test in `tests/real_client.rs` unless it says
//! otherwise; the session lock's own list is at the top of
//! `protocols::session_lock` and its tests are in `tests/session_lock.rs`.
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
//! 6. **A frame is drawn for a window that has attached nothing.** A window with no
//!    committed buffer has no scene node and contributes no layer, and the copied
//!    buffer is never read as if it were pixels:
//!    `a_window_without_a_buffer_is_not_drawn` and
//!    `a_stride_padded_buffer_is_read_from_the_right_offsets` in `tests/render.rs`.
//! 7. **A move leaves the old pixels behind.** The scene damages both rectangles and
//!    the frame repaints exactly those:
//!    `moving_a_window_moves_its_pixels_and_clears_the_old_rectangle`.
//! 8. **An idle iteration repaints the output.** [`FrameStats`] counts iterations and
//!    frames separately, so a session with nothing to redraw leaves the frame count
//!    still: `an_idle_session_draws_no_frames` and `frame::tests`.
//! 9. **A rotated window is drawn as its bounding box, or a half-opaque one is drawn
//!    opaque.** The composer refuses the rotation and the renderer blends the
//!    opacity: `overlapping_windows_are_drawn_in_stacking_order` and
//!    `a_half_transparent_window_blends_with_the_background`.
//!
//! # Determinism
//!
//! Window identities are handed out in creation order, the window manager is
//! deterministic, and the only clock the protocol side reads is the serial counter.
//! A test therefore asserts on window order and geometry rather than on timing. The
//! session lock reads no clock either: its barrier is a generation counter, and the
//! loop's turn is the only thing that decides when a dead connection is noticed.

mod frame;
mod observation;
mod protocols;
mod server;
mod state;
mod xwayland;

pub use frame::{FrameStats, SCREENSHOT_TIMEOUT};
pub use observation::{LockObservation, ObservedWindow, SessionSnapshot};
pub use server::{Config, ControlHandle, LoopError, Server, StartupError};
pub use state::State;
pub use xwayland::XwaylandSession;

// The dispatch glue: one call implements `Dispatch` and `GlobalDispatch` for every
// `Dispatch2` implementation below, which is why no protocol module carries its own
// delegate macro.
smithay::delegate_dispatch2!(State);
