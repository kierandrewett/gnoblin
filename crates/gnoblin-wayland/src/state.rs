//! The compositor state: the Smithay protocol state, the window manager and the
//! translation between them.
//!
//! Every protocol handler in [`crate::protocols`] turns a request into an event for
//! [`gnoblin_wm::Wm::apply`] and hands the returned effects back here, where they
//! become client configures, seat focus and output enter/leave. Policy never sees a
//! Smithay type and this module never decides policy: the same split the plan draws
//! between the composition root and the pure crates.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use smithay::backend::input::{ButtonState, InputTime};
use smithay::backend::renderer::utils::with_renderer_surface_state;
use smithay::desktop::PopupManager;
use smithay::input::keyboard::KeyboardHandle;
use smithay::input::pointer::{ButtonEvent, CursorImageStatus, MotionEvent, RelativeMotionEvent};
use smithay::input::{Seat, SeatState, pointer::PointerHandle};
use smithay::output::{Mode, Output, PhysicalProperties, Scale as OutputScale, Subpixel};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::Display;
use smithay::reexports::wayland_server::backend::{GlobalId, ObjectId};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, DisplayHandle, Resource};
use smithay::utils::{Logical, Point as SmithayPoint, Rectangle, SERIAL_COUNTER, Size as SmithaySize, Transform};
use smithay::wayland::compositor::{CompositorState, SurfaceAttributes, with_states};
use smithay::wayland::content_type::ContentTypeState;
use smithay::wayland::cursor_shape::CursorShapeManagerState;
use smithay::wayland::foreign_toplevel_list::ForeignToplevelListState;
use smithay::wayland::fractional_scale::FractionalScaleManagerState;
use smithay::wayland::idle_inhibit::IdleInhibitManagerState;
use smithay::wayland::input_method::{InputMethodManagerState, PopupSurface};
use smithay::wayland::keyboard_shortcuts_inhibit::{KeyboardShortcutsInhibitState, KeyboardShortcutsInhibitor};
use smithay::wayland::output::OutputManagerState;
use smithay::wayland::pointer_constraints::PointerConstraintsState;
use smithay::wayland::pointer_gestures::PointerGesturesState;
use smithay::wayland::presentation::PresentationState;
use smithay::wayland::relative_pointer::RelativePointerManagerState;
use smithay::wayland::security_context::{SecurityContextListenerSource, SecurityContextState};
use smithay::wayland::selection::data_device::DataDeviceState;
use smithay::wayland::selection::ext_data_control::DataControlState as ExtDataControlState;
use smithay::wayland::selection::primary_selection::PrimarySelectionState;
use smithay::wayland::selection::wlr_data_control::DataControlState as WlrDataControlState;
use smithay::wayland::session_lock::SessionLockManagerState;
use smithay::wayland::shell::wlr_layer::WlrLayerShellState;
use smithay::wayland::shell::xdg::decoration::XdgDecorationState;
use smithay::wayland::shell::xdg::dialog::{ToplevelDialogHint, XdgDialogState};
use smithay::wayland::shell::xdg::{ToplevelStateSet, ToplevelSurface, XdgShellState, XdgToplevelSurfaceData};
use smithay::wayland::shm::ShmState;
use smithay::wayland::single_pixel_buffer::SinglePixelBufferState;
use smithay::wayland::text_input::TextInputManagerState;
use smithay::wayland::viewporter::ViewporterState;
use smithay::wayland::virtual_keyboard::VirtualKeyboardManagerState;
use smithay::wayland::xdg_activation::XdgActivationState;
use smithay::wayland::xdg_system_bell::XdgSystemBellState;
use smithay::wayland::xdg_toplevel_icon::{ToplevelIconCachedState, XdgToplevelIconManager};
use smithay::wayland::xdg_toplevel_tag::{XdgToplevelTagManager, XdgToplevelTagSurfaceData};
use smithay::wayland::xwayland_keyboard_grab::XWaylandKeyboardGrabState;
use smithay::wayland::xwayland_shell::XWaylandShellState;
use smithay::xwayland::xwm::X11Surface;

use gnoblin_backend::render::compose::{Composer, Content};
use gnoblin_backend::{Host, InputEvent};
use gnoblin_config::Protocols;
use gnoblin_input::Activity;
use gnoblin_layers::InputPolicy;
use gnoblin_lock::{Embargo, Lock, State as LockState};
use gnoblin_output::{LayoutConfig, compute};
use gnoblin_render::Mat4;
use gnoblin_render_frame::frame::{ColourMatrix, Layer, OutputFrame, View};
use gnoblin_types::{OutputId, Rect, RectF, Scale, Size, Tick, Vec3, WindowId, WorkspaceId};
use gnoblin_wm::layout::{LayoutEngine, Placement, Transform3};
use gnoblin_wm::{ActivationDecision, Effect, Event, TiledEdges, WindowKind, WindowSpec, Wm, WmOptions};

use crate::access::{Access, AccessPolicy, ClientRole, Global};
use crate::keyboard::KeyboardTables;
use crate::modal::{Modal, ModalEvent};
use crate::observation::{
    LockObservation, MenuAnchor, MenuRequest, MenuSource, ObservedWindow, Observer, SessionSnapshot,
};
use crate::output_control::OutputControlState;
use crate::protocols::capture_shared::CaptureTables;
use crate::protocols::ext_idle_notify_v1::ExtIdleNotifyV1Tables;
use crate::protocols::ext_session_lock_v1::ExtSessionLockV1Tables;
use crate::protocols::extension::{Extension, TitlebarActions};
use crate::protocols::foreign_toplevel_shared::ForeignToplevels;
use crate::protocols::gnoblin_session_host_v1::GnoblinSessionHostV1Tables;
use crate::protocols::gnoblin_window_frame_v1::GnoblinWindowFrameV1Tables;
use crate::protocols::wl_data_device_manager::WlDataDeviceManagerSelections;
use crate::protocols::wp_presentation_time::WpPresentationTimeTables;
use crate::protocols::wp_tearing_control_v1::WpTearingControlV1Hint;
use crate::protocols::xdg_activation_v1::XdgActivationV1;
use crate::protocols::zwlr_layer_shell_v1::ZwlrLayerShellV1Layers;
use crate::protocols::zwlr_output_management_v1::ZwlrOutputManagementV1State;
use crate::protocols::zwp_pointer_constraints_v1::ZwpPointerConstraintsV1Tables;
use crate::server::StartupError;

use crate::log;

/// How many workspaces a session declares when its configuration names none.
const DEFAULT_WORKSPACES: u32 = 1;

/// How many window menu requests the session keeps for a reader to catch up on. A reader
/// that polls slower than requests arrive loses the oldest, so the bound is generous: a
/// request is a human action, not a stream.
const MENU_REQUESTS_KEPT: usize = 32;

/// The keyboard repeat settings a session starts with: the delay before a held key
/// repeats, and the gap between repeats, both in milliseconds. These are the values the
/// configuration reference defaults to.
pub(crate) const KEY_REPEAT_DELAY: i32 = 200;
/// The gap, in milliseconds, between repeats once a key repeat has started.
pub(crate) const KEY_REPEAT_INTERVAL: i32 = 40;

/// The protocol object behind a managed window.
///
/// A Wayland window is its `xdg_toplevel`; an X11 window is the X11 surface the window
/// manager created for it, and its `wl_surface` exists only once Xwayland has associated
/// one. Keeping both in one enum is what lets the rest of this module treat "a managed
/// window" without knowing which protocol it came from.
///
/// The X11 surface is boxed because it is four times the size of a toplevel and the two
/// live in the same table: an unboxed enum would pay for the larger variant on every
/// Wayland window in the session.
pub(crate) enum Managed {
    /// An `xdg_toplevel`.
    Wayland(ToplevelSurface),
    /// An X11 window managed through Xwayland.
    X11(Box<X11Surface>),
}

impl Managed {
    /// The Wayland surface behind the window, when one exists yet.
    pub(crate) fn wl_surface(&self) -> Option<WlSurface> {
        match self {
            Self::Wayland(surface) => Some(surface.wl_surface().clone()),
            Self::X11(surface) => surface.wl_surface(),
        }
    }

    /// The toplevel, for a window that is a Wayland one.
    pub(crate) fn toplevel(&self) -> Option<&ToplevelSurface> {
        match self {
            Self::Wayland(surface) => Some(surface),
            Self::X11(_) => None,
        }
    }
}

/// One toplevel the server is managing.
pub(crate) struct WindowEntry {
    /// The protocol object the client owns.
    pub(crate) surface: Managed,
    /// Whether the client has committed a buffer, which is what "mapped" means for
    /// a toplevel.
    pub(crate) mapped: bool,
    /// The application id the client last sent.
    pub(crate) app_id: String,
    /// The title the client last sent.
    pub(crate) title: String,
    /// The toplevel icon name the client set, if any (`xdg-toplevel-icon-v1`).
    pub(crate) icon_name: Option<String>,
    /// How many icon buffers the client attached (`xdg-toplevel-icon-v1`).
    pub(crate) icon_buffer_count: usize,
    /// The toplevel tag the client set (`xdg-toplevel-tag-v1`), if any.
    pub(crate) tag: Option<String>,
    /// The effect stack the client set through `window.add_effect`, in paint order. The
    /// server owns it: the observed window and the renderer both read it from here.
    pub(crate) effects: Vec<crate::observation::EffectSpec>,
}

impl WindowEntry {
    /// An entry for an X11 window the manager took in.
    ///
    /// An X11 window is mapped when the manager tells the server it is, so it starts
    /// mapped: the entry exists from that point on, and an unmap removes it.
    pub(crate) fn x11(surface: X11Surface) -> Self {
        Self {
            surface: Managed::X11(Box::new(surface)),
            mapped: true,
            app_id: String::new(),
            title: String::new(),
            icon_name: None,
            icon_buffer_count: 0,
            tag: None,
            effects: Vec::new(),
        }
    }
}

/// One output the server presents on.
pub(crate) struct OutputEntry {
    /// The backend's identity for the output.
    pub(crate) backend: OutputId,
    /// The window manager's identity for it.
    policy: OutputId,
    /// The Smithay output, which owns the `wl_output` global.
    pub(crate) output: Output,
    /// The global the output created.
    global: GlobalId,
    /// The logical rectangle the layout assigned.
    rect: Rect,
}

/// The compositor state every protocol handler receives.
///
/// Builds the layout engines for the window manager and takes runtime layout changes.
///
/// The compositor implements this over its per-workspace and per-output overrides, so
/// `set_workspace` and `set_output` store the choice that `engine` later resolves.
pub trait LayoutControl {
    /// Builds a fresh engine for `workspace`, resolving its layout from the overrides.
    fn engine(&self, workspace: WorkspaceId, output: Option<&str>) -> Box<dyn LayoutEngine>;

    /// Stores `name` as `workspace`'s layout override.
    ///
    /// # Errors
    /// A refusal message when `name` is not a known layout.
    fn set_workspace(&self, workspace: WorkspaceId, name: &str) -> Result<(), String>;

    /// Stores `name` as `output`'s default layout.
    ///
    /// # Errors
    /// A refusal message when `name` is not a known layout.
    fn set_output(&self, output: &str, name: &str) -> Result<(), String>;

    /// The layout name `workspace` runs now, as the engine would resolve it.
    fn layout_name(&self, workspace: WorkspaceId, output: Option<&str>) -> String;

    /// The default layout of `output`, which its workspaces fall back to.
    fn output_layout(&self, output: &str) -> String;
}

/// The fields are private: a handler asks for what it needs through the accessor
/// methods below, so a protocol module cannot reach into another one's table.
pub struct State {
    /// The handle every protocol handler needs to create objects and globals.
    pub(crate) display_handle: DisplayHandle,
    /// The Wayland display, taken out for the duration of a dispatch.
    ///
    /// `Display::dispatch_clients` needs `&mut Display` and `&mut State` at the
    /// same time, and the state is the type the display is generic over, so the
    /// two cannot be borrowed at once through `&mut self`. Taking the display out
    /// and putting it back is the safe way to hand both to the dispatcher; the
    /// alternative is an `unsafe` `get_mut` on an event source, which this
    /// workspace forbids.
    display: Option<Display<State>>,

    // The Smithay protocol state, one field per protocol module.
    /// The `wl_compositor` state, which every `wl_surface` is created through.
    pub(crate) compositor_state: CompositorState,
    /// The `xdg_wm_base` state, the shell that every toplevel and popup is created from.
    pub(crate) xdg_shell_state: XdgShellState,
    /// The `wl_shm` state, which advertises the shared-memory formats clients can use.
    pub(crate) shm_state: ShmState,
    /// The seat protocol state, which owns the seat globals and its client list.
    pub(crate) seat_state: SeatState<State>,
    /// The popup tree for `xdg_popup` surfaces, so they are stacked over their parents.
    pub(crate) popups: PopupManager,
    /// Kept alive so the `zxdg_output_manager_v1` global stays registered.
    #[allow(dead_code)]
    pub(crate) output_manager_state: OutputManagerState,
    /// Kept alive so the `wp_viewporter` global stays registered. The protocol
    /// needs no handler trait: Smithay validates a viewport on commit.
    #[allow(dead_code)]
    pub(crate) viewporter_state: ViewporterState,
    /// Kept alive so the `zxdg_decoration_manager_v1` global stays registered; the
    /// handler in [`crate::protocols::zxdg_decoration_v1`] answers its requests.
    #[allow(dead_code)]
    pub(crate) decoration_state: XdgDecorationState,
    /// Kept alive so the `ext_session_lock_manager_v1` global stays registered; the
    /// handler in [`crate::protocols::ext_session_lock_v1`] answers its requests.
    pub(crate) session_lock_state: SessionLockManagerState,
    /// Kept alive so `xwayland-shell-v1` stays registered. Only the Xwayland client can
    /// bind it, so the global is invisible to every other one; it is created only when a
    /// session is configured, and the handler below is only reachable with one.
    pub(crate) xwayland_shell_state: Option<XWaylandShellState>,
    /// Kept alive so `zwp_xwayland_keyboard_grab_manager_v1` stays registered. Privileged
    /// in the same way: Xwayland is its only client.
    #[allow(dead_code)]
    pub(crate) keyboard_grab_state: Option<XWaylandKeyboardGrabState>,
    /// The Xwayland session, when one is configured.
    pub(crate) x11: Option<crate::xwayland::Running>,
    /// The layer shell, which places bars and docks against output edges and feeds
    /// their exclusive zones to the window manager as reserved insets.
    pub(crate) layer_shell_state: WlrLayerShellState,
    /// The standardised toplevel list, which a shell reads through
    /// [`crate::protocols::ext_foreign_toplevel_list_v1`].
    pub(crate) foreign_toplevel_list_state: ForeignToplevelListState,
    /// The activation tokens, decided by `gnoblin-authority`.
    pub(crate) activation_state: XdgActivationState,
    /// The clipboard and the primary selection; both have sources kept beside them.
    pub(crate) data_device_state: DataDeviceState,
    /// The primary selection (middle-click paste) state.
    pub(crate) primary_selection_state: PrimarySelectionState,
    /// The privileged clipboard managers. They are created even when the
    /// configuration hides them, because a state that does not exist cannot be
    /// re-advertised on a reload; the global itself is removed instead.
    pub(crate) ext_data_control_state: ExtDataControlState,
    /// The `wlr-data-control` clipboard manager state.
    pub(crate) wlr_data_control_state: WlrDataControlState,
    /// Application sandboxes: a client that arrives through one is marked sandboxed
    /// and sees no privileged global.
    #[allow(dead_code)]
    pub(crate) security_context_state: SecurityContextState,

