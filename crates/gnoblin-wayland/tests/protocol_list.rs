//! `docs/wayland-protocols.md` lists the globals the compositor advertises, and this test is what keeps it true.
//!
//! A headless session is started, one client connects on the privileged socket and one on the ordinary socket, and
//! each reads the registry. The table in the document is parsed and compared with what the two registries said:
//!
//! - every global the privileged client saw is in the table, at the version the compositor serves;
//! - every row of the table is advertised, unless its notes begin with `Conditional:`, which marks a global that only
//!   some sessions have (a GPU with a render node, an Xwayland, a tablet) and so is absent from a headless one;
//! - a row marked `Restricted` is one the ordinary client did not see, and a row marked `Everyone` is one it did.
//!
//! The Gnoblin-owned `gnoblin_*` globals are not part of the list: they are the session's own channels, documented in
//! `design/`, and are left out on both sides.
//!
//! When it fails, the message names the row to add, change or remove. Adding a global to the compositor without a row
//! here fails `every_advertised_global_is_documented`; the list cannot drift.

use gnoblin_wayland_testkit::session;

use std::collections::BTreeMap;
use std::path::Path;

use session::{Client, Session};

/// One row of the document's table.
#[derive(Debug)]
struct Row {
    version: u32,
    restricted: bool,
    conditional: bool,
}

/// The document's table, by interface name.
fn documented() -> BTreeMap<String, Row> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/wayland-protocols.md");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{} is readable: {error}", path.display()));
    let mut rows = BTreeMap::new();
    for line in text.lines().filter(|line| line.starts_with("| `")) {
        let cells: Vec<&str> = line.trim().trim_matches('|').split('|').map(str::trim).collect();
        assert!(cells.len() == 5, "a table row has five cells: {line}");
        let interface = cells[0].trim_matches('`').to_owned();
        let version: u32 = cells[1]
            .parse()
            .unwrap_or_else(|_| panic!("the version of {interface} is a number, not {:?}", cells[1]));
        let restricted = match cells[3] {
            "Restricted" => true,
            "Everyone" => false,
            other => panic!("the access of {interface} is `Everyone` or `Restricted`, not {other:?}"),
        };
        let row = Row {
            version,
            restricted,
            conditional: cells[4].starts_with("Conditional:"),
        };
        assert!(
            rows.insert(interface.clone(), row).is_none(),
            "{interface} is listed twice"
        );
    }
    assert!(!rows.is_empty(), "the document has a table of globals");
    rows
}

/// What a client's registry said, without the Gnoblin-owned globals.
fn advertised(client: &Client) -> BTreeMap<String, u32> {
    client
        .state
        .globals
        .iter()
        .filter(|(interface, _)| !interface.starts_with("gnoblin_"))
        .map(|(interface, version)| (interface.clone(), *version))
        .collect()
}

/// The two registries of a headless session: the privileged socket's and the ordinary socket's.
fn registries() -> (BTreeMap<String, u32>, BTreeMap<String, u32>) {
    let (sender, receiver) = std::sync::mpsc::channel();
    Session::start().run(move |session| {
        let privileged = Client::connect(session.privileged_socket());
        let ordinary = Client::connect(session.socket());
        let (privileged, ordinary) = (advertised(&privileged), advertised(&ordinary));
        // Captured unless the run says `--nocapture`, which is how the table in the document is written.
        for (interface, version) in &privileged {
            let restricted = !ordinary.contains_key(interface);
            println!(
                "REGISTRY {interface} {version} {}",
                if restricted { "Restricted" } else { "Everyone" }
            );
        }
        sender.send((privileged, ordinary)).expect("the test is listening");
    });
    receiver.recv().expect("the session answered")
}

#[test]
fn every_advertised_global_is_documented() {
    let (privileged, _) = registries();
    let table = documented();
    let missing: Vec<String> = privileged
        .iter()
        .filter(|(interface, _)| !table.contains_key(*interface))
        .map(|(interface, version)| format!("{interface} (version {version})"))
        .collect();
    assert!(
        missing.is_empty(),
        "docs/wayland-protocols.md lacks a row for: {}",
        missing.join(", ")
    );
}

#[test]
fn every_documented_global_is_advertised() {
    let table = documented();
    let (privileged, _) = registries();
    let absent: Vec<&str> = table
        .iter()
        .filter(|(interface, row)| !row.conditional && !privileged.contains_key(*interface))
        .map(|(interface, _)| interface.as_str())
        .collect();
    assert!(
        absent.is_empty(),
        "docs/wayland-protocols.md lists globals the session does not advertise (mark them `Conditional:` if only \
         some sessions do): {}",
        absent.join(", ")
    );
}

#[test]
fn the_documented_versions_are_the_advertised_ones() {
    let table = documented();
    let (privileged, _) = registries();
    let wrong: Vec<String> = table
        .iter()
        .filter_map(|(interface, row)| {
            let served = *privileged.get(interface)?;
            (served != row.version).then(|| format!("{interface}: documented {}, advertised {served}", row.version))
        })
        .collect();
    assert!(wrong.is_empty(), "version drift: {}", wrong.join("; "));
}

#[test]
fn the_documented_access_is_the_advertised_access() {
    let table = documented();
    let (privileged, ordinary) = registries();
    let wrong: Vec<String> = table
        .iter()
        .filter(|(interface, _)| privileged.contains_key(*interface))
        .filter_map(|(interface, row)| {
            let hidden = !ordinary.contains_key(interface);
            (hidden != row.restricted).then(|| {
                format!(
                    "{interface}: documented {}, but the ordinary socket {} it",
                    if row.restricted { "Restricted" } else { "Everyone" },
                    if hidden { "hides" } else { "shows" }
                )
            })
        })
        .collect();
    assert!(wrong.is_empty(), "access drift: {}", wrong.join("; "));
}

#[test]
fn the_ordinary_socket_sees_no_global_the_privileged_one_does_not() {
    let (privileged, ordinary) = registries();
    let extra: Vec<&String> = ordinary
        .keys()
        .filter(|interface| !privileged.contains_key(*interface))
        .collect();
    assert!(
        extra.is_empty(),
        "the privileged socket must see everything the ordinary one does: {extra:?}"
    );
}

#[test]
fn every_protocol_crate_global_is_advertised_at_the_version_it_declares() {
    let (privileged, _) = registries();
    let mut wrong = Vec::new();
    for protocol in gnoblin_wayland_protocols::ALL {
        for interface in protocol.interfaces {
            match privileged.get(interface.name) {
                Some(version) if *version == interface.version => {}
                Some(version) => wrong.push(format!(
                    "{} ({}): declares {}, advertises {version}",
                    interface.name, protocol.name, interface.version
                )),
                None => wrong.push(format!(
                    "{} ({}): declared but not advertised",
                    interface.name, protocol.name
                )),
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "a protocol crate and the registry disagree: {}",
        wrong.join("; ")
    );
}

#[test]
fn a_conditional_protocol_crate_global_is_never_advertised_above_its_declared_version() {
    let (privileged, _) = registries();
    let mut wrong = Vec::new();
    for protocol in gnoblin_wayland_protocols::CONDITIONAL {
        for interface in protocol.interfaces {
            if let Some(version) = privileged.get(interface.name)
                && *version > interface.version
            {
                wrong.push(format!(
                    "{} ({}): declares {}, advertises {version}",
                    interface.name, protocol.name, interface.version
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "a protocol crate and the registry disagree: {}",
        wrong.join("; ")
    );
}
