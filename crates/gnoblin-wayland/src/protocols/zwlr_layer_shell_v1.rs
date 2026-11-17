//! XML protocol `wlr_layer_shell_unstable_v1` (`<protocol name="wlr_layer_shell_unstable_v1">`); interfaces served: `zwlr_layer_shell_v1`.
//!
//! `zwlr_layer_shell_v1`: bars, docks, overlays and what they reserve.
//!
//! A layer surface is placed against an output's edges rather than by the window
//! policy: the client names its layer, its anchors, its margins and its exclusive
//! zone, and the compositor answers with a configure size. This module is the whole
//! of that: it computes the rectangle, sends the configure, and turns the exclusive
//! zones into the output's reserved insets, so the work area a maximised window
//! gets is the space the bars left over.
//!
//! ## What a layer surface is not
//!
//! A layer surface is not a window. The window record places, stacks and focuses
//! windows; a bar is placed by its anchors and focused by its keyboard
//! interactivity, so routing it through the record would invent a window with a
//! work-area constraint of its own. The two meet at exactly one point, the reserved
//! insets, which is the same seam a configuration's monitor insets would use.
//!
//! Nor is it a record in a registry. A layer registry once named every surface and nothing read the names back:
//! stacking, hit testing, zones and keyboard never consulted it, so it was deleted
//! (`design/cutover-survey.md` 16.1). What the session knows of a layer surface is
//! `SessionSnapshot::layers`: its namespace, layer, rectangle, zone and whether it is mapped. The protocol owns
//! the rest.
//!
//! ## Drawing
//!
//! The composer draws a layer surface's buffer (`gnoblin-backend`, `render/compose/layers.rs`) at the rectangle this module
//! computes, or the shell's, and `layer_draw.rs` is what the pointer, the keyboard and the frame callbacks do with it. A commit
//! hands the composer the buffer and places every surface of the output again, because a bar's strip moves the surfaces that
//! come after it (`State::layer_arrange`). The tests that read pixels, the pointer and the callbacks are in
//! `tests/layer_draw.rs`; the ones above assert on the configure, the published rectangle and the work area.
//!
//! ## The shell's say
//!
//! Each layer surface has a `surface` node in the tree, made when the client creates it and removed with it. A shell that
//! holds the tree reads what the client asked for there (`namespace`, `layer`, `anchor`, `margin`, `requested_size`,
//! `exclusive_zone`) and may take the placement over with `placement_override` (`design/node-core.md`, "Surface overrides").
//! The protocol's arithmetic (`place`) still runs and stays what the node shows while the shell does not place the
//! surface. When it does, the shell's rectangle stands, the surface is configured at that size and the surface's popups
//! and the published rectangle follow it, and the shell's `reserve` replaces the strip the exclusive zone takes out of
//! the work area. A locked session, a shell that stops placing the surface and a shell that detaches all hand the surface
//! back to the protocol at the next frame (`State::follow_layer_overrides`).
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
//! 7. **A surface that moves to another layer is still published in the old one.** The next commit carries the
//!    layer, and the published entry follows it without a second entry or a moved strip —
//!    `a_surface_that_changes_layer_is_published_in_the_one_it_moved_to`.
//! 8. **Two surfaces in one namespace collide.** A namespace is the client's name for its surface, not a key, so
//!    two surfaces may share it: both are kept, each reserves its own strip, and the first to exit gives back
//!    only its own — `two_bars_in_one_namespace_each_keep_their_own_strip`.
//! 9. **A surface below the shell layer holds the keyboard.** Only top and overlay surfaces are granted
//!    exclusive focus, and a window that may take focus gets the keyboard from a bottom one —
//!    `a_top_layer_surface_holds_the_keyboard_from_a_window_that_may_take_focus` and
//!    `a_bottom_layer_surface_does_not_hold_the_keyboard_from_a_window`.
//! 10. **A bar's menu is placed against the monitor instead of the bar.** The popup is configured where its
//!     positioner put it, measured from the bar, and a reposition keeps it inside the bar's rectangle —
//!     `a_popup_of_a_bar_is_placed_against_the_bar` and
//!     `a_popup_of_a_bar_that_is_repositioned_stays_inside_the_bar`.
//! 11. **A shell's placement outlives its permission.** A surface the shell stopped placing, the session's lock and the
//!     shell's detach put the protocol's rectangle, configure and strip back, and a write to the node's rectangle that is
//!     not honoured is written back —
//!     `a_layer_surface_goes_back_to_the_protocol_when_the_shell_lets_go_or_stops_placing_it` and
//!     `a_write_to_a_layer_surfaces_rectangle_without_the_override_is_not_honoured_and_is_written_back`.
//! 12. **The shell's rectangle is not what the client is told.** The client is configured at the size the shell chose, an
//!     axis the shell left at 0 keeps the protocol's —
//!     `a_shell_that_places_a_layer_surface_sets_its_rectangle_its_configure_and_its_strip`.

