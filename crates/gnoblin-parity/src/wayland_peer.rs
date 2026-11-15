//! The Wayland client both live targets drive.
//!
//! A scenario that must run on the Rust candidate and on stock Mutter can only observe what any compositor shows
//! a client: the events of the core and `xdg-shell` protocols. This module is that client. It maps toplevels and
//! popups, answers pings and configures the way a toolkit does, and records the events a scenario compares: whether
//! a toplevel is `activated`, where a popup was placed, whether a popup was dismissed.
//!
//! The client has no knowledge of either compositor, so the same code produces both sides of a comparison.

use std::collections::BTreeMap;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, poll};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_keyboard, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool,
    wl_surface,
};
use wayland_client::{Connection as WaylandConnection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols::xdg::shell::client::{xdg_popup, xdg_positioner, xdg_surface, xdg_toplevel, xdg_wm_base};

use crate::target::TargetError;

/// How often a waiting probe looks again.
const POLL: Duration = Duration::from_millis(5);
/// The size every toplevel is mapped at.
const WINDOW_SIZE: (i32, i32) = (200, 150);

/// Names one toplevel or popup in the dispatch state, so an event finds the record of the object it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Tag(u32);

/// What the compositor last told a toplevel.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ToplevelFocus {
    /// Whether the last configure carried the `activated` state, which is how a client learns it has focus.
    pub activated: bool,
    /// The size of the last configure, `(0, 0)` while the compositor leaves the size to the client.
    pub size: (i32, i32),
}

/// What the seat's keyboard and pointer delivered to this client, in arrival order. The input probes read it after
/// they inject input through EIS, so it is the only place a compositor's handling of that input is visible.
#[derive(Debug, Default, Clone)]
pub(crate) struct InputLog {
    /// Whether `wl_keyboard.enter` arrived: the keyboard focus is on one of this client's surfaces.
    pub keyboard_entered: bool,
    /// Key edges as `(evdev code, pressed)`.
    pub keys: Vec<(u32, bool)>,
    /// Whether `wl_pointer.enter` arrived: the pointer is over one of this client's surfaces.
    pub pointer_entered: bool,
    /// Pointer positions of `wl_pointer.motion`, in surface coordinates.
    pub motions: Vec<(f64, f64)>,
    /// Button edges as `(evdev code, pressed)`.
    pub buttons: Vec<(u32, bool)>,
    /// Axis events as `(vertical, value)`; `vertical` is false for the horizontal axis.
    pub axes: Vec<(bool, f64)>,
}

/// What the compositor last told a popup.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct PopupState {
    /// The position and size of the last configure, relative to the parent's window geometry.
    pub placed: Option<(i32, i32, i32, i32)>,
    /// Whether the compositor dismissed the popup.
    pub done: bool,
    /// The token of the last `repositioned` event.
    pub repositioned: Option<u32>,
}

/// The client's dispatch state.
#[derive(Default)]
pub(crate) struct Protocol {
    toplevels: BTreeMap<Tag, ToplevelFocus>,
    popups: BTreeMap<Tag, PopupState>,
    /// What the seat's keyboard and pointer delivered.
    input: InputLog,
    /// When each `wl_surface.frame` callback of this client fired, in order.
    frames: Vec<Instant>,
    /// The seat's devices, kept so the objects live as long as the connection.
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    /// What the seat announced it has, once it has.
    capabilities: Option<wl_seat::Capability>,
}

/// A real Wayland client connection, with the globals toplevels and popups need.
pub(crate) struct WaylandPeer {
    target: &'static str,
    queue: EventQueue<Protocol>,
    state: Protocol,
    qh: QueueHandle<Protocol>,
    compositor: wl_compositor::WlCompositor,
    shm: wl_shm::WlShm,
    wm_base: xdg_wm_base::XdgWmBase,
    seat: Option<wl_seat::WlSeat>,
    next_tag: u32,
}

