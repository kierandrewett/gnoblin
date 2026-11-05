//! The Protocol Buffers text the IPC wire is compiled from.
//!
//! The schema is the source of truth and the `.proto` text is build input
//! (design/rust-compositor.md, "One IPC" and D16). `gnoblin-proto`'s build script
//! renders this text into `OUT_DIR` and compiles it there, so nothing generated is
//! checked in and a clean checkout builds with a plain `cargo build`.
//!
//! The renderer lives beside the model rather than in the crate that compiles it
//! because the schema is its only input, and because `cargo xtask gen-proto`
//! writes the same text to a readable directory: one implementation is what keeps
//! the copy a reviewer opens and the copy the build compiles the same bytes.
//!
//! One output file per namespace, because a Protocol Buffers package is a
//! namespace and two files with one package may not declare the same message. All
//! files that declare a namespace therefore merge into one document, in schema
//! load order, which is file-name order.
//!
//! [`sensitive_table`] renders the Rust table of members the schema marks
//! `secret`, so sensitivity is decided in one place and the envelope cannot be
//! built or decoded with the wrong flag.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;

use crate::model::{Catalog, ErrorDecl, Field, Namespace, ObjectType, Type};
use gnoblin_types::Version;

/// Path of the schema files, relative to the workspace root.
///
/// It appears in every generated header, so a reader of the generated text can
/// find the file that decided it. Build scripts resolve it against
/// `CARGO_MANIFEST_DIR`; `cargo xtask gen-proto` resolves it against the
/// workspace root.
pub const SCHEMA_DIR: &str = "crates/gnoblin-schema/schema";

/// One generated `.proto` document: its file name and its text.
///
/// The name is relative, because a build script writes it under `OUT_DIR` and
/// `cargo xtask gen-proto` writes it under `target/`.
pub fn proto_files(catalog: &Catalog) -> Vec<(String, String)> {
    let mut files = Vec::new();
    for namespace in catalog.namespaces() {
        let Some(version) = catalog.version(&namespace) else {
            continue;
        };
        let name = format!("{}.proto", namespace.as_str().replace('.', "_"));
        files.push((name, render(catalog, &namespace, version)));
    }
    files
}

/// The name a frame's `member` field carries: the schema's qualified member path.
///
/// A member name alone is ambiguous across objects, and the envelope carries one
/// namespace and one member, so a member is named by its object type and its name,
/// `request.answer`. That is also the spelling a peer writes in its own schema, so an
/// extension's frames and the compositor's are addressed the same way.
///
/// The generated sensitive table is keyed by this path, which is what lets a layer
/// that *is* an exchange (the request layer) be declared in the schema at all: the
/// frame that carries a secret answer is addressed exactly as the schema addresses
/// it, so the flag is set without the client knowing which members are secret.
#[must_use]
pub fn member_path(object: &str, member: &str) -> String {
    format!("{object}.{member}")
}

/// The Rust table the envelope builders and the frame decoder read.
///
/// It is generated rather than hand-kept so the `secret` flag in the schema is the
/// only place sensitivity is decided: a member marked secret is flagged on every
/// constructor and on decode, and cannot be unflagged by a caller.
///
/// The text carries no inner doc comments, because it is `include!`d inside a
/// module rather than compiled as one.
pub fn sensitive_table(catalog: &Catalog) -> String {
    // Sorted, so the table does not move when a schema file is reordered.
    let mut members: BTreeSet<(String, String)> = BTreeSet::new();
    for namespace in catalog.namespaces() {
        for object in catalog.objects(&namespace) {
            for method in &object.methods {
                if method.is_sensitive() || object.secret {
                    members.insert((namespace.to_string(), member_path(&object.name, &method.name)));
                }
            }
            for signal in &object.signals {
                if signal.is_sensitive() || object.secret {
                    members.insert((namespace.to_string(), member_path(&object.name, &signal.name)));
                }
            }
        }
    }

    let mut text = String::new();
    let _ = writeln!(
        text,
        "// Generated from the object schema by gnoblin-proto's build script. Do not"
    );
    let _ = writeln!(text, "// edit: change the schema instead.");
    let _ = writeln!(text, "//");
    let _ = writeln!(
        text,
        "// Members whose frames always carry secret material. The schema marks a member,"
    );
    let _ = writeln!(
        text,
        "// or one of its fields, `secret`; the envelope builders set `FLAG_SENSITIVE` from"
    );
    let _ = writeln!(
        text,
        "// this table and the frame decoder re-applies it, so a secret frame is flagged"
    );
    let _ = writeln!(
        text,
        "// whether or not the caller remembered, and a peer cannot clear the flag by"
    );
    let _ = writeln!(text, "// sending zero.");
    let _ = writeln!(text);
    let _ = writeln!(
        text,
        "/// Every `(namespace, member path)` pair the schema marks secret."
    );
    let _ = writeln!(text, "///");
    let _ = writeln!(
        text,
        "/// Sorted, so the generated table is stable. The layout is fixed on purpose,"
    );
    let _ = writeln!(text, "/// because this table is generated and must not be reformatted.");
    let _ = writeln!(text, "#[rustfmt::skip]");
    if members.is_empty() {
        let _ = writeln!(text, "pub(crate) const SENSITIVE_MEMBERS: &[(&str, &str)] = &[];");
    } else {
        let _ = writeln!(text, "pub(crate) const SENSITIVE_MEMBERS: &[(&str, &str)] = &[");
        for (namespace, member) in &members {
            let _ = writeln!(text, "    (\"{namespace}\", \"{member}\"),");
        }
        let _ = writeln!(text, "];");
    }
    let _ = writeln!(text);
    let _ = writeln!(
        text,
        "/// Whether every frame for this namespace and member is sensitive."
    );
    let _ = writeln!(
        text,
        "pub(crate) fn is_sensitive(namespace: &str, member: &str) -> bool {{"
    );
    let _ = writeln!(text, "    SENSITIVE_MEMBERS.contains(&(namespace, member))");
    let _ = writeln!(text, "}}");
    text
}

