//! The client side of the protocols a host compositor speaks to a nested session.
//!
//! Every `Dispatch` implementation below is an adapter: a protocol event becomes one
//! call on [`Table`] (the parent-facing state) or on the seat, and nothing else. No
//! handler decides anything, which is what lets the decisions be tested without a
//! socket and keeps the `wayland-client` types out of the rest of the crate.
//!
//! # What the parent drives and what it does not
//!
//! - **Size and scale.** `xdg_toplevel.configure` states the size the parent wants,
//!   and `wp_fractional_scale_v1.surface_preferred_scale` the scale to draw it at.
//!   The configure is acknowledged at once: the size it states is applied to the
//!   target before the next commit, so the buffer always matches the configure it
//!   answers.
//! - **Pacing.** `wl_surface.frame` callbacks say when the parent is ready for
//!   another frame. The callback carries the output's index as its user data, so a
//!   callback that arrives late for a target that has been resized still clears the
//!   right one.
//! - **Buffers.** `wl_buffer.release` carries the pool generation and the slot, so a
//!   release for a pool that a resize replaced is credited to the pool that made the
//!   buffer and ignored by the one that replaced it.
//! - **Input.** The seat's keyboard, pointer and touch objects are created from
//!   `wl_seat.capabilities` and their events are converted in [`super::input`]. The
//!   keyboard's keymap is read out of the file descriptor the parent sends and kept
//!   for the caller: the backend's [`InputEvent`](crate::InputEvent) states scan
//!   codes, so the layout has to be applied by whoever owns the keymap.
//!
//! # What is bridged and what is not
//!
//! - **The cursor.** The session draws its own cursor into the frame, so the parent's pointer is told to show none
//!   over the session's surfaces ([`wl_pointer::WlPointer::set_cursor`] with no surface). One cursor is visible, the
//!   same one a capture shows, instead of the parent's drawn over it.
//! - **Compositor shortcuts.** Each output asks the parent to inhibit its own shortcuts while the window has the
//!   keyboard (`zwp_keyboard_shortcuts_inhibit_v1`), so the session's `Super` bindings reach the session. Whether the
//!   parent granted it is [`Handler::shortcuts_inhibited`].
//! - **Gestures.** A parent that offers `zwp_pointer_gestures_v1` has its swipe and pinch gestures delivered as the
//!   same raw touchpad samples a libinput backend produces.
//! - **Scale.** The parent's fractional scale is followed per surface; without it, the integer scale of the parent
//!   outputs a surface is on (`wl_surface.enter`) is, updated when the surface moves or an output's scale changes.
//!
//! - **The clipboard and the primary selection.** [`host_selection`]: the parent's selection is offered to the session
//!   while the window has the keyboard, and the session's is offered back, with the limits in [`crate::selection`].
//! - **Tablets.** [`host_tablet`]: the parent's tools and pads become [`TabletEvent`](crate::TabletEvent)s.
//! - **The window frame.** `host_frame`: each toplevel asks the parent to draw its frame (`xdg-decoration`), and a parent
//!   that does not gets a minimal one the session draws as subsurfaces. The pointer over that frame is the frame's own
//!   and never reaches the session.
//!
//! Text input is not bridged: it needs a hook in the [`Backend`](crate::Backend) contract the session's own
//! protocols can call, and `crates/gnoblin-backend/parity.md` lists it as missing on this backend. A parent that offers it
//! sees it unbound.

pub(super) mod dmabuf;
mod host_frame;
pub(super) mod host_selection;
mod host_tablet;

use std::io::Read;

use wayland_client::backend::ObjectId;
use wayland_client::globals::GlobalListContents;
use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_data_device_manager, wl_keyboard, wl_output, wl_pointer, wl_registry,
    wl_seat, wl_shm, wl_shm_pool, wl_subcompositor, wl_surface, wl_touch,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols::wp::cursor_shape::v1::client::{wp_cursor_shape_device_v1, wp_cursor_shape_manager_v1};
use wayland_protocols::wp::fractional_scale::v1::client::{wp_fractional_scale_manager_v1, wp_fractional_scale_v1};
use wayland_protocols::wp::keyboard_shortcuts_inhibit::zv1::client::{
    zwp_keyboard_shortcuts_inhibit_manager_v1, zwp_keyboard_shortcuts_inhibitor_v1,
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1;
use wayland_protocols::wp::pointer_constraints::zv1::client::{
    zwp_confined_pointer_v1, zwp_locked_pointer_v1, zwp_pointer_constraints_v1,
};
use wayland_protocols::wp::pointer_gestures::zv1::client::{
    zwp_pointer_gesture_pinch_v1, zwp_pointer_gesture_swipe_v1, zwp_pointer_gestures_v1,
};
use wayland_protocols::wp::primary_selection::zv1::client::zwp_primary_selection_device_manager_v1;
use wayland_protocols::wp::relative_pointer::zv1::client::{zwp_relative_pointer_manager_v1, zwp_relative_pointer_v1};
use wayland_protocols::wp::tablet::zv2::client::{zwp_tablet_manager_v2, zwp_tablet_seat_v2};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols::xdg::decoration::zv1::client::{zxdg_decoration_manager_v1, zxdg_toplevel_decoration_v1};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols::xdg::toplevel_icon::v1::client::{xdg_toplevel_icon_manager_v1, xdg_toplevel_icon_v1};

use gnoblin_config::NestedDecorations;
use gnoblin_paint::SharedPainter;
use gnoblin_types::{OutputId, Size};

use crate::{PointerConstraint, RawTouchpadKind};

use super::input::position;
use super::records::Table;
use super::scale_from_parent;
use host_frame::{HostFrame, PointerFocus};

/// The index of one nested output, as an object's user data.
///
/// It is the record index rather than the [`OutputId`], and that is deliberate: the
/// index is what the target table is keyed by, so a callback does not have to search
/// for the output it belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct OutputKey(pub usize);

/// Which pool generation and which slot of it a buffer belongs to.
///
/// The generation is what makes a release unambiguous: a resize replaces the pool,
/// and a release for the pool that was replaced must not free a slot of the new one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct SlotKey {
    /// The record index of the output the buffer is drawn for.
    pub index: usize,
    /// The pool generation the slot belongs to.
    pub generation: u32,
    /// The slot within that pool.
    pub slot: u8,
}

