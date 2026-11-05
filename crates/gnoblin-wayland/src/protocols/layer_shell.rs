//! `zwlr_layer_shell_v1`: bars, docks, overlays and what they reserve.
//!
//! A layer surface is placed against an output's edges rather than by the window
//! manager: the client names its layer, its anchors, its margins and its exclusive
//! zone, and the compositor answers with a configure size. This module is the whole
//! of that: it computes the rectangle, sends the configure, and turns the exclusive
//! zones into [`gnoblin_wm::Event::OutputReserved`] so the work area a maximised
//! window gets is the space the bars left over.
//!
//! ## What a layer surface is not
//!
//! A layer surface is not a [`gnoblin_wm`] window. The window manager places, stacks
//! and focuses windows; a bar is placed by its anchors and focused by its keyboard
//! interactivity, so routing it through the policy would invent a window with a
//! work-area constraint of its own. The two meet at exactly one point, the reserved
//! insets, which is the same seam a configuration's monitor insets would use.
//!
//! ## Drawing
//!
//! This slice composes application windows; a layer surface's buffer is not in the
//! scene yet, so a bar reserves its strip and draws nothing. That is why the tests
//! assert on the configure size, the published rectangle and the work area rather
//! than on pixels.
//!
//! ## Failure modes first
//!
//! Each is a test in `tests/layer_shell.rs` unless it says otherwise.
//!
//! 1. **A layer surface with no anchors and no size covers the output.** The
//!    protocol has no answer for "fill the screen" without anchors, and a surface
//!    that asked for zero in one axis without anchoring both edges is a
//!    client error: Smithay refuses the commit, the client is disconnected and the
//!    session, the work area and every other client are untouched —
//!    `a_zero_size_surface_with_no_anchors_is_a_client_error` (and the pure
//!    `place` unit tests below for the arithmetic).
//! 2. **An exclusive zone survives the surface it belongs to.** The reserved insets
//!    are recomputed from the surfaces that are alive on every change and every
//!    destruction, so a bar that exits gives its strip back —
//!    `a_bar_that_disappears_gives_its_strip_back`.
//! 3. **An exclusive zone is honoured for a surface that may not reserve anything.**
//!    A surface anchored to both edges of an axis, or to neither, reserves nothing
//!    on it (`reserve`), because its size is not a strip along one edge —
//!    `an_exclusive_zone_needs_one_anchored_edge`.
//! 4. **A bar's strip is taken out of the screen instead of the work area.** The
//!    reserved insets go to the window manager, which recomputes the work area, so a
//!    maximised window shrinks and an unmapped output keeps its full rectangle —
//!    `a_bar_shrinks_the_work_area_a_maximised_window_gets`.
//! 5. **A keyboard-exclusive surface loses focus to the next window that maps.** The
//!    grab is recomputed after every change and the window manager's focus effects
//!    respect it, so an exclusive top-layer surface keeps the keyboard —
//!    `an_exclusive_surface_keeps_the_keyboard`.
//! 6. **A layer surface reserves against an output that is gone.** A disconnected
//!    output drops its reservation and its surfaces' rectangles instead of leaving a
//!    strip reserved for ever: `a_disconnected_output_reserves_nothing`.

use smithay::output::Output as SmithayOutput;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Size};
use smithay::wayland::shell::wlr_layer::{
    Anchor, ExclusiveZone, KeyboardInteractivity, Layer, LayerSurface, LayerSurfaceCachedState, LayerSurfaceConfigure,
    WlrLayerShellHandler, WlrLayerShellState,
};
use tracing::{debug, trace};

use gnoblin_types::{Insets, OutputId, Rect};

use crate::State;
use crate::observation::ObservedLayer;

impl WlrLayerShellHandler for State {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(&mut self, surface: LayerSurface, output: Option<WlOutput>, layer: Layer, namespace: String) {
        self.layer_created(surface, output, layer, namespace);
    }