    // The seat and its keyboard features. One field per protocol, as above.
    /// Kept alive so `wp_fractional_scale_manager_v1` stays registered; the handler in
    /// [`crate::protocols::wp_fractional_scale_v1`] answers its requests.
    #[allow(dead_code)]
    pub(crate) fractional_scale_state: FractionalScaleManagerState,
    /// Kept alive so `wp_cursor_shape_manager_v1` stays registered.
    #[allow(dead_code)]
    pub(crate) cursor_shape_state: CursorShapeManagerState,
    /// Kept alive so `zwp_relative_pointer_manager_v1` stays registered.
    #[allow(dead_code)]
    pub(crate) relative_pointer_state: RelativePointerManagerState,
    /// Kept alive so `zwp_pointer_constraints_v1` stays registered; the handler in
    /// [`crate::protocols::zwp_pointer_constraints_v1`] decides which constraints are active.
    #[allow(dead_code)]
    pub(crate) pointer_constraints_state: PointerConstraintsState,
    /// Kept alive so `zwp_pointer_gestures_v1` stays registered.
    #[allow(dead_code)]
    pub(crate) pointer_gestures_state: PointerGesturesState,
    /// Kept alive so `zwp_idle_inhibit_manager_v1` stays registered; the handler in
    /// [`crate::protocols::zwp_idle_inhibit_v1`] records the inhibitors.
    #[allow(dead_code)]
    pub(crate) idle_inhibit_state: IdleInhibitManagerState,
    /// Kept alive so `zwp_keyboard_shortcuts_inhibit_manager_v1` stays registered, and
    /// read by the handler that decides whether a request may activate.
    pub(crate) shortcuts_inhibit_state: KeyboardShortcutsInhibitState,
    /// Kept alive so `wp_presentation` stays registered.
    #[allow(dead_code)]
    pub(crate) presentation_state: PresentationState,
    /// Kept alive so `wp_content_type_manager_v1` stays registered.
    #[allow(dead_code)]
    pub(crate) content_type_state: ContentTypeState,
    /// Kept alive so `wp_single_pixel_buffer_manager_v1` stays registered.
    #[allow(dead_code)]
    pub(crate) single_pixel_buffer_state: SinglePixelBufferState,
    /// Kept alive so `wp_tearing_control_manager_v1` stays registered; the handler in
    /// [`crate::protocols::wp_tearing_control_v1`] records the hints.
    #[allow(dead_code)]
    pub(crate) tearing_control_global: GlobalId,
    /// Kept alive so `zxdg_exporter_v2` and `zxdg_importer_v2` stay registered; the protocol,
    /// including the handle table, is in [`crate::protocols::xdg_foreign`].
    #[allow(dead_code)]
    pub(crate) xdg_foreign_globals: (GlobalId, GlobalId),
    /// The handle table for `zxdg_exporter_v2` and `zxdg_importer_v2`; the protocol is in
    /// [`crate::protocols::xdg_foreign`].
    pub(crate) xdg_foreign: crate::protocols::xdg_foreign::XdgForeign,
    /// Kept alive so `xdg_wm_dialog_v1` stays registered; the handler is in
    /// [`crate::protocols::xdg_dialog_v1`].
    #[allow(dead_code)]
    pub(crate) xdg_dialog_state: XdgDialogState,
    /// Kept alive so `xdg_toplevel_icon_manager_v1` stays registered; the handler is in
    /// [`crate::protocols::xdg_toplevel_icon_v1`].
    #[allow(dead_code)]
    pub(crate) xdg_toplevel_icon_manager: XdgToplevelIconManager,
    /// Kept alive so `xdg_toplevel_tag_manager_v1` stays registered; the handler is in
    /// [`crate::protocols::xdg_toplevel_tag_v1`].
    #[allow(dead_code)]
    pub(crate) xdg_toplevel_tag_manager: XdgToplevelTagManager,
    /// Kept alive so `xdg_system_bell_v1` stays registered; the handler is in
    /// [`crate::protocols::xdg_system_bell_v1`].
    #[allow(dead_code)]
    pub(crate) xdg_system_bell_state: XdgSystemBellState,
    /// Kept alive so `wp_fifo_manager_v1` stays registered; its barriers are released by
    /// [`crate::protocols::wp_fifo_v1`] on each frame tick.
    #[allow(dead_code)]
    pub(crate) fifo_manager_state: smithay::wayland::fifo::FifoManagerState,
    /// Kept alive so `wp_alpha_modifier_v1` stays registered; the factor each surface commits
    /// is read in [`State::sync_content`] and applied by the composer.
    #[allow(dead_code)]
    pub(crate) alpha_modifier_state: smithay::wayland::alpha_modifier::AlphaModifierState,
    /// Kept alive so `ext_idle_notifier_v1` stays registered; the handler in
    /// [`crate::protocols::ext_idle_notify_v1`] answers its notifications.
    #[allow(dead_code)]
    pub(crate) idle_notify_global: GlobalId,
    /// Kept alive so `ext_background_effect_manager_v1` stays registered; the bind is
    /// gated by the access roster in [`crate::protocols::ext_background_effect_v1`].
    #[allow(dead_code)]
    pub(crate) background_effect_global: GlobalId,
    /// Per-output gamma and power control, applied through [`crate::output_control`].
    pub(crate) output_controls: OutputControlState,
    /// The `zwlr_output_management_v1` state, which answers client requests to read and
    /// change output configuration.
    pub(crate) output_manager: ZwlrOutputManagementV1State,
    /// Kept alive so `zwp_text_input_manager_v3` stays registered.
    #[allow(dead_code)]
    pub(crate) text_input_state: TextInputManagerState,
    /// Kept alive so `zwp_input_method_manager_v2` stays registered.
    #[allow(dead_code)]
    pub(crate) input_method_state: InputMethodManagerState,
    /// Kept alive so `zwp_virtual_keyboard_manager_v1` stays registered.
    #[allow(dead_code)]
    pub(crate) virtual_keyboard_state: VirtualKeyboardManagerState,

    /// The seat the keyboard and pointer belong to, which is what the text-input and
    /// shortcut-inhibit handles hang off.
    pub(crate) seat: Seat<State>,
    /// Keyboard and pointer handles for the one seat this slice creates.
    pub(crate) keyboard: KeyboardHandle<State>,
    /// The pointer handle of the seat, the target of pointer focus and motion.
    pub(crate) pointer: PointerHandle<State>,
    /// What each connection is allowed to be, shared with the globals' filters.
    pub(crate) access: Arc<Access>,
    /// The layer surfaces and what they reserve.
    pub(crate) layers: ZwlrLayerShellV1Layers,
    /// The published toplevel list.
    pub(crate) foreign_toplevels: ForeignToplevels,
    /// The activation tokens.
    pub(crate) activation: XdgActivationV1,
    /// The titlebar actions the configuration declares, for the redirects that map a
    /// gesture to one. It is a copy of three configured values rather than a handle on
    /// the configuration, because a protocol handler must not re-read a document on
    /// the session's own turn.
    pub(crate) titlebar_actions: TitlebarActions,
    /// How many times a client rang the bell. The count is published so a reader can
    /// see that it happened; the visual bell is `gnoblin-a11y`'s and the audible one
    /// is the shell's.
    bells: u64,
    /// The visual bell the document declares, read each time a bell rings. A session that
    /// declares none flashes nothing.
    pub(crate) visual_bell: crate::server::VisualBellSettings,
    /// The visual accessibility applied to every output's frame, set at start-up.
    pub(crate) visual_accessibility: crate::server::VisualAccessibilitySettings,
    /// The window menu requests made recently, oldest first. Bounded by
    /// `MENU_REQUESTS_KEPT`; the sequence number is what a reader publishes by.
    menu_requests: VecDeque<MenuRequest>,
    /// The last sequence number handed to a menu request.
    menu_seq: u64,
    /// The name the seat was created with, reported in each menu request.
    seat_name: String,
    /// The current clipboard and primary selections.
    pub(crate) selections: WlDataDeviceManagerSelections,
    /// Sandbox listeners waiting for the event loop to watch them.
    ///
    /// `wp_security_context_v1` hands the compositor a listening socket during a
    /// dispatch, and only the loop can watch one, so the source is queued here and
    /// taken by [`crate::Server`] on the same turn.
    pub(crate) pending_contexts: Vec<SecurityContextListenerSource>,
    /// The session's turn counter: the only clock the protocol side reads, so a token's
    /// lifetime is measured in the work the session did rather than in seconds.
    pub(crate) turn: Tick,

    /// The seat's keyboard features: sources, layout memory and delivery order.
    pub(crate) keyboard_tables: KeyboardTables,
    /// The pointer constraints a client holds.
    pub(crate) constraint_tables: ZwpPointerConstraintsV1Tables,
    /// The idle notifications and inhibitors.
    pub(crate) idle_tables: ExtIdleNotifyV1Tables,
    /// The presentation feedback bookkeeping.
    pub(crate) presentation_tables: WpPresentationTimeTables,
    /// The shortcut inhibitors the compositor granted, so it can revoke them.
    pub(crate) shortcuts_held: Vec<KeyboardShortcutsInhibitor>,
    /// The input method popups, so a renderer can draw one.
    pub(crate) input_method_popups: Vec<PopupSurface>,
    /// The text inputs the focused client has already been told about.
    ///
    /// `enter` is sent when the keyboard focus moves, and Smithay only sends it when a
    /// text input is *created* if an input method already exists. A text input that
    /// arrives afterwards therefore has to be found by the compositor, and this is what
    /// keeps that search from sending `enter` twice for one object.
    pub(crate) announced_text_inputs: HashSet<ObjectId>,
    /// The last cursor image a client set.
    pub(crate) cursor_image: CursorImageStatus,
    /// Set when a client changes the pointer image; the composer is told at the next frame.
    pub(crate) cursor_shape_changed: bool,
    /// The logical cursor size in pixels, from the configuration.
    pub(crate) cursor_size: u16,
    /// The cursor chain that resolves a named shape to the image drawn for it.
    pub(crate) cursor_chain: gnoblin_cursor::CursorChain,
    /// The tearing hints clients set, keyed by surface.
    pub(crate) tearing: Vec<(WlSurface, WpTearingControlV1Hint)>,
    /// The session clock, in milliseconds, which idle and the router read.
    pub(crate) clock: Tick,
    /// The input activity the idle state is derived from.
    pub(crate) activity: Activity,
    /// The liveness pings and the clients that have missed them.
    pub(crate) liveness: crate::liveness::Liveness,

    /// The window management policy.
    ///
    /// Crate-visible because every protocol module turns a request into an
    /// [`gnoblin_wm::Event`] and applies the effects it gets back; the fields around it
    /// stay private to this module.
    pub(crate) wm: Wm,
    /// The protocol objects of the managed windows, keyed by policy identity.
    pub(crate) windows: BTreeMap<WindowId, WindowEntry>,
    /// The outputs, in connection order.
    pub(crate) outputs: Vec<OutputEntry>,
    /// The next window identity to hand out.
    next_window: u32,
    /// The protection generation the frame being drawn was described under.
    ///
    /// Read when the scene is described and reported when the frame is presented, so
    /// a frame described before a cover was raised cannot prove it.
    pub(crate) protection_generation: Option<u64>,
    /// The last pointer position the backend reported, in global logical
    /// coordinates.
    pointer_position: SmithayPoint<f64, Logical>,
    /// How many pointer buttons are held. Focus does not follow the pointer while one is.
    held_buttons: u32,

    /// The scene the frame path composes from: window nodes, stacking and damage.
    pub(crate) composer: Composer,
    /// The modal overlay, if one is set; it takes input from the clients while it stands.
    pub(crate) modal: Option<Modal>,
    /// The modal events queued since the last take, in the order they arrived.
    pub(crate) modal_events: Vec<ModalEvent>,
    /// The reason each workspace's last layout run failed, until its next placement.
    pub(crate) layout_failures: BTreeMap<WorkspaceId, &'static str>,
    /// The layout control the compositor installed, which builds engines and holds the
    /// runtime layout overrides. `None` until a layout is installed.
    layouts: Option<Rc<dyn LayoutControl>>,
    /// The output whose frame the host described last, and its damage, so the frame
    /// hook can record what was drawn.
    last_frame: Option<(OutputId, Vec<Rect>)>,

    /// The session lock and protection-cover policy. Every request, output change
    /// and committed lock surface is an event for it, and the effects it returns are
    /// what the protocol side in [`crate::protocols::ext_session_lock_v1`] performs.
    pub(crate) lock: Lock,
    /// The embargo the lock last announced. It is what the capture and selection
    /// queries answer, so a consumer reads what the lock decided rather than a second
    /// opinion about it.
    pub(crate) lock_embargo: Embargo,
    /// The protocol bookkeeping the lock needs: held confirmations, lock surfaces, the
    /// cover generation and the presentation barrier.
    pub(crate) lock_tables: ExtSessionLockV1Tables,

    /// The capture sessions, the pixels a capture copies from and the capture globals.
    /// It is one value so the compositor's own fields stay a short list, and so the whole
    /// of the capture state reads in one place (`protocols::capture_shared`).
    pub(crate) capture: CaptureTables,

    /// The frame renderers' managers and the frames they were offered, which is
    /// `gnoblin_window_frame_v1` (`protocols::gnoblin_window_frame_v1`).
    pub(crate) frame_tables: GnoblinWindowFrameV1Tables,
    /// The hosted sessions, their protection covers and their requests, which is
    /// `gnoblin_session_host_v1` (`protocols::gnoblin_session_host_v1`).
    pub(crate) session_host: GnoblinSessionHostV1Tables,

    /// The snapshot tests read.
    observation: Arc<Mutex<SessionSnapshot>>,
    /// A reader outside the loop that is told when the snapshot changed. It is the
    /// compositor's own seam: the object graph it publishes is fed from the same
    /// snapshot every other reader sees, so no second description of the window
    /// state can drift from this one.
    observer: Option<Arc<dyn Observer>>,
}

/// How long one loop turn may wait for a layout run that is still in flight.
const LAYOUT_FRAME_BUDGET: Duration = Duration::from_millis(4);

/// How long a layout command waits for its engine's answer before the workspace floats.
const LAYOUT_COMMAND_WAIT: Duration = Duration::from_secs(2);