/// The proxies of one nested output.
pub(crate) struct Objects {
    /// The surface the output is drawn on.
    pub surface: wl_surface::WlSurface,
    /// The viewport a fractional scale is applied through, if the parent has one.
    pub viewport: Option<wp_viewport::WpViewport>,
    /// The xdg surface that gives the output's surface its window role.
    xdg_surface: xdg_surface::XdgSurface,
    /// The toplevel the parent shows for the output, which carries its title and app id.
    toplevel: xdg_toplevel::XdgToplevel,
    /// The request to the parent to draw the toplevel's frame, when the parent offers `xdg-decoration`.
    decoration: Option<zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1>,
    /// The fractional scale object, present only when the parent offers the protocol.
    fractional: Option<wp_fractional_scale_v1::WpFractionalScaleV1>,
    /// The parent outputs the surface is on, by their object ids, from `wl_surface.enter` and `leave`.
    entered: Vec<ObjectId>,
    /// The request to keep the parent's shortcuts from this surface, when the parent offers the protocol.
    inhibitor: Option<Inhibitor>,
    /// The window frame the session draws for this toplevel, when the parent does not draw one.
    frame: HostFrame,
}

/// One output's shortcut inhibitor and whether the parent granted it.
struct Inhibitor {
    proxy: zwp_keyboard_shortcuts_inhibitor_v1::ZwpKeyboardShortcutsInhibitorV1,
    /// Whether the parent last said `active`, which it does while the window holds the keyboard.
    active: bool,
}

/// One output of the parent, as far as the session follows it: its integer scale and its refresh rate.
pub(crate) struct HostOutput {
    /// The registry name, so a removal names the output it removes.
    pub name: u32,
    /// The bound proxy, whose id a surface's `enter` names.
    pub proxy: wl_output::WlOutput,
    /// The output's integer scale; one until it says otherwise.
    scale: i32,
    /// The current mode's refresh rate in millihertz, or zero when the output has not said.
    refresh_mhz: u32,
}

impl HostOutput {
    /// An output just bound.
    pub fn new(name: u32, proxy: wl_output::WlOutput) -> Self {
        Self {
            name,
            proxy,
            scale: 1,
            refresh_mhz: 0,
        }
    }
}

/// What the connection bound before the first output exists.
pub(crate) struct Bound {
    /// The parent's compositor global.
    pub compositor: wl_compositor::WlCompositor,
    /// The parent's shared-memory global.
    pub shm: wl_shm::WlShm,
    /// The parent's window manager base.
    pub wm_base: xdg_wm_base::XdgWmBase,
    /// The parent's seat.
    pub seat: wl_seat::WlSeat,
    /// The parent's fractional scale manager, if it offers one.
    pub fractional_manager: Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,
    /// The parent's viewporter, if it offers one.
    pub viewporter: Option<wp_viewporter::WpViewporter>,
    /// The parent's shortcut inhibit manager, if it offers one.
    pub shortcuts_manager: Option<zwp_keyboard_shortcuts_inhibit_manager_v1::ZwpKeyboardShortcutsInhibitManagerV1>,
    /// The parent's pointer gestures global, if it offers one.
    pub gestures_manager: Option<zwp_pointer_gestures_v1::ZwpPointerGesturesV1>,
    /// The parent's toplevel icon manager, if it offers one.
    pub icon_manager: Option<xdg_toplevel_icon_manager_v1::XdgToplevelIconManagerV1>,
    /// The parent's relative pointer manager, if it offers one.
    pub relative_manager: Option<zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1>,
    /// The parent's pointer constraints manager, if it offers one.
    pub constraints_manager: Option<zwp_pointer_constraints_v1::ZwpPointerConstraintsV1>,
    /// The parent's data device manager, which carries the clipboard.
    pub data_manager: Option<wl_data_device_manager::WlDataDeviceManager>,
    /// The parent's primary selection manager, if it offers one.
    pub primary_manager: Option<zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1>,
    /// The parent's tablet manager, if it offers one.
    pub tablet_manager: Option<zwp_tablet_manager_v2::ZwpTabletManagerV2>,
    /// The parent's dmabuf global, if it offers one.
    pub dmabuf: Option<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1>,
    /// The parent's subcompositor, if it offers one: the strips of a window frame are subsurfaces.
    pub subcompositor: Option<wl_subcompositor::WlSubcompositor>,
    /// The parent's `xdg-decoration`, if it offers one.
    pub decoration_manager: Option<zxdg_decoration_manager_v1::ZxdgDecorationManagerV1>,
    /// The parent's cursor shape manager, if it offers one.
    pub cursor_shape_manager: Option<wp_cursor_shape_manager_v1::WpCursorShapeManagerV1>,
    /// Every output the parent has.
    pub outputs: Vec<HostOutput>,
}

/// A constraint asked of the parent on one surface.
enum HostConstraint {
    /// The parent holds the pointer in place.
    Lock(zwp_locked_pointer_v1::ZwpLockedPointerV1),
    /// The parent keeps the pointer inside the surface.
    Confinement(zwp_confined_pointer_v1::ZwpConfinedPointerV1),
}

impl HostConstraint {
    fn destroy(self) {
        match self {
            Self::Lock(lock) => lock.destroy(),
            Self::Confinement(confinement) => confinement.destroy(),
        }
    }
}

