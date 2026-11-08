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
//!   [`gnoblin_lock::Event`], the effects it returns are what `protocols::ext_session_lock_v1`
//!   performs, and a frame carrying the armed cover generation on every output is what
//!   lets the locker hear `locked`. While anything is covered the frame path describes
//!   the opaque cover instead of the desktop, input reaches the owner's lock surface or
//!   nothing, and
//!   [`State::capture`]/[`State::capture_allowed`]/[`State::selection_allowed`] say
//!   what a capture or clipboard path may do.
//! - Injected input is the seat's own path. [`InputSink`] is the input-only handle a
//!   producer outside this crate delivers through — the EIS server in `gnoblin-input` is
//!   the one the plan names — and what it queues becomes an [`gnoblin_backend::InputEvent`]
//!   at the backend's own input hook, so the lock gate, grabs, bindings and focus apply
//!   to it exactly as they apply to a device.
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
//! # Protocols, and what is still missing
//!
//! The seat and input slice is served. Each entry below says what a client can rely on
//! today and what it still cannot.
//!
//! **Served, with a test in `tests/protocols.rs`:**
//!
//! - **`wp_fractional_scale_v1`** — a surface is told its output's preferred scale, and is
//!   told again when it moves to an output of another scale.
//! - **`wp_cursor_shape_v1`** — a client asks for a pointer shape by name and the session
//!   records it; the shape is not yet drawn, because there is no cursor plane in a headless
//!   output.
//! - **`zwp_relative_pointer_v1`** — relative motion is delivered to a client that asked
//!   for it, alongside the ordinary absolute motion.
//! - **`zwp_pointer_gestures_v1`** — hold, pinch and swipe exist as the interfaces a client
//!   binds. No backend of this slice reports a touchpad gesture, so nothing is delivered
//!   through them yet; `gnoblin-input`'s recogniser is what will emit the events.
//! - **`zwp_pointer_constraints_v1`** — a lock and a confine are granted, the lock reports
//!   relative motion, focus moving away releases it, and a session lock releases every
//!   lock it holds.
//! - **`ext_idle_notify_v1`** — a notification fires after its timeout, a resumed input
//!   stops it, and an inhibitor suppresses the whole session's idleness without leaking
//!   past the surface that made it.
//! - **`zwp_idle_inhibit_manager_v1`** — the same inhibitors through the older protocol.
//! - **`zwp_keyboard_shortcuts_inhibit_manager_v1`** — an inhibitor is granted to the
//!   focused client and refused while the session is locked.
//! - **`wp_presentation`** — a client that asks for feedback is answered after the frame it
//!   described reached the output, with a timestamp read from the frame.
//! - **`wp_content_type_v1`** — the hint a client sets is kept per surface and read back
//!   through [`State::content_type`], so a policy has one function to call. The frame path
//!   of this slice reads no content type: it composes and presents every surface alike.
//! - **`wp_single_pixel_buffer_v1`** — a one-pixel buffer is a complete buffer, which is
//!   what a solid background or a session lock's cover needs.
//! - **`wp_tearing_control_v1`** — the hint is kept per surface and read back through
//!   [`State::tearing_hint`]. A headless output presents nothing, so there is no vsync for
//!   it to lift and the hint has no visible effect.
//! - **`xdg_system_bell_v1`** — a ring is counted on the session snapshot, and flashes the screen
//!   when the visual bell is configured; the flash is suppressed while the session is locked.
//!   The request may name the surface that rang, which attributes the ring to its window.
//! - **`zwp_text_input_v3`** and **`zwp_input_method_v2`** — a text input is told it has
//!   focus, the input method is told when one is enabled, and a `commit_string` reaches the
//!   focused text input. A text input created on an already-focused surface is told it has
//!   focus too, which Smithay alone does not do.
//! - **`zwp_virtual_keyboard_v1`** — an injected key is routed through the same
//!   [`State::dispatch_key`] as a physical one, so a binding still wins over an on-screen
//!   keyboard.
//! - **xkb layouts, per-window layout memory and the router.** The document's
//!   `input-sources` become the keymap, a switch rebuilds it, a window remembers the layout
//!   it had when focus left it and gets it back when focus returns, and key delivery
//!   applies the evdev offset so a client's `keysym` is what the evdev code meant. The
//!   delivery order is `gnoblin-input`'s: grab, binding, client.
//!
//! **Not served, and not pretended:**
//!
//! - **Touch gestures that carry data.** The seat serves `wl_touch` points (down, motion, up,
//!   cancel) and a point goes to the window under it. `wl_touch.shape` and
//!   `wl_touch.orientation` are never sent: the backends of this slice report position only.
//! - **Pointer gestures that carry data.** The gesture interfaces bind, but no hold, pinch or
//!   swipe ever arrives: the backends of this slice report motion, buttons, keys, scroll and
//!   touch points only.
//! - **Cursor drawing.** A `wp_cursor_shape_v1` or `wl_cursor` request is recorded and never
//!   painted, for the same reason: a headless output has no pointer plane.
//! - **Text input on a surface that is not a `xdg_toplevel`** — there is no input-panel or
//!   popup text input path, and an input method's popups are recorded for a renderer that
//!   does not draw them yet.
//! - **`zwp_input_method_v1`** (the original, deprecated input-method protocol). Only v2 is
//!   served.
//! - **Idle inhibition by a client's own request** beyond what the two protocols above
//!   state: the session's idleness is `gnoblin-input`'s counter, and a protocol inhibitor is
//!   the only way to hold it.
//!
//! # Failure modes first
//!
//! Each entry names the integration test in `tests/real_client.rs` unless it says
//! otherwise; the session lock's own list is at the top of
//! `protocols::ext_session_lock_v1` and its tests are in `tests/session_lock.rs`.
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