impl State {
    /// Builds the state, its globals and its outputs.
    ///
    /// # Errors
    ///
    /// [`StartupError::Keyboard`] when xkbcommon cannot build a keymap — a machine
    /// with no XKB data cannot run a seat with a keyboard.
    // Eight arguments, because a session is assembled from eight independent answers:
    // the display, its outputs, the seat, the policy options, the workspaces, Xwayland,
    // the observer and the protocol configuration. A struct would only move the list.
    /// Builds the state for one session and creates every protocol global it advertises,
    /// after which the session's seat, outputs and window manager are wired to it.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        display: Display<State>,
        backend_outputs: &[gnoblin_backend::Output],
        seat_name: &str,
        options: WmOptions,
        workspaces: u32,
        xwayland: Option<crate::xwayland::XwaylandSession>,
        observer: Option<Arc<dyn Observer>>,
        protocols: &Protocols,
        titlebar_actions: TitlebarActions,
        cursor: crate::server::CursorSettings,
        visual_bell: crate::server::VisualBellSettings,
        visual_accessibility: crate::server::VisualAccessibilitySettings,
        extensions: &[Arc<dyn Extension>],
    ) -> Result<Self, StartupError> {
        let display_handle = display.handle();
        let output_manager = ZwlrOutputManagementV1State::new(display_handle.clone());

        let compositor_state = CompositorState::new::<Self>(&display_handle);
        let xdg_shell_state = XdgShellState::new::<Self>(&display_handle);
        let shm_state = ShmState::new::<Self>(&display_handle, Vec::new());
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&display_handle);
        let viewporter_state = crate::protocols::wp_viewporter::global(&display_handle);
        let decoration_state = XdgDecorationState::new::<Self>(&display_handle);
        // The lock global is advertised to every client: the standard protocol makes
        // the socket the admission boundary, and `docs/session-lock.md` states that
        // the first request owns the lock.
        let session_lock_state = SessionLockManagerState::new::<Self, _>(&display_handle, |_| true);
        // The two Xwayland-only globals exist only with a session: `xwayland-shell-v1`
        // and the keyboard grab manager are bound by Xwayland alone. The data device is
        // not one of them: it is the clipboard, which every session serves, and
        // Smithay's X11 window manager reaches it through the same state.
        let (xwayland_shell_state, keyboard_grab_state) = match &xwayland {
            Some(_) => (
                Some(XWaylandShellState::new::<Self>(&display_handle)),
                Some(XWaylandKeyboardGrabState::new::<Self>(&display_handle)),
            ),
            None => (None, None),
        };

        // Every privileged global takes its per-client filter from this one roster, so
        // a protocol cannot forget the policy: the closure is the only way it is
        // created, and the roster is the only answer it gives.
        let access = Arc::new(Access::new());

        let layer_shell_state = WlrLayerShellState::new_with_filter::<Self, _>(&display_handle, {
            let access = Arc::clone(&access);
            move |client: &Client| AccessPolicy::allows(&*access, &client.id(), Global::LayerShell)
        });
        let foreign_toplevel_list_state = ForeignToplevelListState::new_with_filter::<Self>(&display_handle, {
            let access = Arc::clone(&access);
            move |client: &Client| AccessPolicy::allows(&*access, &client.id(), Global::ForeignToplevelList)
        });
        let activation_state = XdgActivationState::new::<Self>(&display_handle);
        let data_device_state = DataDeviceState::new::<Self>(&display_handle);
        let primary_selection_state = PrimarySelectionState::new::<Self>(&display_handle);
        let ext_data_control_state =
            ExtDataControlState::new::<Self, _>(&display_handle, Some(&primary_selection_state), {
                let access = Arc::clone(&access);
                move |client: &Client| AccessPolicy::allows(&*access, &client.id(), Global::DataControl)
            });
        let wlr_data_control_state =
            WlrDataControlState::new::<Self, _>(&display_handle, Some(&primary_selection_state), {
                let access = Arc::clone(&access);
                move |client: &Client| AccessPolicy::allows(&*access, &client.id(), Global::WlrDataControl)
            });
        // A sandbox may not create another sandbox: `wp_security_context_v1` says
        // nesting is how a sandboxed client escapes its own restrictions, so the
        // manager is hidden from the connections that already have one. Every other
        // connection may create one, and what the sandbox buys is a *denied* role for
        // the connection it creates.
        let security_context_state = crate::protocols::wp_security_context_v1::global(&display_handle, {
            let access = Arc::clone(&access);
            move |client: &Client| access.role(&client.id()) != ClientRole::Sandboxed
        });

        let foreign_toplevel_global =
            crate::protocols::zwlr_foreign_toplevel_management_v1::global(&display_handle, &access);
        let background_effect_global = crate::protocols::ext_background_effect_v1::global(&display_handle, &access);
        // Created when the configuration advertises them; whether the host can honour
        // them is checked at start-up, before any client connects.
        let gamma_global = Global::GammaControl
            .is_advertised(protocols)
            .then(|| crate::protocols::zwlr_gamma_control_v1::global(&display_handle, &access));
        let power_global = Global::OutputPower
            .is_advertised(protocols)
            .then(|| crate::protocols::zwlr_output_power_management_v1::global(&display_handle, &access));
        if Global::OutputManagement.is_advertised(protocols) {
            crate::protocols::zwlr_output_management_v1::global(&display_handle, &access);
        }

        // A global the configuration turned off is removed rather than skipped, so
        // every protocol state exists for its handler and no client sees it.
        let capture = CaptureTables::new(&display_handle, &access);
        let frame_tables = GnoblinWindowFrameV1Tables::new(&display_handle, &access);
        let session_host = GnoblinSessionHostV1Tables::new(&display_handle, &access);
        remove_disabled_globals(
            &display_handle,
            protocols,
            &[
                (Global::LayerShell, layer_shell_state.shell_global()),
                (Global::Screencopy, capture.screencopy_global()),
                (Global::ImageCopyCapture, capture.image_copy_global()),
                (Global::ForeignToplevelList, foreign_toplevel_list_state.global()),
                (Global::ForeignToplevelList, foreign_toplevel_list_state.global()),
                (Global::ForeignToplevelManagement, foreign_toplevel_global),
                (Global::DataControl, ext_data_control_state.global()),
                (Global::WlrDataControl, wlr_data_control_state.global()),
                (Global::BackgroundEffect, background_effect_global.clone()),
            ],
        );

        // The seat's protocols. Each global is created here and answered by its own
        // module; the argument this list carries is the access policy for the globals
        // that can act on the whole session.
        let fractional_scale_state = crate::protocols::wp_fractional_scale_v1::global(&display_handle);
        let cursor_shape_state = crate::protocols::wp_cursor_shape_v1::global(&display_handle);
        let relative_pointer_state = crate::protocols::zwp_relative_pointer_v1::global(&display_handle);
        let pointer_constraints_state = crate::protocols::zwp_pointer_constraints_v1::global(&display_handle);
        let pointer_gestures_state = crate::protocols::zwp_pointer_gestures_v1::global(&display_handle);
        let idle_inhibit_state = crate::protocols::zwp_idle_inhibit_v1::global(&display_handle);
        let shortcuts_inhibit_state = crate::protocols::zwp_keyboard_shortcuts_inhibit_v1::global(&display_handle);
        let presentation_state = crate::protocols::wp_presentation_time::global(&display_handle);
        let content_type_state = crate::protocols::wp_content_type_v1::global(&display_handle);
        let single_pixel_buffer_state = crate::protocols::wp_single_pixel_buffer_v1::global(&display_handle);
        let tearing_control_global = crate::protocols::wp_tearing_control_v1::global(&display_handle);
        let idle_notify_global = crate::protocols::ext_idle_notify_v1::global(&display_handle, &access);
        let xdg_foreign_globals = crate::protocols::xdg_foreign::global(&display_handle);
        let xdg_dialog_state = crate::protocols::xdg_dialog_v1::global(&display_handle);
        let xdg_toplevel_icon_manager = crate::protocols::xdg_toplevel_icon_v1::global(&display_handle);
        let xdg_toplevel_tag_manager = crate::protocols::xdg_toplevel_tag_v1::global(&display_handle);
        let xdg_system_bell_state = crate::protocols::xdg_system_bell_v1::global(&display_handle);
        let fifo_manager_state = crate::protocols::wp_fifo_v1::global(&display_handle);
        let alpha_modifier_state = crate::protocols::wp_alpha_modifier_v1::global(&display_handle);
        let text_input_state = crate::protocols::zwp_text_input_v3::text_input_global(&display_handle);
        let input_method_state = crate::protocols::zwp_input_method_v2::input_method_global(&display_handle, &access);
        let virtual_keyboard_state =
            crate::protocols::zwp_virtual_keyboard_v1::virtual_keyboard_global(&display_handle, &access);

        let mut seat_state = SeatState::new();
        let mut seat: Seat<Self> = seat_state.new_wl_seat(&display_handle, seat_name);
        let keyboard = seat
            .add_keyboard(Default::default(), KEY_REPEAT_DELAY, KEY_REPEAT_INTERVAL)
            .map_err(StartupError::Keyboard)?;
        let pointer = seat.add_pointer();

        let mut state = Self {
            display_handle,
            display: Some(display),
            compositor_state,
            xdg_shell_state,
            shm_state,
            seat_state,
            popups: PopupManager::default(),
            output_manager_state,
            viewporter_state,
            decoration_state,
            session_lock_state,
            xwayland_shell_state,
            keyboard_grab_state,
            x11: None,
            layer_shell_state,
            foreign_toplevel_list_state,
            activation_state,
            data_device_state,
            primary_selection_state,
            ext_data_control_state,
            wlr_data_control_state,
            fractional_scale_state,
            cursor_shape_state,
            relative_pointer_state,
            pointer_constraints_state,
            pointer_gestures_state,
            idle_inhibit_state,
            shortcuts_inhibit_state,
            presentation_state,
            content_type_state,
            single_pixel_buffer_state,
            tearing_control_global,
            idle_notify_global,
            background_effect_global,
            xdg_foreign_globals,
            xdg_foreign: Default::default(),
            xdg_dialog_state,
            xdg_toplevel_icon_manager,
            xdg_toplevel_tag_manager,
            xdg_system_bell_state,
            fifo_manager_state,
            alpha_modifier_state,
            output_controls: OutputControlState::new(gamma_global, power_global),
            output_manager,
            text_input_state,
            input_method_state,
            virtual_keyboard_state,
            security_context_state,
            keyboard,
            pointer,
            seat,
            access,
            layers: ZwlrLayerShellV1Layers::default(),
            foreign_toplevels: ForeignToplevels::default(),
            activation: XdgActivationV1::default(),
            titlebar_actions,
            bells: 0,
            visual_bell,
            visual_accessibility,
            menu_requests: VecDeque::new(),
            menu_seq: 0,
            seat_name: seat_name.to_owned(),
            selections: WlDataDeviceManagerSelections::default(),
            pending_contexts: Vec::new(),
            turn: Tick::ZERO,
            keyboard_tables: KeyboardTables::default(),
            constraint_tables: ZwpPointerConstraintsV1Tables::default(),
            idle_tables: ExtIdleNotifyV1Tables::default(),
            presentation_tables: WpPresentationTimeTables::default(),
            shortcuts_held: Vec::new(),
            input_method_popups: Vec::new(),
            announced_text_inputs: HashSet::new(),
            cursor_image: CursorImageStatus::default_named(),
            cursor_shape_changed: false,
            cursor_chain: gnoblin_cursor::CursorChain::new(
                std::sync::Arc::new(gnoblin_cursor::FsFiles::from_environment()),
                &cursor.theme,
                cursor
                    .providers
                    .iter()
                    .copied()
                    .map(gnoblin_cursor::ProviderKind::provider)
                    .collect(),
                crate::protocols::wp_cursor_shape_v1::CURSOR_CACHE_BYTES,
            ),
            cursor_size: cursor.size,
            tearing: Vec::new(),
            clock: Tick::ZERO,
            activity: Activity::new(),
            liveness: crate::liveness::Liveness::new(std::time::Duration::from_secs(5)),
            wm: Wm::new(options),
            windows: BTreeMap::new(),
            outputs: Vec::new(),
            next_window: 1,
            protection_generation: None,
            pointer_position: SmithayPoint::from((0.0, 0.0)),
            held_buttons: 0,
            composer: Composer::new(),
            modal: None,
            modal_events: Vec::new(),
            layout_failures: BTreeMap::new(),
            layouts: None,
            last_frame: None,
            lock: Lock::new(),
            lock_embargo: Embargo::RELEASED,
            lock_tables: ExtSessionLockV1Tables::default(),
            capture,
            frame_tables,
            session_host,
            observation: Arc::new(Mutex::new(SessionSnapshot::default())),
            observer,
        };

        for raw in 0..workspaces.max(DEFAULT_WORKSPACES) {
            state.wm.apply(Event::WorkspaceAdded {
                workspace: WorkspaceId::new(u64::from(raw) + 1),
            });
        }
        // Every extension registers before the loop runs, so no client can observe a
        // session with half its globals.
        crate::protocols::extension::Extensions::default()
            .register(extensions, &state.display_handle)
            .map_err(StartupError::Extension)?;
        state.sync_outputs(backend_outputs);
        if let Some(session) = xwayland {
            state.start_xwayland(session);
        }
        state.publish();
        Ok(state)
    }

    /// The snapshot of the window state the tests and a diagnostics reader copy.
    #[must_use]
    pub fn observation(&self) -> Arc<Mutex<SessionSnapshot>> {
        Arc::clone(&self.observation)
    }

    /// The window management policy, for a caller that wants to read it.
    #[must_use]
    pub fn wm(&self) -> &Wm {
        &self.wm
    }

    /// The access roster, so a caller can build a connection's data from it.
    #[must_use]
    pub(crate) fn access(&self) -> Arc<Access> {
        Arc::clone(&self.access)
    }

    /// The toplevel object of one managed window, when it is a Wayland one.
    pub(crate) fn toplevel_of(&self, window: WindowId) -> Option<ToplevelSurface> {
        self.windows.get(&window)?.surface.toplevel().cloned()
    }

    /// The surface behind one managed window, for a check that must not care whether
    /// it is a Wayland or an X11 window.
    pub(crate) fn surface_of(&self, window: WindowId) -> Option<WlSurface> {
        self.windows.get(&window)?.surface.wl_surface()
    }

    /// The outputs, with their Smithay objects, for a protocol that must address a
    /// client's `wl_output`.
    pub(crate) fn output_entries(&self) -> impl Iterator<Item = &OutputEntry> {
        self.outputs.iter()
    }

    /// The outputs the server presents on, as the backend numbered them.
    #[must_use]
    pub fn output_ids(&self) -> Vec<OutputId> {
        self.outputs.iter().map(|entry| entry.backend).collect()
    }

    /// Inserts a connected client with the data its handlers expect.
    ///
    /// The connection's role is recorded here rather than inferred from a request,
    /// because every request arrives *after* the registry answer: a connection that is
    /// not recorded before its first bind could only be refused, never hidden.
    pub(crate) fn insert_client(
        &mut self,
        stream: std::os::unix::net::UnixStream,
        data: Arc<crate::protocols::wl_compositor::WlCompositorClientState>,
        role: ClientRole,
    ) -> std::io::Result<Client> {
        let client = self.display_handle.insert_client(
            stream,
            Arc::clone(&data) as Arc<dyn smithay::reexports::wayland_server::backend::ClientData>,
        )?;
        // The pid and the process's start time are read now, while the peer still holds
        // the pid. The start time tells the process apart from a later one that reuses
        // the number, so a kill can refuse a process that has changed.
        let pid = client
            .get_credentials(&self.display_handle)
            .ok()
            .and_then(|credentials| u32::try_from(credentials.pid).ok());
        if let Some(identity) = pid.and_then(crate::ProcessIdentity::of) {
            let _ = data.process.set(identity);
        }
        // A role the compositor promised to the process takes the place of the socket's
        // default. Only an unprivileged connection can be promoted: a sandboxed one
        // stays sandboxed whatever a promise says, and the privileged socket already
        // sees more than either single-purpose role.
        let role = match role {
            ClientRole::Unprivileged => pid.and_then(|pid| self.access.promised(pid)).unwrap_or(role),
            _ => role,
        };
        self.access.record(&client.id(), role);
        // From here the roster entry lives exactly as long as the connection: the
        // client's data owns the registration that removes it.
        data.registration.attach(&client.id());
        Ok(client)
    }

    /// Promises a role to a process, so its next connection is recorded with it.
    ///
    /// The composition root records the promise and then starts the renderer or the
    /// hosted session; the accept path in [`State::insert_client`] picks it up.
    pub fn grant_role(&self, pid: u32, role: ClientRole) {
        self.access.promise(pid, role);
    }

    /// Accepts a connection that arrived through a `wp_security_context_v1` sandbox.
    ///
    /// The connection is a client of its own — it has its own registry, its own
    /// objects and its own resources — and it is marked sandboxed before its first
    /// request is dispatched, so it never sees a privileged global.
    pub(crate) fn accept_sandbox(&mut self, stream: std::os::unix::net::UnixStream) -> std::io::Result<Client> {
        let client = self.insert_client(
            stream,
            Arc::new(crate::protocols::wl_compositor::WlCompositorClientState::new(
                &self.access,
            )),
            ClientRole::Sandboxed,
        )?;
        log::debug!(area = "access", client = ?client.id(), "a sandboxed connection joined");
        Ok(client)
    }

    /// The listeners `wp_security_context_v1` created, for the event loop to watch.
    pub(crate) fn take_pending_contexts(&mut self) -> Vec<SecurityContextListenerSource> {
        std::mem::take(&mut self.pending_contexts)
    }

    /// The policy identity a backend output's rectangle is known by.
    pub(crate) fn policy_output(&self, output: OutputId) -> Option<OutputId> {
        self.outputs
            .iter()
            .find(|entry| entry.backend == output)
            .map(|entry| entry.policy)
    }

    /// The scale the output a surface is on draws it at.
    ///
    /// A surface with no output of its own takes the first output's scale, and a session
    /// with no output at all answers 1: a client that is told nothing renders at the
    /// smallest scale rather than at a scale nothing draws.
    #[must_use]
    pub(crate) fn surface_scale(&self, surface: &WlSurface) -> f64 {
        let output_id = self
            .window_of(surface)
            .and_then(|window| self.wm.window(window))
            .and_then(|window| window.output);
        let entry = output_id
            .and_then(|policy| self.outputs.iter().find(|entry| entry.policy == policy))
            .or_else(|| self.outputs.first());
        entry.map_or(1.0, |entry| entry.output.current_scale().fractional_scale())
    }

    /// Whether the pointer is currently over a surface.
    #[must_use]
    pub(crate) fn pointer_is_over(&self, surface: &WlSurface) -> bool {
        self.pointer.current_focus().as_ref() == Some(surface)
    }

    /// Sends a relative motion vector to the surface the pointer is on.
    ///
    /// It is what a locked pointer reports and what an unlocked one adds to its position
    /// report, so a client that reads vectors alone never has to difference positions.
    pub(crate) fn send_relative_motion(&mut self, delta: SmithayPoint<f64, Logical>, time: InputTime) {
        let Some(surface) = self.pointer.current_focus() else {
            return;
        };
        let Some((_, origin)) = self.surface_under(self.pointer_position) else {
            return;
        };
        let event = RelativeMotionEvent {
            delta,
            delta_unaccel: delta,
            time,
        };
        let pointer = self.pointer.clone();
        pointer.relative_motion(self, Some((surface, origin)), &event);
    }

    /// An output's Smithay object and the rectangle it occupies, for the frame path.
    pub(crate) fn frame_target(&self, output: OutputId) -> Option<(Output, Rect)> {
        self.outputs
            .iter()
            .find(|entry| entry.backend == output)
            .map(|entry| (entry.output.clone(), entry.rect))
    }

    /// The surfaces a frame for `rect` drew, topmost last.
    ///
    /// It is what a presentation feedback may be answered for: a window that is not visible,
    /// has no buffer or does not intersect the output was not drawn.
    #[must_use]
    pub(crate) fn drawn_surfaces(&self, rect: Rect) -> Vec<WlSurface> {
        let active = self.wm.active_workspace();
        let mut surfaces = Vec::new();
        for window in self.wm.stack() {
            let (Some(entry), Some(managed)) = (self.windows.get(window), self.wm.window(*window)) else {
                continue;
            };
            if !entry.mapped || !managed.is_visible(active) {
                continue;
            }
            let geometry = self.shown_geometry(managed);
            let intersects = i64::from(geometry.x) < rect.right()
                && i64::from(rect.x) < geometry.right()
                && i64::from(geometry.y) < rect.bottom()
                && i64::from(rect.y) < geometry.bottom();
            if intersects && let Some(surface) = entry.surface.wl_surface() {
                surfaces.push(surface);
            }
        }
        surfaces
    }

    /// Applies what a raised cover changes outside the lock's own bookkeeping.
    ///
    /// A covered session must hold no client's pointer and no client's shortcuts: a pointer
    /// lock that survived the cover would keep capturing the pointer for a window underneath
    /// it, and a shortcut inhibitor would keep the keys the lock screen needs. Both are
    /// released here, once, before the cover is presented.
    pub(crate) fn cover_raised(&mut self) {
        self.release_constraints();
        self.revoke_shortcut_inhibitors();
        self.clear_pointer_focus();
    }

    /// The display, taken out for the caller that drives a dispatch.
    pub(crate) fn take_display(&mut self) -> Option<Display<State>> {
        self.display.take()
    }

    /// Puts the display back after a dispatch.
    pub(crate) fn restore_display(&mut self, display: Display<State>) {
        self.display = Some(display);
    }

    /// Reconciles the outputs with what the backend reports.
    ///
    /// The whole layout is recomputed from the connected set, so a hotplug, a
    /// removal and a mode change all end in the same place: one declarative layout
    /// from `gnoblin-output`, applied to the Smithay outputs and to the window
    /// manager. A layout the engine refuses leaves the previous one in place and is
    /// logged, because a display configuration mistake must not end the session.
    pub(crate) fn sync_outputs(&mut self, outputs: &[gnoblin_backend::Output]) {
        let connected: Vec<&gnoblin_backend::Output> = outputs
            .iter()
            .filter(|output| output.mode.width > 0 && output.mode.height > 0)
            .collect();
        if connected.is_empty() {
            self.remove_outputs_unless(&[]);
            self.report_outputs();
            return;
        }
        let descriptors: Vec<_> = connected.iter().map(|output| output.descriptor()).collect();
        let connectors: Vec<String> = connected.iter().map(|output| output.connector.clone()).collect();
        let layout = match compute(&descriptors, &LayoutConfig::row_over(&connectors)) {
            Ok(layout) => layout,
            Err(error) => {
                log::warn!(
                    area = "output",
                    ?error,
                    "the layout was refused; keeping the previous one"
                );
                return;
            }
        };

        self.remove_outputs_unless(&connected.iter().map(|output| output.id).collect::<Vec<_>>());
        for output in &connected {
            let Some(monitor) = layout.monitor_of(output.id) else {
                log::warn!(area = "output", connector = %output.connector, "the layout does not place this output");
                continue;
            };
            match self.outputs.iter().position(|entry| entry.backend == output.id) {
                Some(index) => self.move_output(index, monitor.rect),
                None => self.add_output(output, monitor.rect),
            }
        }
        // The lock hears the whole layout once, after every change landed: it restarts
        // its presentation barrier from this set, so a hotplugged output is covered
        // before it shows anything.
        self.report_outputs();
    }

    /// Removes every output the connected set no longer names.
    fn remove_outputs_unless(&mut self, keep: &[OutputId]) {
        let gone: Vec<usize> = self
            .outputs
            .iter()
            .enumerate()
            .filter(|(_, entry)| !keep.contains(&entry.backend))
            .map(|(index, _)| index)
            .collect();
        for index in gone.into_iter().rev() {
            let entry = self.outputs.remove(index);
            self.display_handle.remove_global::<Self>(entry.global);
            self.composer.remove_output(entry.backend);
            self.output_controls_removed(entry.backend);
            log::debug!(area = "output", connector = %entry.output.name(), "disconnected");
            let effects = self.wm.apply(Event::OutputDisconnected { output: entry.policy });
            self.apply(effects);
        }
    }

    /// The Smithay scale for a backend scale: a whole value as an integer, any other
    /// value as a fractional scale.
    fn smithay_scale(scale: gnoblin_types::Scale) -> OutputScale {
        if scale.denominator() == 1
            && let Ok(value) = i32::try_from(scale.numerator())
        {
            return OutputScale::Integer(value);
        }
        OutputScale::Fractional(scale.as_f64())
    }

    /// Creates one output's Smithay object, global and window-manager entry.
    fn add_output(&mut self, output: &gnoblin_backend::Output, rect: Rect) {
        let smithay_output = Output::new(
            output.connector.clone(),
            PhysicalProperties {
                size: SmithaySize::from((0, 0)),
                subpixel: Subpixel::Unknown,
                make: "Gnoblin".to_owned(),
                model: output.connector.clone(),
                serial_number: String::new(),
            },
        );
        let mode = smithay_mode(output);
        smithay_output.set_preferred(mode);
        smithay_output.change_current_state(
            Some(mode),
            Some(Transform::Normal),
            Some(Self::smithay_scale(output.scale)),
            Some(SmithayPoint::from((rect.x, rect.y))),
        );
        let global = smithay_output.create_global::<Self>(&self.display_handle);

        let policy = OutputId::new(self.outputs.len() as u64);
        let policy_rect = rect;
        let effects = self.wm.apply(Event::OutputConnected {
            output: policy,
            rect: policy_rect,
        });
        self.apply(effects);
        let identified = self.wm.apply(Event::OutputIdentified {
            output: policy,
            name: output.connector.clone(),
            scale: Scale::new(1, 1),
        });
        self.apply(identified);
        self.composer.set_output(output.id, rect);
        log::debug!(area = "output", connector = %output.connector, ?rect, "connected");
        self.outputs.push(OutputEntry {
            backend: output.id,
            policy,
            output: smithay_output,
            global,
            rect: policy_rect,
        });
    }

    /// Applies a new layout rectangle to an output that is already connected.
    fn move_output(&mut self, index: usize, rect: Rect) {
        let Some(entry) = self.outputs.get_mut(index) else {
            return;
        };
        let policy = entry.policy;
        let policy_rect = rect;
        if entry.rect == policy_rect {
            return;
        }
        entry.rect = policy_rect;
        entry
            .output
            .change_current_state(None, None, None, Some(SmithayPoint::from((rect.x, rect.y))));
        self.composer.set_output(entry.backend, policy_rect);
        let effects = self.wm.apply(Event::OutputMoved {
            output: policy,
            rect: policy_rect,
        });
        self.apply(effects);
    }

    /// Applies the effects the window manager returned.
    ///
    /// This is the only place a policy answer becomes observable behaviour, so
    /// every arm says what a window manager command means to a client.
    pub(crate) fn apply(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Configure {
                    window,
                    geometry,
                    state,
                } => {
                    self.composer.place_window(window, geometry);
                    self.configure(window, geometry, state, TiledEdges::NONE, None);
                }
                Effect::SetVisible { window, visible } => {
                    self.composer.set_window_visible(window, visible);
                    self.set_visible(window, visible);
                }
                Effect::Stack { order } => {
                    // The policy states the order topmost first; the scene keeps its
                    // paint order bottom first.
                    self.composer.set_stacking(&order);
                    log::debug!(log = ?order, "[stack]");
                }
                Effect::Focus { focused, .. } => self.move_focus(focused),
                Effect::ActivationDenied { window } => {
                    log::debug!(area = "focus", %window, "activation refused by the caller");
                }
                Effect::WorkspaceShown { workspace } => log::debug!(area = "workspace", %workspace, "shown"),
                Effect::RuleRejected { window, rule, reason } => {
                    log::warn!(area = "rules", %window, rule, ?reason, "rule did not apply")
                }
                Effect::Rejected { reason } => log::debug!(area = "wm", ?reason, "the event changed nothing"),
                // A settled layout places its window: its transform reaches the scene whole.
                Effect::PlacementTarget {
                    workspace,
                    placement,
                    tiled,
                    work_area,
                } => self.place_tiled(workspace, placement, tiled, work_area),
                Effect::LayoutFailed { workspace, reason } => self.layout_failed(workspace, reason),
            }
        }
    }

    /// Applies a configure to a toplevel and sends it if the client is ready for it.
    ///
    /// The state is written to the surface's pending state either way; a toplevel
    /// that has not had its initial configure yet carries this state into that
    /// configure, which the client's first commit triggers.
    ///
    /// `tiled` and `work_area` come from the layout that placed the window. A window that no
    /// layout places passes [`TiledEdges::NONE`] and no work area, so it is sent no bounds.
    fn configure(
        &mut self,
        window: WindowId,
        geometry: Rect,
        state: gnoblin_wm::WindowState,
        tiled: TiledEdges,
        work_area: Option<Rect>,
    ) {
        let Some(surface) = self.windows.get(&window).map(|entry| &entry.surface) else {
            // The window closed between the policy call and this action, which is
            // the race the policy's rejections describe.
            return;
        };
        match surface {
            Managed::Wayland(surface) => {
                surface.with_pending_state(|pending| {
                    // The work area is the size the window should fit; a window with no
                    // output has no work area, so the client picks its own size.
                    pending.bounds = work_area.map(|area| SmithaySize::from((area.width as i32, area.height as i32)));
                    pending.size = Some(SmithaySize::from((geometry.width as i32, geometry.height as i32)));
                    set_flag(&mut pending.states, xdg_toplevel::State::Maximized, state.maximised);
                    set_flag(&mut pending.states, xdg_toplevel::State::Fullscreen, state.fullscreen);
                    // Each tiled edge says the window sits against a neighbour or the work
                    // area's edge there, so the client drops the border and rounding it draws.
                    set_flag(&mut pending.states, xdg_toplevel::State::TiledLeft, tiled.left);
                    set_flag(&mut pending.states, xdg_toplevel::State::TiledRight, tiled.right);
                    set_flag(&mut pending.states, xdg_toplevel::State::TiledTop, tiled.top);
                    set_flag(&mut pending.states, xdg_toplevel::State::TiledBottom, tiled.bottom);
                });
                if surface.is_initial_configure_sent() {
                    surface.send_configure();
                }
            }
            Managed::X11(surface) => {
                // An X11 configure is a request on the X connection, so the client hears
                // a ConfigureNotify and answers with its own geometry. Smithay keeps the
                // surface's `last_configure` in step, which is what popup positioning
                // reads.
                if let Err(error) = surface.configure(Some(x11_geometry(geometry))) {
                    log::debug!(area = "wm", %error, %window, "the X11 configure failed");
                }
            }
        }
        log::debug!(area = "wm", %window, width = geometry.width, height = geometry.height, "configured");
    }

    /// Shows or hides a window's surface, and tells a Wayland toplevel whether it is suspended.
    ///
    /// A hidden window is minimised or sits on another workspace, so the client is told it is
    /// `suspended` and may stop drawing it. That is the same signal that drives output enter and
    /// leave, so the two never disagree.
    fn set_visible(&mut self, window: WindowId, visible: bool) {
        let Some(entry) = self.windows.get(&window) else {
            return;
        };
        if let Some(surface) = entry.surface.wl_surface() {
            for output in &self.outputs {
                if visible {
                    output.output.enter(&surface);
                } else {
                    output.output.leave(&surface);
                }
            }
        }
        if let Some(toplevel) = entry.surface.toplevel() {
            set_suspended(toplevel, !visible);
        }
    }

    /// Moves keyboard focus to a window, or to nothing.
    ///
    /// While the session is covered nothing moves: the lock's gate routes every key to
    /// the owner's lock surface or to nothing, and the window manager's answer is
    /// applied again when the reveal completes.
    fn move_focus(&mut self, focused: Option<WindowId>) {
        if self.lock.state() != LockState::Unlocked {
            log::debug!(area = "focus", ?focused, "withheld while the session is covered");
            return;
        }
        // A keyboard-exclusive layer surface above the shell layer keeps the keyboard:
        // the protocol makes that promise for the top and overlay layers, so a bar's
        // own focus is not taken by whatever window maps next.
        if let Some(grab) = self.layer_keyboard_focus() {
            self.set_keyboard_focus(Some(grab));
            return;
        }
        let target = focused
            .and_then(|window| self.windows.get(&window))
            .and_then(|entry| entry.surface.wl_surface());
        // X11 windows also carry their activation state in `_NET_WM_STATE_FOCUSED`, which
        // is what draws a title bar as focused; a Wayland window has no such property and
        // the loop finds nothing to do.
        for (window, entry) in &self.windows {
            if let Managed::X11(surface) = &entry.surface
                && let Err(error) = surface.set_activated(Some(*window) == focused)
            {
                log::debug!(area = "focus", %error, %window, "the X11 activation state was not sent");
            }
        }
        self.set_keyboard_focus(target);
    }

    /// Installs the keyboard focus target. This is the only place focus is set.
    ///
    /// The clipboard follows the keyboard: `wl_data_device` delivers a selection to the
    /// focused client and to nobody else, so a client that cannot be pasted into has no
    /// offer to paste from.
    ///
    /// Focus is four things at once, and they have to move together: the layout a window
    /// remembers is stored against the window that is leaving, the text input has to hear
    /// `leave` before `enter`, the seat's `wl_keyboard` moves, and the window arriving gets
    /// the layout it remembers.
    pub(crate) fn set_keyboard_focus(&mut self, target: Option<WlSurface>) {
        self.remember_layout();
        {
            use smithay::wayland::text_input::TextInputSeat;
            let text_input = self.seat.text_input().clone();
            text_input.leave();
            text_input.set_focus(target.clone());
            // The connection leaving was told; the one arriving has not been, so the
            // announcement set starts again and the sweep below tells every text input the
            // new focus owns exactly once.
            self.announced_text_inputs.clear();
            if target.is_some() {
                crate::protocols::zwp_text_input_v3::announce_new_text_inputs(self);
            }
        }
        let serial = SERIAL_COUNTER.next_serial();
        let client = target.as_ref().and_then(|surface| surface.client());
        let keyboard = self.keyboard.clone();
        keyboard.set_focus(self, target.clone(), serial);
        self.set_selection_focus(client.as_ref());
        self.restore_layout(target.as_ref());
    }

    /// Moves the pointer to a layout position and gives it the focus it lands on.
    ///
    /// The serial is remembered because a client names it in a grab or an activation
    /// request, which is the same rule a button follows.
    fn move_pointer(&mut self, position: SmithayPoint<f64, Logical>, time: InputTime) {
        self.pointer_position = position;
        // A frame renderer hears the hover before the pointer focus changes: the region
        // under the pointer is the renderer's, the surface is the window's, and the two
        // are decided independently.
        self.frame_pointer(Some(position), None);
        let focus = self.surface_under(position);
        let pointer = self.pointer.clone();
        let serial = SERIAL_COUNTER.next_serial();
        self.remember_input_serial(u32::from(serial));
        pointer.motion(
            self,
            focus,
            &MotionEvent {
                location: position,
                serial,
                time,
            },
        );
    }

    /// The layout position nearest `wanted` that is inside an output.
    ///
    /// A relative move has no output to be relative to, so it is the seat that decides
    /// where it lands: the output that holds the position keeps it, and a position
    /// outside every output is clamped into the nearest one. The first output in policy
    /// order wins a tie, which keeps the answer deterministic.
    fn within_layout(&self, wanted: SmithayPoint<f64, Logical>) -> SmithayPoint<f64, Logical> {
        if self.outputs.iter().any(|entry| contains(&entry.rect, wanted)) {
            return wanted;
        }
        let mut best: Option<(f64, SmithayPoint<f64, Logical>)> = None;
        for entry in &self.outputs {
            let rect = entry.rect;
            let x = wanted.x.clamp(
                f64::from(rect.x),
                f64::from(rect.x + i32::try_from(rect.width).unwrap_or(i32::MAX) - 1),
            );
            let y = wanted.y.clamp(
                f64::from(rect.y),
                f64::from(rect.y + i32::try_from(rect.height).unwrap_or(i32::MAX) - 1),
            );
            let distance = (x - wanted.x).powi(2) + (y - wanted.y).powi(2);
            if best.is_none_or(|(best_distance, _)| distance < best_distance) {
                best = Some((distance, SmithayPoint::from((x, y))));
            }
        }
        best.map_or(self.pointer_position, |(_, position)| position)
    }

    /// Drops the pointer's focus, so a covered client cannot receive a button.
    pub(crate) fn clear_pointer_focus(&mut self) {
        let serial = SERIAL_COUNTER.next_serial();
        let position = self.pointer_position;
        let time = InputTime::now();
        let pointer = self.pointer.clone();
        pointer.motion(
            self,
            None,
            &MotionEvent {
                location: position,
                serial,
                time,
            },
        );
    }

    /// The surface of the window the window manager has focused.
    pub(crate) fn focused_window_surface(&self) -> Option<WlSurface> {
        let window = self.wm.focused()?;
        if self.window_is_covered(window) {
            // A hosted session under a protection cover keeps no input: its windows
            // are not on screen, so focusing one would type into a screen nobody can
            // see (design/rust-compositor.md, "Hosting sessions").
            return None;
        }
        self.windows.get(&window).and_then(|entry| entry.surface.wl_surface())
    }

    /// The identity of a `wl_output` global this session created.
    pub(crate) fn output_id_of(&self, output: &Output) -> Option<OutputId> {
        self.outputs
            .iter()
            .find(|entry| entry.output == *output)
            .map(|entry| entry.backend)
    }

    /// The logical rectangle an output occupies.
    pub(crate) fn output_rect(&self, output: OutputId) -> Option<Rect> {
        self.outputs
            .iter()
            .find(|entry| entry.backend == output)
            .map(|entry| entry.rect)
    }

    /// The scale an output composes at, as the backend reported it.
    ///
    /// A capture buffer is in physical pixels and every rectangle this crate keeps is
    /// logical, so the capture path needs the factor that converts one to the other.
    pub(crate) fn output_scale(&self, output: OutputId) -> Option<f64> {
        self.outputs
            .iter()
            .find(|entry| entry.backend == output)
            .map(|entry| entry.output.current_scale().fractional_scale())
    }

    /// The output whose rectangle contains a global position.
    pub(crate) fn output_under(&self, position: SmithayPoint<f64, Logical>) -> Option<OutputId> {
        self.outputs
            .iter()
            .find(|entry| {
                let rect = entry.rect;
                position.x >= f64::from(rect.x)
                    && position.y >= f64::from(rect.y)
                    && position.x < rect.right() as f64
                    && position.y < rect.bottom() as f64
            })
            .map(|entry| entry.backend)
    }

    /// A client created a toplevel.
    pub(crate) fn toplevel_created(&mut self, surface: ToplevelSurface) {
        if self
            .windows
            .values()
            .any(|entry| entry.surface.wl_surface().as_ref() == Some(surface.wl_surface()))
        {
            return;
        }
        let window = WindowId::new(u64::from(self.next_window));
        self.next_window = self.next_window.saturating_add(1);
        let wl_surface = surface.wl_surface().clone();
        self.windows.insert(
            window,
            WindowEntry {
                surface: Managed::Wayland(surface),
                mapped: false,
                app_id: String::new(),
                title: String::new(),
                icon_name: None,
                icon_buffer_count: 0,
                tag: None,
                effects: Vec::new(),
            },
        );
        // The window is placed now and takes focus when it first maps; the
        // activation event is sent from `toplevel_committed` for that reason.
        let effects = self.wm.apply(Event::WindowOpened(WindowSpec::normal(window, "")));
        // The scene learns the window before the effects are applied, so the initial
        // configure's geometry has a window to place.
        if let Some(managed) = self.wm.window(window) {
            self.composer.insert_window(window, managed.geometry);
        }
        self.apply(effects);
        log::debug!(area = "window", %window, surface = ?wl_surface.id(), "opened");
        self.publish();
    }

    /// A toplevel is gone, whether the client destroyed it or disconnected.
    pub(crate) fn toplevel_destroyed(&mut self, surface: &ToplevelSurface) {
        let Some(window) = self.window_of(surface.wl_surface()) else {
            return;
        };
        if let Some(entry) = self.windows.remove(&window) {
            if let Some(wl_surface) = entry.surface.wl_surface() {
                for output in &self.outputs {
                    output.output.leave(&wl_surface);
                }
                // A constraint the window's client held cannot outlive the surface it
                // was created for.
                self.constraint_tables.held.retain(|held| held.surface != wl_surface);
                log::debug!(area = "window", %window, surface = ?wl_surface.id(), "closed");
            }
            self.composer.remove_window(window);
            // The layout a window remembered goes with it: a recycled identity must not
            // inherit another window's source.
            self.forget_layout(window);
        }
        // The sessions that captured the window go with it: nothing may serve the pixels
        // of a window that is no longer on screen, and each client hears `stopped`.
        self.capture.window_closed(window);
        // A renderer hears `closed` once and its frame is forgotten, so the window is
        // never left with a decoration nothing will update.
        self.close_window_frame(window);
        let effects = self.wm.apply(Event::WindowClosed { window });
        self.apply(effects);
        self.publish();
    }

    /// A toplevel committed: record its title, application id and buffer.
    ///
    /// An X11 window's `wl_surface` also reaches this commit hook, because the compositor
    /// hook runs for every surface; it has no `xdg_toplevel` attributes and its content
    /// arrives through the X11 surface, so it is not a toplevel commit.
    pub(crate) fn toplevel_committed(&mut self, surface: &WlSurface) {
        let Some(window) = self.window_of(surface) else {
            return;
        };
        if self
            .windows
            .get(&window)
            .is_some_and(|entry| entry.surface.toplevel().is_none())
        {
            return;
        }
        let (app_id, title) = with_states(surface, |states| {
            let attributes = states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .expect("a toplevel always has its attributes")
                .lock()
                .expect("the attributes lock is never poisoned");
            (
                attributes.app_id.clone().unwrap_or_default(),
                attributes.title.clone().unwrap_or_default(),
            )
        });
        let mut effects = Vec::new();
        let mut newly_mapped = false;
        let mut initial_configure = None;
        if let Some(entry) = self.windows.get_mut(&window) {
            if entry.app_id != app_id {
                entry.app_id.clone_from(&app_id);
                effects.push(Event::WindowAppIdChanged { window, app_id });
            }
            if entry.title != title {
                entry.title.clone_from(&title);
                effects.push(Event::WindowTitleChanged { window, title });
            }
            let mapped = has_buffer(surface);
            newly_mapped = mapped && !entry.mapped;
            entry.mapped = mapped;
            if let Some(toplevel) = entry.surface.toplevel()
                && !toplevel.is_initial_configure_sent()
            {
                initial_configure = Some(toplevel.clone());
            }
        }
        // The icon and tag are read from the surface on each commit: a client sets them
        // as cached surface state, so the commit is where a change becomes visible.
        let (icon_name, icon_buffer_count, tag) = with_states(surface, |states| {
            let mut icon = states.cached_state.get::<ToplevelIconCachedState>();
            let icon = icon.current();
            let tag = states
                .data_map
                .get::<XdgToplevelTagSurfaceData>()
                .and_then(|data| data.tag())
                .map(|tag| tag.to_string());
            (icon.icon_name().map(str::to_owned), icon.buffers().len(), tag)
        });
        if let Some(entry) = self.windows.get_mut(&window) {
            entry.icon_name = icon_name;
            entry.icon_buffer_count = icon_buffer_count;
            entry.tag = tag;
        }
        self.sync_window_role(window);
        if let Some(surface) = initial_configure {
            // The client's first commit is answered with the initial configure,
            // carrying whatever the window manager wrote to the pending state.
            surface.send_configure();
        }
        if newly_mapped {
            // A new window takes focus only as `window_management.focus_new_windows`
            // allows; the window manager decides, so this path does not guess.
            let decision = self.wm.new_window_decision(window);
            effects.push(Event::ActivationRequested { window, decision });
        }
        self.apply_events(effects);
        self.sync_content(window, surface);
        // A mapped window is offered a frame, and a window whose geometry or metadata
        // changed gets a fresh configure. A renderer that never answers changes nothing
        // for the client: nothing here waits for it.
        self.refresh_window_frame(window);
        self.publish();
    }

    /// Records what a window's client last committed into the scene.
    ///
    /// The buffer and its view come from Smithay's renderer surface state rather than
    /// from `SurfaceAttributes`: `on_commit_buffer_handler` takes the buffer out of the
    /// attributes when it applies a commit, so the renderer's copy is the only one that
    /// still knows what the client drew.
    fn sync_content(&mut self, window: WindowId, surface: &WlSurface) {
        let factor = crate::protocols::wp_alpha_modifier_v1::committed_factor(surface);
        self.composer.set_window_surface_alpha(window, factor);
        let committed = with_renderer_surface_state(surface, |state| {
            let buffer = state.buffer()?.clone();
            let view = state.view()?;
            let size = state.buffer_size()?;
            let scale = f64::from(state.buffer_scale());
            let transform = state.buffer_transform();
            let buffer_size = SmithaySize::from((f64::from(size.w), f64::from(size.h)));
            let src = view.src.to_buffer(scale, transform, &buffer_size);
            Some((
                buffer,
                Content {
                    src: RectF::new(src.loc.x, src.loc.y, src.size.w, src.size.h),
                    size: Size::new(view.dst.w.max(0).unsigned_abs(), view.dst.h.max(0).unsigned_abs()),
                    transform,
                },
            ))
        })
        .flatten();
        match committed {
            Some((buffer, content)) => {
                self.composer.set_window_content(window, Some(content));
                // `Buffer` derefs to the protocol object; the scene holds the handle and
                // Smithay keeps the object alive for as long as a frame can read it.
                self.composer.set_window_buffer(window, Some((*buffer).clone()));
            }
            None => {
                self.composer.set_window_content(window, None);
                self.composer.set_window_buffer(window, None);
            }
        }
    }

    /// Applies a batch of events to the policy and then their effects.
    fn apply_events(&mut self, events: Vec<Event>) {
        let mut effects = Vec::new();
        for event in events {
            effects.extend(self.wm.apply(event));
        }
        self.apply(effects);
    }

    /// Installs the factory that makes one layout engine per workspace and submits
    /// every workspace's arrangement.
    pub(crate) fn install_layouts(&mut self, control: Rc<dyn LayoutControl>) {
        let factory = Rc::clone(&control);
        self.layouts = Some(control);
        let effects = self
            .wm
            .set_layout_engines(move |workspace, output| factory.engine(workspace, output));
        self.apply(effects);
        self.publish();
    }

    /// Sets one workspace's layout at runtime. The workspace's engine is replaced; the
    /// old one is dropped with its run.
    ///
    /// # Errors
    /// The control's refusal when `name` is not a known layout; nothing changes.
    pub(crate) fn set_workspace_layout(&mut self, workspace: WorkspaceId, name: &str) -> Result<(), String> {
        let control = self
            .layouts
            .clone()
            .ok_or_else(|| "no layouts are installed".to_owned())?;
        control.set_workspace(workspace, name)?;
        let effects = self.wm.rebuild_layout(workspace);
        self.apply(effects);
        self.publish();
        Ok(())
    }

    /// Sends `command` to `workspace`'s layout and applies the placements it answers
    /// with. The answer is the refusal: `unknown layout` when the workspace has no
    /// layout to ask, `refused` when the layout does not know the command. A layout
    /// that fails while answering reports `layout_failed` and the workspace floats.
    ///
    /// # Errors
    /// The refusal message, and nothing changes.
    pub(crate) fn layout_command(
        &mut self,
        workspace: WorkspaceId,
        command: gnoblin_wm::layout::LayoutCommand,
    ) -> Result<(), String> {
        let deadline = std::time::Instant::now() + LAYOUT_COMMAND_WAIT;
        match self.wm.layout_command(workspace, command, deadline) {
            Ok(effects) => {
                self.apply(effects);
                self.publish();
                Ok(())
            }
            Err(gnoblin_wm::CommandError::UnknownLayout) => {
                Err(format!("unknown layout: workspace {} has no layout", workspace.get()))
            }
            Err(gnoblin_wm::CommandError::Refused) => Err("refused: the layout does not know that command".to_owned()),
        }
    }

    /// Every workspace the window manager arranges, with the layout it runs.
    fn observed_workspaces(&self) -> Vec<crate::observation::ObservedWorkspace> {
        let Some(control) = self.layouts.as_ref() else {
            return Vec::new();
        };
        let output = self.wm.arrange_output_name();
        self.wm
            .workspaces()
            .iter()
            .map(|&id| crate::observation::ObservedWorkspace {
                id,
                layout: control.layout_name(id, output),
            })
            .collect()
    }

    /// Every output with its connector name and default layout.
    fn observed_outputs(&self) -> Vec<crate::observation::ObservedOutput> {
        let Some(control) = self.layouts.as_ref() else {
            return Vec::new();
        };
        self.outputs
            .iter()
            .map(|entry| {
                let name = entry.output.name();
                crate::observation::ObservedOutput {
                    id: entry.backend,
                    layout: control.output_layout(&name),
                    name,
                }
            })
            .collect()
    }

    /// Sets an output's default layout at runtime. Every workspace's engine is replaced,
    /// so a workspace with its own override gets a fresh engine for that override.
    ///
    /// # Errors
    /// The control's refusal when `name` is not a known layout, or when no output has
    /// that name; nothing changes.
    pub(crate) fn set_output_layout(&mut self, output: &str, name: &str) -> Result<(), String> {
        let control = self
            .layouts
            .clone()
            .ok_or_else(|| "no layouts are installed".to_owned())?;
        control.set_output(output, name)?;
        let effects = self.wm.rebuild_all_layouts();
        self.apply(effects);
        self.publish();
        Ok(())
    }

    /// Appends an effect to a window's stack, in paint order, and publishes the window.
    ///
    /// A blur costs `passes` passes of the renderer's budget; a shadow or shader costs
    /// one. The request is refused, and the stack is unchanged, when the kind is unknown
    /// or the total would exceed `MAX_PASSES`. A shader with an empty `source` is kept
    /// but inactive, since it has nothing to run.
    ///
    /// # Errors
    /// `unknown-effect-kind:`, `too-many-passes:` or `unknown window`; each message
    /// begins with the error code the caller reports.
    pub(crate) fn add_effect(
        &mut self,
        window: WindowId,
        mut spec: crate::observation::EffectSpec,
    ) -> Result<(), String> {
        let cost = match spec.kind.as_str() {
            "blur" => spec.passes as usize,
            "shadow" | "shader" => 1,
            other => return Err(format!("unknown-effect-kind: `{other}` is not blur, shadow or shader")),
        };
        let entry = self
            .windows
            .get_mut(&window)
            .ok_or_else(|| "unknown window".to_owned())?;
        let used: usize = entry
            .effects
            .iter()
            .map(|effect| match effect.kind.as_str() {
                "blur" => effect.passes as usize,
                _ => 1,
            })
            .sum();
        let budget = gnoblin_render::MAX_PASSES;
        if used + cost > budget {
            return Err(format!(
                "too-many-passes: {used} of {budget} passes are in use and this effect needs {cost}"
            ));
        }
        match crate::effects::inactive_reason(&spec) {
            Some(reason) => {
                spec.active = false;
                spec.reason = reason;
            }
            None => {
                spec.active = true;
                spec.reason.clear();
            }
        }
        entry.effects.push(spec);
        // The scene draws the chain of active, enabled effects; push it on every change so
        // the renderer never lags the stack the client can read back.
        self.composer
            .set_window_effects(window, crate::effects::chain(&entry.effects));
        self.publish();
        Ok(())
    }

    /// What is known about the process behind a window's client. `None` when no such
    /// window exists.
    pub(crate) fn window_process(&self, window: WindowId) -> Option<crate::WindowProcess> {
        let entry = self.windows.get(&window)?;
        // An X11 window has no Wayland client, so there is no process to name.
        let Some(toplevel) = entry.surface.toplevel() else {
            return Some(crate::WindowProcess::Unknown);
        };
        let Some(client) = toplevel.wl_surface().client() else {
            return Some(crate::WindowProcess::Disconnected);
        };
        let identity = client
            .get_data::<crate::protocols::wl_compositor::WlCompositorClientState>()
            .and_then(|data| data.process.get().copied());
        Some(match identity {
            Some(identity) => crate::WindowProcess::Connected(identity),
            None => crate::WindowProcess::Unknown,
        })
    }

    /// Lets the window manager collect the layout runs that finished and applies the
    /// placements they settle on.
    ///
    /// The server calls this once per loop turn. Without it an installed layout never
    /// settles, and the tiling events the window manager holds back for it are never
    /// applied.
    pub(crate) fn poll_layouts(&mut self) {
        let deadline = Instant::now() + LAYOUT_FRAME_BUDGET;
        let effects = self.wm.apply(Event::Frame { deadline });
        if !effects.is_empty() {
            self.apply(effects);
            self.publish();
        }
    }

    /// Applies one tiled placement from a settled layout run.
    ///
    /// The window is configured to the placement's size. Its position is the geometry's
    /// origin in the composer; the transform's depth offset, rotation and scale ride in the
    /// node's base transform, so a 3D placement reaches the scene whole.
    ///
    /// The tiled edges and the work area come from the layout run. The work area is sent as
    /// the window's configure bounds, so a client that sizes itself to the bounds fits it.
    ///
    /// A matrix transform is refused: the scene takes a decomposed transform only.
    fn place_tiled(
        &mut self,
        workspace: WorkspaceId,
        placement: Placement,
        tiled: TiledEdges,
        work_area: Option<Rect>,
    ) {
        let window = placement.window;
        let Transform3::Trs {
            position,
            rotation_degrees,
            scale,
        } = placement.transform
        else {
            log::warn!(area = "layout", %window, %workspace, "a matrix placement is refused: unsupported-transform");
            return;
        };
        // A window closed since the run was submitted has nothing left to place.
        let Some(state) = self.wm.window(window).map(|entry| entry.state) else {
            return;
        };
        // A placement from a settled run means the workspace's layout works again.
        self.layout_failures.remove(&workspace);
        let geometry = placed_geometry(RectF::new(position.x, position.y, placement.width, placement.height));
        let base = gnoblin_types::Transform {
            offset: Vec3::new(0.0, 0.0, position.z),
            rotation_degrees,
            scale,
            depth: 0.0,
        };
        self.composer.place_window_animated(window, geometry, base);
        self.composer.set_window_visible(window, placement.visible);
        self.composer.set_window_opacity(window, placement.opacity as f32);
        self.configure(window, geometry, state, tiled, work_area);
    }

    /// Records that a workspace's layout run failed. The window manager already runs that
    /// workspace floating; the reason is kept for the compositor to report.
    fn layout_failed(&mut self, workspace: WorkspaceId, reason: &'static str) {
        log::warn!(area = "layout", %workspace, reason, "failed; the workspace runs floating");
        self.layout_failures.insert(workspace, reason);
    }

    /// The reason a workspace's layout last failed, or `None` when it has no failure on record.
    pub(crate) fn layout_failure(&self, workspace: WorkspaceId) -> Option<&'static str> {
        self.layout_failures.get(&workspace).copied()
    }

    /// The matrix a window's node is drawn with and its projected bounds, or `None` when it
    /// is not drawn.
    pub(crate) fn drawn_node(&self, window: WindowId) -> Option<(Mat4, Rect)> {
        Some((
            self.composer.window_matrix(window)?,
            self.composer.window_projected_bounds(window)?,
        ))
    }

    /// Sets whether property changes land at once, see [`Composer::set_reduced_motion`].
    pub(crate) fn set_reduced_motion(&mut self, reduced: bool) {
        self.composer.set_reduced_motion(reduced);
    }

    /// Advances the window animations by the real time one loop turn took.
    pub(crate) fn advance_motion(&mut self, elapsed_ms: u32) {
        self.composer.advance_motion(elapsed_ms);
    }

    /// Advances the session clock and gives the window manager the new time, so a rest
    /// focus that has come due takes effect without any pointer event.
    pub(crate) fn tick_clock(&mut self, milliseconds: u64) {
        self.advance_clock(milliseconds);
        self.apply_pointer(Event::PointerTick { at: self.clock });
    }

    /// Feeds a window's role to the window manager when it changed.
    ///
    /// The parent a client names through `xdg-foreign` or `set_parent`, and the dialog
    /// hint `xdg-dialog` sets, are both kept by smithay on the toplevel's role. The
    /// window manager learns them through [`Event::WindowRoleChanged`], and only when
    /// its record differs, so an unchanged commit does nothing.
    pub(crate) fn sync_window_role(&mut self, window: WindowId) {
        let Some(toplevel) = self.windows.get(&window).and_then(|entry| entry.surface.toplevel()) else {
            return;
        };
        let parent = toplevel.parent();
        let hint = with_states(toplevel.wl_surface(), |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .map(|data| data.lock().expect("the attributes lock is never poisoned").dialog_hint)
                .unwrap_or_default()
        });
        let transient_for = parent.as_ref().and_then(|parent| self.window_of(parent));
        // xdg_toplevel.set_parent makes a window transient, so any parented toplevel is a
        // dialog unless it asked to be modal; a parentless toplevel is normal whatever hint it sent.
        let kind = match (transient_for, hint) {
            (Some(_), ToplevelDialogHint::Modal) => WindowKind::Modal,
            (Some(_), _) => WindowKind::Dialog,
            (None, _) => WindowKind::Normal,
        };
        let Some(current) = self.wm.window(window) else {
            return;
        };
        if current.kind == kind && current.transient_for == transient_for {
            return;
        }
        self.apply_events(vec![Event::WindowRoleChanged {
            window,
            kind,
            transient_for,
        }]);
        self.publish();
    }

    /// The policy identity of the window owning a surface.
    pub(crate) fn window_of(&self, surface: &WlSurface) -> Option<WindowId> {
        self.windows
            .iter()
            .find(|(_, entry)| entry.surface.wl_surface().as_ref() == Some(surface))
            .map(|(window, _)| *window)
    }

    /// The window an xdg-foreign handle names: the window owning the exported surface.
    /// `None` for a handle never minted, revoked, or whose surface has gone, and also when
    /// the surface has no window (for example it was never mapped as a toplevel).
    pub(crate) fn exported_window(&self, handle: &str) -> Option<WindowId> {
        let surface = self.xdg_foreign.exported_surface(handle)?;
        self.window_of(surface)
    }

    /// Applies one event about the window a toplevel belongs to.
    pub(crate) fn window_event(&mut self, surface: &ToplevelSurface, event: impl FnOnce(WindowId) -> Event) {
        let Some(window) = self.window_of(surface.wl_surface()) else {
            return;
        };
        let effects = self.wm.apply(event(window));
        self.apply(effects);
        self.publish();
    }

    /// The window manager's state for a toplevel.
    pub(crate) fn window_state(&self, surface: &ToplevelSurface) -> Option<gnoblin_wm::WindowState> {
        let window = self.window_of(surface.wl_surface())?;
        Some(self.wm.window(window)?.state)
    }

    /// Dispatches every pending client request.
    ///
    /// The display is taken out for the call so the dispatcher can be handed
    /// `&mut State` at the same time: see the field's documentation.
    ///
    /// # Errors
    ///
    /// Any error the Wayland backend reports. A protocol error raised by a client
    /// is not one of them — it is reported on that client's connection and this
    /// call still succeeds, which is what keeps one bad client from ending the
    /// session.
    pub(crate) fn dispatch(&mut self) -> std::io::Result<usize> {
        // The turn advances before the dispatch, so every event of one turn reads the
        // same reading: the protocol side's clock, never a wall clock.
        self.turn = self.turn.saturating_add(1);
        let Some(mut display) = self.take_display() else {
            return Ok(0);
        };
        let result = display.dispatch_clients(self);
        self.restore_display(display);
        // A dispatch is the only place a text input can appear, so this is where one that
        // arrived on an already-focused surface is told it has focus.
        crate::protocols::zwp_text_input_v3::announce_new_text_inputs(self);
        result
    }

    /// Writes every queued event to its client.
    pub(crate) fn flush(&mut self) {
        let Some(mut display) = self.take_display() else {
            return;
        };
        let result = display.flush_clients();
        self.restore_display(display);
        if let Err(error) = result {
            log::warn!(%error, "flushing a client failed");
        }
    }

    /// The window's geometry, for popup positioning. `None` when it closed.
    pub(crate) fn geometry_of(&self, window: WindowId) -> Option<Rect> {
        Some(self.shown_geometry(self.wm.window(window)?))
    }

    /// Asks for a window's menu at an anchor, on behalf of a source, and publishes it.
    ///
    /// The request is numbered and kept in a bounded log, so a reader that publishes
    /// each request once can see it even after another arrives first. A window that
    /// has closed gets no request: there is no menu to open for it.
    pub(crate) fn request_window_menu(&mut self, window: WindowId, anchor: MenuAnchor, source: MenuSource) -> bool {
        if self.wm.window(window).is_none() {
            log::debug!(area = "menu", ?window, "a request for a closed window was dropped");
            return false;
        }
        self.menu_seq = self.menu_seq.saturating_add(1);
        let request = MenuRequest::new(window, self.menu_seq, anchor, source, &self.seat_name);
        log::debug!(
            area = "menu",
            ?window,
            seq = self.menu_seq,
            source = source.as_str(),
            "requested"
        );
        self.menu_requests.push_back(request);
        while self.menu_requests.len() > MENU_REQUESTS_KEPT {
            self.menu_requests.pop_front();
        }
        self.publish();
        true
    }

    /// The rectangle a window is drawn at: the placement a settled layout gave it, or
    /// otherwise its own geometry. A tiled window's own geometry is the floating rectangle
    /// it returns to when it floats, not where it is shown.
    pub(crate) fn shown_geometry(&self, window: &gnoblin_wm::Window) -> Rect {
        self.wm.placed_rect(window.id).map_or(window.geometry, placed_geometry)
    }

    /// The monitor rectangle of the output the window is on.
    pub(crate) fn monitor_of(&self, window: WindowId) -> Option<Rect> {
        let output = self.wm.window(window)?.output?;
        Some(self.wm.output(output)?.rect())
    }

    /// Fires the frame callback every mapped window is waiting for.
    ///
    /// A client asks for a callback with `wl_surface.frame` and gets exactly one answer
    /// per frame the compositor shows; anything that paces itself by that answer draws
    /// one frame and then waits for ever without it. That is every well-behaved client,
    /// and it is above all a nested session, whose whole output *is* one of these
    /// surfaces: without this call a session on the nested backend would show its first
    /// frame and then stall.
    pub(crate) fn send_frame_callbacks(&self, time: u32) {
        for entry in self.windows.values() {
            // A window with no buffer has nothing on screen to have drawn, and Smithay's
            // own helper refuses those for the same reason.
            if !entry.mapped {
                continue;
            }
            let Some(surface) = entry.surface.wl_surface() else {
                continue;
            };
            with_states(&surface, |states| {
                for callback in states
                    .cached_state
                    .get::<SurfaceAttributes>()
                    .current()
                    .frame_callbacks
                    .drain(..)
                {
                    callback.done(time);
                }
            });
        }
    }

    /// Ends the iteration the frame hook ran, dropping the damage it consumed.
    pub(crate) fn end_frame(&mut self) {
        self.composer.end_frame();
    }

    /// The damage of the frame the host described for `output`, if it described one.
    ///
    /// The frame hook asks immediately after rendering that output, which is why one
    /// remembered frame is enough.
    /// Forces every output to repaint, for a change the scene graph cannot see.
    ///
    /// Taking the cover down is exactly that: the composer was not asked to draw
    /// anything while the session was covered, so its damage for those outputs was
    /// consumed and the desktop has to be painted again from scratch.
    pub(crate) fn damage_outputs(&mut self) {
        for output in self.output_ids() {
            self.composer.damage_output(output);
        }
    }

    /// Queues the modal events an input edge becomes while a modal is set. Returns
    /// `false` for an edge that is not a modal event, which then routes as usual.
    fn route_to_modal(&mut self, event: &InputEvent) -> bool {
        match *event {
            InputEvent::Key { keycode, pressed } => {
                let state = if pressed {
                    gnoblin_types::KeyState::Pressed
                } else {
                    gnoblin_types::KeyState::Released
                };
                self.modal_events.push(ModalEvent::Key { keycode, state });
            }
            InputEvent::PointerButton { button, pressed } => {
                let state = if pressed {
                    gnoblin_types::KeyState::Pressed
                } else {
                    gnoblin_types::KeyState::Released
                };
                self.modal_events.push(ModalEvent::PointerButton {
                    x: self.pointer_position.x,
                    y: self.pointer_position.y,
                    button,
                    state,
                });
            }
            InputEvent::PointerMotion { .. }
            | InputEvent::PointerMotionRelative { .. }
            | InputEvent::PointerLeave
            | InputEvent::PointerAxis { .. }
            | InputEvent::Touch { .. } => {}
            InputEvent::DeviceAdded { .. } | InputEvent::DeviceRemoved { .. } => return false,
        }
        true
    }

    /// Sets or clears the modal and damages the outputs it covers, so the change is drawn.
    ///
    /// Only a `ModalToOutput` layer takes input here. `Passive` and `Grab` layers have
    /// no engine path yet, so a modal that declares either is refused and the current
    /// modal stands, rather than being honoured under a policy it does not hold.
    pub(crate) fn set_modal(&mut self, modal: Option<Modal>) {
        if let Some(modal) = modal
            .as_ref()
            .filter(|modal| modal.input() != InputPolicy::ModalToOutput)
        {
            log::warn!(area = "modal", policy = ?modal.input(), "refused a modal whose input policy the engine does not honour");
            return;
        }
        for output in [self.modal.as_ref(), modal.as_ref()]
            .into_iter()
            .flatten()
            .map(Modal::output)
        {
            self.composer.damage_output(output);
        }
        self.modal = modal;
    }

    /// Takes the modal events queued since the last take.
    pub(crate) fn take_modal_events(&mut self) -> Vec<ModalEvent> {
        std::mem::take(&mut self.modal_events)
    }

    /// The damage rectangles of the frame just described for `output`, or `None` when
    /// that output's frame was not described, so nothing was drawn for it this cycle.
    pub(crate) fn frame_damage(&self, output: OutputId) -> Option<&[Rect]> {
        match &self.last_frame {
            Some((described, damage)) if *described == output => Some(damage),
            _ => None,
        }
    }

    /// Moves a window through the window manager, as a drag would.
    ///
    /// It is the seam a test and the parity harness drive window movement with until
    /// the input layer produces drag rectangles of its own: the event takes the same
    /// path a drag's would, including the work-area constraint and the configure the
    /// client receives.
    pub(crate) fn move_window(&mut self, window: WindowId, geometry: Rect) {
        let effects = self.wm.apply(Event::MoveResize { window, geometry });
        self.apply(effects);
        self.publish();
    }

    /// Sets a window's scene opacity, as an animation would.
    pub(crate) fn set_window_opacity(&mut self, window: WindowId, opacity: f32) {
        self.composer.set_window_opacity(window, opacity);
    }

    /// Rings the session bell: counts it for the published snapshot and, when the document
    /// asks for a visual bell, flashes the screen.
    ///
    /// The window is the one that asked, when the protocol named one. A shell that wanted to
    /// say *which* application is asking reads it from the snapshot; the flash itself does
    /// not depend on it, since a frame flash covers the focused window.
    pub(crate) fn ring_bell(&mut self, window: Option<WindowId>) {
        self.bells = self.bells.saturating_add(1);
        log::debug!(area = "bell", ?window, bells = self.bells, "rung");
        self.flash_bell();
        self.publish();
    }

    /// Flashes the screen for a rung bell, when the document declares a visual bell and the
    /// session is unlocked.
    ///
    /// A locked session shows nothing, so a bell cannot reveal what the lock hides. A frame
    /// flash covers the focused window; with no focused window there is nothing to flash.
    fn flash_bell(&mut self) {
        if !self.visual_bell.enabled || self.lock.state() != LockState::Unlocked {
            return;
        }
        let target = match self.visual_bell.scope {
            crate::server::VisualBellScope::Fullscreen => None,
            crate::server::VisualBellScope::Frame => match self.wm.focused() {
                Some(window) => Some(window),
                None => return,
            },
        };
        self.composer.flash_window(target);
    }

    /// The managed window an X11 window id names, when Xwayland has one.
    ///
    /// `gnoblin-xwayland` owns the window table, so the answer comes from the session's
    /// table rather than from a second mapping here. `None` covers a session with no
    /// Xwayland and an X11 window this session did not manage, which are the same
    /// answer to the protocol that asks.
    pub(crate) fn x11_window(&self, xwindow: u32) -> Option<WindowId> {
        self.x11.as_ref()?.window_for_x11(xwindow)
    }

    /// Pings the clients with mapped toplevels, and publishes the window state when a
    /// client's unresponsive state changed.
    pub(crate) fn poll_liveness(&mut self) {
        let clients: Vec<_> = self
            .windows
            .values()
            .filter(|entry| entry.mapped)
            .filter_map(|entry| match &entry.surface {
                Managed::Wayland(surface) => Some((crate::liveness::owner(surface)?, surface.client())),
                Managed::X11(_) => None,
            })
            .collect();
        if self.liveness.poll(self.clock, &clients) {
            self.publish();
        }
    }

    /// Records a pong from `client`, publishing the window state when the client had
    /// been unresponsive.
    pub(crate) fn record_pong(&mut self, client: &smithay::wayland::shell::xdg::ShellClient) {
        let owner = self.windows.values().find_map(|entry| match &entry.surface {
            Managed::Wayland(surface) if surface.client() == *client => crate::liveness::owner(surface),
            _ => None,
        });
        if owner.is_some_and(|id| self.liveness.pong(&id)) {
            self.publish();
        }
    }

    /// Publishes the window state for the tests and for a diagnostics reader.
    pub(crate) fn publish(&mut self) {
        let active = self.wm.active_workspace();
        // A covered window keeps the window manager's focus so the reveal can restore
        // it, but it holds no keyboard focus: the lock gate routes keys to the lock
        // surface or to nothing, so the snapshot says nothing is focused.
        let focused = if self.lock.state() == LockState::Unlocked {
            self.wm.focused()
        } else {
            None
        };
        // The published lists follow the window manager, so a shell and a test read
        // the same answer as the compositor's own record.
        self.sync_authority_lock();
        self.sync_foreign_toplevels();
        let workspaces = self.observed_workspaces();
        let output_layouts = self.observed_outputs();
        let mut snapshot = SessionSnapshot {
            outputs: self.outputs.iter().map(|entry| entry.backend).collect(),
            stack: self.wm.stack().to_vec(),
            workspaces,
            output_layouts,
            output_rects: self
                .outputs
                .iter()
                .filter_map(|entry| self.output_rect(entry.backend).map(|rect| (entry.backend, rect)))
                .collect(),
            layout_failures: self.layout_failures.iter().map(|(ws, reason)| (*ws, *reason)).collect(),
            focus: focused,
            lock: LockObservation {
                state: self.lock.state(),
                capture_allowed: !self.lock_embargo.capture,
            },
            capture_sessions: self.capture.sessions(),
            idle_ms: self.activity.idle_ticks(self.clock),
            inhibitors: self.idle_inhibitors(),
            layers: self.layers.observed(),
            activation_denied: self.activation.denied().cloned(),
            bells: self.bells,
            menu_requests: self.menu_requests.iter().cloned().collect(),
            selection: self.clipboard_mime_types(),
            ..SessionSnapshot::default()
        };
        for window in self.wm.windows() {
            let entry = self.windows.get(&window.id);
            let mapped = entry.is_some_and(|entry| entry.mapped);
            snapshot.windows.insert(
                window.id,
                ObservedWindow {
                    id: window.id,
                    title: window.title.clone(),
                    app_id: window.app_id.clone(),
                    mapped,
                    visible: window.is_visible(active),
                    focused: focused == Some(window.id),
                    geometry: self.shown_geometry(window),
                    icon_name: entry.and_then(|entry| entry.icon_name.clone()),
                    icon_buffer_count: entry.map_or(0, |entry| entry.icon_buffer_count),
                    tag: entry.and_then(|entry| entry.tag.clone()),
                    effects: entry.map_or_else(Vec::new, |entry| entry.effects.clone()),
                    modal: window.kind == WindowKind::Modal,
                    unresponsive: matches!(
                        entry.map(|entry| &entry.surface),
                        Some(Managed::Wayland(surface)) if crate::liveness::owner(surface).is_some_and(|id| self.liveness.is_unresponsive(&id))
                    ),
                    transient_for: window.transient_for,
                },
            );
        }
        match self.observation.lock() {
            Ok(mut guard) => {
                *guard = snapshot;
                Self::announce(self.observer.as_deref(), &guard);
            }
            // A reader that panicked while holding the lock left the snapshot in
            // place. The loop keeps running and this publish still replaces it, so the
            // state stays whole rather than the session losing its observation.
            Err(poisoned) => {
                let mut guard = poisoned.into_inner();
                *guard = snapshot;
                Self::announce(self.observer.as_deref(), &guard);
            }
        }
    }

    /// Tells the observer what the snapshot now holds.
    ///
    /// Stored first, then announced: a reader that wakes on this call reads the
    /// state this call is about, which is what lets it treat the notification as a
    /// hint rather than a payload.
    fn announce(observer: Option<&dyn Observer>, snapshot: &SessionSnapshot) {
        if let Some(observer) = observer {
            observer.published(snapshot);
        }
    }

    /// Asks the window manager to focus a window, as a shell's `window.focus` does.
    ///
    /// It takes the same path a click's activation takes, so the policy decides and
    /// the result is what the next snapshot says. The return value is whether the
    /// window manager holds focus on it now, which is what a caller that has to
    /// answer an operation needs; a window that has closed is not focused and is not
    /// invented.
    pub(crate) fn focus_window(&mut self, window: WindowId) -> bool {
        if self.wm.window(window).is_none() {
            return false;
        }
        let effects = self.wm.apply(Event::ActivationRequested {
            window,
            decision: ActivationDecision::Allow,
        });
        self.apply(effects);
        self.publish();
        self.wm.focused() == Some(window)
    }

    /// Asks the window manager to focus the window under the pointer.
    fn activate_under_pointer(&mut self) {
        let Some((surface, _)) = self.surface_under(self.pointer_position) else {
            return;
        };
        let Some(window) = self.window_of(&surface) else {
            return;
        };
        let effects = self.wm.apply(Event::PointerPressed { window });
        self.apply(effects);
        self.publish();
    }

    /// Reports the window under the pointer to the window manager, which decides whether
    /// focus follows it.
    fn report_pointer(&mut self) {
        match self.surface_under(self.pointer_position) {
            Some((surface, _)) => {
                // A surface that is no window's toplevel (the lock surface) says nothing about
                // the windows, so it is not reported as leaving them.
                if let Some(window) = self.window_of(&surface) {
                    self.report_window(Some(window));
                }
            }
            None => self.report_window(None),
        }
    }

    /// Sends the window the pointer is over, or none, to the window manager.
    fn report_window(&mut self, window: Option<WindowId>) {
        self.apply_pointer(Event::PointerMoved { window, at: self.clock });
    }

    /// Gives the window manager a pointer event. A locked session, a modal and an exclusive
    /// layer surface keep focus where it is, so no pointer event reaches the window manager.
    fn apply_pointer(&mut self, event: Event) {
        let held = self.lock.state() != LockState::Unlocked || self.modal.is_some();
        if held || self.layer_keyboard_focus().is_some() {
            return;
        }
        let effects = self.wm.apply(event);
        if !effects.is_empty() {
            self.apply(effects);
            self.publish();
        }
    }

    /// Counts pointer button presses and releases. The window manager hears the grab when the
    /// first button goes down and its release when the last one comes up.
    fn button_grab(&mut self, pressed: bool) {
        let was_held = self.held_buttons > 0;
        self.held_buttons = if pressed {
            self.held_buttons.saturating_add(1)
        } else {
            self.held_buttons.saturating_sub(1)
        };
        if (self.held_buttons > 0) != was_held {
            let effects = self.wm.apply(Event::PointerGrab {
                active: self.held_buttons > 0,
            });
            self.apply(effects);
        }
    }

    /// The surface under a global position, topmost first, with the surface's own
    /// origin in global logical coordinates.
    ///
    /// While the session is covered the lock answers instead: the only surface a
    /// pointer event may reach is the owner's lock surface, and a miss reaches nothing
    /// at all.
    ///
    /// The origin is what Smithay's pointer needs, not the position inside the surface:
    /// its `MotionEvent` carries the global location and every `PointerTarget` makes it
    /// surface-relative by subtracting this. Handing it the local position instead
    /// makes every client's pointer coordinates relative to the wrong corner, which for
    /// a window at the origin is a pointer that never moves.
    pub(crate) fn surface_under(
        &self,
        position: SmithayPoint<f64, Logical>,
    ) -> Option<(WlSurface, SmithayPoint<f64, Logical>)> {
        if self.lock.state() != LockState::Unlocked {
            return self.lock_surface_under(position);
        }
        let active = self.wm.active_workspace();
        for window in self.wm.stack().iter() {
            let (Some(entry), Some(managed)) = (self.windows.get(window), self.wm.window(*window)) else {
                continue;
            };
            if !managed.is_visible(active) || self.window_is_covered(*window) {
                continue;
            }
            let geometry = self.shown_geometry(managed);
            let inside = position.x >= f64::from(geometry.x)
                && position.y >= f64::from(geometry.y)
                && position.x < geometry.right() as f64
                && position.y < geometry.bottom() as f64;
            if inside {
                let origin = SmithayPoint::from((f64::from(geometry.x), f64::from(geometry.y)));
                return entry.surface.wl_surface().map(|surface| (surface, origin));
            }
        }
        None
    }
}

