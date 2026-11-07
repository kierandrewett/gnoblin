//! The stacking order.
//!
//! The stack is one list of window ids, topmost first, divided into layers. Every
//! change to a window's visibility as an insert, a raise, a lower, a layer change
//! or a removal goes through this type, so the two properties the compositor relies
//! on hold by construction:
//!
//! - every managed window appears exactly once;
//! - layers never interleave, so a fullscreen window is above every normal window.
//!
//! Focus is not part of the stack. Whether focus raises a window is a policy
//! decision that arrives with the activation request (`raise_on_click`,
//! `focus_new_windows`), so the stack records what happened rather than deciding.

use std::collections::BTreeMap;

use gnoblin_layers::LayerHandle;

use gnoblin_types::WindowId;

/// The layers the stack orders, lowest first.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum StackLayer {
    /// Ordinary application windows.
    Normal,
    /// Fullscreen windows, which cover normal windows on the same output.
    Fullscreen,
}

/// Who placed a window-scope layer. Gnoblin claims stack above extension claims on
/// the same target, whatever order they arrive in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum EntryOwner {
    Gnoblin,
    Extension,
}

/// A window-scope layer that sits directly above its target window.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct LayerEntry {
    layer: LayerHandle,
    target: WindowId,
    owner: EntryOwner,
}

/// One position in the stack, topmost first: a window, or a layer above one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum StackItem {
    /// A managed window.
    Window(WindowId),
    /// A window-scope layer, directly above its target.
    Layer(LayerHandle),
}

/// Window ids in painting order: index zero is the topmost window. Window-scope
/// layers are kept apart in `entries` and interleaved by [`Stack::items`].
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub(crate) struct Stack {
    order: Vec<WindowId>,
    layers: BTreeMap<WindowId, StackLayer>,
    /// Newest first, so the newest entry on one target sits topmost.
    entries: Vec<LayerEntry>,
}

impl Stack {
    /// The order, topmost first.
    pub(crate) fn order(&self) -> &[WindowId] {
        &self.order
    }

    /// How many windows are stacked.
    pub(crate) fn len(&self) -> usize {
        self.order.len()
    }

    /// The layer a window is in.
    pub(crate) fn layer_of(&self, id: WindowId) -> Option<StackLayer> {
        self.layers.get(&id).copied()
    }

    /// Attaches a window-scope layer above its target. Refused when the target is
    /// not stacked or the layer is already attached.
    ///
    /// Gnoblin entries sit above extension entries on the same target, whatever
    /// order they arrive in. Within one owner the newest entry is topmost.
    pub(crate) fn attach(&mut self, layer: LayerHandle, target: WindowId, owner: EntryOwner) -> bool {
        if !self.layers.contains_key(&target) || self.is_attached(layer) {
            return false;
        }
        let entry = LayerEntry { layer, target, owner };
        let index = match owner {
            EntryOwner::Gnoblin => 0,
            EntryOwner::Extension => self
                .entries
                .iter()
                .rposition(|other| other.target == target && other.owner == EntryOwner::Gnoblin)
                .map(|index| index + 1)
                .or_else(|| self.entries.iter().position(|other| other.target == target))
                .unwrap_or(self.entries.len()),
        };
        self.entries.insert(index, entry);
        true
    }

    /// Whether a layer is attached to a window.
    pub(crate) fn is_attached(&self, layer: LayerHandle) -> bool {
        self.entries.iter().any(|entry| entry.layer == layer)
    }

