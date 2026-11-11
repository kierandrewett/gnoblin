//! Xwayland: the process, the X11 window manager and the policy seam.
//!
//! `gnoblin-xwayland` owns the decisions: the start parameters, the restart policy and
//! the X11 window table. This module is the Wayland half, which has to live here because
//! it needs the display, the event loop and the seat:
//!
//! - **The process.** [`smithay::xwayland::XWayland::spawn`] starts the server, creates
//!   the X11 socket pair and the `DISPLAY` lock file, and inserts the Xwayland client into
//!   this display. The readiness pipe is polled from the frame loop, so an instance that
//!   never becomes ready is caught by the policy's deadline rather than by a hang.
//! - **The window manager.** [`smithay::xwayland::xwm::X11Wm`] is the X11 window manager
//!   proper: it takes the root window, honours the ICCCM and EWMH properties and reports
//!   what X11 clients do. Its callbacks are the only place X11 windows become
//!   [`gnoblin_backend::x11::NodeChange`]s.
//! - **The policy.** Every callback builds a plain
//!   [`gnoblin_backend::x11::WindowEvent`], asks the configured
//!   [`gnoblin_backend::x11::Windows`] what it means and performs the answer: the changes
//!   open, rename, place and close the same window node a Wayland toplevel gets, and the
//!   X11 requests go through the surface.
//!
//! # Failure modes first
//!
//! Each entry names the test in `crates/gnoblin-xwayland/tests/real_xwayland.rs` unless it
//! says otherwise.
//!
//! 1. **A server that never starts hangs the session.** The policy's startup deadline
//!    turns a silent instance into a failed attempt: `a_start_that_never_becomes_ready_fails`
//!    in the policy's own module, and the restart test covers the loop.
//! 2. **A crash leaves the old windows behind.** A disconnect removes every X11 window
//!    from the manager before the next attempt, because the clients that owned them went
//!    with the server: `killing_xwayland_restarts_it_and_a_fresh_client_works`.
//! 3. **A restart reuses the dead display.** The new instance reports the number it took
//!    and the old display's clients are gone; nothing pretends otherwise.
//! 4. **The X server outlives the session.** Dropping the session drops the process handle
//!    and the X11 connection, which is what ends the server; a session that is never
//!    started has no process at all.
//! 5. **A menu takes the keyboard.** The keyboard grab handler asks the policy before it
//!    installs a grab, so an override-redirect window's grab is refused when the
//!    configuration does not allow grabs.
//!
//! # What this slice does not do
//!
//! - An X11 window's pixels reach the scene through the commit of its `wl_surface`, by the
//!   same path a toplevel's do ([`State::x11_committed`]): the node, its title and
//!   application id, its ready announcement and its geometry are the ones a Wayland window
//!   has. A window with no content contributes no layer, which is the same rule a Wayland
//!   window with no buffer follows.
//! - Interactive move and resize from an X11 client's title bar are reported by Smithay
//!   and answered with nothing; the same drag works from the Wayland side.
//! - Maximise, fullscreen, minimise, above/below, sticky and shade requests from X11
//!   clients are not carried into the policy, so an X11 client that asks for them keeps
//!   its floating geometry.

use std::os::unix::io::OwnedFd;
use std::process::Stdio;
use std::time::Instant;

use smithay::input::Seat;
use smithay::input::dnd::DndGrabHandler;
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::wayland_server::Client;
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Rectangle, SERIAL_COUNTER};

use smithay::wayland::selection::SelectionTarget;
use smithay::wayland::selection::data_device::{
    clear_data_device_selection, request_data_device_client_selection, set_data_device_selection,
};
use smithay::wayland::selection::primary_selection::{
    clear_primary_selection, request_primary_client_selection, set_primary_selection,
};
use smithay::wayland::xwayland_keyboard_grab::{
    XWaylandKeyboardGrab, XWaylandKeyboardGrabHandler, XWaylandKeyboardGrabState,
};
use smithay::wayland::xwayland_shell::{XWaylandShellHandler, XWaylandShellState};
use smithay::xwayland::xwm::settings::Value as SettingValue;
use smithay::xwayland::xwm::{Reorder, ResizeEdge, WmWindowProperty, X11Surface, X11Wm, XwmId};
use smithay::xwayland::{XWayland, XWaylandClientData, XwmHandler};

use gnoblin_backend::x11::{
    self, Launch, NodeChange, Opened, Restart, Session, Start, WindowEvent, WindowMeta, X11Request,
};
use gnoblin_layers::{Band, LayerProvider, LayerScope, LayerSpec, LocalName};
use gnoblin_types::{Rect, WindowId};
use gnoblin_wm::{ActivationDecision, Effect, Event, WindowProtocol, WindowSpec, Wm};

use crate::selection_bridge::ToWayland;
use crate::state::State;

use crate::log;

/// One Xwayland session, as the compositor runs it.
///
/// The policy half is created once and lives as long as the session; an attempt that
/// dies leaves the policy in place, so the retry ladder and the window manager's
/// knowledge of what was open survive a restart.
pub struct XwaylandSession {
    /// The start and restart policy.
    pub session: Box<dyn Session>,
    /// The X11 window policy.
    pub windows: Box<dyn x11::Windows>,
}