/// Removes the globals the session's configuration turned off.
///
/// Every protocol state is still created, because a reload can turn a protocol back on
/// and a handler needs somewhere to answer from; what a disabled protocol loses is its
/// global, which is the only thing a client can discover.
fn remove_disabled_globals(display_handle: &DisplayHandle, protocols: &Protocols, globals: &[(Global, GlobalId)]) {
    for (global, id) in globals {
        if global.is_advertised(protocols) {
            continue;
        }
        display_handle.remove_global::<State>(id.clone());
        log::debug!(area = "access", global = global.as_str(), "not advertised");
    }
}

/// The Smithay mode for a backend output.
fn smithay_mode(output: &gnoblin_backend::Output) -> Mode {
    Mode {
        size: SmithaySize::from((output.mode.width as i32, output.mode.height as i32)),
        refresh: output.mode.refresh_mhz as i32,
    }
}

/// A layout rectangle as the window manager states it.
/// Sets or clears an xdg toplevel state flag.
fn set_flag(states: &mut ToplevelStateSet, state: xdg_toplevel::State, enabled: bool) {
    if enabled {
        states.set(state);
    } else {
        states.unset(state);
    }
}

/// Sets `suspended` on a toplevel and sends a configure when that changes it.
///
/// The change is judged against the pending state, which is the state the next configure carries. A
/// configure before the first one carries the state in the initial configure, so nothing is sent then.
fn set_suspended(toplevel: &ToplevelSurface, suspended: bool) {
    let changed = toplevel.with_pending_state(|pending| {
        let was = pending.states.contains(xdg_toplevel::State::Suspended);
        set_flag(&mut pending.states, xdg_toplevel::State::Suspended, suspended);
        was != suspended
    });
    if changed && toplevel.is_initial_configure_sent() {
        toplevel.send_configure();
    }
}