/// One toplevel's protocol objects.
pub(crate) struct Toplevel {
    tag: Tag,
    surface: wl_surface::WlSurface,
    xdg_surface: xdg_surface::XdgSurface,
    toplevel: xdg_toplevel::XdgToplevel,
    /// The buffer the window shows and its size, so a redraw can attach it again.
    buffer: wl_buffer::WlBuffer,
    size: (i32, i32),
}

/// One popup's protocol objects.
pub(crate) struct Popup {
    tag: Tag,
    surface: wl_surface::WlSurface,
    xdg_surface: xdg_surface::XdgSurface,
    popup: xdg_popup::XdgPopup,
}

/// What a popup asks its positioner for. Every field maps to one `xdg_positioner` request.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Place {
    /// The anchor rectangle in the parent's window geometry: x, y, width, height.
    pub anchor_rect: (i32, i32, i32, i32),
    /// The popup's size.
    pub size: (i32, i32),
    pub anchor: xdg_positioner::Anchor,
    pub gravity: xdg_positioner::Gravity,
    /// A further offset of the popup from the anchor point.
    pub offset: (i32, i32),
}

impl Toplevel {
    /// The `xdg_surface` a popup names as its parent.
    pub(crate) fn xdg_surface(&self) -> &xdg_surface::XdgSurface {
        &self.xdg_surface
    }
}

impl Popup {
    /// The `xdg_surface` a nested popup names as its parent.
    pub(crate) fn xdg_surface(&self) -> &xdg_surface::XdgSurface {
        &self.xdg_surface
    }
}

impl WaylandPeer {
    /// Connects to `socket` and binds the globals a toplevel and a popup need. `target` names the compositor in
    /// an error.
    pub(crate) fn connect(target: &'static str, socket: &Path) -> Result<Self, TargetError> {
        let live = |detail: String| TargetError::Live {
            target: target.to_owned(),
            detail,
        };
        let stream = UnixStream::connect(socket)
            .map_err(|error| live(format!("cannot connect to the Wayland socket: {error}")))?;
        let connection = WaylandConnection::from_socket(stream)
            .map_err(|error| live(format!("cannot start a Wayland connection: {error}")))?;
        let (globals, queue) = registry_queue_init::<Protocol>(&connection)
            .map_err(|error| live(format!("the session's global list: {error}")))?;
        let qh = queue.handle();
        let compositor: wl_compositor::WlCompositor = globals
            .bind(&qh, 1..=4, ())
            .map_err(|error| live(format!("the session does not advertise wl_compositor: {error}")))?;
        let shm: wl_shm::WlShm = globals
            .bind(&qh, 1..=1, ())
            .map_err(|error| live(format!("the session does not advertise wl_shm: {error}")))?;
        let wm_base: xdg_wm_base::XdgWmBase = globals
            .bind(&qh, 1..=6, ())
            .map_err(|error| live(format!("the session does not advertise xdg_wm_base: {error}")))?;
        let seat = globals.bind(&qh, 1..=5, ()).ok();
        Ok(Self {
            target,
            queue,
            state: Protocol::default(),
            qh,
            compositor,
            shm,
            wm_base,
            seat,
            next_tag: 0,
        })
    }

    fn live(&self, detail: impl Into<String>) -> TargetError {
        TargetError::Live {
            target: self.target.to_owned(),
            detail: detail.into(),
        }
    }

    fn tag(&mut self) -> Tag {
        self.next_tag += 1;
        Tag(self.next_tag)
    }

    /// Processes every event the session has sent.
    pub(crate) fn roundtrip(&mut self) -> Result<(), TargetError> {
        self.queue
            .roundtrip(&mut self.state)
            .map(|_| ())
            .map_err(|error| self.live(format!("the Wayland round trip failed: {error}")))
    }

