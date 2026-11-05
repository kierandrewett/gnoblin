//! The document layer against the recorded contract inventory.
//!
//! `tests/parity/contracts/config-keys.json` is generated from the C
//! configuration core by the parity lane, so it is the record of which top-level
//! keys today's Gnoblin accepts and which protocol names it knows. Comparing the
//! Rust lists against it turns "did we forget a key?" into a failing test instead
//! of a bug report after the cutover.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use gnoblin_config::{Protocol, TOP_LEVEL_KEYS};

fn inventory() -> serde_json::Value {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("../../tests/parity/contracts/config-keys.json");
    let text = fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_str(&text).expect("the inventory is JSON")
}

#[test]
fn the_document_accepts_exactly_the_recorded_top_level_keys() {
    let inventory = inventory();
    let entries = inventory["entries"].as_array().expect("entries");
    let recorded: BTreeSet<&str> = entries
        .iter()
        .map(|entry| entry["key"].as_str().expect("a key"))
        .collect();
    let ours: BTreeSet<&str> = TOP_LEVEL_KEYS.iter().copied().collect();
    assert_eq!(
        ours, recorded,
        "every recorded top-level key is accepted, and nothing else"
    );
}

#[test]
fn the_protocol_enum_matches_the_recorded_enumeration() {
    let inventory = inventory();
    let enumerations = inventory["enumerations"].as_array().expect("enumerations");
    let protocols = enumerations
        .iter()
        .find(|enumeration| enumeration["name"] == "protocols")
        .expect("the recorded protocols enumeration");
    let recorded: Vec<&str> = protocols["values"]
        .as_array()
        .expect("values")
        .iter()
        .map(|value| value.as_str().expect("a name"))
        .collect();
    assert_eq!(Protocol::VARIANT_TEXTS, recorded);
}