/// The whole-pixel rectangle a placement occupies. The scene and the configure draw
/// with it, and the observers read the same rectangle, so the two never disagree.
fn placed_geometry(rect: RectF) -> Rect {
    Rect::new(
        rect.x.round() as i32,
        rect.y.round() as i32,
        rect.width.round() as u32,
        rect.height.round() as u32,
    )
}

/// An X11 rectangle in the policy's coordinates.
///
/// The X11 window manager works in logical pixels and so does the policy, so this is a
/// type conversion, not a scale: a fractional client scale reaches Xwayland through its
/// own `wl_surface` scale, which is where Smithay reads it.
pub(crate) fn x11_geometry(rect: Rect) -> Rectangle<i32, Logical> {
    Rectangle::new(
        (rect.x, rect.y).into(),
        (
            i32::try_from(rect.width).unwrap_or(i32::MAX),
            i32::try_from(rect.height).unwrap_or(i32::MAX),
        )
            .into(),
    )
}

/// Whether a rectangle holds a layout point.
///
/// The upper edges are exclusive, because a point on the right or bottom edge of a
/// monitor is a point on the next one, and a point on the last one is outside it.
/// Sets `frame`'s magnified view and colour transform from `settings`, for the output
/// whose rectangle is `rect`.
///
/// A missing rectangle or a settings value the frame rejects leaves that part off, so an
/// output the session no longer knows is drawn unmagnified rather than wrongly.
fn apply_visual_accessibility(
    settings: crate::server::VisualAccessibilitySettings,
    rect: Option<Rect>,
    frame: &mut OutputFrame,
) {
    let view = match (settings.magnifier_zoom, rect) {
        (Some(zoom), Some(rect)) => magnifier_view(zoom, rect),
        _ => None,
    };
    frame.set_view(view);
    frame.set_colour_matrix(settings.colour_matrix.and_then(ColourMatrix::new));
}

