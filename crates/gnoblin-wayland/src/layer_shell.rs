//! The layer surfaces' place in the session: arranged against their output, configured, drawn and followed.
//!
//! The protocol crate (`gnoblin-wayland-zwlr-layer-shell-v1`) holds the global, the table of surfaces and the arithmetic that
//! turns anchors, margins and a size into a rectangle. This module is the compositor's side of it: it places every surface
//! of an output, sends the configures, hands the composer the nodes and the pixels, turns the exclusive zones into the
//! reserved insets the window manager reads, and gives the keyboard to the surface the layer rules want.

use gnoblin_backend::render::compose::TreeOwner;
use gnoblin_render_frame::frame::SurfaceSource;
use gnoblin_types::{Insets, OutputId, Rect};
use gnoblin_wayland_protocols::zwlr_layer_shell_v1::{
    ZwlrLayerShellV1LayerEntry, ZwlrLayerShellV1Layers, anchor_names, band_of, configure_size, exclusive_value,
    layer_name, place, reservation_edge,
};
use gnoblin_wayland_types::layer::{LayerNodeInfo, LayerOverride};
use smithay::output::Output as SmithayOutput;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::Size;
use smithay::wayland::shell::wlr_layer::{ExclusiveZone, KeyboardInteractivity, Layer, LayerSurface};

use crate::State;
use crate::observation::ObservedLayer;

use crate::log;

/// The surfaces as they are published to a reader.
pub(crate) fn observed(layers: &ZwlrLayerShellV1Layers) -> Vec<ObservedLayer> {
    layers
        .entries
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
                let content = crate::state::committed_content(surface.wl_surface()).map(|(buffer, content)| {
                    let damage = crate::state::committed_damage(surface.wl_surface());
                    (content, SurfaceSource::of_buffer(buffer), Some(damage))
                });
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