/// The state every event on the parent connection is dispatched into.
pub(crate) struct Handler {
    /// The per-output records and the parent connection state.
    table: Table,
    /// The proxies for each output, by output index; [`None`] once the output is closed.
    objects: Vec<Option<Objects>>,
    /// The parent's compositor global, used to create each output's surface.
    pub(crate) compositor: wl_compositor::WlCompositor,
    /// The parent's shared-memory global, used to create buffer pools.
    pub(crate) shm: wl_shm::WlShm,
    /// The parent's window manager base, used to give each surface an xdg role.
    wm_base: xdg_wm_base::XdgWmBase,
    /// The parent's seat, which the pointer, keyboard and touch objects come from.
    seat_global: wl_seat::WlSeat,
    /// The parent's fractional scale manager, if it offers one.
    fractional_manager: Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,
    /// The parent's viewporter, if it offers one, used to apply a fractional scale.
    viewporter: Option<wp_viewporter::WpViewporter>,
    /// The seat's pointer, present while the seat advertises pointer.
    pointer: Option<wl_pointer::WlPointer>,
    /// The seat's keyboard, present while the seat advertises keyboard.
    keyboard: Option<wl_keyboard::WlKeyboard>,
    /// The seat's touch device, present while the seat advertises touch.
    touch: Option<wl_touch::WlTouch>,
    /// The XKB keymap the parent sent for the keyboard, or [`None`] until one arrives.
    keymap: Option<String>,
    /// The shared-memory pixel format, chosen from the parent's formats with ARGB preferred.
    format: Option<wl_shm::Format>,
    /// Every output of the parent, for the scale and the refresh rate a surface on it has.
    host_outputs: Vec<HostOutput>,
    /// The parent's shortcut inhibit manager, if it offers one.
    shortcuts_manager: Option<zwp_keyboard_shortcuts_inhibit_manager_v1::ZwpKeyboardShortcutsInhibitManagerV1>,
    /// The parent's pointer gestures global, if it offers one.
    gestures_manager: Option<zwp_pointer_gestures_v1::ZwpPointerGesturesV1>,
    /// The swipe gesture object of the seat's pointer.
    swipe: Option<zwp_pointer_gesture_swipe_v1::ZwpPointerGestureSwipeV1>,
    /// The pinch gesture object of the seat's pointer.
    pinch: Option<zwp_pointer_gesture_pinch_v1::ZwpPointerGesturePinchV1>,
    /// The app id set on every toplevel.
    app_id: String,
    /// The parent's toplevel icon manager, if it offers one.
    icon_manager: Option<xdg_toplevel_icon_manager_v1::XdgToplevelIconManagerV1>,
    /// The icon name set on every toplevel when the parent offers a manager.
    icon_name: Option<String>,
    /// The parent's relative pointer manager, if it offers one.
    relative_manager: Option<zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1>,
    /// The parent's pointer constraints manager, if it offers one.
    constraints_manager: Option<zwp_pointer_constraints_v1::ZwpPointerConstraintsV1>,
    /// The relative pointer of the seat's pointer, while the parent offers one.
    relative: Option<zwp_relative_pointer_v1::ZwpRelativePointerV1>,
    /// What the session's clients hold the pointer to, as the host last told this backend.
    wanted: PointerConstraint,
    /// The constraint asked of the parent now, and the surface it is on.
    held: Option<(HostConstraint, ObjectId)>,
    /// The size an output takes when the parent's configure leaves the size to the session.
    initial_size: Size,
    /// The parent's clipboard and primary selection, and what the session offers back.
    selection: host_selection::Bridge,
    /// The parent's tablet manager, if it offers one.
    tablet_manager: Option<zwp_tablet_manager_v2::ZwpTabletManagerV2>,
    /// The tablet seat asked of the manager, while there is one.
    tablet_seat: Option<zwp_tablet_seat_v2::ZwpTabletSeatV2>,
    /// The parent's tablets, tools and pads.
    tablets: super::tablet::Tablets,
    /// The parent's dmabuf global, what it imports, and the buffers it created for this session.
    dmabuf: dmabuf::DmabufHost,
    /// The parent's subcompositor, if it offers one: the strips of a window frame are subsurfaces.
    subcompositor: Option<wl_subcompositor::WlSubcompositor>,
    /// The parent's `xdg-decoration`, if it offers one: every toplevel asks it to draw the frame.
    decoration_manager: Option<zxdg_decoration_manager_v1::ZxdgDecorationManagerV1>,
    /// Whether the session draws a frame of its own when the parent does not.
    decorations: NestedDecorations,
    /// What the strips of a window frame are painted with.
    painter: SharedPainter,
    /// The parent's cursor shape manager, if it offers one: the cursor over a window frame is the parent's own.
    cursor_shape_manager: Option<wp_cursor_shape_manager_v1::WpCursorShapeManagerV1>,
    /// The cursor shape device of the seat's pointer, while the seat has a pointer and the parent a manager.
    shape_device: Option<wp_cursor_shape_device_v1::WpCursorShapeDeviceV1>,
    /// The serial of the pointer's last enter, which a cursor shape is asked for with.
    pointer_serial: u32,
    /// Whether the pointer is over a strip of a window frame, and where.
    pointer_on: PointerFocus,
}

impl Handler {
    /// A handler for a connection whose globals have been bound.
    pub fn new(
        bound: Bound,
        app_id: String,
        icon_name: Option<String>,
        initial_size: Size,
        decorations: NestedDecorations,
    ) -> Self {
        Self {
            table: Table::new(),
            objects: Vec::new(),
            compositor: bound.compositor,
            shm: bound.shm,
            wm_base: bound.wm_base,
            seat_global: bound.seat,
            fractional_manager: bound.fractional_manager,
            viewporter: bound.viewporter,
            pointer: None,
            keyboard: None,
            touch: None,
            keymap: None,
            format: None,
            host_outputs: bound.outputs,
            shortcuts_manager: bound.shortcuts_manager,
            gestures_manager: bound.gestures_manager,
            swipe: None,
            pinch: None,
            app_id,
            icon_manager: bound.icon_manager,
            icon_name,
            relative_manager: bound.relative_manager,
            constraints_manager: bound.constraints_manager,
            relative: None,
            wanted: PointerConstraint::Free,
            held: None,
            initial_size,
            selection: host_selection::Bridge::new(bound.data_manager, bound.primary_manager),
            tablet_manager: bound.tablet_manager,
            tablet_seat: None,
            tablets: super::tablet::Tablets::new(),
            dmabuf: dmabuf::DmabufHost::new(bound.dmabuf),
            subcompositor: bound.subcompositor,
            decoration_manager: bound.decoration_manager,
            decorations,
            painter: SharedPainter::system(),
            cursor_shape_manager: bound.cursor_shape_manager,
            shape_device: None,
            pointer_serial: 0,
            pointer_on: PointerFocus::Elsewhere,
        }
    }