/// The magnified view of an output of size `rect`: centred on the middle of the output, so
/// each output is magnified about its own middle. `None` when the zoom is the identity or
/// the frame rejects it.
pub(crate) fn magnifier_view(zoom: f32, rect: Rect) -> Option<View> {
    if zoom <= gnoblin_render_frame::frame::MIN_ZOOM {
        return None;
    }
    View::new(zoom, rect.width as f32 / 2.0, rect.height as f32 / 2.0)
}

fn contains(rect: &Rect, point: SmithayPoint<f64, Logical>) -> bool {
    let right = f64::from(rect.x + i32::try_from(rect.width).unwrap_or(i32::MAX));
    let bottom = f64::from(rect.y + i32::try_from(rect.height).unwrap_or(i32::MAX));
    point.x >= f64::from(rect.x) && point.x < right && point.y >= f64::from(rect.y) && point.y < bottom
}

/// Whether a surface has a committed buffer, which is what makes a toplevel
/// mapped.
///
/// The buffer is read from the renderer's surface state rather than from
/// `SurfaceAttributes`: `on_commit_buffer_handler` takes the buffer out of the
/// attributes when it applies a commit, so the attributes are empty by the time this
/// runs and only the renderer's copy records that the client has content.
pub(crate) fn has_buffer(surface: &WlSurface) -> bool {
    with_renderer_surface_state(surface, |state| state.buffer().is_some()).unwrap_or(false)
}