    fn new_popup(&mut self, parent: LayerSurface, popup: smithay::wayland::shell::xdg::PopupSurface) {
        self.layer_popup_created(&parent, &popup);
    }

    fn ack_configure(&mut self, surface: WlSurface, configure: LayerSurfaceConfigure) {
        // Like a toplevel, a layer surface's acknowledgement is its own business:
        // the next commit carries the state and nothing waits on it here.
        trace!(
            serial = ?configure.serial,
            size = ?configure.state.size,
            surface = ?surface.id(),
            "[layer] configure acknowledged"
        );
    }

    fn layer_destroyed(&mut self, surface: LayerSurface) {
        self.layer_destroyed(&surface);
    }
}

/// Which edge of an output a surface's exclusive zone lies along.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

/// One layer surface the session is showing.
struct LayerEntry {
    /// The protocol object, which is also what a configure is sent through.
    surface: LayerSurface,
    /// The namespace the client named, which is how a shell recognises its own bars.
    namespace: String,
    /// The layer it asked to be drawn in.
    layer: Layer,
    /// The output it is on, resolved from the client's hint or the first output.
    output: Option<OutputId>,
    /// The rectangle it occupies in global logical coordinates.
    rect: Rect,
    /// The exclusive zone it asked for, as the protocol states it.
    exclusive: ExclusiveZone,
    /// The edge that zone lies along, when the anchors make it meaningful.
    edge: Option<Edge>,
    /// How much keyboard it wants.
    keyboard: KeyboardInteractivity,
    /// Whether it has committed a buffer, which is what "shown" means here.
    mapped: bool,
}

impl LayerEntry {
    /// The reserved insets this surface contributes to its output.
    ///
    /// Only a strip along one edge reserves anything: the protocol makes an
    /// exclusive zone meaningless for a surface anchored to both edges of an axis (it
    /// is a whole band, not a strip) or to neither (it is not against an edge at
    /// all), so both reserve nothing.
    fn reserve(&self) -> Insets {
        let ExclusiveZone::Exclusive(amount) = self.exclusive else {
            return Insets::ZERO;
        };
        let amount = i32::try_from(amount).unwrap_or(i32::MAX);
        match self.edge {
            Some(Edge::Top) => Insets::new(amount, 0, 0, 0),
            Some(Edge::Bottom) => Insets::new(0, 0, amount, 0),
            Some(Edge::Left) => Insets::new(0, 0, 0, amount),
            Some(Edge::Right) => Insets::new(0, amount, 0, 0),
            None => Insets::ZERO,
        }
    }
}

/// The layer surfaces, in creation order, and what they have reserved.
#[derive(Default)]
pub(crate) struct Layers {
    entries: Vec<LayerEntry>,
    /// The insets each output last reported to the window manager.
    reserved: std::collections::BTreeMap<OutputId, Insets>,
}

impl Layers {
    /// The surface that holds the keyboard, if any.
    ///
    /// Only the top and overlay layers take exclusive focus: a background or bottom
    /// surface asking for it may still be focused by the user, which is the
    /// protocol's own distinction between the two.
    fn keyboard_grab(&self) -> Option<&LayerEntry> {
        self.entries
            .iter()
            .rev()
            .find(|entry| entry.keyboard == KeyboardInteractivity::Exclusive)
            .filter(|entry| matches!(entry.layer, Layer::Top | Layer::Overlay))
    }

    /// The rectangle of one surface, for its popups.
    fn rect_of(&self, surface: &WlSurface) -> Option<Rect> {
        self.entries
            .iter()
            .find(|entry| entry.surface.wl_surface() == surface)
            .map(|entry| entry.rect)
    }