    /// Every attached window-scope layer with the window it sits above, newest
    /// first. The invariant check reads this to find entries that outlived their
    /// layer or their target.
    pub(crate) fn attached(&self) -> impl Iterator<Item = (LayerHandle, WindowId)> + '_ {
        self.entries.iter().map(|entry| (entry.layer, entry.target))
    }

    /// Drops every entry whose layer no longer passes `live`, so an ended layer
    /// leaves the stack with its record.
    pub(crate) fn retain_layers(&mut self, live: impl Fn(LayerHandle) -> bool) {
        self.entries.retain(|entry| live(entry.layer));
    }

    /// Every stacked item, topmost first. A window's entries sit directly above it,
    /// in their own order.
    pub(crate) fn items(&self) -> Vec<StackItem> {
        let mut items = Vec::with_capacity(self.order.len() + self.entries.len());
        for &window in &self.order {
            items.extend(
                self.entries
                    .iter()
                    .filter(|entry| entry.target == window)
                    .map(|entry| StackItem::Layer(entry.layer)),
            );
            items.push(StackItem::Window(window));
        }
        items
    }

    /// Adds a window to the top of its layer.
    ///
    /// `anchor` places the window directly above another window in the same layer,
    /// which is how a dialog opens above its parent. A duplicate id, or an anchor in
    /// another layer, is ignored rather than corrupting the order.
    pub(crate) fn insert(&mut self, id: WindowId, layer: StackLayer, anchor: Option<WindowId>) {
        if self.layers.contains_key(&id) {
            return;
        }
        self.layers.insert(id, layer);
        let anchored = anchor
            .filter(|anchor| *anchor != id && self.layers.get(anchor) == Some(&layer))
            .and_then(|anchor| self.order.iter().position(|entry| *entry == anchor));
        match anchored {
            Some(index) => self.order.insert(index, id),
            None => self.place_at_top_of_layer(id, layer),
        }
    }

    /// Moves a window to the top of its layer.
    pub(crate) fn raise(&mut self, id: WindowId) {
        let Some(layer) = self.layers.get(&id).copied() else {
            return;
        };
        self.order.retain(|entry| *entry != id);
        self.place_at_top_of_layer(id, layer);
    }

    /// Moves a window to the bottom of its layer, never below a lower layer.
    pub(crate) fn lower(&mut self, id: WindowId) {
        let Some(layer) = self.layers.get(&id).copied() else {
            return;
        };
        self.order.retain(|entry| *entry != id);
        let index = self
            .order
            .iter()
            .position(|entry| self.layers.get(entry).is_some_and(|other| *other < layer))
            .unwrap_or(self.order.len());
        self.order.insert(index, id);
    }

    /// Changes a window's layer, keeping it at the top of the new layer.
    ///
    /// The layer must follow the window's state (fullscreen or not); changing it in
    /// place is what keeps the order valid while the window stays where it is.
    pub(crate) fn set_layer(&mut self, id: WindowId, layer: StackLayer) {
        let Some(current) = self.layers.get(&id).copied() else {
            return;
        };
        if current == layer {
            return;
        }
        self.layers.insert(id, layer);
        self.order.retain(|entry| *entry != id);
        self.place_at_top_of_layer(id, layer);
    }

    /// Removes a window from the stack, and every entry placed on it.
    pub(crate) fn remove(&mut self, id: WindowId) {
        self.layers.remove(&id);
        self.order.retain(|entry| *entry != id);
        self.entries.retain(|entry| entry.target != id);
    }

    /// Inserts at the top of the window's layer, above everything else in it.
    fn place_at_top_of_layer(&mut self, id: WindowId, layer: StackLayer) {
        let index = self
            .order
            .iter()
            .position(|entry| self.layers.get(entry).is_some_and(|other| *other <= layer))
            .unwrap_or(self.order.len());
        self.order.insert(index, id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_layers::{Band, LayerProvider, LayerRegistry, LayerScope, LayerSpec, LocalName};

    fn id(raw: u64) -> WindowId {
        WindowId::new(raw)
    }

    fn stack_with_normal() -> Stack {
        let mut stack = Stack::default();
        for raw in [1, 2, 3] {
            stack.insert(id(raw), StackLayer::Normal, None);
        }
        stack
    }

    #[test]
    fn a_new_window_goes_on_top_of_its_layer() {
        assert_eq!(stack_with_normal().order(), [id(3), id(2), id(1)]);
    }

    #[test]
    fn anchors_put_a_dialog_directly_above_its_parent() {
        let mut stack = stack_with_normal();
        stack.insert(id(9), StackLayer::Normal, Some(id(2)));
        assert_eq!(stack.order(), [id(3), id(9), id(2), id(1)]);
    }

    #[test]
    fn an_anchor_in_another_layer_is_ignored() {
        let mut stack = stack_with_normal();
        stack.insert(id(9), StackLayer::Fullscreen, Some(id(2)));
        assert_eq!(stack.layer_of(id(9)), Some(StackLayer::Fullscreen));
        assert_eq!(stack.order().first(), Some(&id(9)));
    }

    #[test]
    fn raising_and_lowering_stay_inside_the_layer() {
        let mut stack = Stack::default();
        stack.insert(id(1), StackLayer::Normal, None);
        stack.insert(id(2), StackLayer::Normal, None);
        stack.insert(id(3), StackLayer::Fullscreen, None);
        assert_eq!(stack.order(), [id(3), id(2), id(1)]);

        stack.raise(id(1));
        assert_eq!(stack.order(), [id(3), id(1), id(2)]);
        stack.lower(id(1));
        assert_eq!(stack.order(), [id(3), id(2), id(1)]);
        stack.raise(id(3));
        assert_eq!(stack.order(), [id(3), id(2), id(1)]);
    }

    #[test]
    fn becoming_fullscreen_lifts_a_window_above_normal_windows() {
        let mut stack = stack_with_normal();
        stack.set_layer(id(1), StackLayer::Fullscreen);
        assert_eq!(stack.order(), [id(1), id(3), id(2)]);
        stack.set_layer(id(1), StackLayer::Normal);
        assert_eq!(stack.order(), [id(1), id(3), id(2)]);
        assert_eq!(stack.layer_of(id(1)), Some(StackLayer::Normal));
    }

    #[test]
    fn a_duplicate_insert_and_an_unknown_operation_are_ignored() {
        let mut stack = stack_with_normal();
        stack.insert(id(1), StackLayer::Fullscreen, None);
        assert_eq!(stack.layer_of(id(1)), Some(StackLayer::Normal));
        assert_eq!(stack.len(), 3);
        stack.raise(id(99));
        stack.lower(id(99));
        stack.set_layer(id(99), StackLayer::Fullscreen);
        assert_eq!(stack.order(), [id(3), id(2), id(1)]);
    }

    #[test]
    fn removal_leaves_no_hole_and_no_duplicate() {
        let mut stack = stack_with_normal();
        stack.remove(id(2));
        assert_eq!(stack.order(), [id(3), id(1)]);
        assert_eq!(stack.layer_of(id(2)), None);
        stack.remove(id(2));
        assert_eq!(stack.len(), 2);
    }

    #[test]
    fn layers_never_interleave() {
        let mut stack = Stack::default();
        stack.insert(id(1), StackLayer::Fullscreen, None);
        stack.insert(id(2), StackLayer::Normal, None);
        stack.insert(id(3), StackLayer::Fullscreen, None);
        stack.insert(id(4), StackLayer::Normal, None);
        let layers: Vec<StackLayer> = stack
            .order()
            .iter()
            .map(|entry| match stack.layer_of(*entry) {
                Some(layer) => layer,
                None => panic!("a stacked window has a layer"),
            })
            .collect();
        assert_eq!(
            layers,
            [
                StackLayer::Fullscreen,
                StackLayer::Fullscreen,
                StackLayer::Normal,
                StackLayer::Normal
            ]
        );
    }

    fn w(raw: u64) -> StackItem {
        StackItem::Window(id(raw))
    }

    fn l(layer: LayerHandle) -> StackItem {
        StackItem::Layer(layer)
    }

    /// Real handles, because a `LayerHandle` can only come from a registry.
    fn handles(count: usize) -> Vec<LayerHandle> {
        let mut registry = LayerRegistry::new();
        (0..count)
            .map(|index| {
                let local = LocalName::new(&format!("layer-{index}")).expect("a plain local name is valid");
                let spec = LayerSpec::new(local, LayerScope::Window(Some(id(1))), Band::Windows);
                registry
                    .register(&LayerProvider::Gnoblin("gnoblin-wm"), spec)
                    .expect("the layer registers")
            })
            .collect()
    }

    #[test]
    fn an_entry_sits_directly_above_its_target() {
        let mut stack = stack_with_normal();
        let layers = handles(1);
        assert!(stack.attach(layers[0], id(2), EntryOwner::Gnoblin));
        assert_eq!(stack.items(), vec![w(3), l(layers[0]), w(2), w(1)]);
    }

    #[test]
    fn an_entry_moves_with_its_target_on_raise_and_lower() {
        let mut stack = stack_with_normal();
        let layers = handles(1);
        assert!(stack.attach(layers[0], id(1), EntryOwner::Gnoblin));
        stack.raise(id(1));
        assert_eq!(stack.items(), vec![l(layers[0]), w(1), w(3), w(2)]);
        stack.lower(id(1));
        assert_eq!(stack.items(), vec![w(3), w(2), l(layers[0]), w(1)]);
    }

    #[test]
    fn an_entry_follows_its_target_into_the_fullscreen_layer() {
        let mut stack = stack_with_normal();
        let layers = handles(1);
        assert!(stack.attach(layers[0], id(1), EntryOwner::Gnoblin));
        stack.set_layer(id(1), StackLayer::Fullscreen);
        assert_eq!(stack.items(), vec![l(layers[0]), w(1), w(3), w(2)]);
    }

    #[test]
    fn closing_the_target_removes_its_entries_and_no_others() {
        let mut stack = stack_with_normal();
        let layers = handles(2);
        assert!(stack.attach(layers[0], id(1), EntryOwner::Gnoblin));
        assert!(stack.attach(layers[1], id(2), EntryOwner::Gnoblin));
        stack.remove(id(2));
        assert_eq!(stack.items(), vec![w(3), l(layers[0]), w(1)]);
        assert!(!stack.is_attached(layers[1]));
    }

    #[test]
    fn two_entries_on_one_target_keep_their_own_order() {
        let mut stack = stack_with_normal();
        let layers = handles(3);
        assert!(stack.attach(layers[0], id(1), EntryOwner::Gnoblin));
        assert!(stack.attach(layers[1], id(1), EntryOwner::Gnoblin));
        assert_eq!(stack.items(), vec![w(3), w(2), l(layers[1]), l(layers[0]), w(1)]);
        // Raising another window must not reorder the entries on this one.
        stack.raise(id(2));
        assert_eq!(stack.items(), vec![w(2), w(3), l(layers[1]), l(layers[0]), w(1)]);
        // A newer entry goes on top of the older ones on the same target.
        assert!(stack.attach(layers[2], id(1), EntryOwner::Gnoblin));
        assert_eq!(
            stack.items(),
            vec![w(2), w(3), l(layers[2]), l(layers[1]), l(layers[0]), w(1)]
        );
    }

    #[test]
    fn a_gnoblin_entry_sits_above_an_extension_entry_on_the_same_target_in_either_order() {
        let layers = handles(3);

        let mut extension_first = stack_with_normal();
        assert!(extension_first.attach(layers[0], id(1), EntryOwner::Extension));
        assert!(extension_first.attach(layers[1], id(1), EntryOwner::Gnoblin));
        assert_eq!(
            extension_first.items(),
            vec![w(3), w(2), l(layers[1]), l(layers[0]), w(1)]
        );

        let mut gnoblin_first = stack_with_normal();
        assert!(gnoblin_first.attach(layers[1], id(1), EntryOwner::Gnoblin));
        assert!(gnoblin_first.attach(layers[0], id(1), EntryOwner::Extension));
        assert_eq!(
            gnoblin_first.items(),
            vec![w(3), w(2), l(layers[1]), l(layers[0]), w(1)]
        );

        // A later Gnoblin entry still goes above an earlier extension entry.
        assert!(extension_first.attach(layers[2], id(1), EntryOwner::Gnoblin));
        assert_eq!(
            extension_first.items(),
            vec![w(3), w(2), l(layers[2]), l(layers[1]), l(layers[0]), w(1)]
        );
    }

    #[test]
    fn newer_extension_entries_sit_above_older_ones() {
        let mut stack = stack_with_normal();
        let layers = handles(2);
        assert!(stack.attach(layers[0], id(1), EntryOwner::Extension));
        assert!(stack.attach(layers[1], id(1), EntryOwner::Extension));
        assert_eq!(stack.items(), vec![w(3), w(2), l(layers[1]), l(layers[0]), w(1)]);
    }

    #[test]
    fn an_entry_on_an_unknown_window_is_refused() {
        let mut stack = stack_with_normal();
        let layers = handles(1);
        assert!(!stack.attach(layers[0], id(9), EntryOwner::Gnoblin));
        assert!(!stack.is_attached(layers[0]));
        assert_eq!(stack.items(), vec![w(3), w(2), w(1)]);
    }

    #[test]
    fn a_layer_attaches_at_most_once() {
        let mut stack = stack_with_normal();
        let layers = handles(1);
        assert!(stack.attach(layers[0], id(1), EntryOwner::Gnoblin));
        assert!(!stack.attach(layers[0], id(2), EntryOwner::Gnoblin));
        assert_eq!(stack.items(), vec![w(3), w(2), l(layers[0]), w(1)]);
    }

    #[test]
    fn a_fullscreen_target_keeps_its_entries_above_it_inside_its_layer() {
        let mut stack = Stack::default();
        stack.insert(id(1), StackLayer::Normal, None);
        stack.insert(id(2), StackLayer::Fullscreen, None);
        stack.insert(id(3), StackLayer::Normal, None);
        let layers = handles(1);
        assert!(stack.attach(layers[0], id(2), EntryOwner::Gnoblin));
        assert_eq!(stack.items(), vec![l(layers[0]), w(2), w(3), w(1)]);
        // Raising a normal window cannot slide it between the fullscreen window
        // and its entry, and lowering the fullscreen window keeps the entry on it.
        stack.raise(id(1));
        assert_eq!(stack.items(), vec![l(layers[0]), w(2), w(1), w(3)]);
        stack.lower(id(2));
        assert_eq!(stack.items(), vec![l(layers[0]), w(2), w(1), w(3)]);
    }

    #[test]
    fn ended_layers_are_pruned_and_their_target_stays() {
        let mut stack = stack_with_normal();
        let layers = handles(2);
        assert!(stack.attach(layers[0], id(1), EntryOwner::Gnoblin));
        assert!(stack.attach(layers[1], id(2), EntryOwner::Gnoblin));
        stack.retain_layers(|layer| layer != layers[0]);
        assert!(!stack.is_attached(layers[0]));
        assert_eq!(stack.items(), vec![w(3), l(layers[1]), w(2), w(1)]);
    }
}