impl Host for State {
    fn begin_frame(&mut self) {
        // The pointer may have moved since the last frame; its old and new places are
        // damaged before the damage is taken.
        let redraw = std::mem::take(&mut self.cursor_shape_changed);
        self.update_cursor(redraw);
        // The damage is taken once for every output, because the backend renders one
        // output at a time and a per-output take would clear the others' damage.
        self.composer.begin_frame();
        self.last_frame = None;
        self.protection_generation = None;
    }

    fn frame(&mut self, output: OutputId, frame: &mut OutputFrame) -> bool {
        // The accessibility is set before the frame is composed, so the composer draws this
        // output's magnified view and colour transform. A locked session draws no
        // accessibility, so the lock cover reads the same whatever the user has set.
        let settings = if self.lock.state() == LockState::Unlocked {
            self.visual_accessibility
        } else {
            crate::server::VisualAccessibilitySettings::default()
        };
        apply_visual_accessibility(settings, self.output_rect(output), frame);
        let changed = if self.lock.state() == LockState::Unlocked {
            self.composer.compose(output, frame)
        } else {
            self.cover_frame(output, frame)
        };
        if changed {
            self.last_frame = Some((output, frame.damage().to_vec()));
            // The frame was described after the hide, so it can prove a cover: the
            // generation is read here rather than at present time, which is what makes
            // a frame queued before the cover worthless.
            self.protection_generation = Some(self.session_host.protection.generation());
        }
        frame.set_cursor_image(self.pointer_cursor(output));
        // The modal overlay is the last layer on the output it covers. `compose` and
        // `cover_frame` clear the frame first, so a cleared or moved modal leaves no stale
        // overlay behind.
        if let Some(modal) = self.modal.as_ref().filter(|modal| modal.output() == output) {
            frame.push_layer(Layer::Overlay(modal.overlay().clone()));
        }
        changed
    }

