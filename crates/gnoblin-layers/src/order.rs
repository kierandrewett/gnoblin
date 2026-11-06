//! Paint order: the one function that turns live layers into a stack.
//!
//! The registry keeps layers in a map keyed by handle, which says nothing about
//! where they sit. [`paint_order`] answers that question. It returns every live
//! layer from the bottom of the stack to the top, in one pass over the layers in
//! registration order.
//!
//! # The rules
//!
//! The bands come first, in [`Band::ALL`] order. Inside one band, each layer is
//! placed as its [`Order`] says, processed oldest first:
//!
//! - `Default` goes on top of the band, so the newest registration is topmost.
//!   An extension layer with a window scope goes below the lowest built-in
//!   layer on the same window, so an extension can never bury Gnoblin's own
//!   chrome on that window. With no such layer it goes on top.
//! - `AboveTarget` goes directly above the topmost layer already on the same
//!   window. With none, it goes on top of the band.
//! - `Above(name)` goes directly above the named layer, and `Below(name)`
//!   directly below it. The registry guarantees the named layer is live and in
//!   this band. It is also older, so it has already been placed.
//!
//! Each rule is one insertion, so the result is deterministic. Two calls over the
//! same registry give the same stack.
//!
//! # Why a single pass
//!
//! Every reference points at an older layer, so processing in registration order
//! means each target is already placed when a layer needs it. There is no
//! sorting and no fixed-point loop.

use gnoblin_types::WindowId;

use crate::band::Band;
use crate::name::LayerName;
use crate::provider::LayerProvider;
use crate::registry::{LayerHandle, LayerRecord, LayerRegistry};
use crate::scope::{LayerScope, Order};

/// Returns every live layer, bottom to top, as handles.
///
/// The result has one entry per live layer, and each band's layers are
/// contiguous and in [`Band::ALL`] order.
#[must_use]
pub fn paint_order(registry: &LayerRegistry) -> Vec<LayerHandle> {
    let mut stack: Vec<LayerHandle> = Vec::with_capacity(registry.len());
    // Oldest first: every `Above`, `Below` and `AboveTarget` names an older layer.
    let mut records: Vec<&LayerRecord> = registry.records().collect();
    records.sort_by_key(|record| record.sequence);

    for band in Band::ALL {
        for record in records.iter().filter(|record| record.spec.band == band) {
            place(&mut stack, registry, record);
        }
    }
    stack
}

/// Inserts one layer into the stack according to its order.
fn place(stack: &mut Vec<LayerHandle>, registry: &LayerRegistry, record: &LayerRecord) {
    let band = record.spec.band;
    match &record.spec.order {
        Order::Default => {
            let below = match (&record.provider, &record.spec.scope) {
                (LayerProvider::Extension(_), LayerScope::Window(Some(target))) => {
                    lowest_builtin_on(stack, registry, *target, band)
                }
                _ => None,
            };
            match below {
                Some(index) => stack.insert(index, record.handle),
                None => stack.push(record.handle),
            }
        }
        Order::AboveTarget => {
            let target = match record.spec.scope {
                LayerScope::Window(Some(target)) => target,
                // The registry refuses this combination, so this only keeps the
                // placement total.
                _ => return stack.push(record.handle),
            };
            match topmost_on(stack, registry, target, band) {
                Some(index) => stack.insert(index + 1, record.handle),
                None => stack.push(record.handle),
            }
        }
        Order::Above(name) => insert_beside(stack, registry, name, record.handle, 1),
        Order::Below(name) => insert_beside(stack, registry, name, record.handle, 0),
    }
}

/// Inserts `handle` next to the layer named `name`. `offset` is `1` for above and
/// `0` for below. A missing target appends, which the registry should prevent.
fn insert_beside(
    stack: &mut Vec<LayerHandle>,
    registry: &LayerRegistry,
    name: &LayerName,
    handle: LayerHandle,
    offset: usize,
) {
    let target = registry.by_name(name).map(|record| record.handle);
    match target.and_then(|t| stack.iter().position(|&h| h == t)) {
        Some(index) => stack.insert(index + offset, handle),
        None => stack.push(handle),
    }
}

/// The index of the lowest built-in layer on `target` in `band`, if any.
fn lowest_builtin_on(stack: &[LayerHandle], registry: &LayerRegistry, target: WindowId, band: Band) -> Option<usize> {
    stack.iter().position(|&handle| {
        registry.get(handle).is_some_and(|record| {
            record.spec.band == band
                && record.provider.is_builtin()
                && record.spec.scope == LayerScope::Window(Some(target))
        })
    })
}

