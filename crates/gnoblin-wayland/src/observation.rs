//! The window state a test or a diagnostics reader observes.
//!
//! The server owns its policy state on its own thread, so a test cannot borrow it.
//! What it can do is read a snapshot: the server republishes [`SessionSnapshot`] after
//! every change and a caller holds the [`std::sync::Arc`] the server was built
//! with. That is the in-process observation recorder `gnoblin-testkit` will build
//! on, kept here so the first real client test needs nothing else.

use std::collections::BTreeMap;

use gnoblin_lock::State as LockState;
use gnoblin_types::{OutputId, Rect, WindowId, WorkspaceId};

/// One window, as the server last saw it.
#[derive(Clone, PartialEq, Eq, Debug)]
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
    /// The icon name the client set with `xdg-toplevel-icon-v1`, if any.
    pub icon_name: Option<String>,
    /// How many icon buffers the client attached with `xdg-toplevel-icon-v1`.
    pub icon_buffer_count: usize,
    /// The tag the client set with `xdg-toplevel-tag-v1`, if any.
    pub tag: Option<String>,
    /// Whether the window is a modal dialog: a parented toplevel that asked to be modal.
    /// The window manager enforces it: while the window is open its parent cannot take focus.
    pub modal: bool,
    /// Whether the window floats, outside the tiled set of its workspace's layout.
    pub floating: bool,
    /// The window this one is a transient of, set by `xdg-foreign` or the client's parent.
    pub transient_for: Option<WindowId>,
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
    pub id: WorkspaceId,
    pub layout: String,
}

/// One output at one instant: its connector name and the default layout its workspaces
/// fall back to.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct ObservedOutput {
    pub id: OutputId,
    pub name: String,
    pub layout: String,
}

/// The window state at one instant.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct SessionSnapshot {
    pub(crate) windows: BTreeMap<WindowId, ObservedWindow>,
    pub(crate) outputs: Vec<OutputId>,
    /// Every workspace the window manager arranges, with the layout it runs.
    pub(crate) workspaces: Vec<ObservedWorkspace>,
    /// Every output the session presents on: its connector name and its default layout.
    pub(crate) output_layouts: Vec<ObservedOutput>,
    /// Each workspace whose last layout run failed, with the reason it failed.
    pub(crate) layout_failures: Vec<(WorkspaceId, &'static str)>,
    pub(crate) focus: Option<WindowId>,
    pub(crate) lock: LockObservation,
    pub(crate) layers: Vec<ObservedLayer>,
    pub(crate) activation_denied: Option<ActivationDenial>,
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
            layout_failures: Vec::new(),
            focus,
            lock: LockObservation::default(),
            layers: Vec::new(),
            activation_denied: None,
            selection: None,
            bells: 0,
            capture_sessions: 0,
        }
    }
}
