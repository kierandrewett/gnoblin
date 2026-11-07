//! The window state a test or a diagnostics reader observes.
//!
//! The server owns its policy state on its own thread, so a test cannot borrow it.
//! What it can do is read a snapshot: the server republishes [`SessionSnapshot`] after
//! every change and a caller holds the [`std::sync::Arc`] the server was built
//! with. That is the in-process observation recorder `gnoblin-testkit` will build
//! on, kept here so the first real client test needs nothing else.

use std::collections::BTreeMap;

use gnoblin_lock::State as LockState;
use gnoblin_types::{OutputId, Rect, Scale, WindowId, WorkspaceId};

/// One window, as the server last saw it.
#[derive(Clone, PartialEq, Debug)]
pub struct ObservedWindow {
    /// The policy identity of the window.
    pub id: WindowId,
    /// The title the client last sent.
    pub title: String,
    /// The application id the client last sent.
    pub app_id: String,
    /// Whether the client has committed a buffer, so the window is drawn.
    pub mapped: bool,
    /// Whether the window manager considers the window visible now.
    pub visible: bool,
    /// Whether the window holds keyboard focus.
    pub focused: bool,
    /// The window's frame rectangle in global logical coordinates.
    pub geometry: Rect,
    /// The pid of the process behind the window's client, read from the socket's
    /// credentials. `None` for an X11 window, a client whose credentials could not be
    /// read, and a client that has gone. Never derived from the app id or the title.
    pub pid: Option<u32>,
    /// The icon name the client set with `xdg-toplevel-icon-v1`, if any.
    pub icon_name: Option<String>,
    /// How many icon buffers the client attached with `xdg-toplevel-icon-v1`.
    pub icon_buffer_count: usize,
    /// The tag the client set with `xdg-toplevel-tag-v1`, if any.
    pub tag: Option<String>,
    /// Whether the window is a modal dialog: a parented toplevel that asked to be modal.
    /// The window manager enforces it: while the window is open its parent cannot take focus.
    pub modal: bool,
    /// Whether the client has missed its liveness ping for longer than the check-alive timeout.
    pub unresponsive: bool,
    /// The window this one is a transient of, set by `xdg-foreign` or the client's parent.
    pub transient_for: Option<WindowId>,
    /// The window's effect stack in paint order, first to last. Each entry is what a client
    /// asked for through `window.add_effect`; `active` and `reason` are what the renderer
    /// reports back, so a refused effect stays visible.
    pub effects: Vec<EffectSpec>,
}

/// One window effect as the session carries it: the flat parameter set of the schema's
/// `effect` object. A kind uses only its own parameters; the rest are ignored, as
/// `effect.toml` documents. Defaults match the schema's documented defaults.
#[derive(Clone, PartialEq, Debug)]
pub struct EffectSpec {
    /// `blur`, `shadow` or `shader`. Fixed when the effect is added.
    pub kind: String,
    /// Whether the effect is applied. A disabled effect stays in the stack and draws nothing.
    pub enabled: bool,
    /// Whether the renderer draws it. Set by the compositor, never by the client.
    pub active: bool,
    /// Why the effect is inactive, as a short sentence; empty when `active` is true.
    pub reason: String,
    /// Blur: the backdrop is sampled at `1 / downscale` of full resolution. Default 2.
    pub downscale: u32,
    /// Blur: the number of blur passes. Default 3.
    pub passes: u32,
    /// Blur: `gaussian`, `kawase` or `box`. Default `kawase`.
    pub algorithm: String,
    /// Blur: grain from 0.0 to 1.0. Default 0.0.
    pub noise: f64,
    /// Blur: brightness multiplier. Default 1.0.
    pub brightness: f64,
    /// Blur: contrast multiplier. Default 1.0.
    pub contrast: f64,
    /// Blur: saturation multiplier. Default 1.0.
    pub saturation: f64,
    /// Blur: tint as `#rrggbb` or `#rrggbbaa`; empty is no tint.
    pub tint: String,
    /// Shadow: horizontal offset in logical pixels. Default 0.0.
    pub offset_x: f64,
    /// Shadow: vertical offset in logical pixels. Default 0.0.
    pub offset_y: f64,
    /// Shadow: blur radius of the edge in logical pixels. Default 12.0.
    pub blur_radius: f64,
    /// Shadow: growth before blurring in logical pixels. Default 0.0.
    pub spread: f64,
    /// Shadow: colour as `#rrggbb` or `#rrggbbaa`. Default `#00000080`.
    pub colour: String,
    /// Shader: the shader module name. Empty is inactive. Default empty.
    pub source: String,
    /// Shader: the entry point in `source`. Default `main`.
    pub entry: String,
}