    /// Round-trips until `done` holds or `limit` passes, and reports whether it held. A probe that expects a state
    /// to change waits with this, then reports the state either way, so a compositor that never changes it shows up
    /// as a difference in the observation and not as a failed run.
    fn pump_until(&mut self, limit: Duration, done: impl Fn(&Protocol) -> bool) -> Result<bool, TargetError> {
        let deadline = Instant::now() + limit;
        loop {
            self.roundtrip()?;
            if done(&self.state) {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            std::thread::sleep(POLL);
        }
    }

    /// Waits for the compositor to tell `window` that it has (`active`) or has not got focus.
    pub(crate) fn wait_activated(
        &mut self,
        window: &Toplevel,
        active: bool,
        limit: Duration,
    ) -> Result<bool, TargetError> {
        let tag = window.tag;
        self.pump_until(limit, |state| {
            state
                .toplevels
                .get(&tag)
                .is_some_and(|window| window.activated == active)
        })
    }

    /// Waits until `done` holds of what the compositor told `popup`.
    pub(crate) fn wait_popup(
        &mut self,
        popup: &Popup,
        limit: Duration,
        done: impl Fn(&PopupState) -> bool,
    ) -> Result<bool, TargetError> {
        let tag = popup.tag;
        self.pump_until(limit, |state| state.popups.get(&tag).is_some_and(&done))
    }

    /// What the compositor last told this toplevel.
    pub(crate) fn toplevel_state(&self, window: &Toplevel) -> ToplevelFocus {
        self.state.toplevels.get(&window.tag).copied().unwrap_or_default()
    }

    /// What the compositor last told this popup.
    pub(crate) fn popup_state(&self, popup: &Popup) -> PopupState {
        self.state.popups.get(&popup.tag).copied().unwrap_or_default()
    }

    /// Creates a toplevel, lets the session configure it, and maps it with a buffer.
    pub(crate) fn map_toplevel(&mut self, app_id: &str, title: &str) -> Result<Toplevel, TargetError> {
        self.map_toplevel_sized(app_id, title, false)
    }

    /// Creates a toplevel that asks to be fullscreen and maps it at the size the session configures, so it covers
    /// the output and a pointer position anywhere on the output is a position over it. The input probes use it:
    /// two compositors place an ordinary window differently, and a fullscreen window is where they agree.
    pub(crate) fn map_fullscreen(&mut self, app_id: &str, title: &str) -> Result<Toplevel, TargetError> {
        self.map_toplevel_sized(app_id, title, true)
    }

    fn map_toplevel_sized(&mut self, app_id: &str, title: &str, fullscreen: bool) -> Result<Toplevel, TargetError> {
        let tag = self.tag();
        let surface = self.compositor.create_surface(&self.qh, ());
        let xdg_surface = self.wm_base.get_xdg_surface(&surface, &self.qh, ());
        let toplevel = xdg_surface.get_toplevel(&self.qh, tag);
        toplevel.set_app_id(app_id.to_owned());
        toplevel.set_title(title.to_owned());
        if fullscreen {
            toplevel.set_fullscreen(None);
        }
        surface.commit();
        self.roundtrip()?;
        let size = if fullscreen {
            // The configure that follows `set_fullscreen` carries the output's size. A compositor that never sends
            // one leaves the client the default size, which the probe then reports as a difference.
            self.pump_until(Duration::from_secs(3), |state| {
                state.toplevels.get(&tag).is_some_and(|window| window.size.0 > 0)
            })?;
            self.state
                .toplevels
                .get(&tag)
                .map(|window| window.size)
                .filter(|size| size.0 > 0 && size.1 > 0)
                .unwrap_or(WINDOW_SIZE)
        } else {
            WINDOW_SIZE
        };
        let buffer = self.attach_buffer(&surface, size)?;
        Ok(Toplevel {
            tag,
            surface,
            xdg_surface,
            toplevel,
            buffer,
            size,
        })
    }

    /// Binds the seat's keyboard and pointer, which the input probes read. The seat announces what it has after a
    /// round trip, so the devices are made here and not at connect: a probe that needs no input leaves the seat as
    /// it found it. `eis_capabilities` are the capability names the EIS server offered the sender
    /// ([`crate::ei_peer::EiPeer::ready`]): a compositor tells its Wayland clients about a device after it tells the
    /// EIS client, so the seat is given time to advertise the keyboard and pointer the EIS side has.
    pub(crate) fn enable_input(&mut self, eis_capabilities: &[String]) -> Result<(), TargetError> {
        if self.state.keyboard.is_some() || self.state.pointer.is_some() {
            return Ok(());
        }
        let seat = self
            .seat
            .clone()
            .ok_or_else(|| self.live("the session does not advertise wl_seat, so no input can arrive"))?;
        self.pump_until(Duration::from_secs(3), |state| state.capabilities.is_some())?;
        // A compositor that adds a device after the seat's first announcement announces again; the round trip brings
        // that in, so the keyboard asked for below is one the seat has.
        self.roundtrip()?;
        for (name, capability) in [
            ("keyboard", wl_seat::Capability::Keyboard),
            ("pointer", wl_seat::Capability::Pointer),
        ] {
            if eis_capabilities.iter().any(|offered| offered == name) {
                // A seat that never advertises it is reported by the probe that needs it, so the wait is soft.
                self.pump_until(Duration::from_secs(3), |state| {
                    state.capabilities.is_some_and(|have| have.contains(capability))
                })?;
            }
        }
        let capabilities = self
            .state
            .capabilities
            .ok_or_else(|| self.live("the seat never announced its capabilities"))?;
        if capabilities.contains(wl_seat::Capability::Keyboard) {
            self.state.keyboard = Some(seat.get_keyboard(&self.qh, ()));
        }
        if capabilities.contains(wl_seat::Capability::Pointer) {
            self.state.pointer = Some(seat.get_pointer(&self.qh, ()));
        }
        self.roundtrip()
    }

    /// What the seat's keyboard and pointer delivered since the last [`Self::clear_input`].
    pub(crate) fn input(&self) -> &InputLog {
        &self.state.input
    }

    /// Forgets the input received so far, so a probe reads only what its own injection caused.
    pub(crate) fn clear_input(&mut self) {
        let entered = (self.state.input.keyboard_entered, self.state.input.pointer_entered);
        self.state.input = InputLog {
            keyboard_entered: entered.0,
            pointer_entered: entered.1,
            ..InputLog::default()
        };
    }

    /// Waits until `done` holds of the input received, or `limit` passes, and reports whether it held.
    pub(crate) fn wait_input(
        &mut self,
        limit: Duration,
        done: impl Fn(&InputLog) -> bool,
    ) -> Result<bool, TargetError> {
        self.pump_until(limit, |state| done(&state.input))
    }

    /// Creates a popup of `parent`, takes its initial configure and maps it with a buffer of the size it asked
    /// for. A `grab` asks for an explicit grab with serial 0, the serial of a client that has received no input.
    pub(crate) fn map_popup(
        &mut self,
        parent: &xdg_surface::XdgSurface,
        place: &Place,
        grab: bool,
    ) -> Result<Popup, TargetError> {
        let tag = self.tag();
        let surface = self.compositor.create_surface(&self.qh, ());
        let xdg_surface = self.wm_base.get_xdg_surface(&surface, &self.qh, ());
        let positioner = self.positioner(place);
        let popup = xdg_surface.get_popup(Some(parent), &positioner, &self.qh, tag);
        positioner.destroy();
        if grab {
            let seat = self
                .seat
                .as_ref()
                .ok_or_else(|| self.live("the session does not advertise wl_seat, so a popup cannot grab"))?;
            popup.grab(seat, 0);
        }
        surface.commit();
        self.roundtrip()?;
        if !self.state.popups.get(&tag).is_some_and(|popup| popup.done) {
            self.attach_buffer(&surface, place.size)?;
        }
        Ok(Popup {
            tag,
            surface,
            xdg_surface,
            popup,
        })
    }

    /// Asks the compositor to place `popup` again, as a client does when its anchor moves.
    pub(crate) fn reposition(&mut self, popup: &Popup, place: &Place, token: u32) -> Result<(), TargetError> {
        let positioner = self.positioner(place);
        popup.popup.reposition(&positioner, token);
        positioner.destroy();
        self.roundtrip()
    }

    fn positioner(&self, place: &Place) -> xdg_positioner::XdgPositioner {
        let positioner = self.wm_base.create_positioner(&self.qh, ());
        positioner.set_size(place.size.0, place.size.1);
        let (x, y, width, height) = place.anchor_rect;
        positioner.set_anchor_rect(x, y, width, height);
        positioner.set_anchor(place.anchor);
        positioner.set_gravity(place.gravity);
        positioner.set_offset(place.offset.0, place.offset.1);
        positioner
    }

    /// Attaches a shared-memory buffer and commits, which maps the surface. The buffer is returned so a window can
    /// show it again.
    fn attach_buffer(
        &mut self,
        surface: &wl_surface::WlSurface,
        (width, height): (i32, i32),
    ) -> Result<wl_buffer::WlBuffer, TargetError> {
        let stride = width * 4;
        let length = stride * height;
        let file = tempfile::tempfile().map_err(|error| self.live(format!("cannot create a buffer file: {error}")))?;
        file.set_len(u64::try_from(length).unwrap_or_default())
            .map_err(|error| self.live(format!("cannot size a buffer file: {error}")))?;
        let pool = self.shm.create_pool(file.as_fd(), length, &self.qh, ());
        let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, &self.qh, ());
        surface.attach(Some(&buffer), 0, 0);
        surface.damage(0, 0, width, height);
        surface.commit();
        self.roundtrip()?;
        Ok(buffer)
    }

