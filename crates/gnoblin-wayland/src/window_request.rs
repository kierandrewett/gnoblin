//! Window requests: what a client, a taskbar or the control surface asks of a toplevel, and how the answer reaches
//! the client.
//!
//! While a shell holds the tree and the session is not locked, the shell is the policy authority. Every request
//! becomes a `request` event on the window's node ([`TreeRequest`]) and nothing else happens: the shell decides, and
//! answers by writing the node's `maximised`, `fullscreen`, `minimised`, `width` and `height`, or by calling its
//! `close`. The compositor turns those writes into the toplevel's configure ([`State::answer_window_writes`]) and
//! runs the call ([`State::run_window_calls`]). A request the shell leaves alone changes nothing.
//!
//! With no shell attached the window manager decides, as it always has: the request is applied directly. The
//! protocol files name a [`WindowRequest`] and never reach the window manager themselves.
//!
//! An activation that the compositor itself refuses is not a request: the decision is the compositor's, for
//! security, and the shell never hears of it (`xdg_activation_v1`, `refuse_activation`).

use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::utils::Size as SmithaySize;
use smithay::wayland::shell::xdg::ToplevelSurface;

use gnoblin_render::transaction::TreeCall;
use gnoblin_types::{NodeId, WindowId};
use gnoblin_wm::{ActivationDecision, Event, WindowState};

use crate::State;
use crate::state::set_flag;
use crate::tree_frame::TreeRequest;

use crate::log;

/// What a window is asked to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WindowRequest {
    /// Fill the work area (`true`) or return to the floating rectangle (`false`).
    Maximise(bool),
    /// Cover the output (`true`) or leave fullscreen (`false`).
    Fullscreen(bool),
    /// Hide the window and hand focus on (`true`) or bring it back (`false`).
    Minimise(bool),
    /// Ask the window's client to close.
    Close,
    /// Focus and raise the window. The caller has already decided the activation is allowed.
    Activate,
    /// Put the window on top of its siblings.
    Raise,
    /// Put the window under its siblings.
    Lower,
}

impl WindowRequest {
    /// The request event the shell reads.
    const fn tree(self) -> TreeRequest {
        match self {
            Self::Maximise(true) => TreeRequest::Maximize,
            Self::Maximise(false) => TreeRequest::Unmaximize,
            Self::Fullscreen(true) => TreeRequest::Fullscreen,
            Self::Fullscreen(false) => TreeRequest::Unfullscreen,
            Self::Minimise(true) => TreeRequest::Minimize,
            Self::Minimise(false) => TreeRequest::Unminimize,
            Self::Close => TreeRequest::Close,
            Self::Activate => TreeRequest::Activate,
            Self::Raise => TreeRequest::Raise,
            Self::Lower => TreeRequest::Lower,
        }
    }
}

impl State {
    /// Whether a window is one the compositor manages.
    pub(crate) fn is_managed(&self, window: WindowId) -> bool {
        self.wm.window(window).is_some()
    }

    /// Whether a window holds the seat's keyboard focus, which is the shell's to give while it holds the tree.
    pub(crate) fn is_focused_window(&self, window: WindowId) -> bool {
        self.keyboard
            .current_focus()
            .and_then(|surface| self.window_of(&surface))
            == Some(window)
    }

    /// A window's state, as it stands now: the shell's node values while a shell holds the tree, the window manager's
    /// otherwise. `None` for a window that is gone.
    pub(crate) fn window_flags(&self, window: WindowId) -> Option<WindowState> {
        let state = self.wm.window(window)?.state;
        Some(self.effective_state(window, state))
    }

    /// `state` with `fullscreen`, `maximised` and `minimised` read from the window's node when a shell holds the
    /// tree: the node is where the shell writes them, so the window manager's copy must not be published over it.
    pub(crate) fn effective_state(&self, window: WindowId, mut state: WindowState) -> WindowState {
        if let Some([fullscreen, maximised, minimised]) = self.composer.shell_window_flags(window) {
            state.fullscreen = fullscreen;
            state.maximised = maximised;
            state.minimised = minimised;
        }
        state
    }

    /// Whether the shell decides this window's requests: it holds the tree, the session is not locked, and the
    /// window is a Wayland toplevel with a node.
    fn shell_decides(&self, window: WindowId) -> bool {
        self.lock.state() == gnoblin_lock::State::Unlocked
            && self.composer.shell_places()
            && self.toplevel_of(window).is_some()
            && self.composer.window_node(window).is_some()
    }

    /// Hands a request to whoever decides it: the shell as an event on the window's node, or the window manager
    /// directly when no shell holds the tree.
    pub(crate) fn window_request(&mut self, window: WindowId, request: WindowRequest) {
        if self.shell_decides(window)
            && let Some(node) = self.composer.window_node(window)
        {
            self.tree.event(node, request.tree());
            return;
        }
        self.window_request_directly(window, request);
    }