/// The text of one namespace's `.proto` file.
fn render(catalog: &Catalog, namespace: &Namespace, version: Version) -> String {
    let mut text = String::new();
    let _ = writeln!(
        text,
        "// Generated from the object schema by gnoblin-proto's build script. Do not"
    );
    let _ = writeln!(text, "// edit: change the schema instead.");
    let _ = writeln!(text, "//");
    let _ = writeln!(text, "// Namespace: {namespace}");
    let _ = writeln!(text, "// Version: {version}");
    // File names, not the path they were loaded through, so the text does not
    // depend on which directory the generator read the schema from.
    let origins: Vec<String> = catalog
        .origins(namespace)
        .map(|origin| {
            Path::new(origin)
                .file_name()
                .map_or_else(|| origin.to_owned(), |name| name.to_string_lossy().into_owned())
        })
        .collect();
    let _ = writeln!(text, "// Schema: {SCHEMA_DIR} ({})", origins.join(", "));
    let _ = writeln!(text);
    let _ = writeln!(text, "syntax = \"proto3\";");
    let _ = writeln!(text);
    let _ = writeln!(text, "package {namespace};");

    for object in catalog.objects(namespace) {
        let _ = writeln!(text);
        render_object(&mut text, object);
    }

    let errors: Vec<&ErrorDecl> = catalog.errors(namespace).collect();
    if !errors.is_empty() {
        let _ = writeln!(text);
        render_errors(&mut text, namespace, &errors);
    }
    text
}

fn render_object(text: &mut String, object: &ObjectType) {
    let secret = if object.secret { ", secret" } else { "" };
    let _ = writeln!(text, "// Object `{}`, since {}{secret}.", object.name, object.since);
    let _ = writeln!(text, "message {} {{", to_upper_camel(&object.name));

    if object.properties.is_empty() {
        let _ = writeln!(text, "  message Properties {{}}");
    } else {
        let _ = writeln!(
            text,
            "  // Property values, carried by a property diff. Field numbers never change."
        );
        let _ = writeln!(text, "  message Properties {{");
        for (index, property) in object.properties.iter().enumerate() {
            let mut notes = vec![format!("since {}", property.since)];
            if property.secret {
                notes.push(SECRET_NOTE.to_owned());
            }
            render_field(text, "    ", &property.name, index + 1, &property.ty, &notes);
        }
        let _ = writeln!(text, "  }}");
    }

    for method in &object.methods {
        let secret = if method.secret { ", secret" } else { "" };
        let _ = writeln!(text);
        let _ = writeln!(text, "  // Method `{}`, since {}{secret}.", method.name, method.since);
        let name = format!("{}Args", to_upper_camel(&method.name));
        render_message(text, &name, &method.params, "  ");
    }

    for signal in &object.signals {
        let secret = if signal.secret { ", secret" } else { "" };
        let _ = writeln!(text);
        let _ = writeln!(text, "  // Signal `{}`, since {}{secret}.", signal.name, signal.since);
        let name = format!("{}Payload", to_upper_camel(&signal.name));
        render_message(text, &name, &signal.fields, "  ");
    }

    let _ = writeln!(text, "}}");
}