impl XwaylandSession {
    /// A session from the two halves a caller built.
    #[must_use]
    pub fn new(session: Box<dyn Session>, windows: Box<dyn x11::Windows>) -> Self {
        Self { session, windows }
    }
}

impl std::fmt::Debug for XwaylandSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("XwaylandSession")
    }
}

/// The running attempt, and the tables that belong to it.
///
/// Everything here is dropped with the attempt: a new instance has a new display, a new
/// X11 connection and a new set of windows, and keeping any of the old state would let a
/// destroyed client's window be configured through a socket that is gone.
pub(crate) struct Running {
    /// The start and restart policy.
    session: Box<dyn Session>,
    /// The X11 window policy.
    windows: Box<dyn x11::Windows>,
    /// The running server, while there is one.
    process: Option<XWayland>,
    /// The Xwayland client, until the window manager starts on it.
    client: Option<Client>,
    /// The Xwayland client's id, kept after the window manager takes the client, so a stop
    /// can release it.
    client_id: Option<ClientId>,
    /// The X11 window manager, once the server answered ready.
    wm: Option<X11Wm>,
    /// The X11 surfaces the manager holds, by policy identity.
    pub(crate) surfaces: std::collections::BTreeMap<gnoblin_types::WindowId, X11Surface>,
    /// When the next attempt may start.
    next_start: Option<Instant>,
    /// When the running attempt has to have become ready.
    deadline: Option<Instant>,
    /// Whether the policy refused every attempt, or gave up on them.
    stopped: bool,
    /// Whether the instance died and the next turn should account for it.
    dead: bool,
}

impl Running {
    /// Performs one request the X11 policy answered with: the mapping it chose, or the
    /// close a person asked for. A surface that is gone has nothing to send to.
    fn send(&self, command: X11Request) {
        match command {
            X11Request::SetMapped { window, mapped } => {
                if let Some(surface) = self.surfaces.get(&window).cloned()
                    && let Err(error) = surface.set_mapped(mapped)
                {
                    log::debug!(area = "xwayland", %error, %window, "a mapping request failed");
                }
            }
            X11Request::Close { window } => {
                if let Some(surface) = self.surfaces.get(&window).cloned()
                    && let Err(error) = surface.close()
                {
                    log::debug!(area = "xwayland", %error, %window, "a close request failed");
                }
            }
        }
    }

    fn new(session: Box<dyn Session>, windows: Box<dyn x11::Windows>) -> Self {
        Self {
            session,
            windows,
            process: None,
            client: None,
            client_id: None,
            wm: None,
            surfaces: std::collections::BTreeMap::new(),
            next_start: None,
            deadline: None,
            stopped: false,
            dead: false,
        }
    }

    /// Whether an attempt is up and its window manager is running.
    pub(crate) fn is_ready(&self) -> bool {
        self.wm.is_some()
    }

    /// The display number of the running instance, while there is one.
    pub(crate) fn display(&self) -> Option<u32> {
        self.process.as_ref().map(XWayland::display_number)
    }

    /// Whether the session holds an X11 surface for a window.
    pub(crate) fn surface(&self, window: gnoblin_types::WindowId) -> Option<&X11Surface> {
        self.surfaces.get(&window)
    }

    /// The managed window an X11 window id names, when the window table has one.
    ///
    /// The table is `gnoblin-xwayland`'s policy and this is the one question
    /// `mutter_x11_interop` has of it: an X11 dialog's parent is named by its X11 id,
    /// and only the table knows which of this session's windows that is.
    pub(crate) fn window_for_x11(&self, xwindow: u32) -> Option<gnoblin_types::WindowId> {
        self.windows.managed_window(xwindow)
    }

    /// Advances the session: starts it, watches it and fails it.
    fn advance(&mut self, handle: &LoopHandle<'static, State>, state: &mut State, now: Instant) {
        if let Some(process) = self.process.as_mut() {
            // A dead instance is noticed by the X11 window manager's disconnect, and a
            // start that never answers is noticed by the deadline.
            if self.dead {
                self.fail(state, now, "the X11 connection closed");
                return;
            }
            match process.take_socket() {
                Ok(Some(socket)) => {
                    self.ready(handle, state, socket, now);
                }
                Ok(None) => {
                    if self.deadline.is_some_and(|deadline| now >= deadline) {
                        self.fail(state, now, "the server did not become ready in time");
                    }
                }
                Err(error) => self.fail(state, now, &format!("the readiness pipe failed: {error}")),
            }
            return;
        }
        if self.stopped {
            return;
        }
        if self.next_start.is_some_and(|at| now < at) {
            return;
        }
        match self.session.start(now) {
            Start::Disabled(reason) => {
                // Disabled is not a temporary condition: the configuration or the
                // machine will not change under a running session.
                self.stopped = true;
                log::warn!(area = "xwayland", "{reason}");
            }
            Start::Spawn(launch) => self.spawn(state, launch, now),
        }
    }

