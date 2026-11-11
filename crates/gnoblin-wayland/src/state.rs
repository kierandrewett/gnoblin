//! The compositor state: the Smithay protocol state, the window record and the
//! translation between them.
//!
//! Every protocol handler in [`crate::protocols`] turns a request into a call on the window record
//! ([`crate::fallback::Fallback`]) or, while a shell holds the tree, into an event on the window's node. The
//! decisions the record returns come back here, where they become client configures, seat focus and output
//! enter/leave. The record decides nothing while a shell is attached: the shell does.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use smithay::backend::input::{Axis, AxisSource, ButtonState, InputTime, TouchSlot};
use smithay::backend::renderer::utils::with_renderer_surface_state;
use smithay::desktop::utils::{bbox_from_surface_tree, under_from_surface_tree};
use smithay::desktop::{PopupManager, WindowSurfaceType};
use smithay::input::keyboard::KeyboardHandle;
use smithay::input::pointer::{AxisFrame, ButtonEvent, CursorImageStatus, MotionEvent, RelativeMotionEvent};
use smithay::input::touch::{DownEvent, MotionEvent as TouchMotionEvent, TouchHandle, UpEvent};
use smithay::input::{Seat, SeatState, pointer::PointerHandle};
use smithay::output::{Mode, Output, PhysicalProperties, Scale as OutputScale, Subpixel};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::Display;
use smithay::reexports::wayland_server::backend::{GlobalId, ObjectId};
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, DisplayHandle, Resource};
use smithay::utils::{Logical, Point as SmithayPoint, Rectangle, SERIAL_COUNTER, Size as SmithaySize, Transform};
use smithay::wayland::compositor::{
    CompositorState, SurfaceAttributes, TraversalAction, with_states, with_surface_tree_downward,
};
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
use smithay::wayland::xdg_activation::XdgActivationState;
use smithay::wayland::xdg_system_bell::XdgSystemBellState;
use smithay::wayland::xdg_toplevel_icon::{ToplevelIconCachedState, XdgToplevelIconManager};
use smithay::wayland::xdg_toplevel_tag::{XdgToplevelTagManager, XdgToplevelTagSurfaceData};
use smithay::wayland::xwayland_keyboard_grab::XWaylandKeyboardGrabState;
use smithay::wayland::xwayland_shell::XWaylandShellState;
use smithay::xwayland::xwm::X11Surface;

use gnoblin_backend::render::compose::{Composer, Content, MainSeatPolicy};
use gnoblin_backend::{Host, InputEvent, TouchPhase};
use gnoblin_config::{Protocols, TitlebarAction};
use gnoblin_input::Activity;
use gnoblin_lock::{Embargo, Lock, State as LockState};
use gnoblin_output::{LayoutConfig, compute};
use gnoblin_render::Mat4;
use gnoblin_render::transaction::PointerInput;
use gnoblin_render_frame::frame::{ColourMatrix, Layer, OutputFrame, View};
use gnoblin_types::{NodeId, OutputId, PointF, Rect, RectF, Size, Tick, WindowId, WorkspaceId};

use crate::access::{Access, AccessPolicy, ClientRole, Global};
use crate::fallback::{Fallback, FallbackEffect, FallbackRefusal, Open, Standing, WindowKind, WindowState};
use crate::keyboard::KeyboardTables;
use crate::modal::{Modal, ModalEvent};
use crate::observation::{
    LockObservation, MenuAnchor, MenuRequest, MenuSource, ObservedWindow, Observer, PresentAnswer, SessionSnapshot,
    WindowChange, WindowChangeRefusal, WindowVisual, WindowVisualRefusal,
};
use crate::output_control::OutputControlState;
use crate::policy::WmOptions;
use crate::protocols::capture_shared::CaptureTables;
use crate::protocols::ext_idle_notify_v1::ExtIdleNotifyV1Tables;
use crate::protocols::ext_session_lock_v1::ExtSessionLockV1Tables;
use crate::protocols::extension::{Extension, TitlebarActions};
use crate::protocols::foreign_toplevel_shared::ForeignToplevels;
use crate::protocols::gnoblin_session_host_v1::GnoblinSessionHostV1Tables;
use crate::protocols::wl_data_device_manager::WlDataDeviceManagerSelections;
use crate::protocols::wp_presentation_time::WpPresentationTimeTables;
use crate::protocols::wp_tearing_control_v1::WpTearingControlV1Hint;
use crate::protocols::xdg_activation_v1::XdgActivationV1;
use crate::protocols::zwlr_layer_shell_v1::ZwlrLayerShellV1Layers;
use crate::protocols::zwlr_output_management_v1::ZwlrOutputManagementV1State;
use crate::protocols::zwp_pointer_constraints_v1::ZwpPointerConstraintsV1Tables;
use crate::server::StartupError;
use crate::tree_frame::{TreeFrame, TreeHooks, TreeInput, TreeRequest};
use crate::window_request::{WindowRequest, X11Flags};

use crate::log;

/// The one workspace the compositor keeps. The shell owns the workspaces it arranges; this is the single implicit
/// space the session's object graph publishes, so a control request that names it is a no-op and any other is unknown.
pub(crate) const IMPLICIT_WORKSPACE: WorkspaceId = WorkspaceId::new(1);

/// The Linux evdev code of the left button, which a touch stands for when the shell is told of it.
const BTN_LEFT: u32 = 0x110;

/// How many window menu requests the session keeps for a reader to catch up on. A reader
/// that polls slower than requests arrive loses the oldest, so the bound is generous: a
/// request is a human action, not a stream.
const MENU_REQUESTS_KEPT: usize = 32;

/// The keyboard repeat settings a session starts with: the delay before a held key
/// repeats, and the gap between repeats, both in milliseconds. These are the values the
/// configuration reference defaults to.
pub(crate) const KEY_REPEAT_DELAY: i32 = 200;
/// How many logical pixels one wheel step scrolls. The backend states scroll in 1/120 of a step,
/// and `wl_pointer.axis` wants pixels beside the discrete step, so a client that ignores the
/// discrete value still moves by the distance a notch is worth.
const AXIS_PIXELS_PER_STEP: f64 = 15.0;
/// The gap, in milliseconds, between repeats once a key repeat has started.
pub(crate) const KEY_REPEAT_INTERVAL: i32 = 40;

/// How long the session must be free of input before a window may be presented. A focus
/// change while the user is typing moves their keystrokes to another window, so a present
/// waits for this much quiet, in milliseconds.
pub(crate) const PRESENT_QUIET_MS: u64 = 250;

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

/// A touch point the seat holds: the window it went down on and where it is now.
struct TouchPoint {
    surface: WlSurface,
    location: SmithayPoint<f64, Logical>,
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
        }
    }
}

/// One output the server presents on.
pub(crate) struct OutputEntry {
    /// The backend's identity for the output.
    pub(crate) backend: OutputId,
    /// The mode being driven, as the backend reported it.
    pub(crate) mode: gnoblin_output::DisplayMode,
    /// The scale the output composes at.
    pub(crate) scale: gnoblin_types::Scale,
    /// Whether this output carries the layout's primary logical monitor.
    pub(crate) primary: bool,
    /// The Smithay output, which owns the `wl_output` global.
    pub(crate) output: Output,
    /// The global the output created.
    global: GlobalId,
    /// The logical rectangle the layout assigned.
    rect: Rect,
}