    /// Creates one nested output's surface and asks the parent to place it.
    pub fn create_output(
        &mut self,
        qh: &QueueHandle<Self>,
        output: OutputId,
        connector: &str,
        title: &str,
        fullscreen: bool,
    ) -> usize {
        let index = self.objects.len();
        let surface = self.compositor.create_surface(qh, ());
        let xdg_surface = self.wm_base.get_xdg_surface(&surface, qh, OutputKey(index));
        let toplevel = xdg_surface.get_toplevel(qh, OutputKey(index));
        toplevel.set_app_id(self.app_id.clone());
        toplevel.set_title(title.to_owned());
        // Every toplevel asks the parent to draw its frame. What it answers decides whether the session draws one: a
        // server-side answer is the parent's (or the shell the parent runs) to draw, and anything else is read when the
        // configure arrives.
        let decoration = self.decoration_manager.as_ref().map(|manager| {
            let decoration = manager.get_toplevel_decoration(&toplevel, qh, OutputKey(index));
            decoration.set_mode(zxdg_toplevel_decoration_v1::Mode::ServerSide);
            decoration
        });
        // The icon is advisory: a parent without the manager shows its own default, and a name its theme does not
        // have shows the same.
        if let (Some(manager), Some(name)) = (&self.icon_manager, &self.icon_name) {
            let icon = manager.create_icon(qh, ());
            icon.set_name(name.clone());
            manager.set_icon(&toplevel, Some(&icon));
            icon.destroy();
        }
        if fullscreen {
            // A session that fills the parent's output is what the host expects.
            // A parent that does not implement fullscreen answers with a configure
            // of its own choosing, which the output follows all the same.
            toplevel.set_fullscreen(None);
        }
        let fractional = self
            .fractional_manager
            .as_ref()
            .map(|manager| manager.get_fractional_scale(&surface, qh, OutputKey(index)));
        let viewport = self
            .viewporter
            .as_ref()
            .map(|viewporter| viewporter.get_viewport(&surface, qh, ()));
        // The commit with no buffer is the one that asks for the first configure:
        // xdg-shell states that a surface with no buffer and no configure gets one.
        surface.commit();
        let inhibitor = self.shortcuts_manager.as_ref().map(|manager| Inhibitor {
            proxy: manager.inhibit_shortcuts(&surface, &self.seat_global, qh, OutputKey(index)),
            active: false,
        });
        self.objects.push(Some(Objects {
            surface,
            viewport,
            xdg_surface,
            toplevel,
            fractional,
            entered: Vec::new(),
            inhibitor,
            decoration,
            frame: HostFrame::new(title, self.initial_size),
        }));
        self.table.add(output, connector)
    }

    /// The session's clients now hold the pointer to `wanted`: ask the parent to hold it the same way.
    pub fn pointer_constraint(&mut self, qh: &QueueHandle<Self>, wanted: PointerConstraint) {
        self.wanted = wanted;
        self.apply_constraint(qh);
    }

    /// Gives back the constraint asked of the parent, if one is held. The parent's relative motion stops being the
    /// session's.
    fn release_constraint(&mut self) {
        if let Some((held, _)) = self.held.take() {
            held.destroy();
        }
        self.table.seat().set_locked(false);
    }

    /// Brings the parent's constraint in line with what the session's clients hold, on the surface the pointer is
    /// over. A constraint already on that surface of the right kind is left alone, so a pointer that moves inside a
    /// window does not unlock and lock the parent's pointer.
    fn apply_constraint(&mut self, qh: &QueueHandle<Self>) {
        let surface = self
            .table
            .seat()
            .pointer_output()
            .and_then(|output| self.surface_of_output(output));
        let wanted = self.wanted;
        let already = matches!(
            (&self.held, surface.as_ref()),
            (Some((held, on)), Some(surface))
                if *on == surface.id()
                    && matches!(
                        (held, wanted),
                        (HostConstraint::Lock(_), PointerConstraint::Locked) | (HostConstraint::Confinement(_), PointerConstraint::Confined)
                    )
        );
        if already {
            return;
        }
        self.release_constraint();
        let (Some(manager), Some(pointer), Some(surface)) = (&self.constraints_manager, &self.pointer, surface) else {
            return;
        };
        // Persistent: the parent releases the pointer when the window loses the focus and holds it again when the
        // focus returns, which is what a game's lock is meant to survive.
        let lifetime = zwp_pointer_constraints_v1::Lifetime::Persistent;
        let held = match wanted {
            PointerConstraint::Free => return,
            PointerConstraint::Locked => {
                HostConstraint::Lock(manager.lock_pointer(&surface, pointer, None, lifetime, qh, ()))
            }
            PointerConstraint::Confined => {
                HostConstraint::Confinement(manager.confine_pointer(&surface, pointer, None, lifetime, qh, ()))
            }
        };
        self.held = Some((held, surface.id()));
    }

    /// The surface an output is drawn on.
    fn surface_of_output(&self, output: OutputId) -> Option<wl_surface::WlSurface> {
        let index = (0..self.objects.len()).find(|index| self.table.output(*index) == Some(output))?;
        self.objects(index).map(|objects| objects.surface.clone())
    }

    /// Destroys one output's objects, after the parent asked for it to close.
    pub fn destroy_output(&mut self, index: usize) {
        self.forget_dmabuf_buffers_of(index);
        if let Some(mut objects) = self.objects.get_mut(index).and_then(Option::take) {
            if matches!(self.pointer_on, PointerFocus::Frame { index: on, .. } if on == index) {
                self.pointer_on = PointerFocus::Elsewhere;
            }
            objects.frame.destroy();
            if self.held.as_ref().is_some_and(|(_, on)| *on == objects.surface.id()) {
                self.release_constraint();
            }
            if let Some(inhibitor) = objects.inhibitor {
                inhibitor.proxy.destroy();
            }
            if let Some(fractional) = objects.fractional {
                fractional.destroy();
            }
            if let Some(viewport) = objects.viewport {
                viewport.destroy();
            }
            // The decoration object goes before the toplevel it belongs to: the other order is a protocol error.
            if let Some(decoration) = objects.decoration {
                decoration.destroy();
            }
            objects.toplevel.destroy();
            objects.xdg_surface.destroy();
            objects.surface.destroy();
        }
    }

    /// One output's proxies.
    pub fn objects(&self, index: usize) -> Option<&Objects> {
        self.objects.get(index).and_then(Option::as_ref)
    }

    /// The parent-facing state.
    pub fn table(&self) -> &Table {
        &self.table
    }

    /// The parent-facing state, mutably.
    pub fn table_mut(&mut self) -> &mut Table {
        &mut self.table
    }

    /// The `wl_shm` format the parent and this backend agree on, once known.
    pub fn format(&self) -> Option<wl_shm::Format> {
        self.format
    }

    /// The refresh rate of the parent output a surface is on, in millihertz, or of the first output that reports one
    /// when it is on none yet. Zero when the parent has not said.
    pub fn refresh_mhz(&self, index: usize) -> u32 {
        let entered = self
            .objects
            .get(index)
            .and_then(Option::as_ref)
            .map(|objects| &objects.entered);
        entered
            .into_iter()
            .flatten()
            .filter_map(|id| self.host_outputs.iter().find(|output| output.proxy.id() == *id))
            .chain(self.host_outputs.iter())
            .map(|output| output.refresh_mhz)
            .find(|refresh| *refresh > 0)
            .unwrap_or(0)
    }

