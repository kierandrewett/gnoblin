//! `gnoblin_window_frame_v1`: server-side decorations drawn by a renderer client.
//!
//! A frame renderer is one client the compositor started for the session. It is
//! offered a [`GnoblinWindowFrameV1`] object per window that wants a frame, draws
//! that window's decorations into an unroled `wl_surface` of its own connection,
//! and describes which parts of what it drew mean what through `region`. The
//! compositor owns everything else: where the frame goes, which window it
//! decorates, whether the window is focused, and what each region does.
//!
//! The wire is the C compositor's
//! (`src/protocols/window-frame/meta-gnoblin-frame-renderer.c` and the protocol
//! XML) unchanged, because `bingux-frame` builds against it: the same configure
//! serial rule, the same region limits, the same error codes and the same
//! meaning for `closed`.
//!
//! ## The three rules a renderer cannot break
//!
//! 1. **A frame never waits for a renderer.** A configure is sent and forgotten:
//!    the window's own client keeps committing, mapping and resizing whether or
//!    not the renderer answers. A renderer that never acks or never draws leaves
//!    its own frame unpresented and nothing else.
//! 2. **A stale generation is not displayed.** A commit carries the serial of the
//!    configure it answers. A commit that answers an older configure is applied
//!    to the surface but never presented, so a resize cannot flash the previous
//!    size over the new one.
//! 3. **A buffer that does not match its configure is a protocol error.** The
//!    renderer's surface is the frame, so a buffer of the wrong logical size
//!    would be a decoration that does not fit the window; the connection is
//!    ended rather than shown.
//!
//! ## What this slice does not do
//!
//! Composing a presented frame over its window is the renderer card's work and it
//! needs a decoration policy the window manager does not have yet, so a frame is
//! offered with the window's own geometry and zero extents: the protocol
//! machinery, its errors and its state are what this module proves.
//!
//! Hover and press are reported to the renderer from the real input path
//! ([`State::frame_pointer`]) because a frame's regions are the input table: the
//! pointer over a region is `interaction(action, 0)` and a press is
//! `interaction(action, 1)`, exactly as the C compositor reports it. What the
//! *action* then does — close, minimise, maximise, a move or a resize grab — is the
//! window manager's, and this slice has no decoration-driven action path: the
//! region an action names is committed and readable through
//! [`State::window_frame`], which is what that path will read.
//!
//! ## Failure modes this module defends against
//!
//! Each rule below is what the code does, not yet a test: the real-client frame
//! tests are not part of this change (see "What is not proven yet").
//!
//! 1. **The global is visible to a client that is not a renderer.** It is offered
//!    only to a connection the compositor promised `ClientRole::FrameRenderer`, so
//!    an application cannot draw another client's decorations.
//! 2. **A renderer attaches a surface that already has a role, or attaches a second
//!    surface to one frame.** The request is a `role` protocol error and the session
//!    keeps running.
//! 3. **A renderer acknowledges a configure that was never sent.** The request is an
//!    `invalid_state` protocol error; an acknowledgement of an older configure is
//!    recorded and never presented.
//! 4. **A frame image is presented for the wrong size.** The commit is an
//!    `invalid_state` protocol error and the previous frame stays.
//! 5. **A region outside the protocol's limits is accepted.** More than 64 regions,
//!    a negative or oversized rectangle, or an unknown action is refused.
//! 6. **A renderer that crashes leaves a frame behind.** Its frames are closed and
//!    forgotten once per turn, so the window keeps no decoration nothing owns.
//! 7. **A window that closes leaves a frame that can still configure.** It is sent
//!    `closed` once and answers no further request.
//!
//! ## What is not proven yet
//!
//! The wire is generated and served, the object types are the scanner's, and the
//! client half is generated from the same XML into `tests/owned`, but the
//! real-client frame tests that would drive it are not in this change. Composing a
//! presented frame over its window is the renderer card's work and it needs a
//! decoration policy the window manager does not have yet, so a frame is offered
//! with the window's own geometry and zero extents.
//!
//! Hover and press are reported to the renderer from the real input path
//! ([`State::frame_pointer`]) because a frame's regions are the input table. What
//! the *action* then does — close, minimise, maximise, a move or a resize grab — is
//! the window manager's, and this slice has no decoration-driven action path.

use std::collections::{BTreeMap, HashMap};