    /// Closes a toplevel the way a well-behaved client does.
    pub(crate) fn destroy_toplevel(&mut self, window: Toplevel) -> Result<(), TargetError> {
        window.toplevel.destroy();
        window.xdg_surface.destroy();
        window.surface.destroy();
        window.buffer.destroy();
        self.roundtrip()
    }

    /// Closes a popup the way a well-behaved client does.
    pub(crate) fn destroy_popup(&mut self, popup: Popup) -> Result<(), TargetError> {
        popup.popup.destroy();
        popup.xdg_surface.destroy();
        popup.surface.destroy();
        self.roundtrip()
    }

    /// Shows the window's buffer again and asks for a frame callback, as a client that animates does for every frame.
    pub(crate) fn redraw(&mut self, window: &Toplevel) -> Result<(), TargetError> {
        let (width, height) = window.size;
        window.surface.frame(&self.qh, ());
        window.surface.attach(Some(&window.buffer), 0, 0);
        window.surface.damage(0, 0, width, height);
        window.surface.commit();
        self.queue
            .flush()
            .map_err(|error| self.live(format!("cannot flush the Wayland connection: {error}")))
    }

    /// The times the frame callbacks of this client fired, in order.
    pub(crate) fn frame_times(&self) -> &[Instant] {
        &self.state.frames
    }