    /// Whether the parent granted the shortcut inhibitor of one output: [`None`] when the parent does not offer the
    /// protocol, so the session's shortcuts are the parent's whenever the parent wants them.
    pub fn shortcuts_inhibited(&self, index: usize) -> Option<bool> {
        self.objects
            .get(index)
            .and_then(Option::as_ref)
            .and_then(|objects| objects.inhibitor.as_ref())
            .map(|inhibitor| inhibitor.active)
    }

    /// Recomputes the integer scale of one surface from the outputs it is on, and tells the table. A surface that
    /// is on no output yet keeps the scale it has.
    fn rescale(&mut self, index: usize) {
        let Some(objects) = self.objects.get(index).and_then(Option::as_ref) else {
            return;
        };
        let scale = objects
            .entered
            .iter()
            .filter_map(|id| self.host_outputs.iter().find(|output| output.proxy.id() == *id))
            .map(|output| output.scale.max(1))
            .max();
        if let Some(scale) = scale {
            self.table
                .prefer_integer_scale(index, u32::try_from(scale).unwrap_or(1));
        }
    }

    /// Recomputes the scale of every surface that is on the output `id`.
    fn rescale_on(&mut self, id: &ObjectId) {
        let indices: Vec<usize> = self
            .objects
            .iter()
            .enumerate()
            .filter(|(_, objects)| objects.as_ref().is_some_and(|objects| objects.entered.contains(id)))
            .map(|(index, _)| index)
            .collect();
        for index in indices {
            self.rescale(index);
        }
    }

    /// The parent's keymap, as the `xkb_v1` text it sent.
    ///
    /// [`None`] until the parent has sent one, which is a parent with no keyboard.
    pub fn keymap(&self) -> Option<&str> {
        self.keymap.as_deref()
    }

    /// Whether the parent offers `wp_fractional_scale_v1`, so the session can follow a fractional scale exactly.
    pub fn fractional_scale_offered(&self) -> bool {
        self.fractional_manager.is_some()
    }

    /// How many tablet tool events could not be attributed to an output of the session.
    pub fn tablets_ignored(&self) -> u64 {
        self.tablets.ignored()
    }

    /// What the connection learnt that the session's host has to hear.
    pub fn take_selection_notes(&mut self) -> Vec<host_selection::SelectionNote> {
        self.selection.take_notes()
    }

    /// A client of the session reads the parent's selection.
    pub fn read_selection(
        &mut self,
        target: crate::SelectionTarget,
        mime: &str,
        fd: std::os::fd::OwnedFd,
    ) -> Result<(), crate::SelectionRefusal> {
        self.selection.read(target, mime, fd)
    }

    /// A client of the session set or cleared a selection.
    pub fn offer_selection(
        &mut self,
        qh: &QueueHandle<Self>,
        target: crate::SelectionTarget,
        mime_types: Option<Vec<String>>,
    ) {
        self.selection.offer(target, mime_types, qh);
    }

    /// The output a surface this backend draws on belongs to.
    ///
    /// Answering [`None`] for a surface that has not been configured is what stops
    /// input being attributed to an output that does not exist yet.
    fn output_of(&self, surface: &wl_surface::WlSurface) -> Option<OutputId> {
        let id = surface.id();
        let index = self
            .objects
            .iter()
            .position(|objects| objects.as_ref().is_some_and(|objects| objects.surface.id() == id))?;
        self.table.applied(index)?;
        self.table.output(index)
    }

    /// Records one `wl_shm` format, keeping the first one this backend can draw.
    ///
    /// `Argb8888` is preferred because it is the only 32-bit format every compositor
    /// offers and it carries the alpha the frame already has; `Xrgb8888` is taken
    /// when it is all the parent offers.
    fn note_format(&mut self, format: wl_shm::Format) {
        match (self.format, format) {
            (Some(wl_shm::Format::Argb8888), _) => {}
            (_, wl_shm::Format::Argb8888) => self.format = Some(format),
            (None, wl_shm::Format::Xrgb8888) => self.format = Some(format),
            (Some(_), wl_shm::Format::Xrgb8888) => {}
            _ => {}
        }
    }

    /// Reads the keymap out of the descriptor the parent sent.
    ///
    /// A keymap that is not `xkb_v1`, or one that cannot be read, is left unset: the
    /// backend states scan codes either way, and a session with no keymap is a
    /// session whose seat composes its own.
    fn note_keymap(&mut self, format: &WEnum<wl_keyboard::KeymapFormat>, fd: std::os::fd::OwnedFd, size: u32) {
        if !matches!(format, WEnum::Value(wl_keyboard::KeymapFormat::XkbV1)) {
            return;
        }
        let file = std::fs::File::from(fd);
        let mut text = String::new();
        // The size is the parent's, so it is capped rather than trusted: a keymap
        // larger than this is a bug or an attack, and neither should be a read of
        // whatever the descriptor points at.
        let limit = size.min(MAX_KEYMAP_BYTES);
        if file.take(u64::from(limit)).read_to_string(&mut text).is_ok() {
            self.keymap = Some(text.trim_end_matches('\0').to_owned());
        }
    }