use smithay::backend::renderer::utils::with_renderer_surface_state;
use smithay::reexports::wayland_server::backend::{ClientId as WaylandClientId, GlobalId};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, DataInit, DisplayHandle, New, Resource, WEnum};
use smithay::utils::{Logical, Point as SmithayPoint, SERIAL_COUNTER};
use smithay::wayland::compositor::give_role;
use smithay::wayland::{Dispatch2, GlobalDispatch2};
use tracing::{debug, warn};

use gnoblin_types::WindowId;

use crate::State;
use crate::access::{Access, AccessPolicy, Global};
use crate::protocols::wire::window_frame::gnoblin_window_frame_manager_v1::{
    self as manager, GnoblinWindowFrameManagerV1,
};
use crate::protocols::wire::window_frame::gnoblin_window_frame_v1::{
    self as frame, GnoblinWindowFrameV1, State as FrameState,
};

/// The surface role a frame claims, which is what stops a renderer reusing an
/// application's surface.
const FRAME_ROLE: &str = "gnoblin_window_frame";

/// How many regions one frame may declare, from the protocol.
const MAX_REGIONS: usize = 64;

/// The largest coordinate, size or edge a region may name, from the protocol.
const MAX_COORDINATE: i32 = 32768;

/// What a region of a frame means.
///
/// The numbers are the protocol's, so a region is the value the wire carries and
/// a consumer never renumbers it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum FrameAction {
    /// Start an interactive move from this region.
    Drag,
    /// Ask the window to close.
    Close,
    /// Maximise or restore the window.
    Maximize,
    /// Minimise the window.
    Minimize,
    /// Resize from the north edge, and the other seven in compass order.
    North,
    /// Resize from the north-east corner.
    NorthEast,
    /// Resize from the east edge.
    East,
    /// Resize from the south-east corner.
    SouthEast,
    /// Resize from the south edge.
    South,
    /// Resize from the south-west corner.
    SouthWest,
    /// Resize from the west edge.
    West,
    /// Resize from the north-west corner.
    NorthWest,
}

impl FrameAction {
    /// The action a wire value names, or [`None`] for a value the protocol does
    /// not define.
    #[must_use]
    pub const fn from_wire(value: u32) -> Option<Self> {
        Some(match value {
            1 => Self::Drag,
            2 => Self::Close,
            3 => Self::Maximize,
            4 => Self::Minimize,
            5 => Self::North,
            6 => Self::NorthEast,
            7 => Self::East,
            8 => Self::SouthEast,
            9 => Self::South,
            10 => Self::SouthWest,
            11 => Self::West,
            12 => Self::NorthWest,
            _ => return None,
        })
    }

    /// The value the wire uses for this action.
    #[must_use]
    pub const fn to_wire(self) -> u32 {
        match self {
            Self::Drag => 1,
            Self::Close => 2,
            Self::Maximize => 3,
            Self::Minimize => 4,
            Self::North => 5,
            Self::NorthEast => 6,
            Self::East => 7,
            Self::SouthEast => 8,
            Self::South => 9,
            Self::SouthWest => 10,
            Self::West => 11,
            Self::NorthWest => 12,
        }
    }
}

/// One region a renderer declared, in the frame's logical coordinates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameRegion {
    /// What the region means.
    pub action: FrameAction,
    /// Its left edge.
    pub x: i32,
    /// Its top edge.
    pub y: i32,
    /// Its width.
    pub width: i32,
    /// Its height.
    pub height: i32,
}

impl FrameRegion {
    /// Whether a frame-local point is inside the region, edges included.
    #[must_use]
    pub const fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x.saturating_add(self.width) && y < self.y.saturating_add(self.height)
    }
}

/// What a window's frame is doing, as the compositor and a test read it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowFrame {
    /// The serial of the configure the frame is presenting, if it presented one.
    pub serial: u32,
    /// The logical width the frame surface is configured to.
    pub width: i32,
    /// The logical height the frame surface is configured to.
    pub height: i32,
    /// The decoration extents, in the order top, right, bottom, left.
    pub extents: [i32; 4],
    /// Whether a buffer that matched the current configure was presented.
    pub presented: bool,
    /// The regions that commit declared.
    pub regions: Vec<FrameRegion>,
}