    /// The surfaces as they are published to a reader.
    pub(crate) fn observed(&self) -> Vec<ObservedLayer> {
        self.entries
            .iter()
            .map(|entry| ObservedLayer {
                namespace: entry.namespace.clone(),
                layer: layer_name(entry.layer).to_owned(),
                rect: entry.rect,
                exclusive: exclusive_value(entry.exclusive),
                mapped: entry.mapped,
            })
            .collect()
    }
}

impl State {
    /// A client created a layer surface.
    pub(crate) fn layer_created(
        &mut self,
        surface: LayerSurface,
        output: Option<WlOutput>,
        layer: Layer,
        namespace: String,
    ) {
        let resolved = output
            .as_ref()
            .and_then(SmithayOutput::from_resource)
            .and_then(|output| self.output_id_of(&output))
            .or_else(|| self.output_ids().first().copied());
        debug!(
            namespace = %namespace,
            layer = layer_name(layer),
            surface = ?surface.wl_surface().id(),
            "[layer] created"
        );
        self.layers.entries.push(LayerEntry {
            surface,
            namespace,
            layer,
            output: resolved,
            rect: Rect::new(0, 0, 0, 0),
            exclusive: ExclusiveZone::Neutral,
            edge: None,
            keyboard: KeyboardInteractivity::None,
            mapped: false,
        });
    }

    /// A layer surface committed: recompute its rectangle, answer with a configure and
    /// republish what it reserves.
    pub(crate) fn layer_surface_committed(&mut self, surface: &WlSurface) {
        let Some(index) = self
            .layers
            .entries
            .iter()
            .position(|entry| entry.surface.wl_surface() == surface)
        else {
            return;
        };
        let cached = self.layers.entries[index].surface.with_cached_state(|state| *state);
        let Some(monitor) = self.layers.entries[index]
            .output
            .and_then(|output| self.output_rect(output))
        else {
            // No output to place it on: the surface keeps its place in the list and is
            // configured by the next output that connects, which is what a session
            // starting before its first output does.
            debug!("[layer] no output to place a layer surface on yet");
            return;
        };
        let rect = place(monitor, &cached);
        let configured = configure_size(monitor, &cached);
        let surface = self.layers.entries[index].surface.clone();
        surface.with_pending_state(|state| {
            state.size = Some(configured);
        });
        surface.send_pending_configure();

        let Some(entry) = self.layers.entries.get_mut(index) else {
            return;
        };
        entry.layer = cached.layer;
        entry.exclusive = cached.exclusive_zone;
        entry.edge = reservation_edge(&cached);
        entry.keyboard = cached.keyboard_interactivity;
        entry.rect = rect;
        entry.mapped = crate::state::has_buffer(surface.wl_surface());
        debug!(
            namespace = %entry.namespace,
            ?rect,
            mapped = entry.mapped,
            "[layer] configured"
        );
        self.refresh_reserved();
        self.refresh_layer_focus();
        self.publish();
    }

    /// A layer surface is gone, whether the client destroyed it or disconnected.
    pub(crate) fn layer_destroyed(&mut self, surface: &LayerSurface) {
        let wl_surface = surface.wl_surface();
        let Some(index) = self
            .layers
            .entries
            .iter()
            .position(|entry| entry.surface.wl_surface() == wl_surface)
        else {
            return;
        };
        let entry = self.layers.entries.remove(index);
        debug!(namespace = %entry.namespace, "[layer] destroyed");
        // The strip goes back with the surface: a bar that exits must not keep the
        // work area short for the rest of the session.
        if entry.surface.wl_surface() == wl_surface {
            self.refresh_reserved();
        }
        self.refresh_layer_focus();
        self.publish();
    }

    /// A popup whose parent is a layer surface.
    ///
    /// The popup is tracked like any other, and `xdg_shell`'s placement code finds the
    /// parent's rectangle in [`State::layer_rect`] because a layer surface is not a
    /// managed window. That is the whole difference: a bar's menu is constrained to
    /// the bar rather than to a window's frame.
    pub(crate) fn layer_popup_created(
        &mut self,
        _parent: &LayerSurface,
        popup: &smithay::wayland::shell::xdg::PopupSurface,
    ) {
        if let Err(error) = self.popups.track_popup(smithay::desktop::PopupKind::Xdg(popup.clone())) {
            debug!(?error, "[layer] a popup could not be tracked");
        }
    }