use gnoblin_backend::render::compose::{LayerBand, LayerNodeInfo, LayerOverride, TreeOwner};
use gnoblin_render_frame::frame::SurfaceSource;
use gnoblin_types::{Edge, Insets, NodeId, OutputId, Rect};
use smithay::output::Output as SmithayOutput;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Size};
use smithay::wayland::shell::wlr_layer::{
    Anchor, ExclusiveZone, KeyboardInteractivity, Layer, LayerSurface, LayerSurfaceCachedState, LayerSurfaceConfigure,
    WlrLayerShellHandler, WlrLayerShellState,
};

use crate::State;
use crate::observation::ObservedLayer;

use crate::log;

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
        log::trace!(area = "layer", serial = ?configure.serial,
            size = ?configure.state.size,
            surface = ?surface.id(),
            "configure acknowledged"
        );
    }

    fn layer_destroyed(&mut self, surface: LayerSurface) {
        self.layer_destroyed(&surface);
    }
}

/// One layer surface the session is showing.
pub(crate) struct ZwlrLayerShellV1LayerEntry {
    /// The protocol object, which is also what a configure is sent through.
    pub(crate) surface: LayerSurface,
    /// The namespace the client named, which is how a shell recognises its own bars.
    namespace: String,
    /// The layer it asked to be drawn in.
    pub(crate) layer: Layer,
    /// The output it is on, resolved from the client's hint or the first output.
    pub(crate) output: Option<OutputId>,
    /// The rectangle it occupies in global logical coordinates.
    pub(crate) rect: Rect,
    /// The exclusive zone it asked for, as the protocol states it.
    exclusive: ExclusiveZone,
    /// The edge that zone lies along, when the anchors make it meaningful.
    edge: Option<Edge>,
    /// How much keyboard it wants.
    pub(crate) keyboard: KeyboardInteractivity,
    /// Whether it has committed a buffer, which is what "shown" means here.
    pub(crate) mapped: bool,
    /// Whether its output has been told the surface is on it (`wl_surface.enter`), which it is from the surface's first
    /// buffer.
    pub(crate) entered: bool,
    /// Whether the client has committed since it made the surface. Until it has it is not placed and not told anything: the
    /// protocol answers the first commit with the first configure.
    committed: bool,
    /// What the shell sees of it: the node the tree names it by, made when the client created the surface.
    pub(crate) node: Option<NodeId>,
    /// What the client asked for and where it is, as the node publishes it.
    info: LayerNodeInfo,
    /// What the shell wrote on the node and the compositor acted on at the last placement, when the shell placed the
    /// surface (`placement_override`) and the session was unlocked. Compared with what the node says now, so a change is
    /// noticed without a commit from the client.
    placed: Option<LayerOverride>,
    /// The strip the shell chose for the surface to reserve instead of its own exclusive zone's.
    reserve_override: Option<Insets>,
}