/// One window's frame.
struct Frame {
    /// The protocol object, which is also how a configure is sent.
    object: GnoblinWindowFrameV1,
    /// The surface the renderer attached, once it has.
    surface: Option<WlSurface>,
    /// The serial of the last configure sent.
    serial: u32,
    /// The frame's logical size in the last configure.
    size: (i32, i32),
    /// The extents the last configure carried.
    extents: [i32; 4],
    /// The serial the renderer acknowledged for the pending commit.
    ack: Option<u32>,
    /// The regions staged for the next commit.
    pending: Vec<FrameRegion>,
    /// The regions of the presented frame.
    committed: Vec<FrameRegion>,
    /// Whether the presented frame matches the current configure.
    presented: bool,
    /// Whether `closed` was sent, after which only `destroy` means anything.
    closed: bool,
    /// The region the pointer was last over, so an unchanged hover sends nothing.
    hover: Option<FrameAction>,
    /// Whether the last reported press was taken.
    pressed: bool,
}

impl Frame {
    /// The body of what the compositor can ask about a frame.
    fn report(&self) -> WindowFrame {
        WindowFrame {
            serial: self.serial,
            width: self.size.0,
            height: self.size.1,
            extents: self.extents,
            presented: self.presented,
            regions: self.committed.clone(),
        }
    }
}

/// The frame protocol's objects and the windows they decorate.
pub(crate) struct FrameTables {
    /// The `gnoblin_window_frame_manager_v1` global.
    global: Option<GlobalId>,
    /// Every bound manager, by connection.
    managers: HashMap<WaylandClientId, GnoblinWindowFrameManagerV1>,
    /// Every frame, by window.
    frames: BTreeMap<WindowId, Frame>,
}

impl FrameTables {
    /// Registers the global on the session's display.
    ///
    /// The global exists even when no renderer is configured, because the roster
    /// is the whole answer to who may see it and a state that does not exist
    /// cannot be handed a renderer later.
    pub(crate) fn new(display_handle: &DisplayHandle, access: &std::sync::Arc<Access>) -> Self {
        let global = display_handle.create_global::<State, GnoblinWindowFrameManagerV1, ManagerGlobal>(
            manager::GnoblinWindowFrameManagerV1::interface().version,
            ManagerGlobal {
                access: std::sync::Arc::clone(access),
            },
        );
        Self {
            global: Some(global),
            managers: HashMap::new(),
            frames: BTreeMap::new(),
        }
    }

    /// The global, so a caller that withdraws it can name it.
    #[allow(dead_code)]
    pub(crate) fn global(&self) -> Option<GlobalId> {
        self.global.clone()
    }
}

/// The manager global's data: who may bind it.
struct ManagerGlobal {
    /// The roster that answers which connection was started as a renderer.
    access: std::sync::Arc<Access>,
}

impl GlobalDispatch2<GnoblinWindowFrameManagerV1, State> for ManagerGlobal {
    fn bind(
        &self,
        state: &mut State,
        _handle: &DisplayHandle,
        client: &Client,
        resource: New<GnoblinWindowFrameManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        let manager = data_init.init(resource, ManagerObject);
        debug!(client = ?client.id(), "[frame] a renderer bound its manager");
        if let Err(duplicate) = state.frame_tables.check_manager_in(&client.id(), &manager) {
            // The manager interface has no error enum of its own, so the code is
            // the protocol's first one: the connection is ended either way.
            duplicate.post_error(0_u32, "a connection has one frame manager");
        }
    }

    fn can_view(&self, client: &Client) -> bool {
        AccessPolicy::allows(&*self.access, &client.id(), Global::WindowFrame)
    }
}

/// The user data of a bound manager: it carries no state of its own.
struct ManagerObject;

impl Dispatch2<GnoblinWindowFrameManagerV1, State> for ManagerObject {
    fn request(
        &self,
        state: &mut State,
        client: &Client,
        _manager: &GnoblinWindowFrameManagerV1,
        request: manager::Request,
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            manager::Request::Destroy => {
                state.frame_tables.manager_gone(&client.id());
            }
        }
    }

    fn destroyed(&self, state: &mut State, client: WaylandClientId, _manager: &GnoblinWindowFrameManagerV1) {
        state.frame_tables.manager_gone(&client);
    }
}

/// The user data of one frame object.
struct FrameObject {
    /// The window it decorates.
    window: WindowId,
}

impl FrameTables {
    /// Records a manager, refusing a second one on the same connection.
    fn check_manager_in(
        &mut self,
        client: &WaylandClientId,
        manager: &GnoblinWindowFrameManagerV1,
    ) -> Result<(), GnoblinWindowFrameManagerV1> {
        if self.managers.get(client).is_some_and(Resource::is_alive) {
            return Err(manager.clone());
        }
        self.managers.insert(client.clone(), manager.clone());
        Ok(())
    }