    /// The rectangle a layer surface with this one as its parent occupies.
    pub(crate) fn layer_rect(&self, surface: &WlSurface) -> Option<Rect> {
        self.layers.rect_of(surface)
    }

    /// The keyboard grab a layer surface holds, if any.
    pub(crate) fn layer_keyboard_focus(&self) -> Option<WlSurface> {
        self.layers
            .keyboard_grab()
            .map(|entry| entry.surface.wl_surface().clone())
    }

    /// Recomputed the reserved insets of every output and tells the window manager.
    ///
    /// It runs after every layer change and every destruction, so the answer is always
    /// the sum over the surfaces that exist now: nothing has to be un-reserved
    /// separately, which is what makes a dead bar's strip come back.
    fn refresh_reserved(&mut self) {
        let mut totals: std::collections::BTreeMap<OutputId, Insets> = std::collections::BTreeMap::new();
        for entry in &self.layers.entries {
            let Some(output) = entry.output else {
                continue;
            };
            let insets = entry.reserve();
            let total = totals.entry(output).or_insert(Insets::ZERO);
            total.top = total.top.saturating_add(insets.top);
            total.right = total.right.saturating_add(insets.right);
            total.bottom = total.bottom.saturating_add(insets.bottom);
            total.left = total.left.saturating_add(insets.left);
        }
        let outputs = self.output_ids();
        for output in &outputs {
            let reserved = totals.remove(output).unwrap_or(Insets::ZERO);
            let previous = self.layers.reserved.insert(*output, reserved);
            if previous == Some(reserved) {
                continue;
            }
            let Some(policy) = self.policy_output(*output) else {
                continue;
            };
            let effects = self.wm.apply(gnoblin_wm::Event::OutputReserved {
                output: policy,
                reserved,
            });
            debug!(?reserved, "[layer] reserved");
            self.apply(effects);
        }
        // An output that disconnected keeps no reservation.
        self.layers.reserved.retain(|output, _| outputs.contains(output));
    }

    /// Gives the keyboard to an exclusive layer surface, or back to the window.
    fn refresh_layer_focus(&mut self) {
        if self.lock.state() != gnoblin_lock::State::Unlocked {
            return;
        }
        if let Some(surface) = self.layer_keyboard_focus() {
            self.set_keyboard_focus(Some(surface));
            return;
        }
        let target = self.focused_window_surface();
        self.set_keyboard_focus(target);
    }
}

/// The rectangle a layer surface asks for on a monitor.
///
/// The protocol's arithmetic, stated once so a configure and a published rectangle
/// cannot disagree: an axis anchored to both edges is the space between them, an axis
/// anchored to one edge is the size the client asked for against that edge, and an
/// unanchored axis is centred.
fn place(monitor: Rect, state: &LayerSurfaceCachedState) -> Rect {
    let anchor = state.anchor;
    let margin = state.margin;
    let requested = state.size;
    let (x, width) = axis(
        monitor.x,
        i32::try_from(monitor.width).unwrap_or(i32::MAX),
        margin.left,
        margin.right,
        anchor.contains(Anchor::LEFT),
        anchor.contains(Anchor::RIGHT),
        requested.w,
    );
    let (y, height) = axis(
        monitor.y,
        i32::try_from(monitor.height).unwrap_or(i32::MAX),
        margin.top,
        margin.bottom,
        anchor.contains(Anchor::TOP),
        anchor.contains(Anchor::BOTTOM),
        requested.h,
    );
    Rect::new(x, y, span(width), span(height))
}