    fn input(&mut self, event: InputEvent) {
        let time = InputTime::now();
        // Any input counts as activity: it is what ends an idle period and what the idle
        // notifications are derived from. It is recorded before the event is routed, so a
        // key a binding consumes still counts as the user being present.
        self.record_activity();
        // Focus does not follow the pointer while a button is held. This is counted before the
        // modal route, so a release that a modal takes still ends the grab.
        if let InputEvent::PointerButton { pressed, .. } = event {
            self.button_grab(pressed);
        }
        // While a modal is set it takes the keys and pointer buttons, and the clients
        // see none of them; the rest of the input is dropped for the same reason.
        if self.modal.is_some() && self.route_to_modal(&event) {
            return;
        }
        match event {
            InputEvent::PointerMotion { output, position } => {
                let Some(entry) = self.outputs.iter().find(|entry| entry.backend == output) else {
                    return;
                };
                let position = SmithayPoint::from((
                    f64::from(position.x + entry.rect.x),
                    f64::from(position.y + entry.rect.y),
                ));
                let delta = position - self.pointer_position;
                // A locked pointer does not move: its whole motion becomes a relative
                // vector for the client that asked for the lock, and nothing else changes.
                if self.locked_surface().is_some() {
                    self.send_relative_motion(delta, time);
                    return;
                }
                let position = self.confine(position);
                self.move_pointer(position, time);
                self.report_pointer();
                // The vector goes out with the position, so a client that only wants
                // relative motion does not have to difference two positions.
                self.send_relative_motion(delta, time);
            }
            InputEvent::PointerMotionRelative { dx, dy } => {
                // A relative device states a delta rather than a position, which is what
                // a mouse and an EI client both send. The delta is applied to the
                // position the seat holds and the result is kept inside the layout, so a
                // fast move stops at an edge instead of leaving every output.
                let wanted = SmithayPoint::from((
                    self.pointer_position.x + f64::from(dx),
                    self.pointer_position.y + f64::from(dy),
                ));
                let position = self.within_layout(wanted);
                self.move_pointer(position, time);
                self.report_pointer();
            }
            InputEvent::PointerLeave => {
                self.frame_pointer(None, None);
                let pointer = self.pointer.clone();
                let serial = SERIAL_COUNTER.next_serial();
                pointer.motion(
                    self,
                    None,
                    &MotionEvent {
                        location: self.pointer_position,
                        serial,
                        time,
                    },
                );
                self.report_window(None);
            }
            InputEvent::PointerButton { button, pressed } => {
                self.frame_pointer(Some(self.pointer_position), Some(pressed));
                if pressed {
                    // A press on a window is the activation request the window
                    // manager expects from the caller: attribution decides whether
                    // it may take focus in a later slice, and a trusted input
                    // context is exactly this case.
                    self.activate_under_pointer();
                }
                let pointer = self.pointer.clone();
                let serial = SERIAL_COUNTER.next_serial();
                // A button serial is the one an application names in
                // `xdg_activation_token.set_serial`, so it has to be remembered.
                self.remember_input_serial(u32::from(serial));
                pointer.button(
                    self,
                    &ButtonEvent {
                        serial,
                        time,
                        button,
                        state: if pressed {
                            ButtonState::Pressed
                        } else {
                            ButtonState::Released
                        },
                    },
                );
            }
            InputEvent::Key { keycode, pressed } => {
                let serial = SERIAL_COUNTER.next_serial();
                self.remember_input_serial(u32::from(serial));
                let state = if pressed {
                    gnoblin_types::KeyState::Pressed
                } else {
                    gnoblin_types::KeyState::Released
                };
                // The whole delivery rule is in `dispatch_key`: the router decides whether
                // the edge belongs to a grab, a binding or the client, and only a client
                // edge reaches the seat. The evdev-to-X11 offset is applied there too, so
                // a client's `wl_keyboard.key` is the code the protocol defines.
                self.dispatch_key(keycode, state, serial, time);
            }
            // A scroll needs the axis source and the pointer's focus serial, which the
            // input slice adds; until then a scroll reaches the seat and stops here
            // rather than scrolling a client by the wrong amount.
            InputEvent::PointerAxis { .. } => {}
            // A touch point needs a seat that tracks one point per slot per output,
            // which is the input slice's; nothing is invented about it here.
            InputEvent::Touch { .. } => {}
            // A device joining or leaving changes no focus or pointer state; the seat only
            // needs the release a removal sends for the buttons and touches it held.
            InputEvent::DeviceAdded { .. } | InputEvent::DeviceRemoved { .. } => {}
        }
    }
}

impl std::fmt::Debug for State {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("State")
            .field("windows", &self.windows.len())
            .field("outputs", &self.outputs.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod visual_accessibility_tests {
    //! The magnifier and colour filter reach each output's frame, and nowhere else.

    use super::*;
    use crate::server::VisualAccessibilitySettings;
    use gnoblin_render_frame::frame::MIN_ZOOM;

    /// A 1920 by 1080 output at the origin, and a second one to its right.
    fn left_output() -> Rect {
        Rect::new(0, 0, 1920, 1080)
    }

    fn right_output() -> Rect {
        Rect::new(1920, 0, 1280, 720)
    }

    #[test]
    fn the_default_settings_leave_the_frame_unchanged() {
        let mut frame = OutputFrame::new();
        apply_visual_accessibility(VisualAccessibilitySettings::default(), Some(left_output()), &mut frame);
        assert!(frame.view().is_none(), "no magnifier by default");
        assert!(frame.colour_matrix().is_none(), "no colour transform by default");
    }

    #[test]
    fn a_zoom_of_one_is_the_identity() {
        assert!(magnifier_view(MIN_ZOOM, left_output()).is_none());
        let mut frame = OutputFrame::new();
        let settings = VisualAccessibilitySettings {
            magnifier_zoom: Some(1.0),
            colour_matrix: None,
        };
        apply_visual_accessibility(settings, Some(left_output()), &mut frame);
        assert!(frame.view().is_none());
    }

    #[test]
    fn the_magnifier_is_centred_on_each_output_separately() {
        let left = magnifier_view(2.0, left_output()).expect("a view for 2x");
        assert_eq!((left.zoom, left.centre_x, left.centre_y), (2.0, 960.0, 540.0));
        let right = magnifier_view(2.0, right_output()).expect("a view for 2x");
        assert_eq!((right.zoom, right.centre_x, right.centre_y), (2.0, 640.0, 360.0));
    }

    #[test]
    fn the_zoom_reaches_the_frame_of_its_own_output_only() {
        let settings = VisualAccessibilitySettings {
            magnifier_zoom: Some(2.0),
            colour_matrix: None,
        };
        let mut left = OutputFrame::new();
        apply_visual_accessibility(settings, Some(left_output()), &mut left);
        let mut right = OutputFrame::new();
        apply_visual_accessibility(settings, Some(right_output()), &mut right);
        assert_eq!(left.view().map(|view| view.centre_x), Some(960.0));
        assert_eq!(right.view().map(|view| view.centre_x), Some(640.0));
    }

    #[test]
    fn an_unknown_output_is_drawn_unmagnified() {
        let settings = VisualAccessibilitySettings {
            magnifier_zoom: Some(2.0),
            colour_matrix: None,
        };
        let mut frame = OutputFrame::new();
        apply_visual_accessibility(settings, None, &mut frame);
        assert!(frame.view().is_none());
    }

    #[test]
    fn the_colour_filter_reaches_the_frame() {
        let invert = [
            [-1.0, 0.0, 0.0, 0.0, 1.0],
            [0.0, -1.0, 0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0, 0.0, 1.0],
            [0.0, 0.0, 0.0, 1.0, 0.0],
        ];
        let settings = VisualAccessibilitySettings {
            magnifier_zoom: None,
            colour_matrix: Some(invert),
        };
        let mut frame = OutputFrame::new();
        apply_visual_accessibility(settings, Some(left_output()), &mut frame);
        assert_eq!(frame.colour_matrix().map(|matrix| matrix.rows), Some(invert));
    }

    #[test]
    fn a_rejected_colour_matrix_is_not_applied() {
        let mut bad = [[0.0f32; 5]; 4];
        bad[0][0] = f32::NAN;
        let settings = VisualAccessibilitySettings {
            magnifier_zoom: None,
            colour_matrix: Some(bad),
        };
        let mut frame = OutputFrame::new();
        apply_visual_accessibility(settings, Some(left_output()), &mut frame);
        assert!(frame.colour_matrix().is_none());
    }
}
