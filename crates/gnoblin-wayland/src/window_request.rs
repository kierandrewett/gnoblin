//! Window requests: what a client, a taskbar or the control surface asks of a toplevel, and how the answer reaches
//! the client.
//!
//! While a shell holds the tree and the session is not locked, the shell is the policy authority. Every request
//! becomes a `request` event on the window's node ([`TreeRequest`]) and nothing else happens: the shell decides, and
//! answers by writing the node's `maximised`, `fullscreen`, `minimised`, `width` and `height`, or by calling its
//! `close`. The compositor turns those writes into the window's configure ([`State::answer_window_writes`]) and runs
//! the call ([`State::run_window_calls`]). A request the shell leaves alone changes nothing. A Wayland toplevel is
//! told through its `xdg_toplevel` states and size, an X11 window through its `_NET_WM_STATE` and a configure.
//!
//! With no shell attached the fallback decides ([`crate::fallback`]): the request is applied directly. The protocol
//! files name a [`WindowRequest`] and never reach the record themselves.

use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::utils::Size as SmithaySize;
use smithay::wayland::shell::xdg::ToplevelSurface;

use gnoblin_render::transaction::TreeCall;
use gnoblin_types::{NodeId, OutputId, Rect, WindowId};

use crate::fallback::WindowState;
use crate::observation::WindowChangeRefusal;
use crate::state::{Managed, set_flag, x11_geometry};
use crate::tree_frame::TreeRequest;
use crate::{State, log};

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
        self.fallback.window(window).is_some()
    }

    /// Whether a window holds the seat's keyboard focus, which is the shell's to give while it holds the tree.
    pub(crate) fn is_focused_window(&self, window: WindowId) -> bool {
        self.keyboard
            .current_focus()
            .and_then(|surface| self.window_of(&surface))
            == Some(window)
    }

    /// A window's state, as it stands now: the shell's node values while a shell holds the tree, the record's
    /// otherwise. `None` for a window that is gone.
    pub(crate) fn window_flags(&self, window: WindowId) -> Option<WindowState> {
        let state = self.fallback.window(window)?.state;
        Some(self.effective_state(window, state))
    }

    /// `state` with `fullscreen`, `maximised` and `minimised` read from the window's node when a shell holds the
    /// tree: the node is where the shell writes them, so the record's copy must not be published over it.
    pub(crate) fn effective_state(&self, window: WindowId, mut state: WindowState) -> WindowState {
        if let Some([fullscreen, maximised, minimised]) = self.composer.shell_window_flags(window) {
            state.fullscreen = fullscreen;
            state.maximised = maximised;
            state.minimised = minimised;
        }
        state
    }

    /// Whether the window is drawn: it is on an output and not minimised, as the shell says while it holds the tree.
    pub(crate) fn window_shown(&self, window: WindowId) -> bool {
        self.fallback
            .window(window)
            .is_some_and(|managed| managed.output.is_some())
            && self.window_flags(window).is_some_and(|state| !state.minimised)
    }

    /// Whether the shell decides this window's requests: it holds the tree, the session is not locked, and the
    /// window has a node.
    fn shell_decides(&self, window: WindowId) -> bool {
        self.lock.state() == gnoblin_lock::State::Unlocked
            && self.composer.shell_places()
            && self.composer.window_node(window).is_some()
    }

    /// Hands a request to whoever decides it: the shell as an event on the window's node, or the record directly
    /// when no shell holds the tree.
    pub(crate) fn window_request(&mut self, window: WindowId, request: WindowRequest) {
        if self.shell_decides(window)
            && let Some(node) = self.composer.window_node(window)
        {
            self.tree.event(node, request.tree());
            return;
        }
        self.window_request_directly(window, request);
    }

    /// Applies a request through the record, for a session with no shell. A request that changes nothing
    /// (maximising a maximised window) is dropped, and a window that is gone is a no-op.
    fn window_request_directly(&mut self, window: WindowId, request: WindowRequest) {
        if self.fallback.window(window).is_none() {
            log::trace!(area = "toplevel", %window, ?request, "a request for a window that is gone");
            return;
        }
        let effects = match request {
            WindowRequest::Maximise(on) => self.fallback.set_maximised(window, on),
            WindowRequest::Fullscreen(on) => self.with_seat(|fallback, seat| fallback.set_fullscreen(window, on, seat)),
            WindowRequest::Minimise(on) => self.with_seat(|fallback, seat| fallback.set_minimised(window, on, seat)),
            WindowRequest::Activate => {
                // The caller is privileged (a taskbar's click, or an earned token): the plan's rule against a
                // client demanding focus is about a client, not about the user's own action.
                log::debug!(area = "toplevel", %window, "activated");
                self.with_seat(|fallback, seat| fallback.activate(window, true, seat))
            }
            WindowRequest::Raise => self.with_seat(|fallback, seat| fallback.raise(window, seat)),
            WindowRequest::Lower => self.with_seat(|fallback, seat| fallback.lower(window, seat)),
            WindowRequest::Close => {
                self.close_window(window);
                return;
            }
        };
        self.apply(effects);
        self.publish();
    }

    /// Moves and resizes a window for a control request. Refused while a shell holds the tree: it places its own
    /// windows. Otherwise the record keeps the rectangle inside the work area, as a drag would.
    pub(crate) fn place_by_command(&mut self, window: WindowId, geometry: Rect) -> Result<(), WindowChangeRefusal> {
        if self.composer.shell_places() {
            return Err(WindowChangeRefusal::Refused);
        }
        let effects = self.fallback.move_resize(window, geometry).map_err(Self::refusal_of)?;
        self.apply(effects);
        self.publish();
        Ok(())
    }

    /// Moves a window, and its transients, to another output for a control request, with the same rule as
    /// [`State::place_by_command`].
    pub(crate) fn move_to_output_by_command(
        &mut self,
        window: WindowId,
        output: OutputId,
    ) -> Result<(), WindowChangeRefusal> {
        if self.composer.shell_places() {
            return Err(WindowChangeRefusal::Refused);
        }
        let effects = self.fallback.move_to_output(window, output).map_err(Self::refusal_of)?;
        self.apply(effects);
        self.publish();
        Ok(())
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

    /// The work area of the output a window is on, or `None` when it has no output.
    pub(crate) fn work_area_of(&self, window: WindowId) -> Option<Rect> {
        let output = self.fallback.window(window)?.output?;
        self.fallback.work_area(output)
    }

    /// Brings one window's pending configure in line with its node.
    fn configure_from_node(&mut self, window: WindowId) {
        let Some(managed) = self.fallback.window(window) else {
            return;
        };
        let state = self.effective_state(window, managed.state);
        let size = self
            .composer
            .window_node_size(window)
            .filter(|&(width, height)| width > 0 && height > 0);
        let Some(toplevel) = self.toplevel_of(window) else {
            self.configure_x11_from_node(window, state, size);
            return;
        };
        // A minimised window is hidden: its client is told it is suspended and leaves the outputs, as a window on
        // another workspace does.
        let suspended = toplevel.with_pending_state(|pending| pending.states.contains(xdg_toplevel::State::Suspended));
        if suspended != state.minimised {
            self.set_visible(window, !state.minimised);
        }
        // A tiled window is told all four edges and is sent the work area as its bounds, so its client drops the
        // border and rounding it draws for itself and fits the area.
        let tiled = self.composer.shell_window_tiled(window);
        let bounds = if tiled { self.work_area_of(window) } else { None };
        if write_pending(&toplevel, state, size, tiled, bounds) && toplevel.is_initial_configure_sent() {
            toplevel.send_configure();
        }
        log::debug!(
            area = "toplevel",
            %window,
            maximised = state.maximised,
            fullscreen = state.fullscreen,
            minimised = state.minimised,
            ?size,
            tiled,
            "configured from the shell's node"
        );
    }

    /// Brings an X11 window in line with its node: the three flags become its `_NET_WM_STATE`, and the frame
    /// the shell placed it in becomes its configure, so the client hears where it is and how big.
    fn configure_x11_from_node(&mut self, window: WindowId, state: WindowState, size: Option<(u32, u32)>) {
        let frame = self.composer.window_frame(window);
        let hidden = state.minimised;
        let Some(Managed::X11(surface)) = self.windows.get(&window).map(|entry| &entry.surface) else {
            return;
        };
        if let Err(error) = surface.set_maximized(state.maximised) {
            log::debug!(area = "toplevel", %error, %window, "the X11 maximised state was not sent");
        }
        if let Err(error) = surface.set_fullscreen(state.fullscreen) {
            log::debug!(area = "toplevel", %error, %window, "the X11 fullscreen state was not sent");
        }
        if let Err(error) = surface.set_hidden(hidden) {
            log::debug!(area = "toplevel", %error, %window, "the X11 hidden state was not sent");
        }
        let rect = match (frame, size) {
            (Some(frame), Some((width, height))) => Some(Rect::new(frame.x, frame.y, width, height)),
            (Some(frame), None) => Some(frame),
            (None, _) => None,
        };
        if let Some(rect) = rect
            && let Err(error) = surface.configure(Some(x11_geometry(rect)))
        {
            log::debug!(area = "toplevel", %error, %window, "the X11 configure failed");
        }
        self.set_visible(window, !hidden);
        log::debug!(
            area = "toplevel",
            %window,
            maximised = state.maximised,
            fullscreen = state.fullscreen,
            minimised = state.minimised,
            ?rect,
            "configured an X11 window from the shell's node"
        );
    }
}

/// Writes the shell's answer into a toplevel's pending state. Returns whether it changed, so a write that says what
/// the client already has sends nothing.
fn write_pending(
    toplevel: &ToplevelSurface,
    state: WindowState,
    size: Option<(u32, u32)>,
    tiled: bool,
    bounds: Option<Rect>,
) -> bool {
    toplevel.with_pending_state(|pending| {
        let mut changed = false;
        for edge in [
            xdg_toplevel::State::TiledLeft,
            xdg_toplevel::State::TiledRight,
            xdg_toplevel::State::TiledTop,
            xdg_toplevel::State::TiledBottom,
        ] {
            if pending.states.contains(edge) != tiled {
                set_flag(&mut pending.states, edge, tiled);
                changed = true;
            }
        }
        let bounds = bounds.map(|area| SmithaySize::from((area.width as i32, area.height as i32)));
        if pending.bounds != bounds {
            pending.bounds = bounds;
            changed = true;
        }
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
