//! The generated catalogue against the declarations it is generated from.
//!
//! These are the checks that catch a schema edit that produces a document which
//! cannot behave as declared: a default outside its own range, an alias naming a
//! spelling no enum accepts, a hook the schema lists but no code implements. They
//! read the generated constants, so they test what the build compiled rather than
//! re-reading the schema.

use gnoblin_config::{ALIASES, Default, ENUM_SPECS, Kind, Setting, TOP_LEVEL_KEYS, generated, hooks};

/// Every declared setting, in section order.
fn settings() -> &'static [Setting] {
    generated::SETTINGS
}

/// The accepted spellings of the enum a setting names.
fn variants_of(name: &str) -> &'static [&'static str] {
    for spec in ENUM_SPECS {
        if spec.name == name {
            return spec.variants;
        }
    }
    panic!("no enum `{name}` in ENUM_SPECS");
}

#[test]
fn every_default_is_inside_the_range_it_is_declared_with() {
    for setting in settings() {
        let where_ = format!("{}.{}", setting.section, setting.key);
        let inside = match (setting.kind, setting.default) {
            (_, Default::None) => continue,
            (Kind::Int { min, max }, Default::Integer(value)) => value >= min && value <= max,
            (
                Kind::Float {
                    min,
                    max,
                    exclusive_max,
                },
                Default::Float(value),
            ) => value >= min && if exclusive_max { value < max } else { value <= max },
            (
                Kind::Float {
                    min,
                    max,
                    exclusive_max,
                },
                Default::Integer(value),
            ) => {
                let value = value as f64;
                value >= min && if exclusive_max { value < max } else { value <= max }
            }
            (Kind::Enum(name), Default::Enum(spelling)) => variants_of(name).contains(&spelling),
            (Kind::Bool, Default::Bool(_)) | (Kind::Text { .. }, Default::Text(_)) => continue,
            (Kind::EnumList(_) | Kind::TextList, Default::List(_)) => continue,
            (kind, default) => panic!("{where_} declares {kind:?} with default {default:?}"),
        };
        assert!(inside, "{where_} has a default outside its declared range or spellings");
    }
}

#[test]
fn every_alias_names_a_current_spelling_of_its_own_setting() {
    for alias in ALIASES {
        let specs: Vec<_> = ENUM_SPECS.iter().filter(|spec| spec.setting == alias.setting).collect();
        assert!(!specs.is_empty(), "alias `{}` names no enum setting", alias.spelling);
        assert!(
            specs.iter().any(|spec| spec.variants.contains(&alias.canonical)),
            "alias `{}` -> `{}`: no enum accepts the canonical spelling",
            alias.spelling,
            alias.canonical
        );
        assert!(
            !specs.iter().any(|spec| spec.variants.contains(&alias.spelling)),
            "alias `{}` is already current, so it could never fire",
            alias.spelling
        );
        assert_eq!(
            alias.removed_in.split('.').count(),
            3,
            "an alias names the release it goes away"
        );
    }
}

#[test]
fn the_hooks_the_schema_declares_are_the_ones_that_exist() {
    assert_eq!(generated::DECLARED_HOOKS, hooks::IMPLEMENTED_HOOKS);
    for hook in generated::DECLARED_HOOKS {
        assert!(!hook.is_empty(), "a hook with no name would call nothing");
    }
}

#[test]
fn the_top_level_keys_are_sorted_and_each_has_a_declared_setting() {
    let mut sorted = TOP_LEVEL_KEYS.to_vec();
    sorted.sort_unstable();
    assert_eq!(sorted, TOP_LEVEL_KEYS);
    for section in TOP_LEVEL_KEYS {
        // A carried section is recognised but has no settings of its own.
        assert!(
            settings().iter().any(|setting| setting.section == *section)
                || gnoblin_config::CARRIED_SECTIONS.contains(section),
            "{section} is a top-level key with no declared setting"
        );
    }
}

#[test]
fn every_setting_names_a_release_and_a_field() {
    for setting in settings() {
        assert_eq!(
            setting.since.split('.').count(),
            3,
            "{}.{} has since `{}`, not x.y.z",
            setting.section,
            setting.key,
            setting.since
        );
        assert!(
            !setting.field.is_empty(),
            "{}.{} names no field",
            setting.section,
            setting.key
        );
        assert!(
            !setting.accepted.is_empty(),
            "{}.{} has no accepted-values text",
            setting.section,
            setting.key
        );
    }
}