    /// Starts the server with the policy's parameters.
    ///
    /// Xwayland's stdout and stderr are inherited, so the reason a server exits (an
    /// unknown switch, a missing display) reaches the compositor's log. Discarding them
    /// left a failed start with only "did not become ready" and no cause.
    fn spawn(&mut self, state: &mut State, launch: Launch, now: Instant) {
        let args = launch.args.clone();
        let env = launch.env.clone();
        let result = XWayland::spawn(
            &state.display_handle,
            launch.display,
            env,
            args,
            launch.open_abstract_socket,
            Stdio::inherit(),
            Stdio::inherit(),
            |_| {},
        );
        match result {
            Ok((process, client)) => {
                if let Some(data) = client.get_data::<XWaylandClientData>() {
                    // The X11 client scale is what makes a fractional scale mean "one
                    // X11 pixel is this many logical pixels" instead of a stretched
                    // surface.
                    let scale = launch.scale.map_or_else(|| 1.0, gnoblin_types::Scale::as_f64);
                    data.compositor_state.set_client_scale(scale);
                }
                let number = process.display_number();
                log::info!(area = "xwayland", display_number = number, "starting");
                self.process = Some(process);
                self.client_id = Some(client.id());
                self.client = Some(client);
                self.deadline = Some(now + self.session.startup_deadline());
            }
            Err(error) => {
                log::warn!(area = "xwayland", "the server could not be started: {error}");
                self.fail(state, now, "the server could not be started");
            }
        }
    }

    /// The server answered ready: start the X11 window manager on it.
    fn ready(
        &mut self,
        handle: &LoopHandle<'static, State>,
        state: &mut State,
        socket: std::os::unix::net::UnixStream,
        now: Instant,
    ) {
        let Some(client) = self.client.take() else {
            self.fail(state, now, "the server answered ready twice");
            return;
        };
        let number = self.process.as_ref().map_or(0, XWayland::display_number);
        self.deadline = None;
        self.session.ready(number, now);
        match X11Wm::start_wm(handle.clone(), &state.display_handle, socket, client) {
            Ok(wm) => {
                log::info!(area = "xwayland", display_number = number, "ready");
                self.wm = Some(wm);
                self.publish_settings();
            }
            Err(error) => {
                let reason = format!("the X11 window manager could not start: {error}");
                self.fail(state, now, &reason);
            }
        }
    }

    /// Hands the XSETTINGS owner's values to the X connection, and reports the answer.
    fn publish_settings(&mut self) {
        let settings = self.session.settings();
        let Some(wm) = self.wm.as_mut() else {
            return;
        };
        if settings.is_empty() {
            return;
        }
        let values = settings.into_iter().map(|(name, value)| {
            let value = match value {
                x11::Setting::Integer(value) => SettingValue::Integer(value),
                x11::Setting::Text(value) => SettingValue::String(value),
                x11::Setting::Colour([red, green, blue, alpha]) => SettingValue::Color([
                    u16::from(red) * 257,
                    u16::from(green) * 257,
                    u16::from(blue) * 257,
                    u16::from(alpha) * 257,
                ]),
            };
            (name, value)
        });
        let result = wm.set_xsettings(values).map_err(|error| error.to_string());
        if let Err(reason) = &result {
            log::debug!(area = "xwayland", %reason, "XSETTINGS were not published");
        }
        self.session.settings_published(result);
    }

    /// Ends the attempt, removes its windows and schedules the next one.
    fn fail(&mut self, state: &mut State, now: Instant, why: &str) {
        let number = self.display();
        // The window manager goes first: it owns the X connection, and dropping it stops
        // its event source before the process handle is dropped.
        self.wm = None;
        self.client = None;
        let client = self.client_id.take();
        self.process = None;
        if let Some(client) = client {
            state.release_client(&client);
        }
        self.deadline = None;
        self.dead = false;
        let gone: Vec<_> = self.surfaces.keys().copied().collect();
        for window in gone {
            state.x11_window_gone(window);
        }
        self.surfaces.clear();
        match self.session.failed(now) {
            Restart::RetryAfter(after) => {
                log::warn!(area = "xwayland", %why,
                    display_number = number,
                    retry_ms = after.as_millis(),
                    "the X11 clients of the old display are gone"
                );
                self.next_start = Some(now + after);
            }
            Restart::Stop(reason) => {
                log::error!(area = "xwayland", %why, "{reason}");
                self.stopped = true;
                self.next_start = None;
            }
        }
    }
}

impl State {
    /// Builds the session state for the configured Xwayland session.
    pub(crate) fn start_xwayland(&mut self, session: XwaylandSession) {
        self.x11 = Some(Running::new(session.session, session.windows));
    }

    /// Replaces the X11 session with `session` from the loop: `None` stops the running server.
    ///
    /// A server with X11 windows open is kept and the call says why, so a window is never
    /// closed under its client. The Xwayland-only globals are created the first time a session
    /// is configured; once created they stay advertised, and no X11 client can bind them
    /// without a server.
    pub(crate) fn reconcile_xwayland(&mut self, session: Option<XwaylandSession>) -> Result<(), String> {
        if self.x11.as_ref().is_some_and(|running| !running.surfaces.is_empty()) {
            return Err("X11 windows are open; close them, then commit the settings again".to_owned());
        }
        // Dropping the running attempt drops its server, which ends the X server process.
        let stopped = self.x11.as_ref().and_then(|running| running.client_id.clone());
        self.x11 = None;
        if let Some(client) = stopped {
            self.release_client(&client);
        }
        let Some(session) = session else {
            return Ok(());
        };
        if self.xwayland_shell_state.is_none() {
            self.xwayland_shell_state = Some(XWaylandShellState::new::<Self>(&self.display_handle));
        }
        if self.keyboard_grab_state.is_none() {
            self.keyboard_grab_state = Some(XWaylandKeyboardGrabState::new::<Self>(&self.display_handle));
        }
        self.start_xwayland(session);
        Ok(())
    }