    /// Dispatches whatever the compositor sends, waiting up to `limit` for the first event instead of sleeping in
    /// steps, so a latency measured around it is not rounded to the step. Returns whether anything was dispatched.
    pub(crate) fn dispatch_for(&mut self, limit: Duration) -> Result<bool, TargetError> {
        let dispatched = self
            .queue
            .dispatch_pending(&mut self.state)
            .map_err(|error| self.live(format!("the Wayland dispatch failed: {error}")))?;
        if dispatched > 0 {
            return Ok(true);
        }
        let Some(guard) = self.queue.prepare_read() else {
            return Ok(true);
        };
        let readable = {
            let fd = guard.connection_fd();
            let mut fds = [PollFd::new(&fd, PollFlags::IN)];
            let timeout = rustix::event::Timespec::try_from(limit).ok();
            poll(&mut fds, timeout.as_ref())
                .map(|ready| ready > 0)
                .map_err(|error| self.live(format!("cannot wait for the compositor: {error}")))?
        };
        if !readable {
            return Ok(false);
        }
        guard
            .read()
            .map_err(|error| self.live(format!("cannot read from the compositor: {error}")))?;
        let dispatched = self
            .queue
            .dispatch_pending(&mut self.state)
            .map_err(|error| self.live(format!("the Wayland dispatch failed: {error}")))?;
        Ok(dispatched > 0)
    }
}