/// A computed length as a rectangle dimension, which is an unsigned width.
fn span(length: i32) -> u32 {
    u32::try_from(length).unwrap_or(0)
}

/// One axis of [`place`].
///
/// `start` and `end` are the edges of the monitor rectangle on this axis, and the two
/// anchors say which of them the surface is attached to.
fn axis(
    start: i32,
    length: i32,
    leading: i32,
    trailing: i32,
    anchored_start: bool,
    anchored_end: bool,
    requested: i32,
) -> (i32, i32) {
    match (anchored_start, anchored_end) {
        (true, true) => {
            let size = (length - leading - trailing).max(0);
            (start + leading, size)
        }
        (true, false) => (start + leading, requested.max(0)),
        (false, true) => (start + length - trailing - requested, requested.max(0)),
        // Neither anchor: the client's own size, centred, which is what a floating
        // widget or an on-screen display is.
        (false, false) => {
            let size = requested.max(0);
            (start + (length - size) / 2, size)
        }
    }
}

/// The size to configure a layer surface with.
///
/// An axis anchored to both edges is decided by the compositor — that is what a
/// client asks for when it sends zero — and every other axis is the client's own
/// answer, so it is echoed back unchanged.
fn configure_size(monitor: Rect, state: &LayerSurfaceCachedState) -> Size<i32, Logical> {
    let rect = place(monitor, state);
    Size::from((
        i32::try_from(rect.width).unwrap_or(i32::MAX),
        i32::try_from(rect.height).unwrap_or(i32::MAX),
    ))
}

/// Which edge an exclusive zone lies along, if the anchors make it a strip.
///
/// A zone is meaningful only when the surface is anchored to exactly one edge of the
/// axis it runs along; anchoring both makes it a band across the output and anchoring
/// neither leaves it away from every edge, and the protocol treats both as neutral.
fn reservation_edge(state: &LayerSurfaceCachedState) -> Option<Edge> {
    if !matches!(state.exclusive_zone, ExclusiveZone::Exclusive(_)) {
        return None;
    }
    let anchor = state.anchor;
    match (
        anchor.contains(Anchor::TOP),
        anchor.contains(Anchor::BOTTOM),
        anchor.contains(Anchor::LEFT),
        anchor.contains(Anchor::RIGHT),
    ) {
        (true, false, _, _) => Some(Edge::Top),
        (false, true, _, _) => Some(Edge::Bottom),
        (false, false, true, false) => Some(Edge::Left),
        (false, false, false, true) => Some(Edge::Right),
        _ => None,
    }
}

/// The protocol's name for a layer, for a log line and an observation.
fn layer_name(layer: Layer) -> &'static str {
    match layer {
        Layer::Background => "background",
        Layer::Bottom => "bottom",
        Layer::Top => "top",
        Layer::Overlay => "overlay",
    }
}

/// The exclusive zone as the protocol states it: a positive amount, `0` neutral and
/// `-1` for a surface that does not care.
fn exclusive_value(zone: ExclusiveZone) -> i32 {
    i32::from(zone)
}

#[cfg(test)]
mod tests {
    use smithay::utils::Size;
    use smithay::wayland::shell::wlr_layer::{
        Anchor, ExclusiveZone, KeyboardInteractivity, Layer, LayerSurfaceCachedState, Margins,
    };

    use gnoblin_types::Rect;

    use super::{Edge, configure_size, place, reservation_edge};

    /// A 1024x768 monitor at the origin, which is the headless backend's only output.
    const MONITOR: Rect = Rect::new(0, 0, 1024, 768);

    fn state(size: (i32, i32), anchor: Anchor, exclusive: i32, margin: Margins) -> LayerSurfaceCachedState {
        LayerSurfaceCachedState {
            size: Size::from(size),
            anchor,
            exclusive_zone: ExclusiveZone::from(exclusive),
            margin,
            layer: Layer::Top,
            keyboard_interactivity: KeyboardInteractivity::None,
            ..Default::default()
        }
    }