/// Render a nested message with `fields`, or an empty one on a single line.
fn render_message(text: &mut String, name: &str, fields: &[Field], indent: &str) {
    if fields.is_empty() {
        let _ = writeln!(text, "{indent}message {name} {{}}");
        return;
    }
    let _ = writeln!(text, "{indent}message {name} {{");
    let field_indent = format!("{indent}  ");
    for (index, field) in fields.iter().enumerate() {
        let mut notes = vec![format!("since {}", field.since)];
        if field.secret {
            notes.push(SECRET_NOTE.to_owned());
        }
        render_field(text, &field_indent, &field.name, index + 1, &field.ty, &notes);
    }
    let _ = writeln!(text, "{indent}}}");
}

fn render_field(text: &mut String, indent: &str, name: &str, tag: usize, ty: &Type, notes: &[String]) {
    let (label, proto_type, note) = proto_type(ty);
    let mut notes = notes.to_vec();
    if let Some(note) = note {
        notes.push(note);
    }
    let _ = writeln!(text, "{indent}// {}", notes.join("; "));
    match label {
        Some(label) => {
            let _ = writeln!(text, "{indent}{label} {proto_type} {name} = {tag};");
        }
        None => {
            let _ = writeln!(text, "{indent}{proto_type} {name} = {tag};");
        }
    }
}

const SECRET_NOTE: &str = "secret: every frame carrying this field is flagged sensitive";

/// The Protocol Buffers label, type and extra note for a schema type.
fn proto_type(ty: &Type) -> (Option<&'static str>, String, Option<String>) {
    match ty {
        Type::Bool => (None, "bool".to_owned(), None),
        Type::Int => (None, "int64".to_owned(), None),
        Type::Uint => (None, "uint64".to_owned(), None),
        Type::Double => (None, "double".to_owned(), None),
        Type::Text => (None, "string".to_owned(), None),
        Type::Bytes => (None, "bytes".to_owned(), None),
        // An object identity is a string here: the wire carries the id and the
        // schema carries the type, so a reader without the schema still knows it
        // is an identity rather than a title.
        Type::ObjectRef(name) => (None, "string".to_owned(), Some(format!("object id of `{name}`"))),
        Type::List(inner) => {
            let (_, proto_type, note) = proto_type(inner);
            (
                Some("repeated"),
                proto_type,
                Some(note.map_or_else(|| "list".to_owned(), |note| format!("list of {note}"))),
            )
        }
    }
}

fn render_errors(text: &mut String, namespace: &Namespace, errors: &[&ErrorDecl]) {
    let prefix = namespace
        .as_str()
        .split('.')
        .map(to_upper_snake)
        .collect::<Vec<String>>()
        .join("_");
    let name = namespace
        .as_str()
        .split('.')
        .map(to_upper_camel)
        .collect::<Vec<String>>()
        .concat();
    let _ = writeln!(
        text,
        "// Error codes `{namespace}` returns, in the envelope's error frame."
    );
    let _ = writeln!(
        text,
        "// The wire carries the code as text, so a peer reports one it does not know."
    );
    let _ = writeln!(text, "enum {name}Error {{");
    let _ = writeln!(text, "  {prefix}_UNSPECIFIED = 0;");
    for (index, error) in errors.iter().enumerate() {
        let _ = writeln!(text, "  // since {}", error.since);
        let _ = writeln!(
            text,
            "  {prefix}_{} = {};",
            error.name.to_uppercase().replace('-', "_"),
            index + 1
        );
    }
    let _ = writeln!(text, "}}");
}