/// The compositor state every protocol handler receives.
///
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
    /// The wl_seat globals the owner created as seat nodes, by node id. Each is removed when its
    /// node leaves the scene.
    pub(crate) extra_seats: std::collections::BTreeMap<gnoblin_types::NodeId, Seat<State>>,
    /// The popup tree for `xdg_popup` surfaces, so they are stacked over their parents.
    pub(crate) popups: PopupManager,
    /// The popup grabs the seats hold, so a window that takes the keyboard from a grabbed popup can end the grab.
    pub(crate) popup_grabs: Vec<crate::popup::HeldGrab>,
    /// The windows whose surface trees are to be built again, and the surfaces that committed since they were.
    pub(crate) surface_trees: crate::surface_tree::SurfaceTrees,
    /// Whether a popup committed since the last frame, so its frame callbacks are answered at the end of this
    /// iteration even when no frame was drawn.
    pub(crate) callbacks_due: bool,
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
    /// Kept alive so `zwp_virtual_keyboard_manager_v1` stays registered; the bind is gated by
    /// the access roster in [`crate::protocols::zwp_virtual_keyboard_v1`].
    #[allow(dead_code)]
    pub(crate) virtual_keyboard_global: GlobalId,

    /// The seat the keyboard and pointer belong to, which is what the text-input and
    /// shortcut-inhibit handles hang off.
    pub(crate) seat: Seat<State>,
    /// Keyboard and pointer handles for the one seat this slice creates.
    pub(crate) keyboard: KeyboardHandle<State>,
    /// The pointer handle of the seat, the target of pointer focus and motion.
    pub(crate) pointer: PointerHandle<State>,
    /// The touch handle of the seat, the target of touch focus and touch points.
    pub(crate) touch: TouchHandle<State>,
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
    /// The declared workspaces, one `id:name` item each, published on the main seat for the shell.
    pub(crate) workspace_names: Vec<String>,
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
    /// Which side owns each selection while the X11 bridge is running.
    pub(crate) selection_owners: crate::selection_bridge::SelectionOwners,
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
    /// The configured touchpad bindings and the gesture in flight.
    pub(crate) touchpad_gestures: crate::touchpad::TouchpadGestures,
    /// The input activity the idle state is derived from.
    pub(crate) activity: Activity,
    /// The liveness pings and the clients that have missed them.
    pub(crate) liveness: crate::liveness::Liveness,

    /// The record of windows and outputs, and the minimal policy applied while no shell holds the tree.
    ///
    /// Crate-visible because every protocol module turns a request into a call on it; while a shell holds
    /// the tree the record is kept current and its decisions are not applied.
    pub(crate) fallback: Fallback,
    /// The window management options, published on the main seat where the shell reads them.
    pub(crate) options: WmOptions,
    /// The window that holds focus: the keyboard's, or, while a layer surface or the lock holds the
    /// keyboard, the one it returns to.
    pub(crate) focus: Option<WindowId>,
    /// The protocol objects of the managed windows, keyed by policy identity.
    pub(crate) windows: BTreeMap<WindowId, WindowEntry>,
    /// The outputs, in connection order.
    pub(crate) outputs: Vec<OutputEntry>,
    /// The next window identity to hand out.
    next_window: u32,
    /// The clients an embedder adopted through [`State::adopt_client`], by the tag it was handed.
    adopted: HashMap<u64, Client>,
    /// The protection generation the frame being drawn was described under.
    ///
    /// Read when the scene is described and reported when the frame is presented, so
    /// a frame described before a cover was raised cannot prove it.
    pub(crate) protection_generation: Option<u64>,
    /// The last pointer position the backend reported, in global logical
    /// coordinates.
    pub(crate) pointer_position: SmithayPoint<f64, Logical>,
    /// Where the surface the pointer is over has its origin, as the pointer last learned it, so a surface that moves
    /// under a pointer that has not is noticed.
    pointer_origin: Option<SmithayPoint<f64, Logical>>,
    /// Whether a device has put the pointer anywhere yet. Until it has, the pointer is at the layout's corner by
    /// default and no surface is under it in any sense a client should hear of.
    pointer_placed: bool,
    /// The shell node the pointer ray is over, when the shell's tree picks one. Its edges are
    /// sent to the shell; no client holds the pointer while it is set.
    shell_hover: Option<NodeId>,
    /// The touch points the seat holds, by device slot: the window each went down on and
    /// where it is now.
    touch_points: HashMap<u32, TouchPoint>,
    /// The window an interactive move or resize is dragging, while the gesture lasts (`crate::interactive`).
    pub(crate) dragged: Option<WindowId>,

    /// The scene the frame path composes from: window nodes, stacking and damage.
    pub(crate) composer: Composer,
    /// The tree's queue, schema and publish hook. Its transactions apply at the start of each frame.
    pub(crate) tree: TreeFrame,
    /// The modal overlay, if one is set; it takes input from the clients while it stands.
    pub(crate) modal: Option<Modal>,
    /// The modal events queued since the last take, in the order they arrived.
    pub(crate) modal_events: Vec<ModalEvent>,
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
    /// The lock's password checks: the `authenticate` calls the shell makes are run here,
    /// off the loop, and settle the session's unlock (see [`crate::lock_auth`]).
    pub(crate) lock_auth: crate::lock_auth::LockAuth,

    /// The capture sessions, the pixels a capture copies from and the capture globals.
    /// It is one value so the compositor's own fields stay a short list, and so the whole
    /// of the capture state reads in one place (`protocols::capture_shared`).
    pub(crate) capture: CaptureTables,

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
        xwayland: Option<crate::xwayland::XwaylandSession>,
        observer: Option<Arc<dyn Observer>>,
        tree: TreeHooks,
        protocols: &Protocols,
        titlebar_actions: TitlebarActions,
        cursor: crate::server::CursorSettings,
        visual_bell: crate::server::VisualBellSettings,
        visual_accessibility: crate::server::VisualAccessibilitySettings,
        extensions: &[Arc<dyn Extension>],
        lock_auth: crate::lock_auth::LockAuth,
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
        let virtual_keyboard_global =
            crate::protocols::zwp_virtual_keyboard_v1::virtual_keyboard_global(&display_handle, &access);

        let mut seat_state = SeatState::new();
        let mut seat: Seat<Self> = seat_state.new_wl_seat(&display_handle, seat_name);
        let keyboard = seat
            .add_keyboard(Default::default(), KEY_REPEAT_DELAY, KEY_REPEAT_INTERVAL)
            .map_err(StartupError::Keyboard)?;
        let pointer = seat.add_pointer();
        let touch = seat.add_touch();

        let mut state = Self {
            display_handle,
            display: Some(display),
            compositor_state,
            xdg_shell_state,
            shm_state,
            seat_state,
            extra_seats: std::collections::BTreeMap::new(),
            popups: PopupManager::default(),
            popup_grabs: Vec::new(),
            surface_trees: crate::surface_tree::SurfaceTrees::default(),
            callbacks_due: false,
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
            virtual_keyboard_global,
            security_context_state,
            keyboard,
            pointer,
            touch,
            seat,
            access,
            layers: ZwlrLayerShellV1Layers::default(),
            foreign_toplevels: ForeignToplevels::default(),
            activation: XdgActivationV1::default(),
            titlebar_actions,
            workspace_names: Vec::new(),
            bells: 0,
            visual_bell,
            visual_accessibility,
            menu_requests: VecDeque::new(),
            menu_seq: 0,
            seat_name: seat_name.to_owned(),
            selections: WlDataDeviceManagerSelections::default(),
            selection_owners: crate::selection_bridge::SelectionOwners::default(),
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
            cursor_chain: crate::protocols::wp_cursor_shape_v1::cursor_chain(&cursor),
            cursor_size: cursor.size,
            tearing: Vec::new(),
            clock: Tick::ZERO,
            touchpad_gestures: crate::touchpad::TouchpadGestures::default(),
            activity: Activity::new(),
            liveness: crate::liveness::Liveness::new(std::time::Duration::from_secs(5)),
            fallback: Fallback::default(),
            options,
            focus: None,
            windows: BTreeMap::new(),
            outputs: Vec::new(),
            next_window: 1,
            adopted: HashMap::new(),
            protection_generation: None,
            pointer_position: SmithayPoint::from((0.0, 0.0)),
            pointer_origin: None,
            pointer_placed: false,
            shell_hover: None,
            touch_points: HashMap::new(),
            dragged: None,
            composer: Composer::with_painter(tree.painter.clone()),
            tree: TreeFrame::with_hooks(tree),
            modal: None,
            modal_events: Vec::new(),
            last_frame: None,
            lock: Lock::new(),
            lock_embargo: Embargo::RELEASED,
            lock_tables: ExtSessionLockV1Tables::default(),
            lock_auth,
            capture,
            session_host,
            observation: Arc::new(Mutex::new(SessionSnapshot::default())),
            observer,
        };

        // The main seat is the compositor's own seat in the scene, named as the wl_seat is.
        state.composer.add_main_seat(seat_name);
        state.publish_seat_policy();
        // The session lock's node: a shell that attaches while the session is locked reads its
        // state in the snapshot.
        state.composer.add_lock_node();
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

    /// The access roster, so a caller can build a connection's data from it.
    #[must_use]
    pub(crate) fn access(&self) -> Arc<Access> {
        Arc::clone(&self.access)
    }

    /// Applies one window-control request from the IPC. The answer is `Err(Gone)` for a closed
    /// window, so a script learns it asked too late rather than seeing a silent success, and
    /// `Err` with the reason when the change is refused. Close goes to the toplevel when the
    /// window has one and to the X11 policy otherwise; the three state changes are window
    /// requests, which the shell answers while it holds the tree. Move, resize and output are
    /// the record's while no shell holds the tree, and refused while one does: a shell places
    /// its own windows.
    pub(crate) fn window_control(&mut self, window: WindowId, change: WindowChange) -> Result<(), WindowChangeRefusal> {
        let Some(managed) = self.fallback.window(window) else {
            return Err(WindowChangeRefusal::Gone);
        };
        let current = self.geometry_of(window).unwrap_or(managed.geometry);
        match change {
            WindowChange::Close => {
                if self.close_window(window) {
                    Ok(())
                } else {
                    Err(WindowChangeRefusal::Gone)
                }
            }
            WindowChange::Minimised(minimised) => {
                self.window_request(window, WindowRequest::Minimise(minimised));
                Ok(())
            }
            WindowChange::Maximised(maximised) => {
                self.window_request(window, WindowRequest::Maximise(maximised));
                Ok(())
            }
            WindowChange::Fullscreen(fullscreen) => {
                self.window_request(window, WindowRequest::Fullscreen(fullscreen));
                Ok(())
            }
            WindowChange::MoveTo { x, y } => self.place_by_command(window, Rect { x, y, ..current }),
            WindowChange::Resize { width, height } => self.place_by_command(
                window,
                Rect {
                    width,
                    height,
                    ..current
                },
            ),
            // The compositor keeps one implicit workspace: naming it changes nothing, and any other is the
            // shell's, which moves a window with `Window:move_to`.
            WindowChange::Workspace(workspace) if workspace == IMPLICIT_WORKSPACE => Ok(()),
            WindowChange::Workspace(_) => Err(WindowChangeRefusal::UnknownTarget),
            WindowChange::Output(output) => self.move_to_output_by_command(window, output),
        }
    }

    /// Applies one shell visual request from the IPC. A closed window is `Gone`, and a locked
    /// session refuses the change, because the cover draws over the window and the value would
    /// show nowhere. A value outside its documented range is refused by the composer and the
    /// window keeps its drawing. The window's rule opacity and placement animation are separate
    /// and keep applying alongside these values.
    pub(crate) fn window_visual(&mut self, window: WindowId, change: WindowVisual) -> Result<(), WindowVisualRefusal> {
        if self.fallback.window(window).is_none() {
            return Err(WindowVisualRefusal::Gone);
        }
        if self.lock.state() != LockState::Unlocked {
            return Err(WindowVisualRefusal::Locked);
        }
        let applied = match change {
            WindowVisual::Opacity(opacity) => self.composer.set_shell_opacity(window, opacity as f32),
            WindowVisual::Scale(scale) => self.composer.set_shell_scale(window, scale as f32),
            WindowVisual::Offset { x, y } => self.composer.set_shell_offset(window, x, y),
            WindowVisual::Reset => {
                self.composer.reset_shell_visuals(window);
                Ok(())
            }
        };
        applied.map_err(|_| WindowVisualRefusal::OutOfRange)?;
        self.damage_outputs();
        Ok(())
    }

    /// Maps the record's reason for refusing a control request to the reply a caller sees. A window or
    /// target that has gone is named apart from a state refusal, so a script can tell that it lost a
    /// race from a change it can never make.
    pub(crate) const fn refusal_of(reason: FallbackRefusal) -> WindowChangeRefusal {
        match reason {
            FallbackRefusal::Gone => WindowChangeRefusal::Gone,
            FallbackRefusal::UnknownOutput => WindowChangeRefusal::UnknownTarget,
            FallbackRefusal::NotFloating => WindowChangeRefusal::NotFloating,
            FallbackRefusal::FollowsParent => WindowChangeRefusal::TransientFollowsParent,
            FallbackRefusal::Duplicate | FallbackRefusal::TooMany | FallbackRefusal::NoOutput => {
                WindowChangeRefusal::Refused
            }
        }
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
    /// The composition root records the promise and then starts the hosted session; the
    /// accept path in [`State::insert_client`] picks it up.
    pub fn grant_role(&self, pid: u32, role: ClientRole) {
        self.access.promise(pid, role);
    }

    /// Adopts a connected socket as an ordinary application and returns the tag that names it.
    ///
    /// The embedder that made both ends of the pair (the wlcs integration, which hands the other end to a conformance
    /// client) cannot tell its clients apart by process, because they all share one. The tag is what it names a client
    /// by afterwards, in [`State::place_surface`]. The role is the unprivileged socket's, as a connection to the
    /// public socket would have.
    ///
    /// # Errors
    ///
    /// The display's own error when it cannot take the stream.
    pub(crate) fn adopt_client(&mut self, stream: std::os::unix::net::UnixStream) -> std::io::Result<u64> {
        let client = self.insert_client(
            stream,
            Arc::new(crate::protocols::wl_compositor::WlCompositorClientState::new(
                &self.access,
            )),
            ClientRole::Unprivileged,
        )?;
        let tag = u64::try_from(self.adopted.len()).unwrap_or(u64::MAX).saturating_add(1);
        self.adopted.insert(tag, client);
        Ok(tag)
    }

    /// Puts the window whose surface a client names by its protocol object id at an exact position, keeping its size.
    ///
    /// The position is not clamped to the work area: the embedder places windows deliberately, which is what the
    /// record's [`Fallback::place`] is for. `false` when the tag names no client, the id names no surface of it, the
    /// surface is no managed window, or the record refuses (a maximised or fullscreen window is not placed).
    pub(crate) fn place_surface(&mut self, tag: u64, surface: u32, at: gnoblin_types::Point) -> bool {
        let Some(client) = self.adopted.get(&tag) else {
            return false;
        };
        let Ok(surface) = client.object_from_protocol_id::<WlSurface>(&self.display_handle, surface) else {
            return false;
        };
        let Some(window) = self.window_of(&surface) else {
            return false;
        };
        let Ok(effects) = self.fallback.place(window, at) else {
            return false;
        };
        self.apply(effects);
        self.publish();
        true
    }

    /// Adopts a connected socket the guardian made as a client with the `view` role.
    ///
    /// The role is the connection's own, recorded before its first request, so the client's
    /// registry is the capture globals and nothing else. It is not looked up by process: the
    /// guardian made both ends of the pair, so the peer credentials of the end this server holds
    /// name the guardian and no promise by pid could tell the extension apart.
    pub(crate) fn adopt_view(&mut self, stream: std::os::unix::net::UnixStream) -> std::io::Result<Client> {
        let client = self.insert_client(
            stream,
            Arc::new(crate::protocols::wl_compositor::WlCompositorClientState::new(
                &self.access,
            )),
            ClientRole::View,
        )?;
        log::debug!(area = "access", client = ?client.id(), "a view connection joined");
        Ok(client)
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

    /// The scale the output a surface is on draws it at.
    ///
    /// A surface with no output of its own takes the first output's scale, and a session
    /// with no output at all answers 1: a client that is told nothing renders at the
    /// smallest scale rather than at a scale nothing draws.
    #[must_use]
    pub(crate) fn surface_scale(&self, surface: &WlSurface) -> f64 {
        let output_id = self
            .window_of(surface)
            .and_then(|window| self.fallback.window(window))
            .and_then(|window| window.output);
        let entry = output_id
            .and_then(|output| self.outputs.iter().find(|entry| entry.backend == output))
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
        pointer.frame(self);
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
        let mut surfaces = Vec::new();
        for window in self.composer.window_stack() {
            let Some(entry) = self.windows.get(&window) else {
                continue;
            };
            if !entry.mapped || !self.window_shown(window) {
                continue;
            }
            let Some(geometry) = self.geometry_of(window) else {
                continue;
            };
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
        self.cancel_touches();
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
                Some(index) => self.refresh_output(index, output, monitor.rect),
                None => self.add_output(output, monitor.rect),
            }
        }
        for entry in &mut self.outputs {
            entry.primary = layout.primary == Some(entry.backend);
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
            let effects = self.with_seat(|fallback, seat| fallback.output_removed(entry.backend, seat));
            self.apply_decided(effects);
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

    /// Creates one output's Smithay object and global, and records it.
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

        self.composer.set_output(output.id, rect);
        log::debug!(area = "output", connector = %output.connector, ?rect, "connected");
        self.outputs.push(OutputEntry {
            backend: output.id,
            output: smithay_output,
            global,
            mode: output.mode,
            scale: output.scale,
            primary: false,
            rect,
        });
        let effects = self.with_seat(|fallback, seat| fallback.output_added(output.id, rect, seat));
        self.apply_decided(effects);
    }

    /// Brings a connected output's mode and scale up to what the backend now reports, then
    /// applies its layout rectangle. The `wl_output` state is updated here, not only in
    /// `add_output`, so a client hears a scale change without a restart.
    fn refresh_output(&mut self, index: usize, output: &gnoblin_backend::Output, rect: Rect) {
        let Some(entry) = self.outputs.get_mut(index) else {
            return;
        };
        entry.mode = output.mode;
        entry.scale = output.scale;
        entry.output.change_current_state(
            Some(smithay_mode(output)),
            None,
            Some(Self::smithay_scale(output.scale)),
            None,
        );
        self.move_output(index, rect);
    }

    /// Applies a new layout rectangle to an output that is already connected.
    fn move_output(&mut self, index: usize, rect: Rect) {
        let Some(entry) = self.outputs.get_mut(index) else {
            return;
        };
        if entry.rect == rect {
            return;
        }
        entry.rect = rect;
        entry
            .output
            .change_current_state(None, None, None, Some(SmithayPoint::from((rect.x, rect.y))));
        let backend = entry.backend;
        self.composer.set_output(backend, rect);
        let effects = self.fallback.output_moved(backend, rect);
        self.apply_decided(effects);
    }

    /// Applies the decisions the record returned.
    ///
    /// This is the only place a decision becomes observable behaviour, so every arm says what it means to a
    /// client.
    pub(crate) fn apply(&mut self, effects: Vec<FallbackEffect>) {
        for effect in effects {
            match effect {
                FallbackEffect::Configure {
                    window,
                    geometry,
                    state,
                } => {
                    self.composer.place_window(window, geometry);
                    self.configure(window, geometry, state);
                    self.reposition_reactive_popups(window);
                }
                FallbackEffect::Visible { window, visible } => {
                    self.composer.set_window_visible(window, visible);
                    self.set_visible(window, visible);
                }
                FallbackEffect::Stack(order) => {
                    // The record states the order topmost first; the scene keeps its paint order bottom first.
                    self.composer.set_stacking(&order);
                    log::debug!(log = ?order, "[stack]");
                }
                FallbackEffect::Focus(focused) => self.move_focus(focused),
            }
        }
        self.refresh_pointer_focus();
    }

    /// Gives the pointer the surface it is over now, when that is not the surface it holds.
    ///
    /// The pointer stays where it is while a window moves, resizes, maps or closes under it, and a client can only
    /// learn of that from the compositor: `enter` and `leave` when the surface under it changes, and a `motion` when
    /// the same surface has moved, so that its coordinates stay true. A pointer a button holds keeps its focus until the
    /// button is released, as a pointer grab does.
    pub(crate) fn refresh_pointer_focus(&mut self) {
        // A gesture's grab holds the pointer's lock while it moves the window, and the pointer is not the client's then.
        if !self.pointer_placed || self.dragged.is_some() || self.pointer.is_grabbed() {
            return;
        }
        let under = self.surface_under(self.pointer_position);
        let held = self.pointer.current_focus();
        let same = match (&under, &held) {
            (None, None) => true,
            (Some((surface, origin)), Some(focus)) => {
                surface == focus
                    && self.pointer_origin.is_some_and(|held_origin| {
                        (origin.x - held_origin.x).abs() < 0.5 && (origin.y - held_origin.y).abs() < 0.5
                    })
            }
            _ => false,
        };
        if !same {
            self.move_pointer(self.pointer_position, InputTime::now());
        }
    }

    /// Applies the decisions of the no-shell fallback. While a shell holds the tree it decides placement,
    /// stacking and focus, so only the window's presence on the outputs is kept: the record has otherwise only been
    /// kept current.
    pub(crate) fn apply_decided(&mut self, effects: Vec<FallbackEffect>) {
        self.apply_filtered(effects, |effect| matches!(effect, FallbackEffect::Visible { .. }));
    }

    /// Applies `effects` in full when no shell holds the tree, and only those `shell_keeps` names while one does.
    fn apply_filtered(&mut self, effects: Vec<FallbackEffect>, shell_keeps: impl Fn(&FallbackEffect) -> bool) {
        if self.composer.shell_places() {
            let (kept, dropped): (Vec<FallbackEffect>, Vec<FallbackEffect>) =
                effects.into_iter().partition(|effect| shell_keeps(effect));
            log::trace!(
                area = "wm",
                dropped = dropped.len(),
                "a shell holds the tree; its decisions stand"
            );
            self.apply(kept);
            return;
        }
        self.apply(effects);
    }

    /// Forgets a closed window: the record drops it. Focus moves on when it held it, as
    /// the record decides without a shell, and is cleared while one holds the tree: the shell moves its own focus.
    pub(crate) fn window_closed(&mut self, window: WindowId) {
        let effects = self.with_seat(|fallback, seat| fallback.close(window, seat));
        if self.composer.shell_places() {
            if self.focus == Some(window) {
                self.focus = None;
            }
        } else {
            self.apply(effects);
        }
    }

    /// Runs a decision of the record with the paint order and the focus it starts from, which the scene and the
    /// keyboard own.
    pub(crate) fn with_seat<R>(&mut self, decide: impl FnOnce(&mut Fallback, Standing<'_>) -> R) -> R {
        let stack = self.composer.window_stack();
        let seat = Standing {
            stack: &stack,
            focus: self.focus,
        };
        decide(&mut self.fallback, seat)
    }

    /// Applies a configure to a toplevel and sends it if the client is ready for it.
    ///
    /// The state is written to the surface's pending state either way; a toplevel that has not had its initial
    /// configure yet carries this state into that configure, which the client's first commit triggers. A window the
    /// record places is untiled and has no work area to fit, so its client is sent no bounds.
    fn configure(&mut self, window: WindowId, geometry: Rect, state: WindowState) {
        let state = self.effective_state(window, state);
        let Some(entry) = self.windows.get(&window) else {
            // The window closed between the decision and this action.
            return;
        };
        let surface = &entry.surface;
        match surface {
            Managed::Wayland(surface) => {
                // A floating window that has had no configure yet is suggested no size (0 by 0): the client picks its
                // own, and the record adopts it at the commit that maps the window. A window that is maximised or
                // fullscreen, even before that, is sent the size the policy decided.
                let sized = state.maximised || state.fullscreen || surface.is_initial_configure_sent();
                surface.with_pending_state(|pending| {
                    pending.bounds = None;
                    pending.size = sized.then(|| SmithaySize::from((geometry.width as i32, geometry.height as i32)));
                    set_flag(&mut pending.states, xdg_toplevel::State::Maximized, state.maximised);
                    set_flag(&mut pending.states, xdg_toplevel::State::Fullscreen, state.fullscreen);
                    for tiled in [
                        xdg_toplevel::State::TiledLeft,
                        xdg_toplevel::State::TiledRight,
                        xdg_toplevel::State::TiledTop,
                        xdg_toplevel::State::TiledBottom,
                    ] {
                        set_flag(&mut pending.states, tiled, false);
                    }
                });
                // Only a changed state is sent: a move changes none of what a client is told, and a client does not
                // need the same configure again.
                if surface.is_initial_configure_sent() {
                    surface.send_pending_configure();
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
        // The state follows the geometry on the X connection as well: `_NET_WM_STATE` is how an X11 client learns it
        // is maximised or fullscreen. `_NET_WM_STATE_HIDDEN` is `set_visible`'s.
        self.sync_x11_flags(window, |flags| X11Flags {
            maximised: state.maximised,
            fullscreen: state.fullscreen,
            ..flags
        });
        log::debug!(area = "wm", %window, width = geometry.width, height = geometry.height, "configured");
    }

    /// Shows or hides a window's surface, and tells a Wayland toplevel whether it is suspended.
    ///
    /// A hidden window is minimised or sits on another workspace, so the client is told it is
    /// `suspended` and may stop drawing it. That is the same signal that drives output enter and
    /// leave, so the two never disagree.
    pub(crate) fn set_visible(&mut self, window: WindowId, visible: bool) {
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
        // An X11 window is told the same thing as `_NET_WM_STATE_HIDDEN`, which is how it learns it may stop drawing.
        self.sync_x11_flags(window, |flags| X11Flags {
            hidden: !visible,
            ..flags
        });
    }

    /// Moves keyboard focus to a window, or to nothing.
    ///
    /// The window is recorded as the focus even while nothing moves: while the session is covered the lock's
    /// gate routes every key to the owner's lock surface or to nothing, and the recorded window is what the
    /// reveal restores.
    fn move_focus(&mut self, focused: Option<WindowId>) {
        self.focus = focused;
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

    /// Tells every Wayland toplevel whether it holds the keyboard: `xdg_toplevel.state.activated` is how a client
    /// learns to draw itself focused, and the only one of the window states the focus decides.
    ///
    /// A toplevel whose state changes is sent a configure with the new state, as the focus-follows rule does for
    /// every other state; one that has not had its initial configure yet carries the state into that configure.
    fn announce_activation(&self, focused: Option<WindowId>) {
        for (window, entry) in &self.windows {
            let Managed::Wayland(surface) = &entry.surface else {
                continue;
            };
            let activated = Some(*window) == focused;
            let changed = surface.with_pending_state(|pending| {
                let was = pending.states.contains(xdg_toplevel::State::Activated);
                set_flag(&mut pending.states, xdg_toplevel::State::Activated, activated);
                was != activated
            });
            if changed && surface.is_initial_configure_sent() {
                surface.send_configure();
            }
        }
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
        // A popup grab belongs to the window it hangs from: a window that takes the keyboard from it ends the grab,
        // and the popups are told before the new window hears it has focus.
        self.dismiss_popup_grabs_for_focus(target.as_ref());
        let focused = target.as_ref().and_then(|surface| self.window_of(surface));
        // A surface, even one that is no window, replaces the shell's node as the seat focus.
        if target.is_some() {
            self.composer.release_shell_focus();
        }
        self.composer.set_main_seat_focus(focused);
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
        self.announce_activation(focused);
        let windows: Vec<WindowId> = self.fallback.windows().map(|window| window.id).collect();
        for window in windows {
            self.publish_window_state(window);
        }
        self.set_selection_focus(client.as_ref());
        self.restore_layout(target.as_ref());
    }

    /// Moves the pointer to a layout position and gives it the focus it lands on.
    ///
    /// The serial is remembered because a client names it in a grab or an activation
    /// request, which is the same rule a button follows.
    fn move_pointer(&mut self, position: SmithayPoint<f64, Logical>, time: InputTime) {
        self.pointer_position = position;
        // The magnifier centres on the pointer, so a moved pointer changes every magnified
        // output. Each output is repainted rather than waiting for other damage to reach it.
        if self.visual_accessibility.magnifier_zoom.is_some() {
            self.damage_outputs();
        }
        self.composer.set_main_seat_pointer(position.x, position.y);
        self.shell_pointer(self.pointer_target(position));
        let focus = self.surface_under(position);
        self.pointer_origin = focus.as_ref().map(|(_, origin)| *origin);
        self.pointer_placed = true;
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
        // The enter, leave and motion of one move are a group: a client that bound `wl_pointer` at version 5 or
        // later (GTK and Qt both do) applies none of them before the frame that closes it.
        pointer.frame(self);
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
        pointer.frame(self);
    }

    /// The surface of the window the window manager has focused.
    pub(crate) fn focused_window_surface(&self) -> Option<WlSurface> {
        let window = self.focus?;
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
            },
        );
        // The window is placed now and takes focus when it first maps; the activation is decided from
        // `toplevel_committed` for that reason.
        let opened = self.with_seat(|fallback, seat| {
            let open = Open {
                id: window,
                kind: WindowKind::Normal,
                transient_for: None,
                requested: None,
            };
            fallback.open(open, seat)
        });
        // The scene learns the window before the decision is applied, so the initial configure's geometry has a
        // window to place. A window the record refused has no geometry: it is inserted at the empty rectangle and
        // stays unmanaged.
        let geometry = self
            .fallback
            .window(window)
            .map_or(Rect::EMPTY, |managed| managed.geometry);
        self.composer.insert_window(window, geometry);
        match opened {
            Ok(effects) => self.apply_decided(effects),
            Err(refusal) => log::debug!(area = "window", %window, ?refusal, "the window is left unmanaged"),
        }
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
                // A touch point on the window cannot outlive it: its client is told the point lifted.
                self.end_touches_on(&wl_surface);
                log::debug!(area = "window", %window, surface = ?wl_surface.id(), "closed");
            }
            // The window fades out from the picture its buffer holds now, before the
            // client can free that buffer.
            self.composer.close_window(window);
        }
        self.release_window(window);
        self.window_closed(window);
        self.publish();
    }

    /// What a window's closing ends, whatever protocol it arrived on: the layout a window
    /// remembered goes with it, so a recycled identity cannot inherit another window's source;
    /// the sessions that captured it end, so nothing serves the pixels of a window that is no
    /// longer on screen and each client hears `stopped`.
    pub(crate) fn release_window(&mut self, window: WindowId) {
        self.forget_layout(window);
        self.capture.window_closed(window);
    }

    /// A toplevel committed: record its title, application id and buffer.
    ///
    /// An X11 window's `wl_surface` also reaches this commit hook, because the compositor
    /// hook runs for every surface; it has no `xdg_toplevel` attributes, so its commit takes
    /// [`State::x11_committed`] instead.
    pub(crate) fn toplevel_committed(&mut self, surface: &WlSurface) {
        let Some(window) = self.window_of(surface) else {
            return;
        };
        if self
            .windows
            .get(&window)
            .is_some_and(|entry| entry.surface.toplevel().is_none())
        {
            self.x11_committed(window, surface);
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
        // The node's read-only texts follow the client's title and app id: each change is a
        // tree `set` at the next frame, and an unchanged pair publishes nothing.
        self.composer.set_window_text(window, &title, &app_id);
        self.publish_window_state(window);
        let mut newly_mapped = false;
        let mut initial_configure = None;
        if let Some(entry) = self.windows.get_mut(&window) {
            entry.app_id = app_id;
            entry.title = title;
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
            // A new window takes focus only as `window_management.focus_new_windows` allows. With no shell the
            // record decides; a shell decides in its own `Window:on_ready`.
            let policy = self.options.new_window_focus;
            let effects = self.with_seat(|fallback, seat| {
                if fallback.may_take_focus(window, policy, seat) {
                    fallback.activate(window, true, seat)
                } else {
                    Vec::new()
                }
            });
            self.apply_decided(effects);
        }
        // Before the content, so a window's node is created already where its geometry puts it.
        self.follow_window_geometry(window, surface);
        self.sync_content(window, surface);
        self.publish();
        // The commit may have moved a subsurface, resized the surface or mapped it under a pointer that has not moved.
        self.refresh_pointer_focus();
    }

    /// Places and sizes a toplevel by the window geometry its client committed.
    ///
    /// The window geometry is the part of the surface the user sees as the window, without the shadow or border a client
    /// draws around it. The record's rectangle is that geometry, so the surface is drawn at the rectangle's origin less the
    /// geometry's offset into the surface: a window placed at a point has its geometry there, and a point inside it
    /// is the same point of the geometry whatever the shadow's width. A client that set no geometry has the whole
    /// surface as its window.
    ///
    /// While a floating window has no shell and is not being resized, the record also takes its size: the initial configure
    /// suggested none, so the client chose.
    fn follow_window_geometry(&mut self, window: WindowId, surface: &WlSurface) {
        let geometry = xdg_geometry(surface);
        let origin = geometry.map_or((0, 0), |geometry| (geometry.loc.x, geometry.loc.y));
        self.composer.set_window_surface_offset(window, origin.0, origin.1);
        // A subsurface can reach past the surface it hangs from, and the pointer and touch reach it there too.
        let tree = bbox_from_surface_tree(surface, (0, 0));
        let extent = (tree.size.w > 0 && tree.size.h > 0)
            .then(|| Rect::new(tree.loc.x, tree.loc.y, tree.size.w as u32, tree.size.h as u32));
        self.composer.set_window_hit_extent(window, extent);
        if self.composer.shell_places() || self.dragged == Some(window) || !has_buffer(surface) {
            return;
        }
        let size = geometry
            .map(|geometry| (geometry.size.w, geometry.size.h))
            .or_else(|| {
                committed_content(surface).map(|(_, content)| (content.size.width as i32, content.size.height as i32))
            })
            .and_then(|(width, height)| Some((u32::try_from(width).ok()?, u32::try_from(height).ok()?)))
            .filter(|&(width, height)| width > 0 && height > 0);
        let Some((width, height)) = size else {
            return;
        };
        if self
            .fallback
            .adopt_size(window, gnoblin_types::Size::new(width, height))
            && let Some(managed) = self.fallback.window(window)
        {
            self.composer.place_window(window, managed.geometry);
        }
    }

    /// Records what a window's client last committed into the scene.
    ///
    /// The buffer and its view come from Smithay's renderer surface state rather than
    /// from `SurfaceAttributes`: `on_commit_buffer_handler` takes the buffer out of the
    /// attributes when it applies a commit, so the renderer's copy is the only one that
    /// still knows what the client drew.
    pub(crate) fn sync_content(&mut self, window: WindowId, surface: &WlSurface) {
        let factor = crate::protocols::wp_alpha_modifier_v1::committed_factor(surface);
        self.composer.set_window_surface_alpha(window, factor);
        match committed_content(surface) {
            Some((buffer, content)) => {
                self.composer.set_window_content(window, Some(content));
                // The scene holds the handle; Smithay keeps the object alive for as long as a
                // frame can read it.
                self.composer.set_window_buffer(window, Some(buffer));
            }
            None => {
                // An unmapped window fades its last picture out, or leaves at once when it has
                // none to show. The composer keeps it either way, for a later commit to reuse.
                self.composer.unmap_window(window);
            }
        }
    }

    /// The workspaces the object graph publishes: the one the compositor keeps. The shell's workspaces are
    /// nodes of its own tree, and the shell owns them.
    fn observed_workspaces() -> Vec<crate::observation::ObservedWorkspace> {
        vec![crate::observation::ObservedWorkspace { id: IMPLICIT_WORKSPACE }]
    }

    /// Every output with the properties the `output` object publishes.
    ///
    /// The backend reports no EDID, so `make` and `model` stay empty and `transform` is
    /// normal: the headless and DRM paths do not rotate yet, and nothing here guesses.
    fn observed_outputs(&self) -> Vec<crate::observation::ObservedOutput> {
        self.outputs
            .iter()
            .map(|entry| {
                let name = entry.output.name();
                let mode = observed_mode(entry.mode);
                crate::observation::ObservedOutput {
                    id: entry.backend,
                    make: String::new(),
                    model: String::new(),
                    modes: vec![mode.clone()],
                    mode,
                    scale: entry.scale,
                    transform: 0,
                    primary: entry.primary,
                    enabled: true,
                    rect: entry.rect,
                    name,
                }
            })
            .collect()
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

    /// The matrix a window's node is drawn with and its projected bounds, or `None` when it
    /// is not drawn.
    pub(crate) fn drawn_node(&self, window: WindowId) -> Option<(Mat4, Rect)> {
        Some((
            self.composer.window_matrix(window)?,
            self.composer.window_projected_bounds(window)?,
        ))
    }

    /// Publishes the window management policy on the main seat, where the stock shell reads
    /// it. Startup and every reload call this, so the shell always reads the live config.
    pub(crate) fn publish_seat_policy(&mut self) {
        let options = self.options;
        let titlebar = self.titlebar_actions;
        let policy = MainSeatPolicy {
            edge_tiling: options.edge_tiling,
            drag_constraint: options.drag_constraint.name(),
            focus_mode: options.focus_mode.name(),
            focus_new_windows: options.new_window_focus.name(),
            raise_on_click: options.raise_on_click,
            auto_raise: options.auto_raise,
            auto_raise_delay: i64::try_from(options.auto_raise_delay).unwrap_or(i64::MAX),
            focus_change_on_pointer_rest: options.focus_change_on_pointer_rest,
            double_click: Self::titlebar_action_name(titlebar.double_click),
            middle_click: Self::titlebar_action_name(titlebar.middle_click),
            right_click: Self::titlebar_action_name(titlebar.right_click),
            workspaces: self.workspace_names.clone(),
        };
        self.composer.publish_main_seat_policy(&policy);
    }

    /// The spelling the stock shell matches on for a titlebar gesture.
    fn titlebar_action_name(action: TitlebarAction) -> &'static str {
        match action {
            TitlebarAction::ToggleMaximize => "toggle-maximize",
            TitlebarAction::ToggleMaximizeHorizontally => "toggle-maximize-horizontally",
            TitlebarAction::ToggleMaximizeVertically => "toggle-maximize-vertically",
            TitlebarAction::Minimize => "minimize",
            TitlebarAction::Lower => "lower",
            TitlebarAction::Menu => "menu",
            TitlebarAction::None => "none",
        }
    }

    /// Sets whether property changes land at once, see [`Composer::set_reduced_motion`], and
    /// republishes the main seat's `reduced_motion` for the stock shell to read.
    pub(crate) fn set_reduced_motion(&mut self, reduced: bool) {
        self.composer.set_reduced_motion(reduced);
        self.publish_seat_policy();
    }

    /// Applies the accepted window management options and titlebar gestures, then republishes
    /// the seat policy the stock shell reads.
    pub(crate) fn apply_window_management(&mut self, options: WmOptions, titlebar: TitlebarActions) {
        self.options = options;
        self.titlebar_actions = titlebar;
        self.publish_seat_policy();
    }

    /// Advances the window animations by the real time one loop turn took.
    pub(crate) fn advance_motion(&mut self, elapsed_ms: u32) {
        self.composer.advance_motion(elapsed_ms);
    }

    /// Advances the session clock.
    pub(crate) fn tick_clock(&mut self, milliseconds: u64) {
        self.advance_clock(milliseconds);
    }

    /// Feeds a window's role to the record when it changed.
    ///
    /// The parent a client names through `xdg-foreign` or `set_parent`, and the dialog
    /// hint `xdg-dialog` sets, are both kept by smithay on the toplevel's role. The
    /// record learns them only when they differ from what it holds, so an unchanged
    /// commit does nothing.
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
        let Some(current) = self.fallback.window(window) else {
            return;
        };
        if current.kind == kind && current.transient_for == transient_for {
            return;
        }
        let effects = self.with_seat(|fallback, seat| fallback.set_role(window, kind, transient_for, seat));
        self.apply_decided(effects);
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

    /// Routes a client's window request to the shell as an event on the window's node, and
    /// returns true, when the shell holds the tree and the session is not locked: the shell is
    /// the policy authority, so the window manager does not act on the request. Otherwise
    /// returns false and the request takes its usual path.
    pub(crate) fn request_to_shell(&mut self, surface: &WlSurface, request: TreeRequest) -> bool {
        if self.lock.state() != LockState::Unlocked || !self.composer.shell_places() {
            return false;
        }
        let Some(node) = self
            .window_of(surface)
            .and_then(|window| self.composer.window_node(window))
        else {
            return false;
        };
        self.tree.event(node, request);
        true
    }

    /// Records a window's app id that changed from a path that is not the toplevel's own commit, and republishes
    /// the node's texts, so the shell's rules read the new one.
    pub(crate) fn window_app_id_set(&mut self, window: WindowId, app_id: &str) -> bool {
        if self.fallback.window(window).is_none() {
            return false;
        }
        let Some(entry) = self.windows.get_mut(&window) else {
            return false;
        };
        entry.app_id = app_id.to_owned();
        let title = entry.title.clone();
        self.composer.set_window_text(window, &title, app_id);
        self.publish();
        true
    }

    /// Publishes a window's focus and window-manager state to its node in the tree. Focus is
    /// read from the keyboard, so the caller only names the window. While a shell holds the tree
    /// the node's own `fullscreen`, `maximised` and `minimised` are published back unchanged: they are
    /// the shell's to write.
    pub(crate) fn publish_window_state(&mut self, window: WindowId) {
        let Some(state) = self.window_flags(window) else {
            return;
        };
        let focused = self.keyboard_focus_window() == Some(window);
        self.composer
            .set_window_state(window, [focused, state.fullscreen, state.maximised, state.minimised]);
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

    /// The rectangle a window is drawn at, for popup positioning and the observers: where the shell that holds the
    /// tree put it, otherwise the record's. `None` when the window closed.
    pub(crate) fn geometry_of(&self, window: WindowId) -> Option<Rect> {
        let managed = self.fallback.window(window)?;
        Some(if self.composer.shell_places() {
            self.composer.window_frame(window).unwrap_or(managed.geometry)
        } else {
            managed.geometry
        })
    }

    /// Asks for a window's menu at an anchor, on behalf of a source, and publishes it.
    ///
    /// The request is numbered and kept in a bounded log, so a reader that publishes
    /// each request once can see it even after another arrives first. A window that
    /// has closed gets no request: there is no menu to open for it.
    pub(crate) fn request_window_menu(&mut self, window: WindowId, anchor: MenuAnchor, source: MenuSource) -> bool {
        if self.fallback.window(window).is_none() {
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

    /// The monitor rectangle of the output the window is on.
    pub(crate) fn monitor_of(&self, window: WindowId) -> Option<Rect> {
        let output = self.fallback.window(window)?.output?;
        Some(self.fallback.output(output)?.rect)
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
        // A covered session shows no window, so no window gets a callback: a client that
        // cannot see its frame must not render one. The owner's lock surfaces are the
        // only thing on screen while covered, so they always get theirs.
        if self.lock.state() == LockState::Unlocked {
            // A window is on screen if a monitor shows it, or a visible View shows its node.
            // The View set is computed once for the whole pass.
            let shown = self.composer.view_shown_nodes();
            for (window, entry) in &self.windows {
                // A window with no buffer has nothing on screen to have drawn, and Smithay's
                // own helper refuses those for the same reason.
                if !entry.mapped {
                    continue;
                }
                let on_monitor = self
                    .fallback
                    .window(*window)
                    .is_some_and(|managed| managed.output.is_some());
                let in_view = self
                    .composer
                    .window_node(*window)
                    .is_some_and(|node| shown.contains(&node));
                if !on_monitor && !in_view {
                    continue;
                }
                let Some(surface) = entry.surface.wl_surface() else {
                    continue;
                };
                Self::done_frame_callbacks(&surface, time);
            }
        }
        for surface in self.owner_mapped_lock_surfaces() {
            Self::done_frame_callbacks(&surface, time);
        }
    }

    /// Answers every frame callback queued on the surface, on its subsurfaces and on its popups, which is what tells a
    /// client its frame was presented and it may draw the next one.
    ///
    /// The callbacks of a subsurface or a popup are the client's to wait for exactly as a toplevel's are: they are
    /// part of the same frame, and a client that paces a subsurface or a menu by them would stall for ever on a
    /// compositor that answered the root alone.
    fn done_frame_callbacks(surface: &WlSurface, time: u32) {
        with_surface_tree_downward(
            surface,
            (),
            |_, _, ()| TraversalAction::DoChildren(()),
            |_, states, ()| {
                for callback in states
                    .cached_state
                    .get::<SurfaceAttributes>()
                    .current()
                    .frame_callbacks
                    .drain(..)
                {
                    callback.done(time);
                }
            },
            |_, _, ()| true,
        );
        for (popup, _) in PopupManager::popups_for_surface(surface) {
            Self::done_frame_callbacks(popup.wl_surface(), time);
        }
    }

    /// Ends the iteration the frame hook ran, dropping the damage it consumed.
    pub(crate) fn end_frame(&mut self) {
        self.composer.end_frame();
    }

    /// Whether a popup committed since this was last asked, which is when its client's frame callbacks are due even
    /// if nothing was drawn. The flag is cleared.
    pub(crate) fn take_callbacks_due(&mut self) -> bool {
        std::mem::take(&mut self.callbacks_due)
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

    /// Replaces the magnifier and colour filter in force and repaints every output, so the
    /// next frame shows the change. A settings value the frame rejects is drawn unfiltered, as
    /// at startup.
    pub(crate) fn apply_visual_accessibility_settings(&mut self, settings: crate::server::VisualAccessibilitySettings) {
        self.visual_accessibility = settings;
        self.damage_outputs();
    }

    /// Replaces the visual bell. The next rung bell flashes by these settings, so no
    /// restart is needed.
    pub(crate) fn apply_visual_bell_settings(&mut self, settings: crate::server::VisualBellSettings) {
        self.visual_bell = settings;
    }

    /// Queues the modal events an input edge becomes while a modal is set. Returns
    /// `false` for an edge that is not a modal event, which then routes as usual.
    pub(crate) fn route_to_modal(&mut self, event: &InputEvent) -> bool {
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
            | InputEvent::Touch { .. }
            | InputEvent::Touchpad(_) => {}
            InputEvent::DeviceAdded { .. } | InputEvent::DeviceRemoved { .. } => return false,
        }
        true
    }

    /// Sets or clears the modal and damages the outputs it covers, so the change is drawn.
    ///
    /// A modal cannot be set while the session is locked: the lock cover owns every
    /// output and every key, so a modal would be drawn over the cover or take the
    /// locker's keys. Clearing is always allowed. A modal stored before the lock is kept
    /// but is not drawn or routed while locked (see [`Self::active_modal`]).
    pub(crate) fn set_modal(&mut self, modal: Option<Modal>) {
        if modal.is_some() && self.lock.state() != LockState::Unlocked {
            log::warn!(area = "modal", "refused a modal set while the session is locked");
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

    /// The modal that is drawn and takes input now, or `None` while the session is locked.
    ///
    /// The lock gate runs before any client sees input, so a modal that is still stored
    /// during a lock must not be drawn over the cover or take the locker's keys. It is
    /// not cleared, so it reappears once the session unlocks.
    pub(crate) fn active_modal(&self) -> Option<&Modal> {
        if self.lock.state() == LockState::Unlocked {
            self.modal.as_ref()
        } else {
            None
        }
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

    /// Moves a window through the record, as a drag would.
    ///
    /// It is the seam a test and the parity harness drive window movement with until the input layer produces
    /// drag rectangles of its own: the request takes the same path a control request's does, including the
    /// work-area constraint and the configure the client receives. It refuses nothing a shell could place: a
    /// shell holding the tree is the one that places windows, and the seam is not for it.
    pub(crate) fn move_window(&mut self, window: WindowId, geometry: Rect) {
        if let Ok(effects) = self.fallback.move_resize(window, geometry) {
            self.apply(effects);
        }
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
            crate::server::VisualBellScope::Frame => match self.focus {
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
        // A covered window keeps the recorded focus so the reveal can restore it, but it holds no keyboard
        // focus: the lock gate routes keys to the lock surface or to nothing, so the snapshot says nothing is
        // focused.
        let focused = if self.lock.state() == LockState::Unlocked {
            self.focus
        } else {
            None
        };
        // The published lists follow the record, so a shell and a test read the same answer as the
        // compositor's own.
        self.sync_authority_lock();
        self.sync_foreign_toplevels();
        let workspaces = Self::observed_workspaces();
        let output_details = self.observed_outputs();
        let mut snapshot = SessionSnapshot {
            outputs: self.outputs.iter().map(|entry| entry.backend).collect(),
            stack: self.composer.window_stack(),
            workspaces,
            output_details,
            output_rects: self
                .outputs
                .iter()
                .filter_map(|entry| self.output_rect(entry.backend).map(|rect| (entry.backend, rect)))
                .collect(),
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
        for window in self.fallback.windows() {
            let entry = self.windows.get(&window.id);
            let mapped = entry.is_some_and(|entry| entry.mapped);
            snapshot.windows.insert(
                window.id,
                ObservedWindow {
                    id: window.id,
                    title: entry.map_or_else(String::new, |entry| entry.title.clone()),
                    app_id: entry.map_or_else(String::new, |entry| entry.app_id.clone()),
                    mapped,
                    visible: self.window_shown(window.id),
                    focused: focused == Some(window.id),
                    geometry: self.geometry_of(window.id).unwrap_or(window.geometry),
                    pid: self.window_process(window.id).and_then(crate::WindowProcess::pid),
                    icon_name: entry.and_then(|entry| entry.icon_name.clone()),
                    icon_buffer_count: entry.map_or(0, |entry| entry.icon_buffer_count),
                    tag: entry.and_then(|entry| entry.tag.clone()),
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

    /// Focuses a window, as a shell's `window.focus` does, through the same decision a click's activation takes. The
    /// return value is whether the record holds focus on it now, which is what a caller that has to answer an
    /// operation needs; a window that has closed is not focused and is not invented.
    ///
    /// It is an explicit command, not a client's request, so it moves the focus whether or not a shell holds the
    /// tree. Raising is the shell's, and the record leaves it alone while one does.
    pub(crate) fn focus_window(&mut self, window: WindowId) -> bool {
        if self.fallback.window(window).is_none() {
            return false;
        }
        let effects = self.with_seat(|fallback, seat| fallback.activate(window, true, seat));
        self.apply_filtered(effects, |effect| {
            matches!(effect, FallbackEffect::Visible { .. } | FallbackEffect::Focus(_))
        });
        self.publish();
        self.focus == Some(window)
    }

    /// Asks for a window to take focus on a shell's behalf, as `Window:present()` does.
    ///
    /// The first state that stops the request is the answer: a closed window, a covered
    /// session, a window that is not on screen, or input within [`PRESENT_QUIET_MS`]. Only
    /// when none holds is focus requested, through [`Self::focus_window`], so the focus policy
    /// still decides. The quiet gate lives here because the input clock does; the caller
    /// retries `Typing` and `NotShown` until its deadline rather than stealing focus mid-keystroke.
    pub(crate) fn present_window(&mut self, window: WindowId) -> PresentAnswer {
        if self.fallback.window(window).is_none() {
            return PresentAnswer::Gone;
        }
        if self.lock.state() != LockState::Unlocked {
            return PresentAnswer::Locked;
        }
        if !self.window_shown(window) {
            return PresentAnswer::NotShown;
        }
        if self.activity.idle_ticks(self.clock) < PRESENT_QUIET_MS {
            return PresentAnswer::Typing;
        }
        PresentAnswer::Focused(self.focus_window(window))
    }

    /// The shell node the pointer ray picks, with the point in its local space. Only an
    /// unlocked session whose shell holds the tree has one: a locked session routes the ray to
    /// the lock surface, and a window topmost at the point takes it.
    fn shell_pick(&self, position: SmithayPoint<f64, Logical>) -> Option<(NodeId, PointF)> {
        if self.lock.state() != LockState::Unlocked || !self.composer.shell_places() {
            return None;
        }
        self.composer.shell_node_at(PointF::new(position.x, position.y))
    }

    /// Sends the shell the pointer edges over its nodes. A ray that enters a node, moves over it
    /// or leaves it sends the matching edge; a pick of `None` leaves the node it was over.
    fn shell_pointer(&mut self, pick: Option<(NodeId, PointF)>) {
        let previous = self.shell_hover;
        let current = pick.map(|(node, _)| node);
        if let Some(old) = previous.filter(|old| Some(*old) != current) {
            self.tree.input(old, TreeInput::Leave);
        }
        if let Some((node, local)) = pick {
            let (x, y) = (local.x, local.y);
            let input = if previous == current {
                TreeInput::Motion { x, y }
            } else {
                TreeInput::Enter { x, y }
            };
            self.tree.input(node, input);
        }
        self.shell_hover = current;
    }

    /// Activates the window under the pointer, as the fallback does for a press when no shell holds the tree.
    fn activate_under_pointer(&mut self) {
        self.activate_at(self.pointer_position);
    }

    /// Focuses the window under a point, and raises it when `raise_on_click` asks. A pointer press and a touch down
    /// are the same request. While a shell holds the tree the press is an edge on the window's node instead
    /// (a touch is a left press and release there), and the shell decides.
    fn activate_at(&mut self, position: SmithayPoint<f64, Logical>) {
        if self.composer.shell_places() {
            let on_window = self
                .pointer_target(position)
                .filter(|(node, _)| self.composer.window_of_node(*node).is_some());
            if let Some((node, local)) = on_window {
                let (x, y) = (local.x, local.y);
                self.tree.input(node, TreeInput::Press { x, y, button: BTN_LEFT });
                self.tree.input(node, TreeInput::Release { x, y, button: BTN_LEFT });
            }
            return;
        }
        let Some((surface, _)) = self.surface_under(position) else {
            return;
        };
        let Some(window) = self.window_of(&surface) else {
            return;
        };
        let raise = self.options.raise_on_click;
        let effects = self.with_seat(|fallback, seat| fallback.activate(window, raise, seat));
        self.apply(effects);
        self.publish();
    }

    /// Routes one touch point. A point goes to the window under it when it goes down; its
    /// motion and up follow its slot, because the protocol ties a point to a surface only
    /// at down.
    fn touch_input(&mut self, slot: u32, location: SmithayPoint<f64, Logical>, phase: TouchPhase, time: InputTime) {
        let touch = self.touch.clone();
        match phase {
            TouchPhase::Down => {
                // A press on a window asks for activation, as a pointer press does.
                self.activate_at(location);
                // A point that lands on nothing reaches no client.
                let Some((surface, origin)) = self.surface_under(location) else {
                    return;
                };
                let serial = SERIAL_COUNTER.next_serial();
                self.remember_input_serial(u32::from(serial));
                self.touch_points.insert(
                    slot,
                    TouchPoint {
                        surface: surface.clone(),
                        location,
                    },
                );
                touch.down(
                    self,
                    Some((surface, origin)),
                    &DownEvent {
                        slot: TouchSlot::from(Some(slot)),
                        location,
                        serial,
                        time,
                    },
                );
                touch.frame(self);
            }
            TouchPhase::Motion => {
                let Some(point) = self.touch_points.get_mut(&slot) else {
                    return;
                };
                point.location = location;
                touch.motion(
                    self,
                    None,
                    &TouchMotionEvent {
                        slot: TouchSlot::from(Some(slot)),
                        location,
                        time,
                    },
                );
                touch.frame(self);
            }
            TouchPhase::Up => {
                if self.touch_points.remove(&slot).is_none() {
                    return;
                }
                let serial = SERIAL_COUNTER.next_serial();
                self.remember_input_serial(u32::from(serial));
                touch.up(
                    self,
                    &UpEvent {
                        slot: TouchSlot::from(Some(slot)),
                        serial,
                        time,
                    },
                );
                touch.frame(self);
            }
            TouchPhase::Cancel => {
                if self.touch_points.contains_key(&slot) {
                    self.cancel_touches();
                }
            }
        }
    }

    /// Ends the touch points that went down on `surface`, as lifts, because the surface is going away.
    ///
    /// A point a surface held ends for that surface's client alone, and the points of every other client go on. This
    /// is a lift, not the protocol's cancel: a cancel ends every point of the seat for every client, which a surface
    /// closing is no reason for.
    pub(crate) fn end_touches_on(&mut self, surface: &WlSurface) {
        let slots: Vec<u32> = self
            .touch_points
            .iter()
            .filter(|(_, point)| &point.surface == surface)
            .map(|(slot, _)| *slot)
            .collect();
        if slots.is_empty() {
            return;
        }
        let touch = self.touch.clone();
        let time = InputTime::now();
        for slot in slots {
            self.touch_points.remove(&slot);
            let serial = SERIAL_COUNTER.next_serial();
            touch.up(
                self,
                &UpEvent {
                    slot: TouchSlot::from(Some(slot)),
                    serial,
                    time,
                },
            );
        }
        touch.frame(self);
    }

    /// Ends every touch point the seat holds. The protocol's cancel names no slot and goes
    /// to every client holding a touch, so every point ends for every client at once.
    ///
    /// Smithay's cancel acts only on a pending frame, and a frame opens with an event, so
    /// each point first gets a motion to the location it was last seen at. That repeats a
    /// position the client already has, and the cancel that follows ends the points.
    fn cancel_touches(&mut self) {
        if self.touch_points.is_empty() {
            return;
        }
        let touch = self.touch.clone();
        let time = InputTime::now();
        let last_seen: Vec<(u32, SmithayPoint<f64, Logical>)> = self
            .touch_points
            .drain()
            .map(|(slot, point)| (slot, point.location))
            .collect();
        for (slot, location) in last_seen {
            touch.motion(
                self,
                None,
                &TouchMotionEvent {
                    slot: TouchSlot::from(Some(slot)),
                    location,
                    time,
                },
            );
        }
        touch.cancel(self);
    }

    /// The window drawn under a position that may take the pointer, with the point in its node's local space, which
    /// is the surface point under the pointer. A window is a candidate only while managed, shown and uncovered.
    fn window_pick(&self, position: SmithayPoint<f64, Logical>) -> Option<(WindowId, PointF)> {
        // The scene holds each window where it is drawn, transforms included, so a rotated window is hit where it
        // appears.
        self.composer.window_at(PointF::new(position.x, position.y), &|window| {
            self.window_is_pickable(window)
        })
    }

    /// Whether a window may take the pointer: managed, shown, uncovered and holding a surface.
    pub(crate) fn window_is_pickable(&self, window: WindowId) -> bool {
        self.fallback.window(window).is_some()
            && self.window_shown(window)
            && !self.window_is_covered(window)
            && self
                .windows
                .get(&window)
                .is_some_and(|entry| entry.surface.wl_surface().is_some())
    }

    /// The node the shell is told the pointer is over while it holds the tree: a node of its own that the ray
    /// picks, otherwise the window under the ray. A window's pointer edges are the shell's to read (its focus and
    /// raise policy follows them), and still reach the window's client as well.
    fn pointer_target(&self, position: SmithayPoint<f64, Logical>) -> Option<(NodeId, PointF)> {
        if let Some(pick) = self.shell_pick(position) {
            return Some(pick);
        }
        if self.lock.state() != LockState::Unlocked || !self.composer.shell_places() {
            return None;
        }
        let (window, local) = self.window_pick(position)?;
        Some((self.composer.window_node(window)?, local))
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
        // A shell node the ray picks covers the windows beneath it: the pointer goes to the shell.
        if self.shell_pick(position).is_some() {
            return None;
        }
        let picked = self.window_pick(position);
        // A popup is above its window and above the windows beneath that one, and it reaches past its window's
        // edge, so it is asked before the window is.
        if let Some(popup) = self.popup_under(position, picked.as_ref().map(|(window, _)| *window)) {
            return Some(popup);
        }
        let (window, local) = picked?;
        let root = self.windows.get(&window)?.surface.wl_surface()?;
        // The scene picks the window by its bounds. The surface the pointer is over is the topmost one of the window's
        // tree, subsurfaces included, whose input region holds the point: a point the root's input region leaves out
        // reaches a subsurface laid over it, or nothing.
        let (surface, offset) = under_from_surface_tree(
            &root,
            SmithayPoint::from((local.x, local.y)),
            (0, 0),
            WindowSurfaceType::ALL,
        )?;
        // The local point is the point of the root surface under the pointer, so the root's origin is the global
        // position less it, and the origin that makes the surface-relative position come out right is the found
        // surface's own, which the tree reports relative to the root's.
        let origin = SmithayPoint::from((
            position.x - local.x + f64::from(offset.x),
            position.y - local.y + f64::from(offset.y),
        ));
        Some((surface, origin))
    }
}

/// A mode as the `<width>x<height>@<refresh>` string the `output` object publishes.
fn observed_mode(mode: gnoblin_output::DisplayMode) -> String {
    format!(
        "{}x{}@{:.3}",
        mode.width,
        mode.height,
        f64::from(mode.refresh_mhz) / 1000.0
    )
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
pub(crate) fn set_flag(states: &mut ToplevelStateSet, state: xdg_toplevel::State, enabled: bool) {
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

/// Sets `frame`'s magnified view and colour transform from `settings`, for the output
/// whose rectangle is `rect`. `pointer` is the layout position of the pointer, which
/// centres the view when it is on this output.
///
/// A missing rectangle or a settings value the frame rejects leaves that part off, so an
/// output the session no longer knows is drawn unmagnified rather than wrongly.
fn apply_visual_accessibility(
    settings: crate::server::VisualAccessibilitySettings,
    rect: Option<Rect>,
    pointer: SmithayPoint<f64, Logical>,
    frame: &mut OutputFrame,
) {
    let view = match (settings.magnifier_zoom, rect) {
        (Some(zoom), Some(rect)) => magnifier_view(zoom, rect, pointer),
        _ => None,
    };
    frame.set_view(view);
    frame.set_colour_matrix(settings.colour_matrix.and_then(ColourMatrix::new));
}

/// The magnified view of an output of size `rect`. The view is centred on the pointer
/// when the pointer is on this output, and on the middle of the output otherwise, so each
/// output shows its own area.
///
/// The centre is clamped so the magnified view stays inside the output. The visible area
/// is `width / zoom` by `height / zoom`, so its centre may come no closer than half of
/// that to an edge. `None` when the zoom is the identity, not a number, or rejected by the
/// frame.
pub(crate) fn magnifier_view(zoom: f32, rect: Rect, pointer: SmithayPoint<f64, Logical>) -> Option<View> {
    if zoom.is_nan() || zoom <= gnoblin_render_frame::frame::MIN_ZOOM {
        return None;
    }
    let width = rect.width as f32;
    let height = rect.height as f32;
    let (centre_x, centre_y) = if contains(&rect, pointer) {
        (
            (pointer.x - f64::from(rect.x)) as f32,
            (pointer.y - f64::from(rect.y)) as f32,
        )
    } else {
        (width / 2.0, height / 2.0)
    };
    let half_w = width / (2.0 * zoom);
    let half_h = height / (2.0 * zoom);
    View::new(
        zoom,
        centre_x.clamp(half_w, width - half_w),
        centre_y.clamp(half_h, height - half_h),
    )
}

/// Whether a rectangle holds a layout point.
///
/// The upper edges are exclusive, because a point on the right or bottom edge of a
/// monitor is a point on the next one, and a point on the last one is outside it.
fn contains(rect: &Rect, point: SmithayPoint<f64, Logical>) -> bool {
    let right = f64::from(rect.x + i32::try_from(rect.width).unwrap_or(i32::MAX));
    let bottom = f64::from(rect.y + i32::try_from(rect.height).unwrap_or(i32::MAX));
    point.x >= f64::from(rect.x) && point.x < right && point.y >= f64::from(rect.y) && point.y < bottom
}

/// The window geometry a surface's client committed with `xdg_surface.set_window_geometry`, in surface coordinates,
/// or `None` while it set none and the whole surface is the window.
pub(crate) fn xdg_geometry(surface: &WlSurface) -> Option<Rectangle<i32, Logical>> {
    with_states(surface, |states| {
        states
            .cached_state
            .get::<smithay::wayland::shell::xdg::SurfaceCachedState>()
            .current()
            .geometry
    })
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

/// What a surface committed right now: its buffer and how that buffer is cropped and placed.
///
/// Read from the renderer's surface state for the same reason as `has_buffer`, so a lock
/// surface and a window answer through one path. `None` when the surface has no buffer.
pub(crate) fn committed_content(surface: &WlSurface) -> Option<(WlBuffer, Content)> {
    with_renderer_surface_state(surface, |state| {
        let buffer = state.buffer()?.clone();
        let view = state.view()?;
        let size = state.buffer_size()?;
        let scale = f64::from(state.buffer_scale());
        let transform = state.buffer_transform();
        let buffer_size = SmithaySize::from((f64::from(size.w), f64::from(size.h)));
        let src = view.src.to_buffer(scale, transform, &buffer_size);
        Some((
            (*buffer).clone(),
            Content {
                src: RectF::new(src.loc.x, src.loc.y, src.size.w, src.size.h),
                size: Size::new(view.dst.w.max(0).unsigned_abs(), view.dst.h.max(0).unsigned_abs()),
                transform,
            },
        ))
    })
    .flatten()
}

impl State {
    /// Makes the wl_seat globals match the seat nodes in the scene: one global per seat node,
    /// named as the node is, created when the node appears and removed when it leaves.
    fn reconcile_seats(&mut self) {
        let wanted: std::collections::BTreeMap<gnoblin_types::NodeId, String> = self
            .composer
            .with_scene(|scene| scene.seat_names().map(|(id, name)| (id, name.to_owned())).collect());
        let gone: Vec<gnoblin_types::NodeId> = self
            .extra_seats
            .keys()
            .filter(|id| !wanted.contains_key(id))
            .copied()
            .collect();
        for id in gone {
            if let Some(seat) = self.extra_seats.remove(&id)
                && let Some(global) = seat.global()
            {
                self.display_handle.remove_global::<State>(global);
            }
        }
        for (id, name) in wanted {
            if self.extra_seats.contains_key(&id) {
                continue;
            }
            let mut seat: Seat<Self> = self.seat_state.new_wl_seat(&self.display_handle, name);
            // The seat's own pointer carries the shell's inputs to windows; the main
            // seat's pointer and its focus are never touched by them.
            seat.add_pointer();
            self.extra_seats.insert(id, seat);
        }
    }
}

impl State {
    /// Delivers the pointer inputs the applied transactions sent: each goes to the client of
    /// its window on the named seat's own pointer, so the main seat's focus is untouched. A
    /// window with no surface yet, or a seat that is gone, takes nothing.
    fn deliver_seat_inputs(&mut self) {
        let time = InputTime::now();
        for input in self.tree.take_inputs() {
            let Some(seat) = self.extra_seats.get(&input.seat) else {
                continue;
            };
            let Some(pointer) = seat.get_pointer() else {
                continue;
            };
            let surface = self
                .composer
                .window_of_node(input.node)
                .and_then(|window| self.surface_of(window));
            let serial = SERIAL_COUNTER.next_serial();
            let origin = SmithayPoint::from((0.0, 0.0));
            match input.input {
                PointerInput::Leave => {
                    pointer.motion(
                        self,
                        None,
                        &MotionEvent {
                            location: origin,
                            serial,
                            time,
                        },
                    );
                }
                PointerInput::Motion { x, y } => {
                    pointer.motion(
                        self,
                        surface.map(|surface| (surface, origin)),
                        &MotionEvent {
                            location: SmithayPoint::from((x, y)),
                            serial,
                            time,
                        },
                    );
                }
                PointerInput::Button { x, y, button, pressed } => {
                    if let Some(surface) = surface.filter(|s| pointer.current_focus().as_ref() != Some(s)) {
                        pointer.motion(
                            self,
                            Some((surface, origin)),
                            &MotionEvent {
                                location: SmithayPoint::from((x, y)),
                                serial,
                                time,
                            },
                        );
                    }
                    let serial = SERIAL_COUNTER.next_serial();
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
                PointerInput::Scroll { x, y, dx, dy } => {
                    if let Some(surface) = surface.filter(|s| pointer.current_focus().as_ref() != Some(s)) {
                        pointer.motion(
                            self,
                            Some((surface, origin)),
                            &MotionEvent {
                                location: SmithayPoint::from((x, y)),
                                serial,
                                time,
                            },
                        );
                    }
                    let mut frame = AxisFrame::new(time).source(AxisSource::Continuous);
                    if dx != 0.0 {
                        frame = frame.value(Axis::Horizontal, dx);
                    }
                    if dy != 0.0 {
                        frame = frame.value(Axis::Vertical, dy);
                    }
                    pointer.axis(self, frame);
                }
            }
            pointer.frame(self);
        }
    }
}

impl Host for State {
    fn begin_frame(&mut self) {
        // The surface trees of the windows that changed reach the composer before it reports its own changes and
        // takes its damage, so a popup that committed since the last frame is drawn in this one.
        self.flush_surface_trees();
        // The pointer may have moved since the last frame; its old and new places are
        // damaged before the damage is taken.
        let redraw = std::mem::take(&mut self.cursor_shape_changed);
        self.update_cursor(redraw);
        // The compositor's own changes since the last frame lead the batch the drain publishes.
        for change in self.composer.take_tree_changes() {
            self.tree.record(change);
        }
        for node in self.composer.take_tree_ready() {
            self.tree.ready(node);
        }
        for node in self.composer.take_lock_requested() {
            self.tree.requested(node);
        }
        // A window's node is created on the frame after it maps, so the focus is published
        // again here once the node it names exists.
        let focused = self.keyboard_focus_window();
        self.composer.set_main_seat_focus(focused);
        // The tree's transactions apply to the scene the composer draws from, before the
        // damage is taken, so a transaction applied here draws in this frame.
        let tree = &mut self.tree;
        self.composer.with_scene(|scene| tree.drain(scene, Instant::now));
        // The shell's timed writes play as tracks, or land at once under reduced motion.
        let motion = self.tree.take_motion();
        self.composer.apply_shell_motion(motion);
        // A `secure` the shell wrote on a window's node marks the window for capture.
        self.composer.follow_shell_secure();
        // A seat focus the shell wrote in the drain moves the keyboard to that window. The
        // compositor's own focus was published above, before the drain, so it never reads back.
        if self.composer.main_seat_focus_written_by_shell() {
            self.focus = self.composer.main_seat_focus_window();
            // A shell node holds the seat focus: the window keeps none, and the keys go to the shell.
            if self.composer.main_seat_focus_window().is_none() {
                self.composer.hold_seat_focus_for_shell();
            }
            let target = self
                .composer
                .main_seat_focus_window()
                .and_then(|window| self.surface_of(window));
            self.set_keyboard_focus(target);
        }
        // The shell's answers to window requests: its `close` calls, then the state and size it wrote.
        let window_calls = self.tree.take_window_calls();
        self.run_window_calls(window_calls);
        let window_writes = self.tree.take_window_writes();
        self.answer_window_writes(window_writes);
        self.reconcile_seats();
        let claims = self.tree.take_claims();
        let authentications = self.tree.take_authentications();
        self.begin_authentications(authentications);
        self.apply_claims(claims);
        let destroyed = self.tree.take_destroyed();
        self.release_seat_nodes(&destroyed);
        self.deliver_seat_inputs();
        // Placement follows the shell's hold as the drain left it.
        self.composer.follow_placement();
        // A node whose effects the renderer cannot draw says so in its read-only `skipped` list.
        self.composer.publish_skipped_effects();
        // The damage is taken once for every output, because the backend renders one
        // output at a time and a per-output take would clear the others' damage.
        self.composer.begin_frame();
        // A View's picture is final for this frame, so a shell's `capture` writes what the View shows now.
        let view_captures = self.tree.take_view_captures();
        self.run_view_captures(view_captures);
        self.last_frame = None;
        self.protection_generation = None;
    }

    fn frame(&mut self, output: OutputId, frame: &mut OutputFrame) -> bool {
        // `compose` and `cover_frame` clear the frame, and clearing drops the magnified view
        // and colour transform, so the settings are applied after them. A locked session
        // draws no accessibility, so the lock cover reads the same whatever the user has set.
        let settings = if self.lock.state() == LockState::Unlocked {
            self.visual_accessibility
        } else {
            crate::server::VisualAccessibilitySettings::default()
        };
        let changed = if self.lock.state() == LockState::Unlocked {
            self.composer.compose(output, frame)
        } else {
            self.cover_frame(output, frame)
        };
        apply_visual_accessibility(settings, self.output_rect(output), self.pointer_position, frame);
        if changed {
            self.last_frame = Some((output, frame.damage().to_vec()));
            // The frame was described after the hide, so it can prove a cover: the
            // generation is read here rather than at present time, which is what makes
            // a frame queued before the cover worthless.
            self.protection_generation = Some(self.session_host.protection.generation());
        }
        // The frame draws its cursor above every layer, so a pointer shown while locked
        // would sit over the cover. A locked output shows no pointer.
        let cursor = if self.lock.state() == LockState::Unlocked {
            self.pointer_cursor(output)
        } else {
            None
        };
        frame.set_cursor_image(cursor);
        // The modal overlay is the last layer on the output it covers. `compose` and
        // `cover_frame` clear the frame first, so a cleared or moved modal leaves no stale
        // overlay behind. A locked session draws no modal: the cover is what the output shows.
        if let Some(modal) = self.active_modal().filter(|modal| modal.output() == output) {
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
        // While a modal is active it takes the keys and pointer buttons, and the clients
        // see none of them; the rest of the input is dropped for the same reason. While the
        // session is locked no modal is active, so the event goes to the lock's own focus.
        if self.active_modal().is_some() && self.route_to_modal(&event) {
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
            }
            InputEvent::PointerLeave => {
                self.shell_pointer(None);
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
                pointer.frame(self);
            }
            InputEvent::PointerButton { button, pressed } => {
                // A press or release over a shell node, or over a window while a shell holds the tree, is an
                // edge the shell reads: its focus and raise policy follows the window's. Without a shell the
                // fallback focuses the window a press lands on.
                if let Some((node, local)) = self.pointer_target(self.pointer_position) {
                    let (x, y) = (local.x, local.y);
                    let input = if pressed {
                        TreeInput::Press { x, y, button }
                    } else {
                        TreeInput::Release { x, y, button }
                    };
                    self.tree.input(node, input);
                } else if pressed {
                    self.activate_under_pointer();
                }
                let pointer = self.pointer.clone();
                let serial = SERIAL_COUNTER.next_serial();
                // A button serial is the one an application names in
                // `xdg_activation_token.set_serial`, so it has to be remembered.
                self.remember_input_serial(u32::from(serial));
                let dragging = self.dragged.is_some();
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
                // A move or resize ends with the last button released. The window moved under a pointer that no client
                // held, so the pointer is given the surface it is over now.
                if dragging && self.dragged.is_none() {
                    self.move_pointer(self.pointer_position, time);
                }
                // A press or release is a group of its own, and a client that bound `wl_pointer` at version 5 or
                // later applies nothing of it before the frame that closes the group.
                pointer.frame(self);
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
            // A scroll goes to the client the pointer is over. The value is in 1/120 of a step, so
            // it is both the discrete `v120` value and, scaled, the pixel distance. The backend
            // cannot tell a wheel from a touchpad finger, so the source is the wheel: a client
            // that smooths scrolling still gets the exact fractional steps.
            InputEvent::PointerAxis { horizontal, vertical } if horizontal != 0 || vertical != 0 => {
                // A wheel over a shell node is the shell's: the tree gets the pixel distance, and no client does.
                if let Some((node, local)) = self.shell_pick(self.pointer_position) {
                    let dx = f64::from(horizontal) / 120.0 * AXIS_PIXELS_PER_STEP;
                    let dy = f64::from(vertical) / 120.0 * AXIS_PIXELS_PER_STEP;
                    self.tree.input(
                        node,
                        TreeInput::Scroll {
                            x: local.x,
                            y: local.y,
                            dx,
                            dy,
                        },
                    );
                } else {
                    let mut frame = AxisFrame::new(time).source(AxisSource::Wheel);
                    for (axis, steps) in [(Axis::Horizontal, horizontal), (Axis::Vertical, vertical)] {
                        if steps != 0 {
                            frame = frame
                                .value(axis, f64::from(steps) / 120.0 * AXIS_PIXELS_PER_STEP)
                                .v120(axis, steps);
                        }
                    }
                    let pointer = self.pointer.clone();
                    pointer.axis(self, frame);
                    pointer.frame(self);
                }
            }
            // A scroll that moved nothing sends nothing, so a client never sees an empty frame.
            InputEvent::PointerAxis { .. } => {}
            // A gesture a shell claimed goes to that shell and to no client or binding; any other is
            // routed to a configured binding and never to a client. While the session is locked it
            // fires nothing: bindings do not run on the lock screen.
            InputEvent::Touchpad(sample) => {
                let lock = if self.lock.state() == LockState::Unlocked {
                    gnoblin_input::GestureLock::Unlocked
                } else {
                    gnoblin_input::GestureLock::Locked
                };
                match self.touchpad_gestures.route(self.clock, lock, sample) {
                    crate::touchpad::TouchpadRoute::Fire(fire) => crate::touchpad::run(&fire),
                    crate::touchpad::TouchpadRoute::Claimed(delivery) => {
                        self.tree.gesture(NodeId::new(delivery.holder), delivery);
                    }
                    crate::touchpad::TouchpadRoute::Dropped => {}
                }
            }
            InputEvent::Touch {
                output,
                slot,
                position,
                phase,
            } => {
                let Some(entry) = self.outputs.iter().find(|entry| entry.backend == output) else {
                    return;
                };
                let location = SmithayPoint::from((
                    f64::from(position.x + entry.rect.x),
                    f64::from(position.y + entry.rect.y),
                ));
                self.touch_input(slot, location, phase, time);
            }
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

    /// A layout point, for a pointer somewhere on the desktop.
    fn point(x: f64, y: f64) -> SmithayPoint<f64, Logical> {
        SmithayPoint::from((x, y))
    }

    /// A point on no output, so a view is centred on the middle of its output.
    fn nowhere() -> SmithayPoint<f64, Logical> {
        point(-100.0, -100.0)
    }

    /// Settings that magnify by `zoom` and apply no colour transform.
    fn zoomed(zoom: f32) -> VisualAccessibilitySettings {
        VisualAccessibilitySettings {
            magnifier_zoom: Some(zoom),
            colour_matrix: None,
        }
    }

    #[test]
    fn the_default_settings_leave_the_frame_unchanged() {
        let mut frame = OutputFrame::new();
        apply_visual_accessibility(
            VisualAccessibilitySettings::default(),
            Some(left_output()),
            nowhere(),
            &mut frame,
        );
        assert!(frame.view().is_none(), "no magnifier by default");
        assert!(frame.colour_matrix().is_none(), "no colour transform by default");
    }

    #[test]
    fn a_zoom_of_one_is_the_identity() {
        assert!(magnifier_view(MIN_ZOOM, left_output(), point(100.0, 100.0)).is_none());
        let mut frame = OutputFrame::new();
        apply_visual_accessibility(zoomed(1.0), Some(left_output()), point(100.0, 100.0), &mut frame);
        assert!(frame.view().is_none());
    }

    #[test]
    fn a_zoom_that_is_not_a_number_is_not_applied() {
        assert!(magnifier_view(f32::NAN, left_output(), nowhere()).is_none());
    }

    #[test]
    fn with_no_pointer_on_the_output_the_view_is_centred() {
        let left = magnifier_view(2.0, left_output(), nowhere()).expect("a view for 2x");
        assert_eq!((left.zoom, left.centre_x, left.centre_y), (2.0, 960.0, 540.0));
        let right = magnifier_view(2.0, right_output(), nowhere()).expect("a view for 2x");
        assert_eq!((right.zoom, right.centre_x, right.centre_y), (2.0, 640.0, 360.0));
    }

    #[test]
    fn the_centre_follows_the_pointer_on_its_output() {
        // Left output: the pointer at (500, 300) is inside the clamp range for 2x.
        let left = magnifier_view(2.0, left_output(), point(500.0, 300.0)).expect("a view");
        assert_eq!((left.centre_x, left.centre_y), (500.0, 300.0));
        // Right output: the origin is (1920, 0), so the pointer is at local (400, 300).
        let right = magnifier_view(2.0, right_output(), point(2320.0, 300.0)).expect("a view");
        assert_eq!((right.centre_x, right.centre_y), (400.0, 300.0));
    }

    #[test]
    fn the_centre_is_clamped_at_each_edge() {
        // At 2x the visible area is half the output, so the centre stays 480 by 270 from the
        // top left and 480 by 270 from the bottom right of a 1920 by 1080 output.
        let top_left = magnifier_view(2.0, left_output(), point(5.0, 5.0)).expect("a view");
        assert_eq!((top_left.centre_x, top_left.centre_y), (480.0, 270.0));
        let bottom_right = magnifier_view(2.0, left_output(), point(1915.0, 1075.0)).expect("a view");
        assert_eq!((bottom_right.centre_x, bottom_right.centre_y), (1440.0, 810.0));
    }

    #[test]
    fn a_pointer_on_a_neighbour_does_not_move_this_output_view() {
        // The pointer is on the left output, so the right output stays on its own middle.
        let right = magnifier_view(2.0, right_output(), point(100.0, 100.0)).expect("a view");
        assert_eq!((right.centre_x, right.centre_y), (640.0, 360.0));
    }

    #[test]
    fn the_zoom_reaches_the_frame_of_its_own_output_only() {
        // The pointer is on the left output, so only the left frame's view follows it.
        let mut left = OutputFrame::new();
        apply_visual_accessibility(zoomed(2.0), Some(left_output()), point(500.0, 300.0), &mut left);
        let mut right = OutputFrame::new();
        apply_visual_accessibility(zoomed(2.0), Some(right_output()), point(500.0, 300.0), &mut right);
        assert_eq!(
            left.view().map(|view| (view.centre_x, view.centre_y)),
            Some((500.0, 300.0))
        );
        assert_eq!(
            right.view().map(|view| (view.centre_x, view.centre_y)),
            Some((640.0, 360.0))
        );
    }

    #[test]
    fn an_unknown_output_is_drawn_unmagnified() {
        let mut frame = OutputFrame::new();
        apply_visual_accessibility(zoomed(2.0), None, nowhere(), &mut frame);
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
        apply_visual_accessibility(settings, Some(left_output()), nowhere(), &mut frame);
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
        apply_visual_accessibility(settings, Some(left_output()), nowhere(), &mut frame);
        assert!(frame.colour_matrix().is_none());
    }
}