    /// Releases the client of a server that was just dropped. The server removes the
    /// connection only in a cleanup that runs on a dispatch of that client, and nothing else
    /// wakes the loop for it, so the old process would linger until some other client event.
    fn release_client(&mut self, client: &ClientId) {
        let Some(mut display) = self.take_display() else {
            return;
        };
        let result = display.backend().dispatch_single_client(self, client.clone());
        self.restore_display(display);
        if let Err(error) = result {
            log::warn!(
                area = "xwayland",
                "releasing the stopped Xwayland client failed: {error}"
            );
        }
    }

    /// Drives the session: one call per loop turn.
    pub(crate) fn poll_xwayland(&mut self, handle: &LoopHandle<'static, State>, now: Instant) {
        let Some(mut running) = self.x11.take() else {
            return;
        };
        running.advance(handle, self, now);
        self.x11 = Some(running);
    }

    /// Whether an X11 window manager is running.
    #[must_use]
    pub fn xwayland_ready(&self) -> bool {
        self.x11.as_ref().is_some_and(Running::is_ready)
    }

    /// The display number X11 clients can connect to, while the server is up.
    #[must_use]
    pub fn xwayland_display(&self) -> Option<u32> {
        self.x11.as_ref().and_then(Running::display)
    }

    /// Removes an X11 window from the manager after its server went away.
    pub(crate) fn x11_window_gone(&mut self, window: WindowId) {
        // The composer may still hold a window the manager has forgotten at its unmap, fading
        // or hidden, so it is closed before the manager is asked.
        self.composer.close_window(window);
        if self.windows.remove(&window).is_none() {
            return;
        }
        for output in &self.outputs {
            if let Some(running) = self.x11.as_ref()
                && let Some(surface) = running.surface(window)
                && let Some(wl_surface) = surface.wl_surface()
            {
                output.output.leave(&wl_surface);
            }
        }
        self.release_window(window);
        let effects = self.wm.apply(Event::WindowClosed { window });
        self.apply(effects);
        self.publish();
    }

    /// Asks the policy what an X11 event means and performs the answer.
    fn x11_event(&mut self, event: WindowEvent<'_>, surface: Option<&X11Surface>) {
        let Some(mut running) = self.x11.take() else {
            return;
        };
        let destroyed = match &event {
            WindowEvent::Destroyed(x11) => running.windows.managed_window(*x11),
            _ => None,
        };
        let outcome = running.windows.handle(event);
        // The window manager hears the changes first, as it does for a toplevel: the scene
        // learns a window with the geometry the manager gave it.
        let mut effects = Vec::new();
        for change in &outcome.changes {
            effects.extend(apply_change(&mut self.wm, change));
        }
        if let (Some(window), Some(surface)) = (outcome.window, surface) {
            running.surfaces.insert(window, surface.clone());
            if !self.windows.contains_key(&window) {
                // A deferred open has no geometry yet; the window is placed when the policy
                // answers, as for a Wayland toplevel.
                let geometry = self.wm.window(window).map_or(Rect::EMPTY, |managed| managed.geometry);
                self.windows
                    .insert(window, crate::state::WindowEntry::x11(surface.clone()));
                self.composer.insert_window(window, geometry);
            }
        }
        for change in &outcome.changes {
            if let NodeChange::Opened(Opened { window, .. })
            | NodeChange::Title { window, .. }
            | NodeChange::AppId { window, .. } = change
            {
                self.publish_x11_text(*window);
            }
        }
        self.apply(effects);
        for command in outcome.commands {
            running.send(command);
        }
        // A destroy closes the window: the composer fades it out and forgets it. An unmap
        // only hides it: the composer fades the last picture and keeps the node and the X11
        // surface, so a remap reuses them. The destroyed window is named before the table
        // forgets it in `handle`.
        if let Some(window) = destroyed {
            self.windows.remove(&window);
            self.composer.close_window(window);
            self.release_window(window);
            running.surfaces.remove(&window);
        } else if let Some(window) = outcome.window
            && self.wm.window(window).is_none()
        {
            self.windows.remove(&window);
            self.composer.unmap_window(window);
            self.release_window(window);
        }
        self.x11 = Some(running);
        self.publish();
    }

    /// Writes a window's title and application id, as the window manager holds them, to its
    /// entry and its node. Each one that changed reaches the shell as a property change.
    fn publish_x11_text(&mut self, window: WindowId) {
        let Some(managed) = self.wm.window(window) else {
            return;
        };
        let (title, app_id) = (managed.title.clone(), managed.app_id.clone());
        self.composer.set_window_text(window, &title, &app_id);
        if let Some(entry) = self.windows.get_mut(&window) {
            entry.title = title;
            entry.app_id = app_id;
        }
    }