fn to_upper_camel(text: &str) -> String {
    text.split(['_', '-'])
        .map(|word| {
            let mut characters = word.chars();
            match characters.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + characters.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

fn to_upper_snake(text: &str) -> String {
    text.to_uppercase().replace('-', "_")
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{SCHEMA_DIR, proto_files, sensitive_table};
    use crate::{Catalog, Namespace, parse_schema};

    fn schema_dir() -> PathBuf {
        // [`SCHEMA_DIR`] is relative to the workspace root, which is two
        // directories above this crate, so the tests read the path the build uses
        // rather than a second spelling of it.
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join(SCHEMA_DIR)
    }

    fn catalog() -> Catalog {
        Catalog::load_dir(&schema_dir()).expect("schema")
    }

    /// A one-namespace catalog from schema text, for tests about the rules rather
    /// than about the files in the tree.
    fn catalog_from(text: &str) -> Catalog {
        let schema = parse_schema("test.toml", text).expect("schema parses");
        Catalog::new(vec![schema]).expect("catalog")
    }

    #[test]
    fn one_file_per_namespace() {
        let names: Vec<String> = proto_files(&catalog()).into_iter().map(|(name, _)| name).collect();
        assert_eq!(names, vec!["bsm_greeter.proto".to_owned(), "gnoblin.proto".to_owned()]);
    }

    #[test]
    fn generating_twice_is_byte_identical() {
        // The file-name order is fixed and the schema is read the same way each
        // time, so a rebuild must not produce a different tree: otherwise every
        // build would look like a change and a reviewer could not diff one.
        let catalog = catalog();
        assert_eq!(proto_files(&catalog), proto_files(&catalog));
        assert_eq!(sensitive_table(&catalog), sensitive_table(&catalog));
    }

    #[test]
    fn a_secret_field_makes_its_member_sensitive() {
        // The whole chain the build relies on, from the schema's `secret` flag to
        // the table the envelope reads: marking the field is the only edit needed.
        let with_secret = catalog_from(
            r#"
namespace = "test.secret"
version = "1.0.0"

[[object]]
name = "thing"
since = "1.0.0"

[[object.method]]
name = "unlock"
since = "1.0.0"

[[object.method.param]]
name = "passphrase"
type = "string"
since = "1.0.0"
secret = true

[[object.method]]
name = "poke"
since = "1.0.0"
"#,
        );
        let text = sensitive_table(&with_secret);
        assert!(text.contains("(\"test.secret\", \"thing.unlock\")"), "{text}");
        assert!(!text.contains("poke"), "{text}");
        assert!(text.contains("#[rustfmt::skip]"), "{text}");

        // A secret object marks every one of its members, not only the fields the
        // schema flagged.
        let secret_object = catalog_from(
            r#"
namespace = "test.secret"
version = "1.0.0"

[[object]]
name = "thing"
since = "1.0.0"
secret = true

[[object.method]]
name = "poke"
since = "1.0.0"

[[object.signal]]
name = "moved"
since = "1.0.0"
"#,
        );
        let text = sensitive_table(&secret_object);
        assert!(text.contains("(\"test.secret\", \"thing.poke\")"), "{text}");
        assert!(text.contains("(\"test.secret\", \"thing.moved\")"), "{text}");
    }

    #[test]
    fn the_sensitive_table_names_every_secret_member() {
        let text = sensitive_table(&catalog());
        assert!(text.contains("(\"bsm.greeter\", \"greeter.auth_response\")"), "{text}");
        // A member with no secret field is not listed.
        assert!(!text.contains("begin_auth"), "{text}");
        assert!(!text.contains("focus"), "{text}");
        assert!(text.contains("#[rustfmt::skip]"), "{text}");
    }

    #[test]
    fn a_secret_field_is_marked_in_the_proto_text() {
        let catalog = catalog();
        let greeter = NameSpace::of(&catalog, "bsm.greeter");
        let text = super::render(&catalog, &greeter, catalog.version(&greeter).expect("version"));

        assert!(text.contains("package bsm.greeter;"), "{text}");
        assert!(text.contains("message BeginAuthArgs {"), "{text}");
        assert!(text.contains("string user = 1;"), "{text}");
        assert!(
            text.contains("string response = 1;") && text.contains("secret: every frame"),
            "{text}"
        );
        assert!(text.contains("message PromptPayload {"), "{text}");
        assert!(text.contains("BSM_GREETER_PROVIDER_GONE = 1;"), "{text}");
    }

    #[test]
    fn renders_objects_methods_signals_and_errors() {
        let catalog = catalog();
        let gnoblin = NameSpace::of(&catalog, "gnoblin");
        let text = super::render(&catalog, &gnoblin, catalog.version(&gnoblin).expect("version"));

        assert!(text.contains("package gnoblin;"), "{text}");
        assert!(text.contains("message Session {"), "{text}");
        assert!(text.contains("string version = 1;"), "{text}");
        assert!(text.contains("message ReadyPayload {}"), "{text}");
        assert!(text.contains("message Window {"), "{text}");
        assert!(text.contains("uint64 id = 1;"), "{text}");
        assert!(text.contains("message FocusArgs {}"), "{text}");
        assert!(text.contains("message ClosedPayload {}"), "{text}");
        assert!(text.contains("enum GnoblinError {"), "{text}");
        assert!(text.contains("GNOBLIN_UNKNOWN_MEMBER = 1;"), "{text}");
    }

    /// Finding a namespace by name, so a test names the one it means.
    struct NameSpace;

    impl NameSpace {
        fn of(catalog: &Catalog, name: &str) -> Namespace {
            catalog
                .namespaces()
                .into_iter()
                .find(|namespace| namespace.as_str() == name)
                .expect("namespace")
        }
    }
}