    #[test]
    fn a_top_bar_anchored_to_both_sides_spans_the_monitor() {
        let bar = state(
            (0, 30),
            Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
            30,
            Margins::default(),
        );
        assert_eq!(place(MONITOR, &bar), Rect::new(0, 0, 1024, 30));
        assert_eq!(configure_size(MONITOR, &bar), Size::from((1024, 30)));
    }

    #[test]
    fn a_left_dock_anchored_to_both_vertical_sides_spans_the_height() {
        let dock = state(
            (40, 0),
            Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT,
            40,
            Margins::default(),
        );
        assert_eq!(place(MONITOR, &dock), Rect::new(0, 0, 40, 768));
    }

    #[test]
    fn margins_move_and_shrink_a_bar() {
        let margins = Margins {
            top: 4,
            right: 8,
            bottom: 0,
            left: 12,
        };
        let bar = state((0, 24), Anchor::TOP | Anchor::LEFT | Anchor::RIGHT, 28, margins);
        assert_eq!(place(MONITOR, &bar), Rect::new(12, 4, 1004, 24));
        assert_eq!(configure_size(MONITOR, &bar), Size::from((1004, 24)));
    }

    #[test]
    fn an_unanchored_surface_is_centred_at_its_own_size() {
        let osd = state((200, 100), Anchor::empty(), -1, Margins::default());
        assert_eq!(place(MONITOR, &osd), Rect::new(412, 334, 200, 100));
    }

    #[test]
    fn one_anchored_edge_keeps_the_requested_size() {
        let bar = state((300, 24), Anchor::TOP | Anchor::LEFT, 24, Margins::default());
        assert_eq!(place(MONITOR, &bar), Rect::new(0, 0, 300, 24));
        let corner = state((300, 24), Anchor::BOTTOM | Anchor::RIGHT, 24, Margins::default());
        assert_eq!(place(MONITOR, &corner), Rect::new(724, 744, 300, 24));
    }

    #[test]
    fn an_exclusive_zone_needs_one_anchored_edge() {
        let top = state(
            (0, 30),
            Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
            30,
            Margins::default(),
        );
        assert_eq!(reservation_edge(&top), Some(Edge::Top));
        // Anchored to both vertical edges: a band across the output, not a strip.
        let band = state((0, 0), Anchor::TOP | Anchor::BOTTOM, 30, Margins::default());
        assert_eq!(reservation_edge(&band), None);
        // Anchored to nothing: not against an edge at all.
        let floating = state((200, 100), Anchor::empty(), 30, Margins::default());
        assert_eq!(reservation_edge(&floating), None);
        // A neutral zone reserves nothing even when the anchor would allow it.
        let neutral = state((0, 30), Anchor::TOP, 0, Margins::default());
        assert_eq!(reservation_edge(&neutral), None);
        // "Do not care" is not a reservation either.
        let dont_care = state((0, 30), Anchor::TOP, -1, Margins::default());
        assert_eq!(reservation_edge(&dont_care), None);
    }

    #[test]
    fn a_reserved_strip_is_the_zone_not_the_drawn_size() {
        let bar = state(
            (0, 30),
            Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
            38,
            Margins::default(),
        );
        let entry_edge = reservation_edge(&bar);
        assert_eq!(entry_edge, Some(Edge::Top));
        // The zone includes the margins, which is why it is used as stated rather than
        // recomputed from the rectangle.
        assert_eq!(configure_size(MONITOR, &bar).h, 30);
    }

    #[test]
    fn a_portion_of_the_monitor_places_a_layer_surface_in_its_own_origin() {
        let second = Rect::new(1024, 0, 800, 600);
        let bar = state(
            (0, 20),
            Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
            20,
            Margins::default(),
        );
        assert_eq!(place(second, &bar), Rect::new(1024, 0, 800, 20));
    }
}