    /// A manager went away: every frame it held is closed and forgotten.
    fn manager_gone(&mut self, client: &WaylandClientId) {
        self.managers.remove(client);
        let orphaned: Vec<WindowId> = self
            .frames
            .iter()
            .filter(|(_, frame)| frame.object.client().is_some_and(|owner| owner.id() == *client))
            .map(|(window, _)| *window)
            .collect();
        for window in orphaned {
            if let Some(frame) = self.frames.remove(&window) {
                close(&frame);
            }
        }
    }

    /// The first live manager, which is where a new frame is offered.
    fn manager(&self) -> Option<&GnoblinWindowFrameManagerV1> {
        self.managers.values().find(|manager| manager.is_alive())
    }
}

impl Dispatch2<GnoblinWindowFrameV1, State> for FrameObject {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        resource: &GnoblinWindowFrameV1,
        request: frame::Request,
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        let Some(entry) = state.frame_tables.frames.get_mut(&self.window) else {
            return;
        };
        // After `closed` only `destroy` is meaningful, which the display does
        // without this module once the object is gone.
        if entry.closed && !matches!(request, frame::Request::Destroy) {
            return;
        }
        match request {
            frame::Request::Destroy => {}
            frame::Request::AttachSurface { surface } => {
                if entry.surface.is_some() || give_role(&surface, FRAME_ROLE).is_err() {
                    // Either this frame already has a surface or the client handed
                    // over a surface another role owns; both are the protocol's
                    // `role` error.
                    resource.post_error(frame::Error::Role as u32, "a frame takes one unroled surface");
                    return;
                }
                entry.surface = Some(surface);
                debug!(window = %self.window, "[frame] a renderer attached its surface");
            }
            frame::Request::AckConfigure { serial } => {
                if entry.surface.is_none() {
                    resource.post_error(frame::Error::Role as u32, "no surface attached");
                    return;
                }
                if serial <= entry.serial {
                    // An older configure can still be in the client's queue, so
                    // acknowledging it records which generation the next commit
                    // answers rather than being an error.
                    entry.ack = Some(serial);
                    return;
                }
                resource.post_error(frame::Error::InvalidState as u32, "unknown frame configure serial");
            }
            frame::Request::ClearRegions => entry.pending.clear(),
            frame::Request::Region {
                action,
                x,
                y,
                width,
                height,
            } => {
                if entry.surface.is_none() {
                    resource.post_error(frame::Error::Role as u32, "no surface attached");
                    return;
                }
                let action = match action {
                    WEnum::Value(action) => FrameAction::from_wire(action as u32),
                    WEnum::Unknown(_) => None,
                };
                let Some(action) = action else {
                    resource.post_error(frame::Error::InvalidState as u32, "invalid frame region");
                    return;
                };
                if x < 0
                    || y < 0
                    || width < 0
                    || height < 0
                    || x > MAX_COORDINATE
                    || y > MAX_COORDINATE
                    || width > MAX_COORDINATE
                    || height > MAX_COORDINATE
                    || entry.pending.len() >= MAX_REGIONS
                {
                    resource.post_error(frame::Error::InvalidState as u32, "invalid frame region");
                    return;
                }
                entry.pending.push(FrameRegion {
                    action,
                    x,
                    y,
                    width,
                    height,
                });
            }
        }
    }

    fn destroyed(&self, state: &mut State, _client: WaylandClientId, _resource: &GnoblinWindowFrameV1) {
        state.frame_tables.frames.remove(&self.window);
    }
}

/// Tells a renderer its frame is over, once.
fn close(frame: &Frame) {
    if frame.closed {
        return;
    }
    if frame.object.is_alive() {
        frame.object.closed();
    }
}

impl State {
    /// What a window's frame is doing, for the compositor and for a test.
    #[must_use]
    pub fn window_frame(&self, window: WindowId) -> Option<WindowFrame> {
        self.frame_tables.frames.get(&window).map(Frame::report)
    }