impl ZwlrLayerShellV1LayerEntry {
    /// The reserved insets this surface contributes to its output.
    ///
    /// Only a strip along one edge reserves anything: the protocol makes an
    /// exclusive zone meaningless for a surface anchored to both edges of an axis (it
    /// is a whole band, not a strip) or to neither (it is not against an edge at
    /// all), so both reserve nothing.
    fn reserve(&self) -> Insets {
        if let Some(insets) = self.reserve_override {
            return insets;
        }
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

    /// Whether the surface takes a strip out of its output: it asked for an exclusive zone, or the shell gave it a strip.
    fn reserves(&self) -> bool {
        matches!(self.exclusive, ExclusiveZone::Exclusive(_)) || !self.reserve().is_zero()
    }

    /// Whether the surface may be given the keyboard when the user clicks it: it asked for focus on demand, or it asked for
    /// exclusive focus on a layer the protocol grants none on (`background` and `bottom` use normal focus semantics).
    pub(crate) fn takes_focus_on_click(&self) -> bool {
        match self.keyboard {
            KeyboardInteractivity::OnDemand => true,
            KeyboardInteractivity::Exclusive => matches!(self.layer, Layer::Background | Layer::Bottom),
            KeyboardInteractivity::None => false,
        }
    }
}

/// The layer surfaces, in creation order, and what they have reserved.
#[derive(Default)]
pub(crate) struct ZwlrLayerShellV1Layers {
    pub(crate) entries: Vec<ZwlrLayerShellV1LayerEntry>,
    /// The insets each output last reported to the window manager.
    reserved: std::collections::BTreeMap<OutputId, Insets>,
    /// The surface that has the keyboard because the user clicked it and it asked for focus on demand. It keeps the
    /// keyboard until the user clicks something else, a window takes the focus, or the surface goes.
    pub(crate) on_demand: Option<WlSurface>,
    /// The surface the keyboard was last given to because of the layer rules (an exclusive surface, or one the user focused
    /// on demand). The keyboard is moved when what the rules want changes and not otherwise, so a bar that redraws does not
    /// take the keyboard back from the popup grab of its own menu.
    pub(crate) keyboard_holder: Option<WlSurface>,
}

impl ZwlrLayerShellV1Layers {
    /// The surface that holds the keyboard, if any.
    ///
    /// Only the top and overlay layers take exclusive focus: a background or bottom
    /// surface asking for it may still be focused by the user, which is the
    /// protocol's own distinction between the two. The seat gives it to the topmost layer that has such a surface, and to the
    /// newest of that layer's, and only to one the user can see: a surface that has not drawn yet holds nothing.
    fn keyboard_grab(&self) -> Option<&ZwlrLayerShellV1LayerEntry> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                entry.mapped
                    && entry.keyboard == KeyboardInteractivity::Exclusive
                    && matches!(entry.layer, Layer::Top | Layer::Overlay)
            })
            .max_by_key(|(index, entry)| (band_of(entry.layer), *index))
            .map(|(_, entry)| entry)
    }

    /// The node of the layer surface whose own surface is `root`.
    pub(crate) fn node_of(&self, root: &WlSurface) -> Option<NodeId> {
        self.entry_of(root)?.node
    }

    /// The layer surface whose own surface is `root`.
    pub(crate) fn entry_of(&self, root: &WlSurface) -> Option<&ZwlrLayerShellV1LayerEntry> {
        self.entries.iter().find(|entry| entry.surface.wl_surface() == root)
    }

    /// The layer surface a node was made for.
    pub(crate) fn entry_of_node(&self, node: NodeId) -> Option<&ZwlrLayerShellV1LayerEntry> {
        self.entries.iter().find(|entry| entry.node == Some(node))
    }

    /// The surface the user focused by clicking it, while it is still mapped and still asks for focus on demand.
    pub(crate) fn focused_on_demand(&self) -> Option<WlSurface> {
        let surface = self.on_demand.as_ref()?;
        self.entry_of(surface)
            .filter(|entry| entry.mapped && entry.takes_focus_on_click())
            .map(|entry| entry.surface.wl_surface().clone())
    }

    /// The rectangle of one surface, for its popups.
    fn rect_of(&self, surface: &WlSurface) -> Option<Rect> {
        self.entry_of(surface).map(|entry| entry.rect)
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
                reserved: entry.reserve(),
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
        log::debug!(area = "layer", namespace = %namespace,
            layer = layer_name(layer),
            surface = ?surface.wl_surface().id(),
            "created"
        );
        let info = LayerNodeInfo {
            namespace: namespace.clone(),
            layer: band_of(layer),
            anchor: Vec::new(),
            margin: [0.0; 4],
            requested: [0.0; 2],
            zone: 0.0,
            mapped: false,
        };
        let node = self.composer.add_layer_node(&info);
        self.layers.entries.push(ZwlrLayerShellV1LayerEntry {
            surface,
            namespace,
            layer,
            output: resolved,
            rect: Rect::new(0, 0, 0, 0),
            exclusive: ExclusiveZone::Neutral,
            edge: None,
            keyboard: KeyboardInteractivity::None,
            mapped: false,
            entered: false,
            committed: false,
            node,
            info,
            placed: None,
            reserve_override: None,
        });
    }

    /// A layer surface committed: it is placed again together with the surfaces that share its output, answered with a
    /// configure, handed to the composer with its pixels, and what it reserves is republished.
    pub(crate) fn layer_surface_committed(&mut self, surface: &WlSurface) {
        let Some(index) = self
            .layers
            .entries
            .iter()
            .position(|entry| entry.surface.wl_surface() == surface)
        else {
            return;
        };
        self.layers.entries[index].committed = true;
        // A surface that was created before the session had an output is placed on the first one there is now.
        if self.layers.entries[index].output.is_none() {
            self.layers.entries[index].output = self.output_ids().first().copied();
        }
        match self.layers.entries[index].output {
            Some(output) => self.layer_arrange(output, Some(index)),
            None => log::debug!(area = "layer", "no output to place a layer surface on yet"),
        }
        self.layer_settled();
    }

    /// What the shell wrote on a layer surface's node, when it places the surface and the session is unlocked. A locked
    /// session ignores it, so the protocol's own placement stands under the lock and the shell's returns with the unlock.
    fn layer_override(&self, index: usize) -> Option<LayerOverride> {
        if !self.surface_overrides_apply() {
            return None;
        }
        self.composer.layer_override(self.layers.entries.get(index)?.node?)
    }

    /// What the session derives from a layer surface's committed state and from what the shell wrote on its node: its
    /// layer, zone, edge and keyboard interactivity, and the shell's rectangle and strip when the shell places it. The
    /// order the surfaces of an output are arranged in is read from it, so it is current before they are.
    fn layer_take(&mut self, index: usize) {
        let cached = self.layers.entries[index].surface.with_cached_state(|state| *state);
        let shell = self.layer_override(index);
        let entry = &mut self.layers.entries[index];
        entry.layer = cached.layer;
        entry.exclusive = cached.exclusive_zone;
        entry.edge = reservation_edge(&cached);
        entry.keyboard = cached.keyboard_interactivity;
        entry.placed = shell;
        entry.reserve_override = shell.and_then(|shell| shell.reserve);
    }

    /// Places every layer surface of an output, in the order the protocol arranges them. `committed` is the surface whose
    /// commit asked for it, if one did: its pixels are handed to the composer again, and the others' stay as they are, so a
    /// bar that redraws every second does not make a wallpaper repaint.
    ///
    /// The surfaces that reserve a strip come first, from the overlay layer down to the background and in the order they
    /// were created within a layer, and each takes its strip out of what the next finds: a second bar on the same edge sits
    /// against the first rather than over it. Every other surface is then placed in what is left, which is what a zone of
    /// `0` asks for ("be moved to avoid occluding the surfaces that have an exclusive zone"). A surface with a zone of `-1`
    /// asked to be left alone and is placed against the whole output.
    fn layer_arrange(&mut self, output: OutputId, committed: Option<usize>) {
        let Some(monitor) = self.output_rect(output) else {
            return;
        };
        // A surface that has not committed is not placed yet, and one whose client has gone is not placed at all: smithay
        // hands it over for removal after its protocol object.
        let mut order: Vec<usize> = self
            .layers
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.output == Some(output) && entry.committed && entry.surface.alive())
            .map(|(index, _)| index)
            .collect();
        for index in &order {
            self.layer_take(*index);
        }
        order.sort_by_key(|index| {
            let entry = &self.layers.entries[*index];
            (!entry.reserves(), std::cmp::Reverse(band_of(entry.layer)), *index)
        });
        let mut usable = monitor;
        for index in order {
            usable = self.layer_place(index, monitor, usable, committed == Some(index));
        }
    }

    /// Places a layer surface and tells it, its popups, the composer and the shell. Returns what the surfaces after it find
    /// of the output: `usable` less the strip this one reserves.
    ///
    /// The protocol's rectangle comes from the anchors, margins and size the client asked for, against `usable`, or against
    /// `full` when the surface asked for a zone of `-1`. When the shell placed the surface (`placement_override` on its node)
    /// the shell's rectangle stands instead, an axis it left at 0 keeping the protocol's, and the client is configured at
    /// that size: it still receives its configure, and still sees the anchors and margins it asked for. The strip it reserves
    /// is the shell's `reserve` when the shell wrote one.
    fn layer_place(&mut self, index: usize, full: Rect, usable: Rect, refresh: bool) -> Rect {
        let cached = self.layers.entries[index].surface.with_cached_state(|state| *state);
        let shell = self.layers.entries[index].placed;
        let bounds = if matches!(cached.exclusive_zone, ExclusiveZone::DontCare) {
            full
        } else {
            usable
        };
        let protocol = place(bounds, &cached);
        let rect = match shell {
            Some(shell) => Rect::new(
                shell.rect.x,
                shell.rect.y,
                if shell.rect.width > 0 {
                    shell.rect.width
                } else {
                    protocol.width
                },
                if shell.rect.height > 0 {
                    shell.rect.height
                } else {
                    protocol.height
                },
            ),
            None => protocol,
        };
        let configured = match shell {
            Some(_) => Size::from((
                i32::try_from(rect.width).unwrap_or(i32::MAX),
                i32::try_from(rect.height).unwrap_or(i32::MAX),
            )),
            None => configure_size(bounds, &cached),
        };
        let surface = self.layers.entries[index].surface.clone();
        surface.with_pending_state(|state| {
            state.size = Some(configured);
        });
        surface.send_pending_configure();

        let mapped = crate::state::has_buffer(surface.wl_surface());
        let Some(entry) = self.layers.entries.get_mut(index) else {
            return usable;
        };
        let moved = entry.rect != rect;
        entry.rect = rect;
        entry.mapped = mapped;
        entry.info = LayerNodeInfo {
            namespace: entry.namespace.clone(),
            layer: band_of(cached.layer),
            anchor: anchor_names(cached.anchor),
            margin: [
                f64::from(cached.margin.top),
                f64::from(cached.margin.right),
                f64::from(cached.margin.bottom),
                f64::from(cached.margin.left),
            ],
            requested: [f64::from(cached.size.w), f64::from(cached.size.h)],
            zone: f64::from(exclusive_value(cached.exclusive_zone)),
            mapped,
        };
        log::debug!(area = "layer", namespace = %entry.namespace,
            ?rect,
            mapped,
            placed_by_shell = shell.is_some(),
            "configured"
        );
        let reserved = entry.reserve();
        let node = entry.node;
        if let Some(node) = node {
            self.composer
                .update_layer_node(node, &self.layers.entries[index].info, rect, shell.is_some());
            // What the client last committed is what the composer draws, at the rectangle it was just given.
            if refresh {
                let content = crate::state::committed_content(surface.wl_surface())
                    .map(|(buffer, content)| (content, SurfaceSource::of_buffer(buffer)));
                self.composer.set_layer_content(node, content);
            }
        }
        self.layer_output_follows(index, mapped);
        if moved && let Some(node) = node {
            self.reposition_reactive_popups(TreeOwner::Layer(node));
        }
        reserved.applied_to(usable)
    }

    /// Tells the output a layer surface is on that it is, once the surface has pixels, and that it is not when it has none
    /// again: `wl_surface.enter` and `leave` are how a client learns which output (and so which scale) it is drawn at.
    fn layer_output_follows(&mut self, index: usize, mapped: bool) {
        let entry = &self.layers.entries[index];
        if entry.entered == mapped {
            return;
        }
        let wl_surface = entry.surface.wl_surface().clone();
        let Some(shown) = entry
            .output
            .and_then(|output| self.outputs.iter().find(|candidate| candidate.backend == output))
        else {
            return;
        };
        if mapped {
            shown.output.enter(&wl_surface);
        } else {
            shown.output.leave(&wl_surface);
        }
        self.layers.entries[index].entered = mapped;
    }

    /// What follows any change to the layer surfaces: the strip they reserve, the keyboard, the surface the pointer is over
    /// and the observers.
    fn layer_settled(&mut self) {
        self.refresh_reserved();
        self.refresh_layer_focus();
        self.refresh_pointer_focus();
        self.publish();
    }

    /// Places the layer surfaces of every output again, as a change to the outputs asks for: a mode that changed moves the
    /// edges the surfaces are anchored to, and a surface created before the first output is placed on it when it comes.
    pub(crate) fn layers_follow_outputs(&mut self) {
        let outputs = self.output_ids();
        for entry in &mut self.layers.entries {
            if entry.output.is_none() {
                entry.output = outputs.first().copied();
            }
        }
        for output in &outputs {
            self.layer_arrange(*output, None);
        }
        self.layer_settled();
    }

    /// Follows what the shell wrote on the layer surfaces' nodes: a surface it started or stopped placing, moved, resized
    /// or gave another strip to reserve is placed again, and the session's lock or the shell's detach hands every surface
    /// back to the protocol. A node the shell wrote that the compositor does not honour is written back.
    pub(crate) fn follow_layer_overrides(&mut self) {
        let mut stale = std::collections::BTreeSet::new();
        for index in 0..self.layers.entries.len() {
            if !self.layers.entries[index].committed {
                continue;
            }
            if self.layer_override(index) != self.layers.entries[index].placed {
                stale.extend(self.layers.entries[index].output);
                continue;
            }
            let entry = &self.layers.entries[index];
            if let Some(node) = entry.node {
                self.composer
                    .update_layer_node(node, &entry.info, entry.rect, entry.placed.is_some());
            }
        }
        for output in &stale {
            self.layer_arrange(*output, None);
        }
        if !stale.is_empty() {
            self.layer_settled();
        }
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
        log::debug!(area = "layer", namespace = %entry.namespace, "destroyed");
        if self.layers.on_demand.as_ref() == Some(wl_surface) {
            self.layers.on_demand = None;
        }
        if let Some(node) = entry.node {
            self.composer.remove_layer_node(node);
        }
        // The strip goes back with the surface, and the surfaces that were placed against it move up to where it was: a
        // bar that exits must not keep the work area short for the rest of the session.
        if let Some(output) = entry.output {
            self.layer_arrange(output, None);
        }
        self.layer_settled();
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
            log::debug!(area = "layer", ?error, "a popup could not be tracked");
        }
    }

    /// The rectangle a layer surface with this one as its parent occupies.
    pub(crate) fn layer_rect(&self, surface: &WlSurface) -> Option<Rect> {
        self.layers.rect_of(surface)
    }

    /// The surface that holds the keyboard because a layer surface asked for it: the exclusive surface of the topmost layer
    /// that has one, and otherwise the surface the user focused by clicking it, while it is still there.
    pub(crate) fn layer_keyboard_focus(&self) -> Option<WlSurface> {
        self.layers
            .keyboard_grab()
            .map(|entry| entry.surface.wl_surface().clone())
            .or_else(|| self.layers.focused_on_demand())
    }

    /// Recomputes the reserved insets of every output and tells the window record.
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
            let effects = self.wm.reserve(*output, reserved);
            log::debug!(area = "layer", ?reserved, "reserved");
            self.apply_decided(effects);
        }
        // An output that disconnected keeps no reservation.
        self.layers.reserved.retain(|output, _| outputs.contains(output));
    }

    /// Gives the keyboard to the layer surface that the layer rules want to hold it, or back to the window when none does
    /// any more. It acts when what the rules want has changed and not otherwise: a surface that commits again takes nothing
    /// from the popup grab of its own menu.
    pub(crate) fn refresh_layer_focus(&mut self) {
        if self.lock.state() != gnoblin_lock::State::Unlocked {
            return;
        }
        let wanted = self.layer_keyboard_focus();
        if wanted == self.layers.keyboard_holder {
            return;
        }
        self.layers.keyboard_holder.clone_from(&wanted);
        match wanted {
            Some(surface) => self.set_keyboard_focus(Some(surface)),
            None => {
                let target = self.focused_window_surface();
                self.set_keyboard_focus(target);
            }
        }
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

/// The edges of an anchor, by the names the node publishes them under.
fn anchor_names(anchor: Anchor) -> Vec<String> {
    [
        (Anchor::TOP, "top"),
        (Anchor::BOTTOM, "bottom"),
        (Anchor::LEFT, "left"),
        (Anchor::RIGHT, "right"),
    ]
    .into_iter()
    .filter(|(edge, _)| anchor.contains(*edge))
    .map(|(_, name)| name.to_owned())
    .collect()
}

/// The composer's layer for the protocol's.
fn band_of(layer: Layer) -> LayerBand {
    match layer {
        Layer::Background => LayerBand::Background,
        Layer::Bottom => LayerBand::Bottom,
        Layer::Top => LayerBand::Top,
        Layer::Overlay => LayerBand::Overlay,
    }
}

/// The protocol's name for a layer, for a log line and an observation.
fn layer_name(layer: Layer) -> &'static str {
    band_of(layer).name()
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