impl Default for EffectSpec {
    /// The schema's defaults for every parameter; `kind` is empty until the client sets it.
    fn default() -> Self {
        Self {
            kind: String::new(),
            enabled: true,
            active: false,
            reason: String::new(),
            downscale: 2,
            passes: 3,
            algorithm: "kawase".to_owned(),
            noise: 0.0,
            brightness: 1.0,
            contrast: 1.0,
            saturation: 1.0,
            tint: String::new(),
            offset_x: 0.0,
            offset_y: 0.0,
            blur_radius: 12.0,
            spread: 0.0,
            colour: "#00000080".to_owned(),
            source: String::new(),
            entry: "main".to_owned(),
        }
    }
}

/// One layer surface, as the server last saw it.
///
/// A layer surface is not a window: it has no work-area constraint, no stacking place
/// in the window manager and no identity there. What it has is a namespace, a layer, a
/// rectangle and, when its anchors make it a strip against one edge, an exclusive zone
/// the work area leaves out.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ObservedLayer {
    /// The namespace the client named, which is how a shell recognises its own bars.
    pub namespace: String,
    /// `background`, `bottom`, `top` or `overlay`.
    pub layer: String,
    /// The rectangle it occupies in global logical coordinates.
    pub rect: Rect,
    /// The exclusive zone as the protocol states it: a positive amount, `0` neutral
    /// and `-1` for a surface that does not care.
    pub exclusive: i32,
    /// Whether it has committed a buffer, which is what "shown" means for one.
    pub mapped: bool,
}

/// A refused activation: what was refused and why.
///
/// The plan calls the signal `activation-denied`; a shell shows it and a test asserts
/// on it, so "focus did not move" is never the only evidence. `reason` is one of
/// `gnoblin-authority`'s own denial names (`unknown-token`, `reused-token`,
/// `session-locked`, ...) or `not-a-window` when the token named a surface that is not
/// a managed window.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ActivationDenial {
    /// The window that asked, when the surface was a managed one.
    pub window: Option<WindowId>,
    /// Why it was refused.
    pub reason: String,
}

/// What a reader may know about the session lock.
///
/// The answers are the ones the lock announced, so a consumer compares what it
/// holds against them instead of reconstructing the policy: `capture_allowed` is
/// the embargo, not a second opinion about it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LockObservation {
    /// Where the lock is.
    pub state: LockState,
    /// Whether a capture path may produce frames at all. The lock scene is not the
    /// desktop, so a path that cannot tell them apart must refuse while this is
    /// false.
    pub capture_allowed: bool,
}

impl Default for LockObservation {
    fn default() -> Self {
        Self {
            // A session starts unlocked, which is the lock's own initial state.
            state: LockState::Unlocked,
            capture_allowed: true,
        }
    }
}

/// One workspace at one instant: its identity and the layout it runs.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct ObservedWorkspace {
    /// The workspace's policy identity.
    pub id: WorkspaceId,
    /// The layout the window manager runs on this workspace.
    pub layout: String,
}

/// One output at one instant: every property the `gnoblin.output` object publishes.
///
/// A value the backend does not report is published as the schema's documented
/// default (an empty string, zero or `false`), never as invented hardware data.
/// `make` and `model` stay empty until the backend reads them from the EDID.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct ObservedOutput {
    /// The output's policy identity.
    pub id: OutputId,
    /// The connector name of the output, such as `DP-1`.
    pub name: String,
    /// The default layout this output's workspaces fall back to.
    pub layout: String,
    /// The vendor, as EDID reports it. Empty when the backend has none.
    pub make: String,
    /// The model name. Empty when the backend has none.
    pub model: String,
    /// Every mode the output supports, as `<width>x<height>@<refresh>`. The backend
    /// reports only the mode it drives, so this holds that one mode.
    pub modes: Vec<String>,
    /// The mode currently set; always one of `modes`.
    pub mode: String,
    /// The logical scale the output composes at.
    pub scale: Scale,
    /// The rotation and flip, in Mutter's numbering; `0` is normal.
    pub transform: u32,
    /// Whether this output carries the primary logical monitor.
    pub primary: bool,
    /// Whether the output is driving a logical monitor.
    pub enabled: bool,
    /// The logical rectangle the output occupies.
    pub rect: Rect,
}