    /// Creates the seat objects the parent says it has.
    fn note_capabilities(&mut self, qh: &QueueHandle<Self>, capabilities: wl_seat::Capability) {
        let keyboard = capabilities.contains(wl_seat::Capability::Keyboard);
        let pointer = capabilities.contains(wl_seat::Capability::Pointer);
        let touch = capabilities.contains(wl_seat::Capability::Touch);
        // The selection devices and the tablet seat belong to the seat, not to one of its capabilities: a tablet or a
        // clipboard needs no keyboard to exist, and the selection is only offered while the keyboard is focused.
        self.selection.bind_devices(&self.seat_global, qh);
        self.bind_tablets(&self.seat_global.clone(), qh);
        if keyboard {
            if self.keyboard.is_none() {
                self.keyboard = Some(self.seat_global.get_keyboard(qh, ()));
            }
        } else if let Some(keyboard) = self.keyboard.take() {
            keyboard.release();
            self.keymap = None;
        }
        if pointer {
            if self.pointer.is_none() {
                let proxy = self.seat_global.get_pointer(qh, ());
                if let Some(manager) = &self.gestures_manager {
                    self.swipe = Some(manager.get_swipe_gesture(&proxy, qh, ()));
                    self.pinch = Some(manager.get_pinch_gesture(&proxy, qh, ()));
                }
                if let Some(manager) = &self.relative_manager {
                    self.relative = Some(manager.get_relative_pointer(&proxy, qh, ()));
                }
                if let Some(manager) = &self.cursor_shape_manager {
                    self.shape_device = Some(manager.get_pointer(&proxy, qh, ()));
                }
                self.pointer = Some(proxy);
            }
        } else if let Some(pointer) = self.pointer.take() {
            self.release_constraint();
            if let Some(relative) = self.relative.take() {
                relative.destroy();
            }
            if let Some(swipe) = self.swipe.take() {
                swipe.destroy();
            }
            if let Some(pinch) = self.pinch.take() {
                pinch.destroy();
            }
            if let Some(device) = self.shape_device.take() {
                device.destroy();
            }
            pointer.release();
        }
        if touch {
            if self.touch.is_none() {
                self.touch = Some(self.seat_global.get_touch(qh, ()));
            }
        } else if let Some(touch) = self.touch.take() {
            touch.release();
        }
        // The devices the session's seat lists are the ones the parent's seat has: a keyboard, a pointer that does
        // gestures when the parent offers them, and touch.
        self.table
            .seat()
            .set_capabilities(keyboard, pointer, touch, self.gestures_manager.is_some());
    }
}

/// The largest keymap this backend will read from the parent.
///
/// A real `xkb_v1` keymap for a large layout is tens of kilobytes; this is three
/// orders of magnitude above that, so the limit never truncates a working keymap and
/// always bounds a hostile one.
const MAX_KEYMAP_BYTES: u32 = 4 * 1024 * 1024;

/// Whether a pointer axis event is horizontal.
fn is_horizontal(axis: &WEnum<wl_pointer::Axis>) -> bool {
    matches!(axis, WEnum::Value(wl_pointer::Axis::HorizontalScroll))
}

