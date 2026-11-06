//! Tests for the model's happy paths: band order, defaults, registration order and
//! removal by the owner. The refusals are in `failure_modes.rs`.

use gnoblin_layers::{
    LayerBand, LayerContent, LayerInput, LayerProvider, LayerProviderKind, LayerRecord, LayerRegistry, LayerScope,
};
use gnoblin_types::{OutputId, OwnerId};

fn builtin(raw: u64) -> LayerProvider {
    LayerProvider {
        id: OwnerId::new(raw),
        kind: LayerProviderKind::Builtin,
    }
}

fn record(name: &str, band: LayerBand) -> LayerRecord {
    LayerRecord::new(
        name,
        builtin(1),
        LayerScope::Output(OutputId::new(1)),
        band,
        LayerContent::Surface,
    )
}

#[test]
fn band_all_is_in_stacking_order_bottom_to_top() {
    let mut sorted = LayerBand::ALL;
    sorted.sort();
    assert_eq!(sorted, LayerBand::ALL, "ALL must match the derived Ord");
    assert!(LayerBand::Background < LayerBand::Cursor);
}

#[test]
fn band_names_are_the_fixed_configuration_names() {
    let names: Vec<&str> = LayerBand::ALL.iter().map(|band| band.as_str()).collect();
    assert_eq!(
        names,
        [
            "background",
            "bottom",
            "windows",
            "top",
            "overlay",
            "system",
            "lock",
            "cursor"
        ]
    );
}

#[test]
fn the_first_four_bands_are_the_four_wlr_layers_in_order() {
    assert_eq!(
        &LayerBand::ALL[..4],
        &[
            LayerBand::Background,
            LayerBand::Bottom,
            LayerBand::Windows,
            LayerBand::Top
        ]
    );
}

#[test]
fn a_new_record_is_passive_and_not_secure() {
    let layer = record("dock", LayerBand::Top);
    assert_eq!(layer.input, LayerInput::Passive);
    assert!(!layer.secure, "secure must default to false");
    assert_eq!(LayerInput::default(), LayerInput::Passive);
}

#[test]
fn list_returns_layers_in_registration_order() {
    let mut registry = LayerRegistry::new();
    registry.register(record("zeta", LayerBand::Top)).unwrap();
    registry.register(record("alpha", LayerBand::Background)).unwrap();
    registry.register(record("mid", LayerBand::Overlay)).unwrap();

    let names: Vec<&str> = registry.list().map(|layer| layer.name.as_str()).collect();

    assert_eq!(names, ["zeta", "alpha", "mid"]);
    assert_eq!(registry.len(), 3);
}

#[test]
fn get_finds_a_registered_layer_by_name() {
    let mut registry = LayerRegistry::new();
    registry.register(record("dock", LayerBand::Top)).unwrap();

    let found = registry.get("dock").expect("registered layer");
    assert_eq!(found.band, LayerBand::Top);
    assert!(registry.get("missing").is_none());
}

#[test]
fn the_owner_may_remove_a_layer_and_its_name_becomes_free() {
    let mut registry = LayerRegistry::new();
    registry.register(record("dock", LayerBand::Top)).unwrap();

    let removed = registry.remove("dock", builtin(1)).expect("owner removes");

    assert_eq!(removed.name, "dock");
    assert!(registry.is_empty());
    registry
        .register(record("dock", LayerBand::Bottom))
        .expect("a freed name can be registered again");
}

#[test]
fn removing_one_layer_keeps_the_others_in_order() {
    let mut registry = LayerRegistry::new();
    registry.register(record("a", LayerBand::Top)).unwrap();
    registry.register(record("b", LayerBand::Top)).unwrap();
    registry.register(record("c", LayerBand::Top)).unwrap();

    registry.remove("b", builtin(1)).unwrap();

    let names: Vec<&str> = registry.list().map(|layer| layer.name.as_str()).collect();
    assert_eq!(names, ["a", "c"]);
}