/// Where a window menu should appear, in output logical coordinates.
///
/// A point has zero size. A client's anchor for a titlebar button is that button's
/// rectangle, so the menu can open beside it rather than over it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuAnchor {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

impl MenuAnchor {
    /// An anchor covering the given rectangle.
    #[must_use]
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self { x, y, width, height }
    }

    /// A zero-size anchor at a point.
    #[must_use]
    pub const fn point(x: i32, y: i32) -> Self {
        Self::new(x, y, 0, 0)
    }

    /// The anchor's left edge.
    #[must_use]
    pub const fn x(&self) -> i32 {
        self.x
    }

    /// The anchor's top edge.
    #[must_use]
    pub const fn y(&self) -> i32 {
        self.y
    }

    /// The anchor's width; zero for a point.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// The anchor's height; zero for a point.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }
}

/// One window change a control request asks for, in the terms the loop applies them.
///
/// A change carries the value the caller set rather than a toggle, so repeating a request
/// is a no-op instead of undoing the first one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowChange {
    /// Ask the window to close, as its close button does.
    Close,
    /// Minimise the window, or restore it.
    Minimised(bool),
    /// Maximise the window, or restore it.
    Maximised(bool),
    /// Fullscreen the window, or restore it.
    Fullscreen(bool),
}

/// Who asked for a window menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuSource {
    /// A client sent `xdg_toplevel.show_window_menu` with a serial the session issued.
    Client,
    /// The window-menu key binding fired while the window was focused.
    Keyboard,
    /// An extension called `window.request_menu`.
    Extension,
}

impl MenuSource {
    /// The name the `menu_requested` signal carries in its `source` field.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Keyboard => "keyboard",
            Self::Extension => "extension",
        }
    }
}

/// One request for a window's menu, as the `menu_requested` signal reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuRequest {
    window: WindowId,
    seq: u64,
    anchor: MenuAnchor,
    source: MenuSource,
    seat: String,
}

impl MenuRequest {
    /// A request numbered `seq`. Numbers only grow, which is how a reader tells a request
    /// it has published from one it has not.
    #[must_use]
    pub fn new(window: WindowId, seq: u64, anchor: MenuAnchor, source: MenuSource, seat: &str) -> Self {
        Self {
            window,
            seq,
            anchor,
            source,
            seat: seat.to_owned(),
        }
    }

    /// The window whose menu was asked for.
    #[must_use]
    pub fn window(&self) -> WindowId {
        self.window
    }

    /// This request's sequence number.
    #[must_use]
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Where the menu should appear.
    #[must_use]
    pub fn anchor(&self) -> MenuAnchor {
        self.anchor
    }

    /// Who asked.
    #[must_use]
    pub fn source(&self) -> MenuSource {
        self.source
    }

    /// The seat that asked.
    #[must_use]
    pub fn seat(&self) -> &str {
        &self.seat
    }
}

