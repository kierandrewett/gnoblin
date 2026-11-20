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
//! With no shell attached the provider decides ([`gnoblin_wm_api::WindowManager`]): the request is applied directly.
//! The protocol files name a [`WindowRequest`] and never reach the record themselves.

use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::utils::Size as SmithaySize;
use smithay::wayland::shell::xdg::ToplevelSurface;
use smithay::xwayland::xwm::X11Surface;

use gnoblin_backend::x11::WindowAsk;
use gnoblin_render::transaction::TreeCall;
use gnoblin_types::{NodeId, OutputId, Rect, WindowId};
use gnoblin_wayland_types::window::WindowRequest;
use gnoblin_wm_api::WindowState;

use crate::observation::WindowChangeRefusal;
use crate::state::{Managed, set_flag, x11_geometry};
use crate::tree_frame::TreeRequest;
use crate::{State, log};

/// The request an X11 client's own ask is: the one an `xdg_toplevel` makes for the same thing.
pub(crate) const fn request_of_ask(ask: WindowAsk) -> WindowRequest {
    match ask {
        WindowAsk::Maximise(on) => WindowRequest::Maximise(on),
        WindowAsk::Fullscreen(on) => WindowRequest::Fullscreen(on),
        WindowAsk::Minimise(on) => WindowRequest::Minimise(on),
        WindowAsk::Close => WindowRequest::Close,
    }
}

/// The request event the shell reads.
const fn tree_request(request: WindowRequest) -> TreeRequest {
    match request {
        WindowRequest::Maximise(true) => TreeRequest::Maximize,
        WindowRequest::Maximise(false) => TreeRequest::Unmaximize,
        WindowRequest::Fullscreen(true) => TreeRequest::Fullscreen,
        WindowRequest::Fullscreen(false) => TreeRequest::Unfullscreen,
        WindowRequest::Minimise(true) => TreeRequest::Minimize,
        WindowRequest::Minimise(false) => TreeRequest::Unminimize,
        WindowRequest::Close => TreeRequest::Close,
        WindowRequest::Activate => TreeRequest::Activate,
        WindowRequest::Raise => TreeRequest::Raise,
        WindowRequest::Lower => TreeRequest::Lower,
    }
}

impl State {
    /// Whether a window is one the compositor manages.
    pub(crate) fn is_managed(&self, window: WindowId) -> bool {
        self.wm.window(window).is_some()
    }

    /// Whether a window holds the seat's keyboard focus, which is the shell's to give while it holds the tree.
    pub(crate) fn is_focused_window(&self, window: WindowId) -> bool {
        self.keyboard_focus_window() == Some(window)
    }

    /// A window's state, as it stands now: the shell's node values while a shell holds the tree, the record's
    /// otherwise. `None` for a window that is gone.
    pub(crate) fn window_flags(&self, window: WindowId) -> Option<WindowState> {
        let state = self.wm.window(window)?.state;
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
        self.wm.window(window).is_some_and(|managed| managed.output.is_some())
            && self.window_flags(window).is_some_and(|state| !state.minimised)
    }

    /// Who decides this window's requests, placement and focus: the shell while the session is unlocked, a holder is
    /// attached to the tree and the window has a node, the window-manager provider otherwise. It is the one place
    /// that asks the question for a window; a question about the holder alone (the lock card's lease, a control
    /// request) reads the scene's `holder_attached`.
    pub(crate) fn decider(&self, window: WindowId) -> Decider {
        decide(
            self.lock.state() == gnoblin_lock::State::Unlocked,
            self.composer.holder_attached(),
            self.composer.window_node(window).is_some(),
        )
    }

    /// Hands a request to whoever decides it: the shell as an event on the window's node, or the provider directly
    /// when no holder decides.
    pub(crate) fn window_request(&mut self, window: WindowId, request: WindowRequest) {
        if self.decider(window) == Decider::Shell
            && let Some(node) = self.composer.window_node(window)
        {
            self.tree.event(node, tree_request(request));
            return;
        }
        self.window_request_directly(window, request);
    }

