//! The three sections that are lists of names: the protocol toggles, the input
//! sources and the frame renderers.
//!
//! Each of them names something the compositor must be able to look up, so each
//! name is checked against the set this build knows. A protocol name that does
//! not exist is a configuration error rather than a warning: a reload that says
//! "unknown protocol" and succeeds leaves the user believing a capture protocol
//! is disabled when it is not.

use std::collections::BTreeMap;

use crate::enums::{InputSourceType, Protocol};
use crate::fields::{Parser, as_table, flag, list, require, setting, strings, text, unknown_keys};
use crate::path::Path;
use crate::problem::Fault;
use crate::value::Value;

/// `protocols`: which Wayland protocols are advertised to new clients.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Protocols {
    /// The protocols the document sets, in name order. Absent protocols keep
    /// their default.
    pub toggles: BTreeMap<Protocol, bool>,
}

/// `input-sources`: the switchable keyboard layouts and input methods.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InputSources {
    /// The sources, in switching order.
    pub sources: Vec<InputSource>,
    /// Remember a different source per window.
    pub per_window: Option<bool>,
}

/// One input source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputSource {
    /// Whether this is an XKB layout or an IBus engine.
    pub kind: InputSourceType,
    /// The layout or engine id, which must be nonempty.
    pub id: String,
}

/// `frame-renderers`: the named frame renderer commands.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FrameRenderers {
    /// Each renderer's argv, in name order.
    pub renderers: BTreeMap<String, Vec<String>>,
}

/// The keys `input-sources` accepts.
const INPUT_SOURCES_KEYS: &[&str] = &["per-window", "sources"];

/// The keys an input source entry accepts.
const INPUT_SOURCE_KEYS: &[&str] = &["id", "type"];

/// The longest frame renderer name, in bytes.
const MAX_RENDERER_NAME_LENGTH: usize = 64;

/// The most arguments a frame renderer command may hold.
const MAX_RENDERER_ARGUMENTS: usize = 32;

/// Parse the `protocols` table.
pub(crate) fn parse_protocols(map: &mut BTreeMap<String, Value>, path: &Path, out: &mut Parser) -> Protocols {
    // The keys are protocol names, not settings, so the accepted-values hint is
    // the protocol list itself and every other key is reported against it.
    let mut toggles: BTreeMap<Protocol, bool> = BTreeMap::new();
    let keys: Vec<String> = map.keys().cloned().collect();
    for key in keys {
        let entry_path = path.key(&key);
        let Some(protocol) = Protocol::parse(&key) else {
            out.problem(
                &entry_path,
                Fault::UnknownKey {
                    accepted: Protocol::VARIANT_TEXTS,
                },
            );
            continue;
        };
        if let Some(value) = flag(map, &key, path, out) {
            toggles.insert(protocol, value);
        }
    }
    Protocols { toggles }
}

/// Parse the `frame-renderers` table.
pub(crate) fn parse_frame_renderers(
    map: &mut BTreeMap<String, Value>,
    path: &Path,
    out: &mut Parser,
) -> FrameRenderers {
    let names: Vec<String> = map.keys().cloned().collect();
    let mut renderers: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for name in names {
        let entry_path = path.key(&name);
        if !renderer_name_shaped(&name) {
            out.problem(
                &entry_path,
                Fault::UnsupportedValue {
                    expected: "a name of at most 64 letters, digits, underscores or dashes, other than native"
                        .to_owned(),
                },
            );
            continue;
        }
        let Some(arguments) = strings(map, &name, path, out) else {
            continue;
        };
        let first_present = arguments.first().is_some_and(|executable| !executable.is_empty());
        if arguments.is_empty() || arguments.len() > MAX_RENDERER_ARGUMENTS || !first_present {
            out.problem(
                &entry_path,
                Fault::UnsupportedValue {
                    expected: format!(
                        "an argv of 1 to {MAX_RENDERER_ARGUMENTS} strings whose first entry names the program"
                    ),
                },
            );
            continue;
        }
        renderers.insert(name, arguments);
    }
    FrameRenderers { renderers }
}

/// Whether a frame renderer name is usable and does not shadow the built-in one.
fn renderer_name_shaped(name: &str) -> bool {
    let mut characters = name.chars();
    characters.next().is_some_and(|first| first.is_ascii_alphanumeric())
        && name.len() <= MAX_RENDERER_NAME_LENGTH
        && name != "native"
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_' || character == '-')
}

/// Parse the `input-sources` table.
pub(crate) fn parse_input_sources(map: &mut BTreeMap<String, Value>, path: &Path, out: &mut Parser) -> InputSources {
    unknown_keys(map, path, out, INPUT_SOURCES_KEYS);
    let per_window = flag(map, "per-window", path, out);
    let sources = if require(map, "sources", path, out, "a list of input sources") {
        parse_sources(map, path, out)
    } else {
        Vec::new()
    };
    InputSources { sources, per_window }
}

/// Parse the `input-sources.sources` list.
fn parse_sources(map: &mut BTreeMap<String, Value>, path: &Path, out: &mut Parser) -> Vec<InputSource> {
    let Some(items) = list(map, "sources", path, out, "a list of input source tables") else {
        return Vec::new();
    };
    let list_path = path.key("sources");
    let mut sources: Vec<InputSource> = Vec::with_capacity(items.len());
    for (index, item) in items.iter_mut().enumerate() {
        let entry_path = list_path.entry(index);
        let Some(entry) = as_table(item, &entry_path, out, "an input source table") else {
            continue;
        };
        unknown_keys(entry, &entry_path, out, INPUT_SOURCE_KEYS);
        let kind = if require(entry, "type", &entry_path, out, "xkb or ibus") {
            setting::<InputSourceType>(entry, "type", "input-source-type", &entry_path, out)
        } else {
            None
        };
        let id = if require(entry, "id", &entry_path, out, "a layout or engine id") {
            text(entry, "id", &entry_path, out).filter(|id| {
                let present = !id.is_empty();
                if !present {
                    out.problem(
                        &entry_path.key("id"),
                        Fault::UnsupportedValue {
                            expected: "a nonempty layout or engine id".to_owned(),
                        },
                    );
                }
                present
            })
        } else {
            None
        };
        if let (Some(kind), Some(id)) = (kind, id) {
            sources.push(InputSource { kind, id });
        }
    }
    sources
}