/// The window state at one instant.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct SessionSnapshot {
    /// Every managed window, keyed by identity. Read through [`SessionSnapshot::window`] or [`SessionSnapshot::windows`].
    pub(crate) windows: BTreeMap<WindowId, ObservedWindow>,
    /// The outputs the session presents on.
    pub(crate) outputs: Vec<OutputId>,
    /// Every workspace the window manager arranges, with the layout it runs.
    pub(crate) workspaces: Vec<ObservedWorkspace>,
    /// Every output the session presents on: its connector name and its default layout.
    pub(crate) output_layouts: Vec<ObservedOutput>,
    /// The logical rectangle each output occupies, so a reader can map a layout point onto it.
    pub(crate) output_rects: Vec<(OutputId, Rect)>,
    /// Each workspace whose last layout run failed, with the reason it failed.
    pub(crate) layout_failures: Vec<(WorkspaceId, &'static str)>,
    /// The window holding keyboard focus, if any.
    pub(crate) focus: Option<WindowId>,
    /// The windows in stacking order, topmost first. A raise moves a window to index 0.
    pub(crate) stack: Vec<WindowId>,
    /// The session lock's state and the capture embargo it announced.
    pub(crate) lock: LockObservation,
    /// The layer surfaces, in creation order. Read through [`SessionSnapshot::layers`].
    pub(crate) layers: Vec<ObservedLayer>,
    /// The last refused activation, if one happened. Read through [`SessionSnapshot::activation_denied`].
    pub(crate) activation_denied: Option<ActivationDenial>,
    /// The mime types of the current clipboard selection, when there is one.
    pub(crate) selection: Option<Vec<String>>,
    /// How many times a client rang the session bell.
    ///
    /// A bell is an event and not a property: nothing reads "the bell is now at
    /// seven", a reader reacts to the change. The counter is what makes the change
    /// observable to a reader that holds a snapshot and can compare it, and it is
    /// `gtk_shell1.system_bell` and `wl_display`'s own bell path that raise it.
    pub(crate) bells: u64,
    /// How many capture sessions are open.
    ///
    /// The plan's capture card needs it: a session ends when the client that opened it
    /// disconnects, so a crashed adapter leaves this at zero rather than leaving a stream
    /// running that nothing can close.
    pub(crate) capture_sessions: usize,
    /// Milliseconds since the last input event, as the `idle` object's `idle_ms` reports.
    pub(crate) idle_ms: u64,
    /// How many idle inhibitors hold idle off, as the `idle` object's `inhibitors` reports.
    pub(crate) inhibitors: usize,
    /// The window menu requests made recently, oldest first, capped at the state's kept
    /// count. A request is an event, so a reader publishes each one once by its sequence
    /// number rather than diffing this list against its last snapshot.
    pub(crate) menu_requests: Vec<MenuRequest>,
}

impl SessionSnapshot {
    /// How many times a client rang the bell since the session started.
    #[must_use]
    pub fn bells(&self) -> u64 {
        self.bells
    }

    /// How many capture sessions are open.
    #[must_use]
    pub fn capture_sessions(&self) -> usize {
        self.capture_sessions
    }

    /// Milliseconds since the last input event.
    #[must_use]
    pub fn idle_ms(&self) -> u64 {
        self.idle_ms
    }

    /// How many idle inhibitors hold idle off.
    #[must_use]
    pub fn inhibitors(&self) -> usize {
        self.inhibitors
    }

    /// Every managed window, in policy identity order.
    pub fn windows(&self) -> impl Iterator<Item = &ObservedWindow> {
        self.windows.values()
    }

    /// One window, if it is managed.
    #[must_use]
    pub fn window(&self, id: WindowId) -> Option<&ObservedWindow> {
        self.windows.get(&id)
    }

    /// How many windows are managed.
    #[must_use]
    pub fn len(&self) -> usize {
        self.windows.len()
    }

    /// Whether no window is managed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    /// The window holding keyboard focus.
    #[must_use]
    pub fn focused(&self) -> Option<&ObservedWindow> {
        self.focus.and_then(|id| self.windows.get(&id))
    }

    /// The identity of the focused window.
    #[must_use]
    pub fn focus(&self) -> Option<WindowId> {
        self.focus
    }

    /// The windows in stacking order, topmost first.
    #[must_use]
    pub fn stacking(&self) -> &[WindowId] {
        &self.stack
    }

    /// The outputs the server presents on.
    #[must_use]
    pub fn outputs(&self) -> &[OutputId] {
        &self.outputs
    }

    /// Every workspace the window manager arranges, with its layout.
    #[must_use]
    pub fn workspaces(&self) -> &[ObservedWorkspace] {
        &self.workspaces
    }

    /// Every output with its connector name and default layout.
    #[must_use]
    pub fn output_layouts(&self) -> &[ObservedOutput] {
        &self.output_layouts
    }

    /// The logical rectangle an output occupies, if the session presents on it.
    #[must_use]
    pub fn output_rect(&self, output: OutputId) -> Option<Rect> {
        self.output_rects
            .iter()
            .find(|(id, _)| *id == output)
            .map(|(_, rect)| *rect)
    }