mod access;
mod clock;
mod effects;
mod frame;
mod keyboard;
mod liveness;
mod modal;
mod observation;
mod output_config;
mod output_control;
mod process;
mod protocols;
mod selection_bridge;
mod server;
mod state;
mod xwayland;

/// Clocks the frame and timeout code reads: the system clock, and a manual one for tests.
pub use clock::{Clock, ManualClock, SystemClock};
/// Frame counters, and the timeout a screenshot waits for before it gives up.
pub use frame::{FrameStats, SCREENSHOT_TIMEOUT};
/// A modal dialog's state, and the events that change it.
pub use modal::{Modal, ModalEvent};
/// Snapshot types a reader observes, and the observer trait it implements.
pub use observation::{
    ActivationDenial, EffectSpec, LockObservation, MenuAnchor, MenuRequest, MenuSource, ObservedLayer, ObservedOutput,
    ObservedWindow, ObservedWorkspace, Observer, SessionSnapshot,
};
pub use observation::{PresentAnswer, WindowChange, WindowChangeRefusal};
/// Output configuration types a host reports and applies, and the host trait.
pub use output_config::{HeadInfo, HeadRequest, ModeInfo, ModeRequest, OutputConfigHost};
/// Output control: the trait a backend implements and the no-op default.
pub use output_control::{NoOutputControl, OutputControl};
/// Server configuration, control handle, input sink, startup and loop errors.
pub use server::{
    Config, ControlHandle, CursorSettings, DEFAULT_PRIVILEGED_SOCKET_NAME, DEFAULT_SOCKET_NAME, FOCUS_TIMEOUT,
    InputSink, LoopError, MonitorSettings, Server, StartupError, VisualAccessibilitySettings, VisualBellScope,
    VisualBellSettings, default_privileged_socket_path, privileged_socket_path_in,
};
/// The dispatch glue a redirect implements.
///
/// `smithay`'s one-call `Dispatch2` and `GlobalDispatch2` plus the `delegate_dispatch2!`
/// that bridges them, re-exported because the redirect adapters reach the transport
/// through this crate: a compat adapter names its transport, never a second copy of
/// the library behind it.
pub use smithay::delegate_dispatch2;
/// Smithay's dispatch traits, which the redirect adapters implement through this crate.
pub use smithay::wayland::{Dispatch2, GlobalDispatch2};

/// The identity and the window process a client's surface belongs to.
pub use process::{ProcessIdentity, WindowProcess};
/// Protocol extension trait and host traits, with the activation and surface types they use.
pub use protocols::extension::{
    Activation, Extension, ExtensionError, Shell1Host, Surface, Titlebar, TitlebarActions, X11InteropHost,
};
/// The layout control and the session's state.
pub use state::{LayoutControl, State};
/// The X11 session that runs Xwayland.
pub use xwayland::XwaylandSession;

/// Access policy: which global a client role may bind, and the registration that records it.
pub use access::{Access, AccessPolicy, ClientRole, Global, Registration};
/// The observed blur rectangle that `ext-background-effect-v1` reports.
pub use protocols::ext_background_effect_v1::ExtBackgroundEffectV1ObservedBlurRect;
/// Session host protocol types: requests, power actions, output hints and hosted sessions.
pub use protocols::gnoblin_session_host_v1::{
    GnoblinSessionHostV1HostedSession, GnoblinSessionHostV1OutputHint, GnoblinSessionHostV1PowerAction,
    GnoblinSessionHostV1Request, GnoblinSessionHostV1RequestKind,
};
/// Window frame protocol types: frame actions, regions and the frame itself.
pub use protocols::gnoblin_window_frame_v1::{
    GnoblinWindowFrameV1Action, GnoblinWindowFrameV1Frame, GnoblinWindowFrameV1Region,
};

gnoblin_log::logger!(log = "wayland");

// The dispatch glue: one call implements `Dispatch` and `GlobalDispatch` for every
// `Dispatch2` implementation below, which is why no protocol module carries its own
// delegate macro.
smithay::delegate_dispatch2!(State);