    /// Applies a request through the record, for a session with no shell. A request that changes nothing
    /// (maximising a maximised window) is dropped, and a window that is gone is a no-op.
    fn window_request_directly(&mut self, window: WindowId, request: WindowRequest) {
        if self.wm.window(window).is_none() {
            log::trace!(area = "toplevel", %window, ?request, "a request for a window that is gone");
            return;
        }
        let effects = match request {
            WindowRequest::Maximise(on) => self.wm.set_maximised(window, on),
            WindowRequest::Fullscreen(on) => self.with_seat(|wm, seat| wm.set_fullscreen(window, on, seat)),
            WindowRequest::Minimise(on) => self.with_seat(|wm, seat| wm.set_minimised(window, on, seat)),
            WindowRequest::Activate => {
                // The caller is privileged (a taskbar's click, or an earned token): the plan's rule against a
                // client demanding focus is about a client, not about the user's own action.
                log::debug!(area = "toplevel", %window, "activated");
                self.with_seat(|wm, seat| wm.activate(window, true, seat))
            }
            WindowRequest::Raise => self.with_seat(|wm, seat| wm.raise(window, seat)),
            WindowRequest::Lower => self.with_seat(|wm, seat| wm.lower(window, seat)),
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
        if self.composer.holder_attached() {
            return Err(WindowChangeRefusal::Refused);
        }
        let effects = self.wm.move_resize(window, geometry).map_err(Self::refusal_of)?;
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
        if self.composer.holder_attached() {
            return Err(WindowChangeRefusal::Refused);
        }
        let effects = self.wm.move_to_output(window, output).map_err(Self::refusal_of)?;
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

    /// Sends SIGKILL to the process behind a window's client, as the shell's `kill` call asks. The signal goes through
    /// a pidfd opened for the process the client connected as, so a pid that was reused since is never signalled; the
    /// compositor's own process and init are refused, as are a window whose process is unknown or has disconnected.
    /// `false` when nothing was signalled.
    pub(crate) fn kill_window(&mut self, window: WindowId) -> bool {
        let identity = match self.window_process_for_control(window) {
            Some(crate::WindowProcess::Connected(identity)) => identity,
            other => {
                log::debug!(area = "toplevel", %window, ?other, "a kill for a window with no process the compositor can name");
                return false;
            }
        };
        match identity.kill_target() {
            Ok(target) => target.kill(),
            Err(refusal) => {
                log::debug!(area = "toplevel", %window, ?refusal, "a kill was refused");
                false
            }
        }
    }

    /// Runs the method calls the shell made on window nodes. `close` asks the client to close, and the client
    /// decides whether it exits. `kill` ends its process.
    pub(crate) fn run_window_calls(&mut self, calls: Vec<TreeCall>) {
        for call in calls {
            let Some(window) = self.composer.window_of_node(call.node) else {
                continue;
            };
            match call.method.as_str() {
                "close" => {
                    self.close_window(window);
                }
                "kill" => {
                    self.kill_window(window);
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
        let output = self.wm.window(window)?.output?;
        self.wm.work_area(output)
    }

    /// Brings one window's pending configure in line with its node.
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
        let answer = x11_answer(state, self.composer.window_frame(window), size);
        // `_NET_WM_STATE_HIDDEN` follows the window's visibility, which `set_visible` owns below.
        self.sync_x11_flags(window, |flags| X11Flags {
            maximised: answer.maximised,
            fullscreen: answer.fullscreen,
            ..flags
        });
        let Some(Managed::X11(surface)) = self.windows.get(&window).map(|entry| &entry.surface) else {
            return;
        };
        if let Some(rect) = answer.frame
            && let Err(error) = surface.configure(Some(x11_geometry(rect)))
        {
            log::debug!(area = "toplevel", %error, %window, "the X11 configure failed");
        }
        self.set_visible(window, !answer.hidden);
        log::debug!(
            area = "toplevel",
            %window,
            maximised = answer.maximised,
            fullscreen = answer.fullscreen,
            minimised = answer.hidden,
            rect = ?answer.frame,
            "configured an X11 window from the shell's node"
        );
    }

    /// Writes the `_NET_WM_STATE` flags an X11 window is told. `update` turns the flags the window holds into the
    /// ones it should, and only the atoms that differ are written, so each caller owns the flags it names and
    /// leaves the rest as they are: a window the table holds no X11 surface for is left alone.
    pub(crate) fn sync_x11_flags(&self, window: WindowId, update: impl FnOnce(X11Flags) -> X11Flags) {
        let Some(Managed::X11(surface)) = self.windows.get(&window).map(|entry| &entry.surface) else {
            return;
        };
        let writes = X11Flags::of(surface).writes_to(update(X11Flags::of(surface)));
        if let Some(maximised) = writes.maximised
            && let Err(error) = surface.set_maximized(maximised)
        {
            log::debug!(area = "toplevel", %error, %window, "the X11 maximised state was not sent");
        }
        if let Some(fullscreen) = writes.fullscreen
            && let Err(error) = surface.set_fullscreen(fullscreen)
        {
            log::debug!(area = "toplevel", %error, %window, "the X11 fullscreen state was not sent");
        }
        if let Some(hidden) = writes.hidden
            && let Err(error) = surface.set_hidden(hidden)
        {
            log::debug!(area = "toplevel", %error, %window, "the X11 hidden state was not sent");
        }
    }
}

/// Who decides a window's requests, placement and focus ([`State::decider`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Decider {
    /// The shell, through an event on the window's node.
    Shell,
    /// The window-manager provider, applied directly. `NullWm` when the session has none.
    Provider,
}

/// Whether the shell decides: the session is unlocked, a holder is attached to the tree and the window has a node.
const fn decide(unlocked: bool, holder_attached: bool, has_node: bool) -> Decider {
    if unlocked && holder_attached && has_node {
        Decider::Shell
    } else {
        Decider::Provider
    }
}

/// What an X11 window is told for the state its node holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct X11Answer {
    /// `_NET_WM_STATE_MAXIMIZED_*`.
    maximised: bool,
    /// `_NET_WM_STATE_FULLSCREEN`.
    fullscreen: bool,
    /// `_NET_WM_STATE_HIDDEN`.
    hidden: bool,
    /// The configure to send: the node's frame with the size the shell wrote, or `None` when the node has no frame.
    frame: Option<Rect>,
}

/// The X11 answer for the shell's flags, the node's frame and the size the shell wrote.
fn x11_answer(state: WindowState, frame: Option<Rect>, size: Option<(u32, u32)>) -> X11Answer {
    let frame = match (frame, size) {
        (Some(frame), Some((width, height))) => Some(Rect::new(frame.x, frame.y, width, height)),
        (frame, _) => frame,
    };
    X11Answer {
        maximised: state.maximised,
        fullscreen: state.fullscreen,
        hidden: state.minimised,
        frame,
    }
}

/// The `_NET_WM_STATE` flags the compositor owns on an X11 window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct X11Flags {
    /// `_NET_WM_STATE_MAXIMIZED_VERT` and `_HORZ`.
    pub(crate) maximised: bool,
    /// `_NET_WM_STATE_FULLSCREEN`.
    pub(crate) fullscreen: bool,
    /// `_NET_WM_STATE_HIDDEN`.
    pub(crate) hidden: bool,
}

/// The flags to write to an X11 window: `None` for one that already holds the wanted value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct X11Writes {
    pub(crate) maximised: Option<bool>,
    pub(crate) fullscreen: Option<bool>,
    pub(crate) hidden: Option<bool>,
}

