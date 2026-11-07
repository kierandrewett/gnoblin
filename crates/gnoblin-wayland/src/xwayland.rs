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
//!   [`gnoblin_wm::Event`]s.
//! - **The policy.** Every callback builds a plain
//!   [`gnoblin_backend::x11::WindowEvent`], asks the configured
//!   [`gnoblin_backend::x11::Windows`] what it means and performs the answer: the effects
//!   go through [`State::apply`], and the X11 requests through the surface.
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
//! - The pixels of an X11 window do not reach the scene yet: the window is managed,
//!   configured and focused, and its buffer arrives on the Xwayland client's `wl_surface`
//!   one commit path later (the X11 damage slice). A window with no content contributes
//!   no layer, which is the same rule a Wayland window with no buffer follows.
//! - Interactive move and resize from an X11 client's title bar are reported by Smithay
//!   and answered with nothing; the same drag works from the Wayland side.
//! - Maximise, fullscreen, minimise, above/below, sticky and shade requests from X11
//!   clients are not carried into the policy, so an X11 client that asks for them keeps
//!   its floating geometry.

use std::process::Stdio;
use std::time::Instant;

use smithay::input::Seat;
use smithay::input::dnd::DndGrabHandler;
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::wayland_server::Client;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Rectangle, SERIAL_COUNTER};

use smithay::wayland::xwayland_keyboard_grab::{XWaylandKeyboardGrab, XWaylandKeyboardGrabHandler};
use smithay::wayland::xwayland_shell::{XWaylandShellHandler, XWaylandShellState};
use smithay::xwayland::xwm::settings::Value as SettingValue;
use smithay::xwayland::xwm::{Reorder, ResizeEdge, WmWindowProperty, X11Surface, X11Wm, XwmId};
use smithay::xwayland::{XWayland, XWaylandClientData, XwmHandler};

use gnoblin_backend::x11::{self, Launch, Restart, Session, Start, WindowEvent, WindowMeta, X11Request};
use gnoblin_types::Rect;
use gnoblin_wm::Event;

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
    fn new(session: Box<dyn Session>, windows: Box<dyn x11::Windows>) -> Self {
        Self {
            session,
            windows,
            process: None,
            client: None,
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
        self.process = None;
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
    pub(crate) fn x11_window_gone(&mut self, window: gnoblin_types::WindowId) {
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
        self.composer.remove_window(window);
        let effects = self.wm.apply(Event::WindowClosed { window });
        self.apply(effects);
        self.publish();
    }

    /// Asks the policy what an X11 event means and performs the answer.
    fn x11_event(&mut self, event: WindowEvent<'_>, surface: Option<&X11Surface>) {
        let Some(mut running) = self.x11.take() else {
            return;
        };
        let outcome = running.windows.handle(event, &mut self.wm);
        if let (Some(window), Some(surface)) = (outcome.window, surface) {
            running.surfaces.insert(window, surface.clone());
            if !self.windows.contains_key(&window) {
                let geometry = self.wm.window(window).map(|managed| managed.geometry);
                self.windows
                    .insert(window, crate::state::WindowEntry::x11(surface.clone()));
                if let Some(geometry) = geometry {
                    self.composer.insert_window(window, geometry);
                }
            }
        }
        self.apply(outcome.effects);
        for command in outcome.commands {
            match command {
                X11Request::SetMapped { window, mapped } => {
                    if let Some(surface) = running.surfaces.get(&window).cloned()
                        && let Err(error) = surface.set_mapped(mapped)
                    {
                        log::debug!(area = "xwayland", %error, %window, "a mapping request failed");
                    }
                }
                X11Request::Close { window } => {
                    if let Some(surface) = running.surfaces.get(&window).cloned()
                        && let Err(error) = surface.close()
                    {
                        log::debug!(area = "xwayland", %error, %window, "a close request failed");
                    }
                }
            }
        }
        // An X11 window exists in the table exactly while the manager holds it, so an
        // unmap or a destroy leaves nothing behind.
        if let Some(window) = outcome.window
            && self.wm.window(window).is_none()
        {
            self.windows.remove(&window);
            self.composer.remove_window(window);
            running.surfaces.remove(&window);
        }
        self.x11 = Some(running);
        self.publish();
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