    /// An X11 window's surface committed: its content, text and state reach its node.
    ///
    /// The commit is where an X11 window's node first has pixels, so it is also where the
    /// shell hears the window is ready, exactly as for a toplevel's first buffer.
    pub(crate) fn x11_committed(&mut self, window: WindowId, surface: &WlSurface) {
        self.publish_x11_text(window);
        self.publish_window_state(window);
        self.sync_content(window, surface);
        self.publish();
    }

    /// Asks the X11 policy to close a window: the same answer a person closing it gets.
    /// The policy names the close and the surface sends it. `false` when no X11 session
    /// holds the window, so nothing was asked.
    pub(crate) fn close_x11_window(&mut self, window: WindowId) -> bool {
        let Some(mut running) = self.x11.take() else {
            return false;
        };
        let outcome = running.windows.close(window);
        let known = outcome.window.is_some();
        for command in outcome.commands {
            running.send(command);
        }
        self.x11 = Some(running);
        self.publish();
        known
    }
}

/// The spec the window manager takes an X11 window in with.
fn window_spec(opened: &Opened) -> WindowSpec {
    let spec = match opened.transient_for {
        Some(parent) => WindowSpec::transient(opened.window, &opened.app_id, parent),
        None => WindowSpec::normal(opened.window, &opened.app_id),
    };
    // The window is an X11 window, so the manager records it as `surface_x11` rather than
    // a Wayland toplevel.
    WindowSpec {
        protocol: WindowProtocol::X11,
        ..spec.with_title(&opened.title).with_requested(opened.requested)
    }
}

/// Registers the `gnoblin.surface_x11:<window id>` record for a managed X11 window.
///
/// Only the X11 side knows the window is an X11 one, so the record is registered here and
/// not by the manager, under the provider name the X11 policy crate has always used (not
/// the compositor's own, which carries more rights). The record is scoped to the window,
/// so closing the window ends it and the close path needs no extra step.
fn register_surface_record(wm: &mut Wm, window: WindowId) {
    if wm.window(window).is_none() {
        // The manager refused the window, and that refusal is already in the effects.
        return;
    }
    let Ok(local) = LocalName::keyed("surface_x11", &window.get().to_string()) else {
        debug_assert!(false, "a numeric window id always forms a valid local name");
        return;
    };
    let spec = LayerSpec::new(local, LayerScope::Window(Some(window)), Band::Windows);
    let result = wm.register_surface(&LayerProvider::Gnoblin("gnoblin-xwayland"), spec);
    debug_assert!(result.is_ok(), "surface record for {window} refused: {result:?}");
}

/// Applies one change the X11 policy made to the window manager and answers its effects.
fn apply_change(wm: &mut Wm, change: &NodeChange) -> Vec<Effect> {
    match change {
        NodeChange::Opened(opened) => {
            let effects = wm.apply(Event::WindowOpened(window_spec(opened)));
            register_surface_record(wm, opened.window);
            effects
        }
        NodeChange::Activate { window } => wm.apply(Event::ActivationRequested {
            window: *window,
            decision: ActivationDecision::Allow,
        }),
        NodeChange::Closed { window } => wm.apply(Event::WindowClosed { window: *window }),
        NodeChange::Geometry { window, geometry } => wm.apply(Event::MoveResize {
            window: *window,
            geometry: *geometry,
        }),
        NodeChange::Title { window, title } => wm.apply(Event::WindowTitleChanged {
            window: *window,
            title: title.clone(),
        }),
        NodeChange::AppId { window, app_id } => wm.apply(Event::WindowAppIdChanged {
            window: *window,
            app_id: app_id.clone(),
        }),
    }
}

/// The plain data the policy reads from a Smithay surface.
struct Meta {
    x11: u32,
    class: String,
    instance: String,
    title: String,
    transient_for: Option<u32>,
    override_redirect: bool,
    geometry: Rect,
}

impl Meta {
    fn of(window: &X11Surface) -> Self {
        let geometry = window.geometry();
        Self {
            x11: window.window_id(),
            class: window.class(),
            instance: window.instance(),
            title: window.title(),
            transient_for: window.is_transient_for(),
            override_redirect: window.is_override_redirect(),
            geometry: rect_of(geometry),
        }
    }

    fn borrowed(&self) -> WindowMeta<'_> {
        WindowMeta {
            x11: self.x11,
            class: &self.class,
            instance: &self.instance,
            title: &self.title,
            transient_for: self.transient_for,
            override_redirect: self.override_redirect,
            geometry: self.geometry,
        }
    }
}

/// An X11 rectangle in the policy's coordinates.
fn rect_of(rectangle: Rectangle<i32, Logical>) -> Rect {
    Rect::new(
        rectangle.loc.x,
        rectangle.loc.y,
        u32::try_from(rectangle.size.w).unwrap_or(1).max(1),
        u32::try_from(rectangle.size.h).unwrap_or(1).max(1),
    )
}

impl State {
    /// The X11 window manager, when an Xwayland session has one running.
    pub(crate) fn x11_wm(&mut self) -> Option<&mut X11Wm> {
        self.x11.as_mut()?.wm.as_mut()
    }

