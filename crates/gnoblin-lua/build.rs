//! Generate the Lua bindings and `gnoblin.d.lua` from the object schema.
//!
//! The schema is the single source of truth for objects, properties, methods and
//! signals (design/rust-compositor.md, "One IPC" and "Contracts are data"), so the
//! Lua surface is generated from it at build time and never written by hand. A
//! member exists in Lua because a schema file declares it, which is what keeps the
//! runtime bindings, the type stubs and the IPC from drifting into three dialects.
//!
//! Two files are written into `OUT_DIR`, which is inside the gitignored `target/`:
//!
//! - `bindings.rs`, `include!`d by `src/bindings.rs`: one static per namespace
//!   with its objects, read-only properties, methods and signals.
//! - `gnoblin.d.lua`, the LuaCATS stub `src/bindings.rs` embeds with `include_str!`
//!   and [`crate::write_stub`] copies out for an editor. `tests/bindings.rs`
//!   re-reads the schema and checks both against it, so a generator that dropped a
//!   member fails a test instead of shipping.
//!
//! Nothing generated is committed: a clean checkout builds with plain
//! `cargo build`, and a schema edit is a rebuild rather than a regeneration step
//! somebody has to remember.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use gnoblin_schema::{Catalog, Field, Namespace, ObjectType, Type};

/// The schema directory, relative to this crate.
const SCHEMA_DIR: &str = "../gnoblin-schema/schema";

/// The namespace the compositor itself serves.
const OWN_NAMESPACE: &str = "gnoblin";