/// The index of the topmost layer on `target` in `band`, if any.
fn topmost_on(stack: &[LayerHandle], registry: &LayerRegistry, target: WindowId, band: Band) -> Option<usize> {
    stack.iter().rposition(|&handle| {
        registry
            .get(handle)
            .is_some_and(|record| record.spec.band == band && record.spec.scope == LayerScope::Window(Some(target)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::name::LocalName;
    use crate::spec::LayerSpec;
    use gnoblin_types::OutputId;

    fn gnoblin() -> LayerProvider {
        LayerProvider::Gnoblin("gnoblin-wayland")
    }

    fn ext(id: &str) -> LayerProvider {
        LayerProvider::Extension(id.to_owned())
    }

    fn spec(local: &str, band: Band) -> LayerSpec {
        LayerSpec::new(
            LocalName::new(local).unwrap(),
            LayerScope::Output(OutputId::new(1)),
            band,
        )
    }

    /// The local names in the stack, bottom to top, for readable assertions.
    fn names(reg: &LayerRegistry) -> Vec<String> {
        paint_order(reg)
            .into_iter()
            .map(|h| reg.get(h).unwrap().name.to_string())
            .collect()
    }

    #[test]
    fn default_order_puts_the_newest_on_top_of_its_band() {
        let mut reg = LayerRegistry::new();
        reg.register(&gnoblin(), spec("a", Band::Top)).unwrap();
        reg.register(&gnoblin(), spec("b", Band::Top)).unwrap();
        assert_eq!(names(&reg), ["gnoblin.a", "gnoblin.b"]);
    }

    #[test]
    fn bands_stack_in_declaration_order_whatever_the_registration_order() {
        let mut reg = LayerRegistry::new();
        reg.register(&gnoblin(), spec("top", Band::Top)).unwrap();
        reg.register(&gnoblin(), spec("bottom", Band::Background)).unwrap();
        assert_eq!(names(&reg), ["gnoblin.bottom", "gnoblin.top"]);
    }

    #[test]
    fn above_and_below_sit_beside_the_named_layer() {
        let mut reg = LayerRegistry::new();
        reg.register(&gnoblin(), spec("a", Band::Top)).unwrap();
        reg.register(&gnoblin(), spec("b", Band::Top)).unwrap();
        let a = LayerName::parse("gnoblin.a").unwrap();
        let mut above = spec("c", Band::Top);
        above.order = Order::Above(a.clone());
        reg.register(&gnoblin(), above).unwrap();
        let mut below = spec("d", Band::Top);
        below.order = Order::Below(a);
        reg.register(&gnoblin(), below).unwrap();
        assert_eq!(names(&reg), ["gnoblin.d", "gnoblin.a", "gnoblin.c", "gnoblin.b"]);
    }

    #[test]
    fn above_target_sits_above_the_topmost_layer_on_its_window() {
        let window = WindowId::new(7);
        let mut reg = LayerRegistry::new();
        let mut first = spec("w1", Band::Windows);
        first.scope = LayerScope::Window(Some(window));
        reg.register(&gnoblin(), first).unwrap();
        reg.register(&gnoblin(), spec("other", Band::Windows)).unwrap();
        let mut chrome = spec("w2", Band::Windows);
        chrome.scope = LayerScope::Window(Some(window));
        chrome.order = Order::AboveTarget;
        reg.register(&gnoblin(), chrome).unwrap();
        assert_eq!(names(&reg), ["gnoblin.w1", "gnoblin.w2", "gnoblin.other"]);
    }

    #[test]
    fn extension_default_goes_below_the_builtins_on_its_window() {
        let window = WindowId::new(7);
        let mut reg = LayerRegistry::new();
        let mut chrome = spec("chrome", Band::Windows);
        chrome.scope = LayerScope::Window(Some(window));
        reg.register(&gnoblin(), chrome).unwrap();
        let mut bar = spec("bar", Band::Windows);
        bar.scope = LayerScope::Window(Some(window));
        reg.register(&ext("bar-ext"), bar).unwrap();
        assert_eq!(names(&reg), ["ext.bar-ext.bar", "gnoblin.chrome"]);
    }

    #[test]
    fn paint_order_is_deterministic_and_covers_every_live_layer() {
        let mut reg = LayerRegistry::new();
        reg.register(&gnoblin(), spec("a", Band::Top)).unwrap();
        reg.register(&ext("bar"), spec("b", Band::Overlay)).unwrap();
        assert_eq!(paint_order(&reg), paint_order(&reg));
        assert_eq!(paint_order(&reg).len(), reg.len());
    }
}