    /// The reason each workspace's last layout run failed.
    #[must_use]
    pub fn layout_failures(&self) -> &[(WorkspaceId, &'static str)] {
        &self.layout_failures
    }

    /// The one managed window, when a test knows there is exactly one.
    #[must_use]
    pub fn only(&self) -> Option<&ObservedWindow> {
        if self.windows.len() == 1 {
            self.windows.values().next()
        } else {
            None
        }
    }

    /// The window with this title, if it is managed.
    #[must_use]
    pub fn titled(&self, title: &str) -> Option<&ObservedWindow> {
        self.windows.values().find(|window| window.title == title)
    }

    /// Where the session lock is.
    #[must_use]
    pub fn lock_state(&self) -> LockState {
        self.lock.state
    }

    /// Whether anything is hidden. This is `ScreenSaver.GetActive`.
    #[must_use]
    pub fn lock_active(&self) -> bool {
        self.lock.state.is_active()
    }

    /// Whether a capture path may produce frames at all.
    #[must_use]
    pub fn capture_allowed(&self) -> bool {
        self.lock.capture_allowed
    }

    /// The layer surfaces, in creation order.
    #[must_use]
    pub fn layers(&self) -> &[ObservedLayer] {
        &self.layers
    }

    /// The layer surface of one namespace, when a test knows there is exactly one.
    #[must_use]
    pub fn layer(&self, namespace: &str) -> Option<&ObservedLayer> {
        self.layers.iter().find(|layer| layer.namespace == namespace)
    }

    /// The last refused activation, if one happened and nothing has refused since.
    #[must_use]
    pub fn activation_denied(&self) -> Option<&ActivationDenial> {
        self.activation_denied.as_ref()
    }

    /// The mime types of the current clipboard selection, when there is one.
    #[must_use]
    pub fn selection(&self) -> Option<&[String]> {
        self.selection.as_deref()
    }
}

/// A reader outside the loop that wants to know when the session's state changed.
///
/// The snapshot is whole state rather than a diff, so the reader is free to miss a
/// notification — coalescing two into one is correct, because it reads the latest
/// snapshot anyway. What the contract does promise is that the snapshot is already
/// stored when this is called, and that it is the state the session has now.
///
/// Implementations must not block: the call happens inside the loop's own turn, so
/// an observer that waits for the loop would deadlock it. A coalescing channel is the
/// intended shape — one queued "look again", not one message per publish.
///
/// It is [`std::fmt::Debug`] so a [`crate::Config`] holding one can still be
/// formatted, which is what makes a start-up failure readable.
pub trait Observer: Send + Sync + std::fmt::Debug {
    /// The snapshot was replaced; `snapshot` is what it now holds.
    fn published(&self, snapshot: &SessionSnapshot);
}

impl SessionSnapshot {
    /// The same snapshot with the outputs it presents on, for a test or a recorded replay.
    #[must_use]
    pub fn with_output_layouts(mut self, outputs: Vec<ObservedOutput>) -> Self {
        self.output_layouts = outputs;
        self
    }

    /// The same snapshot with a bell count, for a reader that replays events.
    ///
    /// A session builds its own snapshot; this is what a test or a recorded replay needs
    /// to state that a bell rang, and it is the only way the count is set from outside
    /// the session.
    #[must_use]
    pub fn with_bells(mut self, bells: u64) -> Self {
        self.bells = bells;
        self
    }

    /// The same snapshot with window menu requests, for a reader that replays events.
    #[must_use]
    pub fn with_menu_requests(mut self, requests: Vec<MenuRequest>) -> Self {
        self.menu_requests = requests;
        self
    }

    /// The window menu requests this snapshot carries, oldest first.
    #[must_use]
    pub fn menu_requests(&self) -> &[MenuRequest] {
        &self.menu_requests
    }

    /// Assemble a snapshot from its parts.
    ///
    /// A session builds its own snapshot in [`crate::State`]; this is for a reader
    /// that replays one it recorded and for a test that needs a state without a
    /// session. The lock state is the released default, because a snapshot built
    /// here describes no lock; a caller that has a lock state reads it from the
    /// session that announced it.
    #[must_use]
    pub fn from_parts(
        outputs: Vec<OutputId>,
        focus: Option<WindowId>,
        windows: Vec<ObservedWindow>,
    ) -> SessionSnapshot {
        SessionSnapshot {
            windows: windows.into_iter().map(|window| (window.id, window)).collect(),
            outputs,
            workspaces: Vec::new(),
            output_layouts: Vec::new(),
            output_rects: Vec::new(),
            layout_failures: Vec::new(),
            focus,
            stack: Vec::new(),
            lock: LockObservation::default(),
            layers: Vec::new(),
            activation_denied: None,
            selection: None,
            bells: 0,
            menu_requests: Vec::new(),
            capture_sessions: 0,
            idle_ms: 0,
            inhibitors: 0,
        }
    }
}