impl X11Flags {
    /// The writes that take a window holding `self` to `wanted`.
    pub(crate) fn writes_to(self, wanted: Self) -> X11Writes {
        let change = |current: bool, wanted: bool| (current != wanted).then_some(wanted);
        X11Writes {
            maximised: change(self.maximised, wanted.maximised),
            fullscreen: change(self.fullscreen, wanted.fullscreen),
            hidden: change(self.hidden, wanted.hidden),
        }
    }

    /// What a window that maps with these flags already set is asking for: one ask per flag it holds.
    pub(crate) fn asks(self) -> Vec<WindowAsk> {
        let mut asks = Vec::new();
        if self.maximised {
            asks.push(WindowAsk::Maximise(true));
        }
        if self.fullscreen {
            asks.push(WindowAsk::Fullscreen(true));
        }
        if self.hidden {
            asks.push(WindowAsk::Minimise(true));
        }
        asks
    }

    /// The flags an X11 surface holds now.
    pub(crate) fn of(surface: &X11Surface) -> Self {
        Self {
            maximised: surface.is_maximized(),
            fullscreen: surface.is_fullscreen(),
            hidden: surface.is_hidden(),
        }
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

#[cfg(test)]
mod tests {
    use gnoblin_backend::x11::WindowAsk;
    use gnoblin_types::Rect;
    use gnoblin_wm_api::WindowState;

    use super::{Decider, X11Answer, X11Flags, X11Writes, decide, request_of_ask, tree_request, x11_answer};
    use crate::tree_frame::TreeRequest;
    use gnoblin_wayland_types::window::WindowRequest;

    const FRAME: Rect = Rect::new(40, 50, 300, 200);

    // An X11 window's maximise, fullscreen and minimise requests, from a client, a taskbar or the control surface,
    // are the same node requests a Wayland toplevel's are: the shell reads them by name.
    #[test]
    fn an_x11_window_state_request_becomes_the_node_request_the_shell_reads() {
        let named = |request: WindowRequest| tree_request(request).name();
        assert_eq!(named(WindowRequest::Maximise(true)), "maximize");
        assert_eq!(named(WindowRequest::Maximise(false)), "unmaximize");
        assert_eq!(named(WindowRequest::Fullscreen(true)), "fullscreen");
        assert_eq!(named(WindowRequest::Fullscreen(false)), "unfullscreen");
        assert_eq!(named(WindowRequest::Minimise(true)), "minimize");
        assert_eq!(named(WindowRequest::Minimise(false)), "unminimize");
        assert_eq!(named(WindowRequest::Close), "close");
        assert_eq!(tree_request(WindowRequest::Activate), TreeRequest::Activate);
        assert_eq!(tree_request(WindowRequest::Raise), TreeRequest::Raise);
        assert_eq!(tree_request(WindowRequest::Lower), TreeRequest::Lower);
    }

    // With a holder attached the request is the shell's; with none, a locked session or a window with no node it is
    // the provider's, which applies it (`gnoblin-wm-api` holds the contract, `gnoblin-std-wm` the stock provider).
    #[test]
    fn a_request_goes_to_the_shell_only_while_it_holds_the_tree_unlocked_and_the_window_has_a_node() {
        assert_eq!(decide(true, true, true), Decider::Shell);
        assert_eq!(decide(true, false, true), Decider::Provider, "no holder is attached");
        assert_eq!(decide(false, true, true), Decider::Provider, "the session is covered");
        assert_eq!(
            decide(true, true, false),
            Decider::Provider,
            "the window has no node yet"
        );
    }

    // The shell writes the three flags and a size; an X11 window hears them as its `_NET_WM_STATE` and a configure
    // at the frame the shell placed it in.
    #[test]
    fn the_shells_flags_and_size_become_the_x11_windows_state_and_frame() {
        let state = WindowState {
            maximised: true,
            fullscreen: false,
            minimised: false,
        };
        assert_eq!(
            x11_answer(state, Some(FRAME), Some((640, 480))),
            X11Answer {
                maximised: true,
                fullscreen: false,
                hidden: false,
                frame: Some(Rect::new(40, 50, 640, 480)),
            },
            "the size the shell wrote replaces the frame's, and the position stays"
        );
        let fullscreen = WindowState {
            maximised: false,
            fullscreen: true,
            minimised: false,
        };
        let answer = x11_answer(fullscreen, Some(FRAME), Some((1920, 1080)));
        assert!(answer.fullscreen && !answer.maximised && !answer.hidden);
    }

    #[test]
    fn a_minimised_x11_window_is_told_it_is_hidden() {
        let state = WindowState {
            minimised: true,
            ..WindowState::default()
        };
        let answer = x11_answer(state, Some(FRAME), None);
        assert!(answer.hidden);
        assert_eq!(
            answer.frame,
            Some(FRAME),
            "with no size written the frame is kept as it is"
        );
        assert!(!x11_answer(WindowState::default(), Some(FRAME), None).hidden);
    }

    // A window whose node has no frame yet is sent its flags and no configure: there is nowhere to put it.
    #[test]
    fn an_x11_window_with_no_frame_is_sent_no_configure() {
        let answer = x11_answer(WindowState::default(), None, Some((640, 480)));
        assert_eq!(answer.frame, None);
    }

    // An X11 client's own ask is the request an `xdg_toplevel` makes for the same thing, so a shell reads one name
    // for both and the record applies one rule to both.
    #[test]
    fn an_x11_clients_own_ask_is_the_request_an_xdg_toplevel_makes() {
        assert_eq!(request_of_ask(WindowAsk::Maximise(true)), WindowRequest::Maximise(true));
        assert_eq!(
            request_of_ask(WindowAsk::Maximise(false)),
            WindowRequest::Maximise(false)
        );
        assert_eq!(
            request_of_ask(WindowAsk::Fullscreen(true)),
            WindowRequest::Fullscreen(true)
        );
        assert_eq!(
            request_of_ask(WindowAsk::Fullscreen(false)),
            WindowRequest::Fullscreen(false)
        );
        assert_eq!(request_of_ask(WindowAsk::Minimise(true)), WindowRequest::Minimise(true));
        assert_eq!(
            request_of_ask(WindowAsk::Minimise(false)),
            WindowRequest::Minimise(false)
        );
        assert_eq!(request_of_ask(WindowAsk::Close), WindowRequest::Close);
        assert_eq!(
            tree_request(request_of_ask(WindowAsk::Maximise(true))).name(),
            "maximize"
        );
        assert_eq!(
            tree_request(request_of_ask(WindowAsk::Minimise(true))).name(),
            "minimize"
        );
    }

    // Only the `_NET_WM_STATE` atoms that differ are written: a state the window already has costs the X server no
    // property change, and a flag the caller does not own is left as the window has it.
    #[test]
    fn only_the_x11_flags_that_change_are_written() {
        let current = X11Flags {
            maximised: true,
            fullscreen: false,
            hidden: false,
        };
        assert_eq!(
            current.writes_to(current),
            X11Writes::default(),
            "nothing changed, nothing written"
        );
        assert_eq!(
            current.writes_to(X11Flags {
                maximised: false,
                fullscreen: true,
                hidden: false,
            }),
            X11Writes {
                maximised: Some(false),
                fullscreen: Some(true),
                hidden: None,
            }
        );
        assert_eq!(
            current.writes_to(X11Flags {
                hidden: true,
                ..current
            }),
            X11Writes {
                hidden: Some(true),
                ..X11Writes::default()
            }
        );
    }

    // A window that maps with `_NET_WM_STATE` already set, or with `WM_HINTS` initial state iconic, is asking for
    // that state: each flag it holds is the ask the same message would have made, and a plain window asks nothing.
    #[test]
    fn a_window_that_maps_with_state_set_asks_for_it() {
        assert_eq!(X11Flags::default().asks(), Vec::<WindowAsk>::new());
        assert_eq!(
            X11Flags {
                maximised: true,
                fullscreen: true,
                hidden: true,
            }
            .asks(),
            vec![
                WindowAsk::Maximise(true),
                WindowAsk::Fullscreen(true),
                WindowAsk::Minimise(true)
            ]
        );
    }
}
