//! One test per way the registry refuses a change, so each refusal is pinned.
//!
//! The registry's rules are the contract that stops one provider from taking
//! another's surface, so every refusal is a test. A failure here is a security
//! regression, not a cosmetic one.

use gnoblin_layers::{
    LayerBand, LayerContent, LayerError, LayerProvider, LayerProviderKind, LayerRecord, LayerRegistry, LayerScope,
    is_reserved_name,
};
use gnoblin_types::{OutputId, OwnerId};

fn builtin(raw: u64) -> LayerProvider {
    LayerProvider {
        id: OwnerId::new(raw),
        kind: LayerProviderKind::Builtin,
    }
}

fn extension(raw: u64) -> LayerProvider {
    LayerProvider {
        id: OwnerId::new(raw),
        kind: LayerProviderKind::Extension,
    }
}

fn record(name: &str, provider: LayerProvider) -> LayerRecord {
    LayerRecord::new(
        name,
        provider,
        LayerScope::Output(OutputId::new(1)),
        LayerBand::Top,
        LayerContent::Surface,
    )
}

#[test]
fn empty_name_is_refused() {
    let mut registry = LayerRegistry::new();
    let result = registry.register(record("", builtin(1)));
    assert_eq!(result, Err(LayerError::EmptyName));
    assert!(registry.is_empty());
}

#[test]
fn a_second_registration_of_a_name_is_refused_even_from_the_same_owner() {
    let mut registry = LayerRegistry::new();
    registry.register(record("dock", builtin(1))).unwrap();

    let result = registry.register(record("dock", builtin(1)));

    assert_eq!(
        result,
        Err(LayerError::NameTaken {
            name: "dock".to_owned()
        })
    );
    assert_eq!(registry.len(), 1);
}

#[test]
fn a_name_taken_by_another_owner_is_refused() {
    let mut registry = LayerRegistry::new();
    registry.register(record("dock", builtin(1))).unwrap();

    let result = registry.register(record("dock", builtin(2)));

    assert_eq!(
        result,
        Err(LayerError::NameTaken {
            name: "dock".to_owned()
        })
    );
    assert_eq!(registry.get("dock").unwrap().provider.id, OwnerId::new(1));
}

#[test]
fn an_extension_cannot_register_a_reserved_name() {
    let mut registry = LayerRegistry::new();
    for name in ["portals", "alerts", "window_menu", "lock"] {
        let result = registry.register(record(name, extension(7)));
        assert_eq!(
            result,
            Err(LayerError::ReservedName { name: name.to_owned() }),
            "{name}"
        );
    }
    assert!(registry.is_empty());
}

#[test]
fn an_extension_cannot_register_a_name_under_the_surface_prefix() {
    let mut registry = LayerRegistry::new();
    let result = registry.register(record("surface_x11", extension(7)));
    assert_eq!(
        result,
        Err(LayerError::ReservedName {
            name: "surface_x11".to_owned()
        })
    );
}

#[test]
fn an_extension_learns_nothing_about_a_built_in_owner() {
    // The reserved check runs before the taken check, so an extension asking for
    // a built-in name gets the same answer whether or not a built-in holds it.
    let mut registry = LayerRegistry::new();
    registry.register(record("portals", builtin(1))).unwrap();

    let result = registry.register(record("portals", extension(7)));

    assert_eq!(
        result,
        Err(LayerError::ReservedName {
            name: "portals".to_owned()
        })
    );
}

#[test]
fn a_built_in_provider_may_use_a_reserved_name() {
    let mut registry = LayerRegistry::new();
    registry.register(record("portals", builtin(1))).unwrap();
    assert_eq!(
        registry.get("portals").unwrap().provider.kind,
        LayerProviderKind::Builtin
    );
}

#[test]
fn removing_a_missing_layer_is_refused() {
    let mut registry = LayerRegistry::new();
    let result = registry.remove("dock", builtin(1));
    assert_eq!(
        result,
        Err(LayerError::NotFound {
            name: "dock".to_owned()
        })
    );
}

#[test]
fn removing_another_providers_layer_is_refused_and_keeps_it() {
    let mut registry = LayerRegistry::new();
    registry.register(record("dock", builtin(1))).unwrap();

    let result = registry.remove("dock", builtin(2));

    assert_eq!(
        result,
        Err(LayerError::NotOwner {
            name: "dock".to_owned(),
            owner: OwnerId::new(1),
            caller: OwnerId::new(2),
        })
    );
    assert!(registry.get("dock").is_some(), "a refused removal must keep the layer");
}

#[test]
fn a_refused_registration_leaves_the_list_unchanged() {
    let mut registry = LayerRegistry::new();
    registry.register(record("dock", builtin(1))).unwrap();
    let before = registry.clone();

    let _ = registry.register(record("dock", builtin(2)));
    let _ = registry.register(record("", builtin(2)));

    assert_eq!(registry, before);
}

#[test]
fn reserved_name_rule_covers_the_built_in_names_and_the_prefix() {
    assert!(is_reserved_name("portals"));
    assert!(is_reserved_name("alerts"));
    assert!(is_reserved_name("window_menu"));
    assert!(is_reserved_name("lock"));
    assert!(is_reserved_name("surface_toplevel"));
    assert!(is_reserved_name("surface_layer_shell"));
    assert!(is_reserved_name("surface_x11"));
    // Near misses are not reserved: the rule is an exact list plus one prefix.
    assert!(!is_reserved_name("surfaces"));
    assert!(!is_reserved_name("window"));
    assert!(!is_reserved_name("dock"));
}

#[test]
fn every_error_has_a_readable_message() {
    let errors = [
        LayerError::EmptyName,
        LayerError::NameTaken {
            name: "dock".to_owned(),
        },
        LayerError::ReservedName {
            name: "portals".to_owned(),
        },
        LayerError::NotFound {
            name: "dock".to_owned(),
        },
        LayerError::NotOwner {
            name: "dock".to_owned(),
            owner: OwnerId::new(1),
            caller: OwnerId::new(2),
        },
    ];
    for error in errors {
        assert!(!error.to_string().is_empty(), "{error:?}");
    }
}