    /// Makes an X11 client's selection the Wayland one: a compositor-owned source that
    /// offers `mime_types`. A paste from it is answered by the X11 owner.
    fn offer_x11_selection(&mut self, target: SelectionTarget, mime_types: Vec<String>) {
        match target {
            SelectionTarget::Clipboard => {
                set_data_device_selection(&self.display_handle, &self.seat, mime_types.clone(), ());
            }
            SelectionTarget::Primary => {
                set_primary_selection(&self.display_handle, &self.seat, mime_types.clone(), ());
            }
        }
        self.record_selection(target, Some(mime_types));
    }

    /// Clears the Wayland selection after the X11 owner of its mirror gave it up.
    fn clear_wayland_selection(&mut self, target: SelectionTarget) {
        match target {
            SelectionTarget::Clipboard => clear_data_device_selection(&self.display_handle, &self.seat),
            SelectionTarget::Primary => clear_primary_selection(&self.display_handle, &self.seat),
        }
        self.record_selection(target, None);
    }
}

/// Sets the X11 owner of `selection` (`None` gives it up), and does not return until the
/// request has left for the X server.
///
/// Smithay's `new_selection` only queues `SetSelectionOwner` in the connection's write
/// buffer. A reader that sends `ConvertSelection` before that buffer is written finds no
/// owner and reads zero bytes. Waiting for any reply flushes the buffer first, so the
/// read-only query below is the flush: its failure is logged and changes nothing, since
/// the owner request was already written when the query ran.
pub(crate) fn set_x11_selection_owner(
    wm: &mut X11Wm,
    selection: SelectionTarget,
    mime_types: Option<Vec<String>>,
) -> Result<(), smithay::reexports::x11rb::errors::ReplyOrIdError> {
    wm.new_selection(selection, mime_types)?;
    if let Err(error) = wm.get_randr_primary_output() {
        log::warn!(area = "selection", %error, "the X11 write buffer could not be flushed");
    }
    Ok(())
}

impl XwmHandler for State {
    fn xwm_state(&mut self, xwm: XwmId) -> &mut X11Wm {
        // The X11 events only arrive through the window manager's own source, so the
        // manager exists whenever a callback asks for it.
        let running = self.x11.as_mut().expect("an X11 event arrived without a session");
        let wm = running
            .wm
            .as_mut()
            .expect("an X11 event arrived without a window manager");
        debug_assert_eq!(wm.id(), xwm);
        wm
    }

    fn allow_selection_access(&mut self, _xwm: XwmId, selection: SelectionTarget) -> bool {
        let allowed = self.selection_allowed();
        self.selection_owners.allow_x11_read(selection, allowed)
    }

