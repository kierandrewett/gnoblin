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

use gnoblin_types::WindowId;

/// The layers the stack orders, lowest first.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum StackLayer {
    /// Ordinary application windows.
    Normal,
    /// Fullscreen windows, which cover normal windows on the same output.
    Fullscreen,
}

/// Window ids in painting order: index zero is the topmost window.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub(crate) struct Stack {
    order: Vec<WindowId>,
    layers: BTreeMap<WindowId, StackLayer>,
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

    /// Removes a window from the stack.
    pub(crate) fn remove(&mut self, id: WindowId) {
        self.layers.remove(&id);
        self.order.retain(|entry| *entry != id);
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
}