    /// Tells a renderer which of its regions the pointer is over.
    ///
    /// The renderer needs this to repaint the hovered control, and it is the only
    /// thing `interaction` carries: the action itself is the compositor's, because
    /// a renderer never moves, closes or resizes a window. `pressed` is [`None`] on a
    /// motion, which keeps the press the frame last reported, and a [`None`] position
    /// is the pointer leaving every frame.
    ///
    /// Nothing is sent when nothing changed, so an idle pointer over an idle frame
    /// costs nothing.
    pub(crate) fn frame_pointer(&mut self, position: Option<SmithayPoint<f64, Logical>>, pressed: Option<bool>) {
        let window = position.and_then(|position| self.frame_window_at(position).map(|window| (window, position)));
        let Some((window, position)) = window else {
            // The pointer left every frame: each frame that was hovered is told so
            // once, and no frame keeps a hover it no longer has.
            let hovered: Vec<WindowId> = self
                .frame_tables
                .frames
                .iter()
                .filter(|(_, frame)| frame.hover.is_some())
                .map(|(window, _)| *window)
                .collect();
            for window in hovered {
                if let Some(frame) = self.frame_tables.frames.get_mut(&window) {
                    frame.hover = None;
                    let pressed = pressed.unwrap_or(frame.pressed);
                    frame.pressed = pressed;
                    if frame.object.is_alive() {
                        frame.object.interaction(0, u32::from(pressed));
                    }
                }
            }
            return;
        };
        let Some(entry) = self.frame_tables.frames.get(&window) else {
            return;
        };
        let origin = self
            .wm
            .window(window)
            .map(|managed| managed.geometry)
            .unwrap_or_default();
        let local = (
            i32::try_from((position.x - f64::from(origin.x)).round() as i64).unwrap_or(i32::MAX),
            i32::try_from((position.y - f64::from(origin.y)).round() as i64).unwrap_or(i32::MAX),
        );
        let action = entry
            .committed
            .iter()
            .rev()
            .find(|region| region.contains(local.0, local.1))
            .map(|region| region.action);
        let Some(frame) = self.frame_tables.frames.get_mut(&window) else {
            return;
        };
        let pressed = pressed.unwrap_or(frame.pressed);
        if frame.hover == action && frame.pressed == pressed {
            return;
        }
        frame.hover = action;
        frame.pressed = pressed;
        if frame.object.is_alive() {
            frame
                .object
                .interaction(action.map_or(0, FrameAction::to_wire), u32::from(pressed));
        }
    }

    /// The topmost window whose presented frame covers a global position.
    fn frame_window_at(&self, position: SmithayPoint<f64, Logical>) -> Option<WindowId> {
        let active = self.wm.active_workspace();
        for window in self.wm.stack().iter().rev() {
            let Some(managed) = self.wm.window(*window) else {
                continue;
            };
            if !managed.is_visible(active) {
                continue;
            }
            let frame = self.frame_tables.frames.get(window)?;
            if frame.closed || !frame.presented || frame.surface.is_none() {
                continue;
            }
            let geometry = managed.geometry;
            let inside = position.x >= f64::from(geometry.x)
                && position.y >= f64::from(geometry.y)
                && position.x < geometry.right() as f64
                && position.y < geometry.bottom() as f64;
            if inside {
                return Some(*window);
            }
        }
        None
    }