    fn send_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_type: String, fd: OwnedFd) {
        let allowed = self.selection_allowed();
        if !self.selection_owners.allow_x11_read(selection, allowed) {
            // Dropping the write end is an empty read, the honest answer to a refused read.
            drop(fd);
            return;
        }
        // The data-device and primary-selection modules each define their own error type, so both are
        // reduced to their message here.
        let result: Result<(), String> = match selection {
            SelectionTarget::Clipboard => {
                request_data_device_client_selection(&self.seat, mime_type, fd).map_err(|error| error.to_string())
            }
            SelectionTarget::Primary => {
                request_primary_client_selection(&self.seat, mime_type, fd).map_err(|error| error.to_string())
            }
        };
        if let Err(error) = result {
            log::warn!(
                area = "selection",
                ?selection,
                %error,
                "a Wayland selection could not be read for X11"
            );
        }
    }

    fn new_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_types: Vec<String>) {
        let allowed = self.selection_allowed();
        match self
            .selection_owners
            .x11_changed(selection, Some(mime_types.as_slice()), allowed)
        {
            ToWayland::Offer(types) => self.offer_x11_selection(selection, types),
            ToWayland::Clear => self.clear_wayland_selection(selection),
            ToWayland::Nothing => {}
        }
    }

    fn cleared_selection(&mut self, _xwm: XwmId, selection: SelectionTarget) {
        match self.selection_owners.x11_cleared(selection) {
            ToWayland::Clear => self.clear_wayland_selection(selection),
            ToWayland::Offer(_) | ToWayland::Nothing => {}
        }
    }

    fn new_window(&mut self, _xwm: XwmId, window: X11Surface) {
        let meta = Meta::of(&window);
        self.x11_event(WindowEvent::Created(meta.borrowed()), Some(&window));
    }

    fn new_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        let meta = Meta::of(&window);
        self.x11_event(WindowEvent::Created(meta.borrowed()), Some(&window));
    }

    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        let meta = Meta::of(&window);
        self.x11_event(WindowEvent::MapRequested(meta.borrowed()), Some(&window));
    }

    fn map_window_notify(&mut self, _xwm: XwmId, window: X11Surface) {
        self.x11_event(WindowEvent::Mapped(window.window_id()), Some(&window));
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        let meta = Meta::of(&window);
        self.x11_event(WindowEvent::MapRequested(meta.borrowed()), Some(&window));
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.x11_event(WindowEvent::Unmapped(window.window_id()), Some(&window));
    }

    fn destroyed_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.x11_event(WindowEvent::Destroyed(window.window_id()), Some(&window));
    }

    fn configure_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        x: Option<i32>,
        y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        let current = rect_of(window.geometry());
        let geometry = Rect::new(
            x.unwrap_or(current.x),
            y.unwrap_or(current.y),
            w.unwrap_or(current.width).max(1),
            h.unwrap_or(current.height).max(1),
        );
        self.x11_event(
            WindowEvent::ConfigureRequested {
                x11: window.window_id(),
                geometry,
                position: x.is_some() || y.is_some(),
                size: w.is_some() || h.is_some(),
            },
            Some(&window),
        );
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        geometry: Rectangle<i32, Logical>,
        _above: Option<u32>,
    ) {
        self.x11_event(
            WindowEvent::Configured {
                x11: window.window_id(),
                geometry: rect_of(geometry),
            },
            Some(&window),
        );
    }

    fn property_notify(&mut self, _xwm: XwmId, window: X11Surface, property: WmWindowProperty) {
        match property {
            WmWindowProperty::Title => {
                let title = window.title();
                self.x11_event(
                    WindowEvent::TitleChanged {
                        x11: window.window_id(),
                        title: &title,
                    },
                    Some(&window),
                );
            }
            WmWindowProperty::Class => {
                let meta = Meta::of(&window);
                self.x11_event(WindowEvent::ClassChanged(meta.borrowed()), Some(&window));
            }
            WmWindowProperty::TransientFor => {
                let meta = Meta::of(&window);
                self.x11_event(WindowEvent::ClassChanged(meta.borrowed()), Some(&window));
            }
            WmWindowProperty::WindowType
            | WmWindowProperty::FrameExtents
            | WmWindowProperty::Hints
            | WmWindowProperty::NormalHints
            | WmWindowProperty::Protocols
            | WmWindowProperty::MotifHints
            | WmWindowProperty::Opacity
            | WmWindowProperty::StartupId
            | WmWindowProperty::Pid
            | WmWindowProperty::Other(_) => {}
        }
    }

    fn active_window_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        _timestamp: u32,
        _currently_active_window: Option<X11Surface>,
    ) {
        self.x11_event(WindowEvent::ActivationRequested(window.window_id()), Some(&window));
    }

    fn resize_request(&mut self, _xwm: XwmId, window: X11Surface, _button: u32, _resize_edge: ResizeEdge) {
        log::debug!(
            area = "xwayland",
            window = window.window_id(),
            "a resize request is not interactive yet"
        );
    }

    fn move_request(&mut self, _xwm: XwmId, window: X11Surface, _button: u32) {
        log::debug!(
            area = "xwayland",
            window = window.window_id(),
            "a move request is not interactive yet"
        );
    }

    fn disconnected(&mut self, xwm: XwmId) {
        if let Some(running) = self.x11.as_mut()
            && running.wm.as_ref().is_some_and(|wm| wm.id() == xwm)
        {
            running.dead = true;
        }
    }
}

impl XWaylandShellHandler for State {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        // The global is created with the session, and only the Xwayland client can bind
        // it, so a request cannot arrive without one.
        self.xwayland_shell_state
            .as_mut()
            .expect("xwayland-shell-v1 exists with the session that binds it")
    }

    /// A window and its surface were paired. Xwayland commits a window's first buffer before
    /// it names the pairing, so the commit that carried the pixels found no window; the pairing
    /// is the moment the window's node can first be given them. A surface that has not
    /// committed yet is a no-op here: its commit follows.
    fn surface_associated(&mut self, _xwm: XwmId, wl_surface: WlSurface, _surface: X11Surface) {
        if let Some(window) = self.window_of(&wl_surface) {
            self.x11_committed(window, &wl_surface);
        }
    }
}

impl DndGrabHandler for State {}

impl XWaylandKeyboardGrabHandler for State {
    fn grab(&mut self, surface: WlSurface, seat: Seat<Self>, grab: XWaylandKeyboardGrab<Self>) {
        let allowed = self.x11.as_mut().is_some_and(|running| {
            running
                .surfaces
                .values()
                .find(|window| window.wl_surface().as_ref() == Some(&surface))
                .is_some_and(|window| running.windows.grab_allowed(&window.class(), &window.instance()))
        });
        if !allowed {
            // A grab the configuration refused is not installed: the key that opened a
            // menu reaches the focused window instead.
            log::debug!(area = "xwayland", "a keyboard grab was refused");
            return;
        }
        if let Some(keyboard) = seat.get_keyboard() {
            keyboard.set_grab(self, grab, SERIAL_COUNTER.next_serial());
        }
    }