/// Member names the bindings already use on every instance.
///
/// A schema that declared one of these would shadow it, so the generator refuses
/// the schema instead of producing a binding a script cannot reach.
const RESERVED: &[&str] = &["on"];

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("[lua] {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let schema_dir = manifest.join(SCHEMA_DIR);
    let out_dir = std::env::var_os("OUT_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| "OUT_DIR is not set".to_owned())?;

    // Every schema file by name, and the directory, because a file appearing or
    // disappearing changes the directory and not the files already inside it.
    watch(&schema_dir)?;

    let catalog = Catalog::load_dir(&schema_dir)
        .map_err(|error| format!("cannot load the schema in {}: {error}", schema_dir.display()))?;

    check_reserved(&catalog)?;
    // The version the runtime advertises in the IPC handshake is the schema's, which is
    // the version the compositor serves: the two sides are built from one tree, so a
    // literal here would be a second place to forget to bump.
    let namespace =
        Namespace::parse(OWN_NAMESPACE).map_err(|reason| format!("namespace `{OWN_NAMESPACE}`: {reason}"))?;
    let version = catalog.version(&namespace).ok_or_else(|| {
        format!("the schema declares no `{OWN_NAMESPACE}` namespace, so there is no IPC version to advertise")
    })?;
    println!("cargo:rustc-env=GNOBLIN_IPC_VERSION={version}");
    fs::write(out_dir.join("bindings.rs"), rust_bindings(&catalog))
        .map_err(|error| format!("cannot write bindings.rs: {error}"))?;
    fs::write(out_dir.join("gnoblin.d.lua"), lua_stub(&catalog))
        .map_err(|error| format!("cannot write gnoblin.d.lua: {error}"))?;
    Ok(())
}

/// Refuse a schema whose member names collide with the bindings.
fn check_reserved(catalog: &Catalog) -> Result<(), String> {
    for namespace in catalog.namespaces() {
        for object in catalog.objects(&namespace) {
            let instance_members = object
                .properties
                .iter()
                .map(|property| property.name.as_str())
                .chain(object.methods.iter().map(|method| method.name.as_str()))
                .chain(object.signals.iter().map(|signal| signal.name.as_str()));
            for member in instance_members {
                if RESERVED.contains(&member) {
                    return Err(format!(
                        "{}.{} declares `{member}`, which every instance already has",
                        namespace.as_str(),
                        object.name
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Ask cargo to rerun when any schema file changes.
fn watch(dir: &Path) -> Result<(), String> {
    println!("cargo:rerun-if-changed={}", dir.display());
    let entries = fs::read_dir(dir).map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
        if entry.path().extension().and_then(|extension| extension.to_str()) == Some("toml") {
            println!("cargo:rerun-if-changed={}", entry.path().display());
        }
    }
    Ok(())
}

/// The Rust table of namespaces the binding installer reads.
///
/// Members are emitted in schema declaration order and namespaces in sorted order,
/// so two builds of one schema produce the same bytes and the drift test can
/// compare member by member.
fn rust_bindings(catalog: &Catalog) -> String {
    let mut text = String::new();
    text.push_str("// Generated from `crates/gnoblin-schema/schema` by `build.rs`. Do not edit.\n");
    text.push_str("//\n");
    text.push_str("// `bindings.rs` in this crate turns it into the Lua tables; `tests/bindings.rs`\n");
    text.push_str("// checks every member against the schema it was generated from.\n\n");
    text.push_str("pub(super) static NAMESPACES: &[NamespaceSpec] = &[\n");
    for namespace in catalog.namespaces() {
        let _ = writeln!(
            text,
            "    NamespaceSpec {{ name: {}, objects: &[",
            text_literal(namespace.as_str())
        );
        for object in catalog.objects(&namespace) {
            let _ = writeln!(
                text,
                "        ObjectSpec {{ name: {}, properties: &[",
                text_literal(&object.name)
            );
            for property in &object.properties {
                let _ = writeln!(
                    text,
                    "            MemberSpec {{ name: {}, ty: {} }},",
                    text_literal(&property.name),
                    rust_type(&property.ty)
                );
            }
            text.push_str("        ], methods: &[\n");
            for method in &object.methods {
                let _ = writeln!(
                    text,
                    "            MethodSpec {{ name: {}, params: &[",
                    text_literal(&method.name)
                );
                for parameter in &method.params {
                    let _ = writeln!(
                        text,
                        "                MemberSpec {{ name: {}, ty: {} }},",
                        text_literal(&parameter.name),
                        rust_type(&parameter.ty)
                    );
                }
                text.push_str("            ] },\n");
            }
            text.push_str("        ], signals: &[\n");
            for signal in &object.signals {
                let _ = writeln!(
                    text,
                    "            SignalSpec {{ name: {}, fields: &[",
                    text_literal(&signal.name)
                );
                for field in &signal.fields {
                    let _ = writeln!(
                        text,
                        "                MemberSpec {{ name: {}, ty: {} }},",
                        text_literal(&field.name),
                        rust_type(&field.ty)
                    );
                }
                text.push_str("            ] },\n");
            }
            text.push_str("        ] },\n");
        }
        text.push_str("    ] },\n");
    }
    text.push_str("];\n");
    text
}

/// One schema type as the `Ty` the bindings carry.
fn rust_type(ty: &Type) -> String {
    match ty {
        Type::Bool => "Ty::Bool".to_owned(),
        Type::Int => "Ty::Int".to_owned(),
        Type::Uint => "Ty::Uint".to_owned(),
        Type::Double => "Ty::Double".to_owned(),
        Type::Text => "Ty::Text".to_owned(),
        Type::Bytes => "Ty::Bytes".to_owned(),
        Type::ObjectRef(name) => format!("Ty::Object({})", text_literal(name)),
        // A promoted constant, so a list needs no allocation in a static.
        Type::List(inner) => format!("Ty::List(&{})", rust_type(inner)),
    }
}

/// A Rust string literal with the escapes a generated file needs.
fn text_literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// The LuaCATS stub for an editor: the same members, spelled for Lua.
fn lua_stub(catalog: &Catalog) -> String {
    let mut text = String::new();
    text.push_str("---@meta\n\n");
    text.push_str("--- The Gnoblin Lua API, generated from `crates/gnoblin-schema/schema` by\n");
    text.push_str("--- `crates/gnoblin-lua/build.rs`. Do not edit: declare a member in the schema\n");
    text.push_str("--- and rebuild.\n\n");
    text.push_str("--- A method call returns an `Operation`; its single completion arrives on the\n");
    text.push_str("--- host's next pump, so a handler never runs inside another handler.\n");
    text.push_str("---@class gnoblin.Operation\n");
    text.push_str("---@field id integer\n");
    text.push_str("---@field method string\n");
    text.push_str("---@field status \"pending\"|\"succeeded\"|\"failed\"\n");
    text.push_str("---@field code string|nil the schema error code after a failure\n\n");
    text.push_str("--- One signal handler registration.\n");
    text.push_str("---@class gnoblin.Subscription\n");
    text.push_str("local Subscription = {}\n");
    text.push_str("--- Stop delivering the signal.\n");
    text.push_str("function Subscription:off() end\n\n");
    text.push_str("--- One timer on the caller's clock.\n");
    text.push_str("---@class gnoblin.Timer\n");
    text.push_str("local Timer = {}\n");
    text.push_str("--- Cancel the timer.\n");
    text.push_str("function Timer:cancel() end\n\n");
    text.push_str("--- Declare configuration. Validated immediately; every problem is raised.\n");
    text.push_str("---@param settings table\n");
    text.push_str("function gnoblin.configure(settings) end\n\n");
    text.push_str("--- Evaluate another configuration file, relative to the calling file.\n");
    text.push_str("---@param path_or_glob string\n");
    text.push_str("---@return boolean\n");
    text.push_str("function gnoblin.load(path_or_glob) end\n\n");
    text.push_str("--- Timers run on the caller's tick, never on a clock of their own.\n");
    text.push_str("---@param ticks integer\n");
    text.push_str("---@param callback fun()\n");
    text.push_str("---@return gnoblin.Timer\n");
    text.push_str("function gnoblin.timers.after(ticks, callback) end\n");
    text.push_str("---@return integer\n");
    text.push_str("function gnoblin.timers.now() end\n");

    for namespace in catalog.namespaces() {
        for object in catalog.objects(&namespace) {
            stub_object(&mut text, &namespace, object);
        }
    }
    text
}

/// One object's class, properties, methods and signals.
fn stub_object(text: &mut String, namespace: &Namespace, object: &ObjectType) {
    let class = format!("{}.{}", namespace.as_str(), object.name);
    let _ = writeln!(text, "\n---@class {class}");
    for property in &object.properties {
        let _ = writeln!(text, "---@field {} {}", property.name, lua_type(&property.ty));
    }
    let _ = writeln!(text, "local {} = {{}}", object.name);
    for method in &object.methods {
        for parameter in &method.params {
            let _ = writeln!(text, "---@param {} {}", parameter.name, lua_type(&parameter.ty));
        }
        let _ = writeln!(text, "---@return gnoblin.Operation");
        let _ = writeln!(
            text,
            "function {}:{}({}) end",
            object.name,
            method.name,
            parameter_list(&method.params)
        );
    }
    for signal in &object.signals {
        let _ = writeln!(text, "---@param name \"{}\"", signal.name);
        let _ = writeln!(
            text,
            "---@param callback fun(fields: {})",
            field_table(signal.fields.as_slice())
        );
        let _ = writeln!(text, "---@return gnoblin.Subscription");
        let _ = writeln!(text, "function {}:on(name, callback) end", object.name);
    }
    // The collection a script reaches an instance through.
    let collection = if namespace.as_str() == OWN_NAMESPACE {
        format!("gnoblin.objects.{}", object.name)
    } else {
        format!("gnoblin.extensions[\"{}\"].objects.{}", namespace.as_str(), object.name)
    };
    let _ = writeln!(text, "---@class {collection}");
    let _ = writeln!(text, "---@field by_id fun(id: integer): {class}|nil");
    let _ = writeln!(text, "---@field all fun(): {class}[]");
}

/// The parameter list of a stub signature.
fn parameter_list(params: &[Field]) -> String {
    params
        .iter()
        .map(|parameter| parameter.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// A table literal type for a signal's fields, so `:on` says what it receives.
fn field_table(fields: &[Field]) -> String {
    if fields.is_empty() {
        return "table".to_owned();
    }
    let members: Vec<String> = fields
        .iter()
        .map(|field| format!("{}: {}", field.name, lua_type(&field.ty)))
        .collect();
    format!("{{ {} }}", members.join(", "))
}

/// One schema type as LuaCATS spells it.
fn lua_type(ty: &Type) -> String {
    match ty {
        Type::Bool => "boolean".to_owned(),
        Type::Int | Type::Uint => "integer".to_owned(),
        Type::Double => "number".to_owned(),
        Type::Text | Type::Bytes => "string".to_owned(),
        Type::ObjectRef(name) => format!("gnoblin.{name}"),
        Type::List(inner) => format!("{}[]", lua_type(inner)),
    }
}