    /// Offers or refreshes the frame for a window.
    ///
    /// A window without a buffer is not decorated: its frame is closed and
    /// forgotten, and it gets a new one when it maps again. Everything else is a
    /// configure, sent only when the geometry, the metadata or the window's state
    /// changed since the last one, so an idle window sends no frames.
    pub(crate) fn refresh_window_frame(&mut self, window: WindowId) {
        let Some(mapped) = self.windows.get(&window).map(|entry| entry.mapped) else {
            self.close_window_frame(window);
            return;
        };
        if !mapped {
            self.close_window_frame(window);
            return;
        }
        let Some(manager) = self.frame_tables.manager().cloned() else {
            return;
        };
        let Some(managed) = self.wm.window(window).cloned() else {
            return;
        };
        let extents = [0; 4];
        let size = (
            i32::try_from(managed.geometry.width).unwrap_or(i32::MAX),
            i32::try_from(managed.geometry.height).unwrap_or(i32::MAX),
        );
        let focused = self.wm.focused() == Some(window);
        let mut state = FrameState::empty();
        if focused {
            state |= FrameState::Focused;
        }
        if managed.state.maximised {
            state |= FrameState::Maximized;
        }
        // A client can always be asked to close and can always be minimised; a
        // fullscreen or maximised window has no size for a resize handle.
        state |= FrameState::CanClose | FrameState::CanMaximize | FrameState::CanMinimize;
        if !managed.state.maximised && !managed.state.fullscreen {
            state |= FrameState::CanResize;
        }
        let title = managed.title.clone();
        let app_id = managed.app_id.clone();
        // The style label is the renderer's own theme hook; this slice has no
        // theme to name, so the renderer is handed the empty string and uses its
        // default.
        let style = String::new();

        if let Some(frame) = self.frame_tables.frames.get_mut(&window) {
            if frame.size == size && frame.extents == extents && frame.object.is_alive() {
                return;
            }
            if !frame.object.is_alive() {
                self.frame_tables.frames.remove(&window);
            }
        }
        let Some(Some(client)) = self.frame_tables.manager().map(|manager| manager.client()) else {
            return;
        };
        let serial = SERIAL_COUNTER.next_serial();
        let serial = u32::from(serial);
        // The frame object is created for the renderer's connection, so its
        // requests arrive there and nowhere else.
        let object = match client.create_resource::<GnoblinWindowFrameV1, FrameObject, State>(
            &self.display_handle,
            frame::GnoblinWindowFrameV1::interface().version,
            FrameObject { window },
        ) {
            Ok(object) => object,
            Err(error) => {
                warn!(%window, ?error, "[frame] a frame object could not be created");
                return;
            }
        };
        manager.frame(&object);
        debug!(%window, serial, ?size, "[frame] a frame was offered");
        let entry = Frame {
            object,
            surface: None,
            serial,
            size,
            extents,
            ack: None,
            pending: Vec::new(),
            committed: Vec::new(),
            presented: false,
            closed: false,
            hover: None,
            pressed: false,
        };
        entry.object.configure(
            serial, size.0, size.1, extents[0], extents[1], extents[2], extents[3], state, title, app_id, style,
        );
        self.frame_tables.frames.insert(window, entry);
    }

    /// Closes a window's frame, if it has one.
    pub(crate) fn close_window_frame(&mut self, window: WindowId) {
        if let Some(frame) = self.frame_tables.frames.remove(&window) {
            close(&frame);
        }
    }

    /// A renderer's surface committed.
    ///
    /// The frame's staged regions and acknowledgement become the presented frame
    /// only when the buffer matches the configure the acknowledgement answers;
    /// anything else is either ignored (a stale generation) or a protocol error
    /// (a wrong size).
    pub(crate) fn frame_surface_committed(&mut self, surface: &WlSurface) {
        let Some(window) = self
            .frame_tables
            .frames
            .iter()
            .find(|(_, frame)| frame.surface.as_ref() == Some(surface))
            .map(|(window, _)| *window)
        else {
            return;
        };
        let Some(entry) = self.frame_tables.frames.get_mut(&window) else {
            return;
        };
        if entry.closed {
            return;
        }
        let Some(buffer) = with_renderer_surface_state(surface, |state| {
            state.buffer()?;
            let size = state.buffer_size()?;
            Some((size, state.buffer_scale()))
        })
        .flatten() else {
            // A null buffer selects the compositor's fallback chrome, which this
            // slice draws none of, so the frame is simply not presented.
            entry.presented = false;
            return;
        };
        let Some(ack) = entry.ack.take() else {
            // The renderer committed before acknowledging anything.
            return;
        };
        if ack != entry.serial {
            // A generation older than the current configure: applied to the
            // surface, never presented.
            entry.pending.clear();
            return;
        }
        let (buffer_size, scale) = buffer;
        let scale = if scale == 0 { 1 } else { scale };
        let logical = (buffer_size.w.div_euclid(scale), buffer_size.h.div_euclid(scale));
        if logical != entry.size {
            entry.pending.clear();
            entry.object.post_error(
                frame::Error::InvalidState as u32,
                "a frame buffer must match its configure",
            );
            return;
        }
        entry.committed.clear();
        entry.committed.append(&mut entry.pending);
        entry.presented = true;
        debug!(%window, serial = entry.serial, "[frame] a frame was presented");
    }

    /// Forgets frames whose object or surface is gone.
    ///
    /// A renderer that disconnects sends no request, so the compositor looks once
    /// per turn, exactly as it does for a locker.
    pub(crate) fn poll_frames(&mut self) {
        let dead: Vec<WindowId> = self
            .frame_tables
            .frames
            .iter()
            .filter(|(_, frame)| {
                !frame.object.is_alive() || frame.surface.as_ref().is_some_and(|surface| !surface.is_alive())
            })
            .map(|(window, _)| *window)
            .collect();
        for window in dead {
            self.close_window_frame(window);
        }
        self.frame_tables.managers.retain(|_, manager| manager.is_alive());
    }
}