    fn keyboard_focus_for_xsurface(&self, surface: &WlSurface) -> Option<Self::KeyboardFocus> {
        Some(surface.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use gnoblin_types::{OutputId, WorkspaceId};
    use gnoblin_wm::{WindowKind, WmOptions};

    fn session() -> Wm {
        let mut wm = Wm::new(WmOptions::default());
        wm.apply(Event::WorkspaceAdded {
            workspace: WorkspaceId::new(1),
        });
        wm.apply(Event::OutputConnected {
            output: OutputId::new(1),
            rect: Rect::new(0, 0, 1920, 1080),
        });
        wm
    }

    fn id(x11: u64) -> WindowId {
        WindowId::new((1 << 63) | x11)
    }

    fn open(x11: u64, app_id: &str, title: &str) -> Opened {
        Opened {
            window: id(x11),
            app_id: app_id.to_owned(),
            title: title.to_owned(),
            transient_for: None,
            requested: Rect::new(0, 0, 300, 200),
        }
    }

    /// The rectangle the manager told a window to use, if it told it anything.
    fn configured(effects: &[Effect], window: WindowId) -> Option<Rect> {
        effects.iter().find_map(|effect| match effect {
            Effect::Configure {
                window: configured,
                geometry,
                ..
            } if *configured == window => Some(*geometry),
            _ => None,
        })
    }

    /// Opening is what takes an X11 window into management, and the manager's answer is
    /// the geometry the client is told to adopt.
    #[test]
    fn an_opened_window_is_managed_with_its_title_and_class() {
        let mut wm = session();

        let effects = apply_change(&mut wm, &NodeChange::Opened(open(3, "XTerm", "kieran@fsociety")));

        let managed = wm.window(id(3)).expect("the window is managed");
        assert_eq!(managed.app_id, "XTerm");
        assert_eq!(managed.title, "kieran@fsociety");
        assert_eq!(configured(&effects, id(3)), Some(managed.geometry));
    }

    /// A managed X11 window is a surface record, so layer tooling sees it beside the Wayland
    /// toplevels. Its record is scoped to the window, so closing it ends the record too.
    #[test]
    fn an_opened_x11_window_registers_a_surface_record_that_closing_ends() {
        let mut wm = session();
        let window = id(3);
        let name = format!("gnoblin.surface_x11:{}", window.get());

        apply_change(&mut wm, &NodeChange::Opened(open(3, "XTerm", "t")));

        let record = wm
            .layers()
            .records()
            .find(|record| record.name.to_string() == name)
            .expect("the opened window has a surface record");
        assert_eq!(record.spec.band, Band::Windows);
        assert_eq!(record.spec.scope, LayerScope::Window(Some(window)));

        apply_change(&mut wm, &NodeChange::Closed { window });
        assert!(wm.layers().records().all(|record| record.name.to_string() != name));
    }

    /// A closed window leaves the manager: no window, no focus, no ghost.
    #[test]
    fn a_closed_window_leaves_the_manager() {
        let mut wm = session();
        apply_change(&mut wm, &NodeChange::Opened(open(4, "XTerm", "t")));
        apply_change(&mut wm, &NodeChange::Activate { window: id(4) });
        assert_eq!(wm.focused(), Some(id(4)), "the opened window took focus");

        apply_change(&mut wm, &NodeChange::Closed { window: id(4) });

        assert!(wm.window(id(4)).is_none(), "the manager holds no window");
        assert_eq!(wm.focused(), None);
    }

    /// A title or application id change reaches the manager's record, which is what rules
    /// and the window's node text read.
    #[test]
    fn a_title_and_an_app_id_change_reach_the_manager() {
        let mut wm = session();
        apply_change(&mut wm, &NodeChange::Opened(open(5, "XTerm", "old")));

        apply_change(
            &mut wm,
            &NodeChange::Title {
                window: id(5),
                title: "new".to_owned(),
            },
        );
        apply_change(
            &mut wm,
            &NodeChange::AppId {
                window: id(5),
                app_id: "Renamed".to_owned(),
            },
        );

        let managed = wm.window(id(5)).expect("still managed");
        assert_eq!(managed.title, "new");
        assert_eq!(managed.app_id, "Renamed");
    }

    /// A geometry change reaches the window manager, so the answer is the geometry the work
    /// area allows rather than the geometry the client asked for.
    #[test]
    fn a_geometry_change_goes_through_the_manager() {
        let mut wm = session();
        apply_change(&mut wm, &NodeChange::Opened(open(5, "XTerm", "t")));
        let before = wm.window(id(5)).expect("managed").geometry;

        let effects = apply_change(
            &mut wm,
            &NodeChange::Geometry {
                window: id(5),
                // Far outside the output, so the manager's clamp is what decides.
                geometry: Rect::new(9_000, 9_000, 600, 400),
            },
        );

        let managed = wm.window(id(5)).expect("still managed");
        assert_ne!(managed.geometry, before, "the request moved the window");
        assert_eq!(managed.geometry.size().width, 600);
        assert_eq!(managed.geometry.size().height, 400);
        assert_eq!(
            configured(&effects, id(5)),
            Some(managed.geometry),
            "the client hears the geometry the manager allowed"
        );
    }

    /// A dialog follows the window it is transient for instead of being placed on its
    /// own, which is what keeps a file chooser over the application that opened it.
    #[test]
    fn a_transient_window_follows_its_parent() {
        let mut wm = session();
        apply_change(&mut wm, &NodeChange::Opened(open(6, "App", "main")));
        let mut dialog = open(7, "App", "open file");
        dialog.transient_for = Some(id(6));

        apply_change(&mut wm, &NodeChange::Opened(dialog));

        let child = wm.window(id(7)).expect("the dialog is managed");
        assert_eq!(child.transient_for, Some(id(6)));
        assert_eq!(child.kind, WindowKind::Dialog);
        let parent = wm.window(id(6)).expect("the parent is managed");
        assert_eq!(child.workspace, parent.workspace);
        assert_eq!(child.output, parent.output);
    }
}