// The interfaces this backend binds and only ever listens to, or binds and never
// hears anything from.
wayland_client::delegate_noop!(Handler: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(Handler: ignore zwp_keyboard_shortcuts_inhibit_manager_v1::ZwpKeyboardShortcutsInhibitManagerV1);
wayland_client::delegate_noop!(Handler: ignore zwp_pointer_gestures_v1::ZwpPointerGesturesV1);
wayland_client::delegate_noop!(Handler: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(Handler: ignore wp_viewporter::WpViewporter);
wayland_client::delegate_noop!(Handler: ignore wp_viewport::WpViewport);
wayland_client::delegate_noop!(Handler: ignore xdg_toplevel_icon_manager_v1::XdgToplevelIconManagerV1);
wayland_client::delegate_noop!(Handler: ignore xdg_toplevel_icon_v1::XdgToplevelIconV1);
wayland_client::delegate_noop!(Handler: ignore zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1);
wayland_client::delegate_noop!(Handler: ignore zwp_pointer_constraints_v1::ZwpPointerConstraintsV1);
wayland_client::delegate_noop!(Handler: ignore zwp_confined_pointer_v1::ZwpConfinedPointerV1);

impl Dispatch<zwp_locked_pointer_v1::ZwpLockedPointerV1, ()> for Handler {
    fn event(
        state: &mut Self,
        _: &zwp_locked_pointer_v1::ZwpLockedPointerV1,
        event: <zwp_locked_pointer_v1::ZwpLockedPointerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The parent says when the lock is held: the window may not have the focus yet, and loses the lock when it
        // loses it, so the session's relative motion follows these two events and not the request.
        match event {
            zwp_locked_pointer_v1::Event::Locked => state.table.seat().set_locked(true),
            zwp_locked_pointer_v1::Event::Unlocked => state.table.seat().set_locked(false),
            _ => {}
        }
    }
}

impl Dispatch<zwp_relative_pointer_v1::ZwpRelativePointerV1, ()> for Handler {
    fn event(
        state: &mut Self,
        _: &zwp_relative_pointer_v1::ZwpRelativePointerV1,
        event: <zwp_relative_pointer_v1::ZwpRelativePointerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_relative_pointer_v1::Event::RelativeMotion { dx, dy, .. } = event {
            state.table.seat().relative_motion(dx, dy);
        }
    }
}
wayland_client::delegate_noop!(Handler: ignore wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1);

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Handler {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: <wl_registry::WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        // `registry_queue_init` keeps the global list in the object's data and consumes the globals that were there
        // at the start. What reaches here is an output the parent plugged in or removed since.
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == wl_output::WlOutput::interface().name => {
                let proxy = registry.bind::<wl_output::WlOutput, _, _>(name, version.min(4), qh, ());
                state.host_outputs.push(HostOutput::new(name, proxy));
            }
            wl_registry::Event::GlobalRemove { name } => {
                let Some(position) = state.host_outputs.iter().position(|output| output.name == name) else {
                    return;
                };
                let gone = state.host_outputs.remove(position);
                let id = gone.proxy.id();
                for objects in state.objects.iter_mut().flatten() {
                    objects.entered.retain(|entered| *entered != id);
                }
                if gone.proxy.version() >= 3 {
                    gone.proxy.release();
                }
                for index in 0..state.objects.len() {
                    state.rescale(index);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for Handler {
    fn event(
        state: &mut Self,
        surface: &wl_surface::WlSurface,
        event: <wl_surface::WlSurface as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let id = surface.id();
        let Some(index) = state
            .objects
            .iter()
            .position(|objects| objects.as_ref().is_some_and(|objects| objects.surface.id() == id))
        else {
            return;
        };
        match event {
            wl_surface::Event::Enter { output } => {
                if let Some(Some(objects)) = state.objects.get_mut(index)
                    && !objects.entered.contains(&output.id())
                {
                    objects.entered.push(output.id());
                }
                state.rescale(index);
            }
            wl_surface::Event::Leave { output } => {
                if let Some(Some(objects)) = state.objects.get_mut(index) {
                    objects.entered.retain(|entered| *entered != output.id());
                }
                state.rescale(index);
            }
            _ => {}
        }
    }
}

impl Dispatch<zwp_keyboard_shortcuts_inhibitor_v1::ZwpKeyboardShortcutsInhibitorV1, OutputKey> for Handler {
    fn event(
        state: &mut Self,
        _: &zwp_keyboard_shortcuts_inhibitor_v1::ZwpKeyboardShortcutsInhibitorV1,
        event: <zwp_keyboard_shortcuts_inhibitor_v1::ZwpKeyboardShortcutsInhibitorV1 as Proxy>::Event,
        key: &OutputKey,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let active = match event {
            zwp_keyboard_shortcuts_inhibitor_v1::Event::Active => true,
            zwp_keyboard_shortcuts_inhibitor_v1::Event::Inactive => false,
            _ => return,
        };
        if let Some(inhibitor) = state
            .objects
            .get_mut(key.0)
            .and_then(Option::as_mut)
            .and_then(|objects| objects.inhibitor.as_mut())
        {
            inhibitor.active = active;
        }
    }
}

impl Dispatch<zwp_pointer_gesture_swipe_v1::ZwpPointerGestureSwipeV1, ()> for Handler {
    fn event(
        state: &mut Self,
        _: &zwp_pointer_gesture_swipe_v1::ZwpPointerGestureSwipeV1,
        event: <zwp_pointer_gesture_swipe_v1::ZwpPointerGestureSwipeV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwp_pointer_gesture_swipe_v1::Event;
        let seat = state.table.seat();
        match event {
            Event::Begin { fingers, .. } => seat.gesture_begin(RawTouchpadKind::Swipe, fingers),
            Event::Update { dx, dy, .. } => seat.gesture_update(RawTouchpadKind::Swipe, dx, dy, 1.0, 0.0),
            Event::End { cancelled, .. } => seat.gesture_end(RawTouchpadKind::Swipe, cancelled != 0),
            _ => {}
        }
    }
}

impl Dispatch<zwp_pointer_gesture_pinch_v1::ZwpPointerGesturePinchV1, ()> for Handler {
    fn event(
        state: &mut Self,
        _: &zwp_pointer_gesture_pinch_v1::ZwpPointerGesturePinchV1,
        event: <zwp_pointer_gesture_pinch_v1::ZwpPointerGesturePinchV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwp_pointer_gesture_pinch_v1::Event;
        let seat = state.table.seat();
        match event {
            Event::Begin { fingers, .. } => seat.gesture_begin(RawTouchpadKind::Pinch, fingers),
            Event::Update {
                dx,
                dy,
                scale,
                rotation,
                ..
            } => seat.gesture_update(RawTouchpadKind::Pinch, dx, dy, scale, rotation),
            Event::End { cancelled, .. } => seat.gesture_end(RawTouchpadKind::Pinch, cancelled != 0),
            _ => {}
        }
    }
}

impl Dispatch<wl_shm::WlShm, ()> for Handler {
    fn event(
        state: &mut Self,
        _: &wl_shm::WlShm,
        event: <wl_shm::WlShm as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_shm::Event::Format {
            format: WEnum::Value(format),
        } = event
        {
            state.note_format(format);
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for Handler {
    fn event(
        state: &mut Self,
        output: &wl_output::WlOutput,
        event: <wl_output::WlOutput as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let id = output.id();
        match event {
            wl_output::Event::Scale { factor } => {
                // A surface on this output composes at its scale, so every surface that is on it is told.
                if let Some(known) = state.host_outputs.iter_mut().find(|known| known.proxy.id() == id) {
                    known.scale = factor.max(1);
                }
                state.rescale_on(&id);
            }
            wl_output::Event::Mode {
                flags: WEnum::Value(flags),
                refresh,
                ..
            } if flags.contains(wl_output::Mode::Current) => {
                if let Some(known) = state.host_outputs.iter_mut().find(|known| known.proxy.id() == id) {
                    known.refresh_mhz = u32::try_from(refresh).unwrap_or(0);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for Handler {
    fn event(
        state: &mut Self,
        _: &wl_seat::WlSeat,
        event: <wl_seat::WlSeat as Proxy>::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        {
            state.note_capabilities(qh, capabilities);
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for Handler {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: <wl_keyboard::WlKeyboard as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Keymap { format, fd, size } => state.note_keymap(&format, fd, size),
            wl_keyboard::Event::Key {
                key,
                state: key_state,
                serial,
                ..
            } => {
                state.selection.note_serial(serial);
                match key_state {
                    WEnum::Value(wl_keyboard::KeyState::Pressed) => state.table.seat().key(key, true),
                    WEnum::Value(wl_keyboard::KeyState::Released) => state.table.seat().key(key, false),
                    _ => {}
                }
            }
            // The parent took the keyboard away from the session's window: every key the session saw pressed is
            // released, or a modifier held while switching windows would stay held in the session for good. The
            // host's selections go with the focus.
            wl_keyboard::Event::Leave { .. } => {
                state.table.seat().release_keys();
                state.selection.focus_out();
            }
            // Which of the parent's surfaces holds the focus is the parent's own business, and the keys held when
            // focus arrives belong to the person, not to a press the session saw. What does arrive with it is the
            // parent's selection, which is offered to the session from here on.
            wl_keyboard::Event::Enter { serial, .. } => state.selection.focus_in(serial),
            _ => {}
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for Handler {
    fn event(
        state: &mut Self,
        pointer: &wl_pointer::WlPointer,
        event: <wl_pointer::WlPointer as Proxy>::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter {
                serial,
                surface,
                surface_x,
                surface_y,
            } => {
                if let Some((index, part)) = state.frame_part(&surface) {
                    // A strip of the session's own window frame: the cursor over it is the parent's, and the pointer is
                    // not over anything the session draws.
                    state.frame_enter(index, part, serial, surface_x, surface_y);
                    return;
                }
                state.pointer_on = PointerFocus::Elsewhere;
                // The session draws its own cursor into the frame, so the parent shows none over it.
                pointer.set_cursor(serial, None, 0, 0);
                if let Some(output) = state.output_of(&surface) {
                    state.table.seat().enter(output, position(surface_x, surface_y));
                    // The pointer is on this window now: a constraint the session's clients hold follows it.
                    state.apply_constraint(qh);
                }
            }
            wl_pointer::Event::Leave { .. } => {
                if matches!(state.pointer_on, PointerFocus::Frame { .. }) {
                    state.frame_leave();
                } else {
                    state.table.seat().leave();
                }
            }
            wl_pointer::Event::Motion {
                surface_x, surface_y, ..
            } => {
                if matches!(state.pointer_on, PointerFocus::Frame { .. }) {
                    state.frame_motion(surface_x, surface_y);
                } else {
                    state.table.seat().motion(position(surface_x, surface_y));
                }
            }
            wl_pointer::Event::Button {
                button,
                state: button_state,
                serial,
                time,
            } => {
                state.selection.note_serial(serial);
                let pressed = match button_state {
                    WEnum::Value(wl_pointer::ButtonState::Pressed) => true,
                    WEnum::Value(wl_pointer::ButtonState::Released) => false,
                    _ => return,
                };
                if matches!(state.pointer_on, PointerFocus::Frame { .. }) {
                    state.frame_button(time, serial, button, pressed);
                } else {
                    state.table.seat().button(button, pressed);
                }
            }
            // The scroll wheel over the window frame scrolls nothing of the session's.
            wl_pointer::Event::Axis { .. }
            | wl_pointer::Event::AxisDiscrete { .. }
            | wl_pointer::Event::AxisValue120 { .. }
                if matches!(state.pointer_on, PointerFocus::Frame { .. }) => {}
            wl_pointer::Event::Axis { axis, value, .. } => {
                state.table.seat().axis(is_horizontal(&axis), value);
            }
            wl_pointer::Event::AxisDiscrete { axis, discrete } => {
                // `wl_pointer.axis_discrete` counts whole steps and is deprecated in
                // favour of `axis_value120`, which states the same movement at a
                // finer resolution. A step is 120 of those units.
                state
                    .table
                    .seat()
                    .axis_discrete(is_horizontal(&axis), discrete.saturating_mul(super::input::AXIS_STEP));
            }
            wl_pointer::Event::AxisValue120 { axis, value120 } => {
                state.table.seat().axis_discrete(is_horizontal(&axis), value120);
            }
            // A frame ends the events of one input timestamp, which is when the
            // accumulated scroll becomes one event.
            wl_pointer::Event::Frame => state.table.seat().frame(),
            _ => {}
        }
    }
}

impl Dispatch<wl_touch::WlTouch, ()> for Handler {
    fn event(
        state: &mut Self,
        _: &wl_touch::WlTouch,
        event: <wl_touch::WlTouch as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_touch::Event::Down {
                serial,
                surface,
                id,
                x,
                y,
                ..
            } => {
                if let Some((index, part)) = state.frame_part(&surface) {
                    // A touch on the window frame is the frame's: a drag on the title bar moves the window.
                    state.frame_touch(index, part, serial, x, y);
                    return;
                }
                let output = state.output_of(&surface);
                state.table.seat().touch_down(id.unsigned_abs(), output, position(x, y));
            }
            wl_touch::Event::Motion { id, x, y, .. } => {
                state.table.seat().touch_motion(id.unsigned_abs(), position(x, y));
            }
            wl_touch::Event::Up { id, .. } => state.table.seat().touch_up(id.unsigned_abs()),
            wl_touch::Event::Cancel => state.table.seat().touch_cancel(),
            _ => {}
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, SlotKey> for Handler {
    fn event(
        state: &mut Self,
        _: &wl_buffer::WlBuffer,
        event: <wl_buffer::WlBuffer as Proxy>::Event,
        key: &SlotKey,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            state.table.released(key.index, key.generation, key.slot);
        }
    }
}

impl Dispatch<wl_callback::WlCallback, OutputKey> for Handler {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: <wl_callback::WlCallback as Proxy>::Event,
        key: &OutputKey,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.table.frame_done(key.0);
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for Handler {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: <xdg_wm_base::XdgWmBase as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // A parent that does not get its ping answered may treat the session as
        // unresponsive, so the answer is sent here rather than queued.
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, OutputKey> for Handler {
    fn event(
        state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: <xdg_surface::XdgSurface as Proxy>::Event,
        key: &OutputKey,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The size the configure belongs to arrives in `xdg_toplevel.configure` in the same batch, and the host's answer to
        // the frame in `zxdg_toplevel_decoration_v1.configure`: this event ends the batch, so they are read together
        // here. The target applies the size before its next commit, so the acknowledgement is immediate and the buffer
        // always matches what it acknowledges.
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            state.close_configure(key.0);
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, OutputKey> for Handler {
    fn event(
        state: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: <xdg_toplevel::XdgToplevel as Proxy>::Event,
        key: &OutputKey,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_toplevel::Event::Configure { width, height, states } => {
                state.note_window_configure(key.0, width, height, &states);
            }
            xdg_toplevel::Event::Close => state.table.close(key.0),
            _ => {}
        }
    }
}

impl Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, OutputKey> for Handler {
    fn event(
        state: &mut Self,
        _: &wp_fractional_scale_v1::WpFractionalScaleV1,
        event: <wp_fractional_scale_v1::WpFractionalScaleV1 as Proxy>::Event,
        key: &OutputKey,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            state.table.prefer_scale(key.0, scale_from_parent(scale));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::UNCONSTRAINED_SIZE;
    use super::super::frame::{Inset, content_size};
    use super::*;

    #[test]
    fn a_configure_size_is_the_size_the_parent_asked_for() {
        assert_eq!(
            content_size(1024, 768, Inset::NONE, UNCONSTRAINED_SIZE),
            Size::new(1024, 768)
        );
    }

    #[test]
    fn a_configure_that_leaves_the_size_to_us_is_not_an_empty_output() {
        assert_eq!(content_size(0, 0, Inset::NONE, UNCONSTRAINED_SIZE), UNCONSTRAINED_SIZE);
        // One axis stated and one left out is not a size either.
        assert_eq!(
            content_size(1024, 0, Inset::NONE, UNCONSTRAINED_SIZE),
            UNCONSTRAINED_SIZE
        );
        assert_eq!(
            content_size(-1, 768, Inset::NONE, UNCONSTRAINED_SIZE),
            UNCONSTRAINED_SIZE
        );
    }

    #[test]
    fn a_session_that_configured_a_starting_size_gets_it_where_the_parent_leaves_the_size_open() {
        let chosen = Size::new(1600, 900);
        assert_eq!(content_size(0, 0, Inset::NONE, chosen), chosen);
        assert_eq!(content_size(800, 600, Inset::NONE, chosen), Size::new(800, 600));
    }

    #[test]
    fn an_horizontal_axis_is_the_one_the_protocol_calls_horizontal() {
        assert!(is_horizontal(&WEnum::Value(wl_pointer::Axis::HorizontalScroll)));
        assert!(!is_horizontal(&WEnum::Value(wl_pointer::Axis::VerticalScroll)));
        assert!(!is_horizontal(&WEnum::Unknown(7)));
    }
}