    /// Applies a request through the window manager, for a session with no shell. A request that changes nothing
    /// (maximising a maximised window) is dropped, and a window that is gone is a no-op.
    fn window_request_directly(&mut self, window: WindowId, request: WindowRequest) {
        let Some(state) = self.wm.window(window).map(|managed| managed.state) else {
            log::trace!(area = "toplevel", %window, ?request, "a request for a window that is gone");
            return;
        };
        let event = match request {
            WindowRequest::Maximise(on) if state.maximised != on => Event::ToggleMaximise { window },
            WindowRequest::Fullscreen(on) if state.fullscreen != on => Event::ToggleFullscreen { window },
            WindowRequest::Minimise(true) if !state.minimised => Event::Minimise { window },
            WindowRequest::Minimise(false) if state.minimised => Event::Unminimise { window },
            WindowRequest::Activate => {
                // The caller is privileged (a taskbar's click, or an earned token): the plan's rule against a
                // client demanding focus is about a client, not about the user's own action.
                log::debug!(area = "toplevel", %window, "activated");
                Event::ActivationRequested {
                    window,
                    decision: ActivationDecision::Allow,
                }
            }
            WindowRequest::Raise => Event::Raise { window },
            WindowRequest::Lower => Event::Lower { window },
            WindowRequest::Close => {
                self.close_window(window);
                return;
            }
            WindowRequest::Maximise(_) | WindowRequest::Fullscreen(_) | WindowRequest::Minimise(_) => return,
        };
        let effects = self.wm.apply(event);
        self.apply(effects);
        self.publish();
    }

    /// Asks a window's client to close: `xdg_toplevel.close` for a Wayland window, the X11 policy's close
    /// otherwise. `false` when nothing was asked.
    pub(crate) fn close_window(&mut self, window: WindowId) -> bool {
        if self.toplevel_of(window).is_some() {
            self.toplevel_close(window);
            true
        } else {
            self.close_x11_window(window)
        }
    }

    /// Runs the method calls the shell made on window nodes. `close` asks the client to close, and the client
    /// decides whether it exits.
    pub(crate) fn run_window_calls(&mut self, calls: Vec<TreeCall>) {
        for call in calls {
            let Some(window) = self.composer.window_of_node(call.node) else {
                continue;
            };
            match call.method.as_str() {
                "close" => {
                    self.close_window(window);
                }
                other => log::trace!(area = "toplevel", %window, method = other, "a window call nothing answers"),
            }
        }
    }

    /// Answers the shell's writes to window nodes with configures: the node's `maximised`, `fullscreen`, `minimised`
    /// and size become the toplevel's pending state, and one configure goes out when any of it changed.
    pub(crate) fn answer_window_writes(&mut self, nodes: Vec<NodeId>) {
        if nodes.is_empty() {
            return;
        }
        for node in nodes {
            if let Some(window) = self.composer.window_of_node(node) {
                self.configure_from_node(window);
            }
        }
        // The taskbars' lists and the snapshot follow the flags the shell wrote.
        self.publish();
    }

    /// Brings one toplevel's pending configure in line with its node.
    fn configure_from_node(&mut self, window: WindowId) {
        let Some(managed) = self.wm.window(window) else {
            return;
        };
        let state = self.effective_state(window, managed.state);
        let size = self
            .composer
            .window_node_size(window)
            .filter(|&(width, height)| width > 0 && height > 0);
        let Some(toplevel) = self.toplevel_of(window) else {
            return;
        };
        // A minimised window is hidden: its client is told it is suspended and leaves the outputs, as a window on
        // another workspace does.
        let suspended = toplevel.with_pending_state(|pending| pending.states.contains(xdg_toplevel::State::Suspended));
        if suspended != state.minimised {
            self.set_visible(window, !state.minimised);
        }
        if write_pending(&toplevel, state, size) && toplevel.is_initial_configure_sent() {
            toplevel.send_configure();
        }
        log::debug!(
            area = "toplevel",
            %window,
            maximised = state.maximised,
            fullscreen = state.fullscreen,
            minimised = state.minimised,
            ?size,
            "configured from the shell's node"
        );
    }
}

/// Writes the shell's answer into a toplevel's pending state. Returns whether it changed, so a write that says what
/// the client already has sends nothing.
fn write_pending(toplevel: &ToplevelSurface, state: WindowState, size: Option<(u32, u32)>) -> bool {
    toplevel.with_pending_state(|pending| {
        let mut changed = false;
        for (flag, on) in [
            (xdg_toplevel::State::Maximized, state.maximised),
            (xdg_toplevel::State::Fullscreen, state.fullscreen),
        ] {
            if pending.states.contains(flag) != on {
                set_flag(&mut pending.states, flag, on);
                changed = true;
            }
        }
        if let Some((width, height)) = size {
            let size = SmithaySize::from((width as i32, height as i32));
            if pending.size != Some(size) {
                pending.size = Some(size);
                changed = true;
            }
        }
        changed
    })
}
