//! The generated `.proto` text and the compiled types must agree on every tag.
//!
//! The Rust types are generated from a descriptor set, and the text is the document
//! `build.rs` compiled to get it. This test parses the text on its own and compares
//! every message field and every enum value by its wire number with what the
//! descriptor says, so a renderer that produced something other than the tags the
//! types carry fails here rather than on a peer's socket.
//!
//! The text side is read from `OUT_DIR`, because that is what was compiled; the
//! envelope, the one hand-written document, is read from the crate's `proto/`
//! directory.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use prost::Message as _;
use prost_types::{DescriptorProto, EnumDescriptorProto, FieldDescriptorProto, FileDescriptorSet};

/// The documents `build.rs` generated from the schema and compiled.
fn generated_dir() -> PathBuf {
    Path::new(env!("OUT_DIR")).join("proto")
}

/// The one hand-written document, which is not generated from the schema.
fn hand_written_proto() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("proto")
        .join("envelope.proto")
}

/// `package.Message.field` and `package.Enum.VALUE` to the wire number a compiled
/// type uses for it.
fn from_descriptor() -> BTreeMap<String, i32> {
    let bytes = include_bytes!(concat!(env!("OUT_DIR"), "/descriptor.bin"));
    let descriptors = FileDescriptorSet::decode(&bytes[..]).expect("the descriptor set written by build.rs");
    let mut numbers = BTreeMap::new();
    for file in &descriptors.file {
        let package = file.package();
        for message in &file.message_type {
            collect_message(message, package, &mut numbers);
        }
        for enum_type in &file.enum_type {
            collect_enum(enum_type, package, &mut numbers);
        }
    }
    numbers
}

fn collect_message(message: &DescriptorProto, prefix: &str, numbers: &mut BTreeMap<String, i32>) {
    let path = join(prefix, message.name());
    for field in &message.field {
        collect_field(field, &path, numbers);
    }
    for nested in &message.nested_type {
        collect_message(nested, &path, numbers);
    }
    for enum_type in &message.enum_type {
        collect_enum(enum_type, &path, numbers);
    }
}

fn collect_field(field: &FieldDescriptorProto, path: &str, numbers: &mut BTreeMap<String, i32>) {
    numbers.insert(format!("{path}.{}", field.name()), field.number());
}

fn collect_enum(enum_type: &EnumDescriptorProto, prefix: &str, numbers: &mut BTreeMap<String, i32>) {
    let path = join(prefix, enum_type.name());
    for value in &enum_type.value {
        numbers.insert(format!("{path}.{}", value.name()), value.number());
    }
}

fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_owned()
    } else {
        format!("{prefix}.{name}")
    }
}

/// The same map, read out of the text that was compiled.
fn from_text() -> BTreeMap<String, i32> {
    let mut paths = proto_paths(&generated_dir());
    assert!(
        !paths.is_empty(),
        "{} holds no generated .proto file: build.rs did not run",
        generated_dir().display()
    );
    paths.push(hand_written_proto());
    paths.sort();

    let mut numbers = BTreeMap::new();
    for path in &paths {
        let text = fs::read_to_string(path).unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        parse_proto(&text, &mut numbers);
    }
    numbers
}

/// Every `.proto` file directly in `dir`.
fn proto_paths(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", dir.display()))
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("proto"))
        .collect()
}

/// One block stack level: the message or enum name, or nothing for a `oneof` or a
/// `service`, whose braces nest without adding a name level.
fn parse_proto(text: &str, numbers: &mut BTreeMap<String, i32>) {
    let mut package = String::new();
    let mut stack: Vec<Option<String>> = Vec::new();

    for raw_line in text.lines() {
        let line = match raw_line.find("//") {
            Some(comment) => &raw_line[..comment],
            None => raw_line,
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("package ") {
            package = rest.trim_end_matches(';').trim().to_owned();
            continue;
        }
        if line.ends_with('{') {
            let mut words = line.trim_end_matches('{').split_whitespace();
            let kind = words.next().unwrap_or_default();
            let name = words.next().unwrap_or_default();
            stack.push(match kind {
                "message" | "enum" => Some(name.to_owned()),
                _ => None,
            });
            continue;
        }
        if line == "}" {
            stack.pop();
            continue;
        }
        let Some(body) = line.strip_suffix(';') else {
            continue;
        };
        // `syntax = "proto3"` and options are not members and do not parse as a
        // name and a number.
        let Some((left, right)) = body.rsplit_once('=') else {
            continue;
        };
        let Ok(number) = right.trim().parse::<i32>() else {
            continue;
        };
        let Some(name) = left.split_whitespace().next_back() else {
            continue;
        };
        let path = path_of(&package, &stack);
        if path.is_empty() {
            continue;
        }
        numbers.insert(format!("{path}.{name}"), number);
    }
}

fn path_of(package: &str, stack: &[Option<String>]) -> String {
    let mut path = package.to_owned();
    for name in stack.iter().flatten() {
        path = join(&path, name);
    }
    path
}

#[test]
fn compiled_tags_match_the_generated_proto() {
    let compiled = from_descriptor();
    let text = from_text();
    assert!(!compiled.is_empty(), "the descriptor set names no members");
    assert_eq!(
        compiled, text,
        "the compiled descriptor and the generated .proto text disagree about a tag"
    );
}

#[test]
fn generated_types_use_the_tags_the_proto_states() {
    // The descriptor test above proves the text matches the compile input; these
    // probes prove the generated Rust really carries those numbers, which is what
    // an old client decodes.
    use crate::bsm_greeter::greeter::AuthResponseArgs;
    use crate::gnoblin::window::Properties;

    let encoded = AuthResponseArgs {
        response: "x".to_owned(),
    }
    .encode_to_vec();
    // Field 1, wire type 2 (length-delimited).
    assert_eq!(encoded.first(), Some(&0x0A), "auth_response.response is tag 1");

    let encoded = Properties {
        id: 1,
        ..Properties::default()
    }
    .encode_to_vec();
    // Field 1, wire type 0 (varint).
    assert_eq!(encoded.first(), Some(&0x08), "window.Properties.id is tag 1");

    let encoded = Properties {
        focused: true,
        ..Properties::default()
    }
    .encode_to_vec();
    // Field 4, wire type 0 (varint).
    assert_eq!(encoded.first(), Some(&0x20), "window.Properties.focused is tag 4");
}
