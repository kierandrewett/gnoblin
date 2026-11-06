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
    let mut recorded: BTreeSet<&str> = entries
        .iter()
        .map(|entry| entry["key"].as_str().expect("a key"))
        .collect();
    for (key, since, reason) in REWRITE_ONLY_KEYS {
        assert!(
            !recorded.contains(key),
            "{key} is in the C reference, so it is not rewrite-only"
        );
        assert!(
            !since.is_empty() && since.split('.').count() == 3,
            "{key} names no x.y.z release"
        );
        assert!(!reason.is_empty(), "{key} has no reason");
        recorded.insert(*key);
    }
    let ours: BTreeSet<&str> = TOP_LEVEL_KEYS.iter().copied().collect();
    assert_eq!(
        ours, recorded,
        "every recorded top-level key is accepted, and nothing else"
    );
}

/// Top-level keys the rewrite added, which the C reference cannot list.
///
/// `config-keys.json` records what the C build accepts. `extensions` is the host
/// configuration of IPC extensions (rust-compositor.md, "IPC extensions"), which the
/// C core never had, so it is listed here with the release that added it.
const REWRITE_ONLY_KEYS: &[(&str, &str, &str)] = &[
    (
        "extensions",
        "0.2.0",
        "the IPC extension section of host configuration, loaded by gnoblin_config::extensions",
    ),
    (
        "layout",
        "0.2.0",
        "the layout section: which Lua layout engine arranges each workspace, loaded by gnoblin-compositor",
    ),
    (
        "compat",
        "0.2.0",
        "the compatibility section: which D-Bus adapters the services role serves, read by the adapters at start",
    ),
    (
        "appearance",
        "0.2.0",
        "the appearance section: the colour scheme and accent published to dconf and the portal; the C core had no single table for them",
    ),
];

/// Protocols the rewrite added, which the C reference cannot list.
///
/// `tests/parity/contracts/config-keys.json` is generated from the C tree, so it
/// records what the C build advertises. A protocol the Rust compositor exposes
/// because a rewrite lane added it belongs here, with the release that added it
/// and why, so the difference between the two lists is a reviewed decision in one
/// place rather than a loosened assertion.
const REWRITE_ONLY_PROTOCOLS: &[(&str, &str, &str)] = &[
    (
        "ext-image-copy-capture",
        "0.2.0",
        "ext-image-copy-capture-v1, added by the capture lane; the C tree only has wlr-screencopy",
    ),
    (
        "wlr-output-management",
        "0.2.0",
        "zwlr_output_manager_v1, added by the WlrOutputManagement lane so a configuration tool can read and test the outputs; the C tree has no output manager",
    ),
];

/// Protocols the C reference advertises that the Rust compositor removed on
/// purpose, with the reason. The C tree keeps them until its session is retired,
/// so they stay in the recorded list; this is the one reviewed place that
/// explains their absence from Rust.
const RETIRED_PROTOCOLS: &[(&str, &str)] = &[(
    "blur-fade",
    "blur follows the opacity of the layer it is on, so no protocol is needed to fade it",
)];

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

    // The recorded list comes from the C tree, so it cannot know a protocol the
    // rewrite added. The rule is therefore: the Rust enum is the C reference
    // enumeration plus a reviewed list of rewrite-only protocols, and nothing
    // else. A Rust name that is neither is a protocol nobody reviewed, and a C
    // name missing from Rust is a setting this build silently dropped.
    for (name, since, reason) in REWRITE_ONLY_PROTOCOLS {
        assert!(
            !recorded.contains(name),
            "{name} is in the C reference, so it is not rewrite-only"
        );
        assert!(
            !since.is_empty() && since.split('.').count() == 3,
            "{name} names no x.y.z release"
        );
        assert!(!reason.is_empty(), "{name} has no reason");
    }
    let retired: BTreeSet<&str> = RETIRED_PROTOCOLS.iter().map(|(name, _)| *name).collect();
    let mut expected: BTreeSet<&str> = recorded
        .iter()
        .copied()
        .filter(|name| !retired.contains(name))
        .collect();
    for (name, _, _) in REWRITE_ONLY_PROTOCOLS {
        let added = expected.insert(name);
        assert!(added, "{name} is listed twice as a rewrite-only protocol");
    }
    let ours: BTreeSet<&str> = Protocol::VARIANT_TEXTS.iter().copied().collect();
    for (name, reason) in RETIRED_PROTOCOLS {
        assert!(
            recorded.contains(name),
            "{name} is retired but the C reference does not record it"
        );
        assert!(!ours.contains(name), "{name} is retired but the Rust enum still has it");
        assert!(!reason.is_empty(), "{name} has no reason");
    }
    let unreviewed: Vec<&&str> = ours.difference(&expected).collect();
    assert!(
        unreviewed.is_empty(),
        "protocols the C reference does not have and no review lists: {unreviewed:?}"
    );
    let missing: Vec<&&str> = expected.difference(&ours).collect();
    assert!(
        missing.is_empty(),
        "protocols the C reference has that Rust lost: {missing:?}"
    );
}