/// Objects whose events a scenario never reads.
macro_rules! ignore_events {
    ($($object:ty),* $(,)?) => {
        $(
            impl Dispatch<$object, ()> for Protocol {
                fn event(
                    _: &mut Self,
                    _: &$object,
                    _: <$object as Proxy>::Event,
                    _: &(),
                    _: &WaylandConnection,
                    _: &QueueHandle<Self>,
                ) {
                }
            }
        )*
    };
}

ignore_events!(
    wl_compositor::WlCompositor,
    wl_surface::WlSurface,
    wl_shm::WlShm,
    wl_shm_pool::WlShmPool,
    wl_buffer::WlBuffer,
    xdg_positioner::XdgPositioner,
);

impl Dispatch<wl_seat::WlSeat, ()> for Protocol {
    fn event(
        state: &mut Self,
        _: &wl_seat::WlSeat,
        event: <wl_seat::WlSeat as Proxy>::Event,
        _: &(),
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        {
            state.capabilities = Some(capabilities);
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for Protocol {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: <wl_callback::WlCallback as Proxy>::Event,
        _: &(),
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.frames.push(Instant::now());
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for Protocol {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: <wl_keyboard::WlKeyboard as Proxy>::Event,
        _: &(),
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Enter { .. } => state.input.keyboard_entered = true,
            wl_keyboard::Event::Leave { .. } => state.input.keyboard_entered = false,
            wl_keyboard::Event::Key {
                key,
                state: WEnum::Value(edge),
                ..
            } => state.input.keys.push((key, edge == wl_keyboard::KeyState::Pressed)),
            _ => {}
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for Protocol {
    fn event(
        state: &mut Self,
        _: &wl_pointer::WlPointer,
        event: <wl_pointer::WlPointer as Proxy>::Event,
        _: &(),
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter { .. } => state.input.pointer_entered = true,
            wl_pointer::Event::Leave { .. } => state.input.pointer_entered = false,
            wl_pointer::Event::Motion {
                surface_x, surface_y, ..
            } => state.input.motions.push((surface_x, surface_y)),
            wl_pointer::Event::Button {
                button,
                state: WEnum::Value(edge),
                ..
            } => state
                .input
                .buttons
                .push((button, edge == wl_pointer::ButtonState::Pressed)),
            wl_pointer::Event::Axis {
                axis: WEnum::Value(axis),
                value,
                ..
            } => state.input.axes.push((axis == wl_pointer::Axis::VerticalScroll, value)),
            _ => {}
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Protocol {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: <wl_registry::WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for Protocol {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: <xdg_wm_base::XdgWmBase as Proxy>::Event,
        _: &(),
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for Protocol {
    fn event(
        _: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: <xdg_surface::XdgSurface as Proxy>::Event,
        _: &(),
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, Tag> for Protocol {
    fn event(
        state: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: <xdg_toplevel::XdgToplevel as Proxy>::Event,
        tag: &Tag,
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Configure { width, height, states } = event {
            let activated = states
                .as_chunks::<4>()
                .0
                .iter()
                .map(|state| u32::from_ne_bytes(*state))
                .any(|state| state == u32::from(xdg_toplevel::State::Activated));
            let window = state.toplevels.entry(*tag).or_default();
            window.activated = activated;
            window.size = (width, height);
        }
    }
}

impl Dispatch<xdg_popup::XdgPopup, Tag> for Protocol {
    fn event(
        state: &mut Self,
        _: &xdg_popup::XdgPopup,
        event: <xdg_popup::XdgPopup as Proxy>::Event,
        tag: &Tag,
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
        let popup = state.popups.entry(*tag).or_default();
        match event {
            xdg_popup::Event::Configure { x, y, width, height } => popup.placed = Some((x, y, width, height)),
            xdg_popup::Event::PopupDone => popup.done = true,
            xdg_popup::Event::Repositioned { token } => popup.repositioned = Some(token),
            _ => {}
        }
    }
}
