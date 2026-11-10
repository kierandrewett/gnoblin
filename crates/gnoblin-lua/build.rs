//! Generate the Lua bindings and `gnoblin.d.lua` from the object schema.
//!
//! The schema is the single source of truth for objects, properties, methods and
//! signals (design/rust-compositor.md, "One IPC" and "Contracts are data"), so the
//! Lua surface is generated from it at build time and never written by hand. A
//! member exists in Lua because a schema file declares it, which is what keeps the
//! runtime bindings, the type stubs and the IPC from drifting into three dialects.
//!
//! Four files are written into `OUT_DIR`, which is inside the gitignored `target/`:
//!
//! - `bindings.rs`, `include!`d by `src/bindings.rs`: one static per namespace
//!   with its objects, read-only properties, methods and signals.
//! - `host_members.rs`, `include!`d by `src/bindings.rs`: the names and tree of the
//!   host-only members `host_members.toml` declares, which the startup check compares
//!   against the hand-written installers.
//! - `gnoblin.d.lua`, the LuaCATS stub `src/bindings.rs` embeds with `include_str!`
//!   and [`crate::write_stub`] copies out for an editor. `tests/bindings.rs`
//!   re-reads the schema and checks both against it, so a generator that dropped a
//!   member fails a test instead of shipping.
//! - `derived_events.rs`, `include!`d by `src/derived.rs`: the events the host raises
//!   from a property change, which `derived_events.toml` declares. Each is also a
//!   signal of its object in the other three files.
//!
//! Nothing generated is committed: a clean checkout builds with plain
//! `cargo build`, and a schema edit is a rebuild rather than a regeneration step
//! somebody has to remember.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use gnoblin_schema::{Catalog, DefaultValue, Field, Namespace, ObjectType, Property, Type, derived};

use serde::Deserialize;

include!("lua_names.rs");

/// The schema directory, relative to this crate.
const SCHEMA_DIR: &str = "../gnoblin-schema/schema";

/// The host-only members, declared in the crate's manifest.
const HOST_MEMBERS_FILE: &str = "host_members.toml";

/// The events the host raises from a property change, declared in the crate's manifest.
const DERIVED_FILE: &str = "derived_events.toml";

/// The schema subdirectory served only on the owner socket, never to Lua.
const OWNER_ONLY_DIR: &str = "tree";

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

    // The tree is owner-socket only (design/node-core.md section 2.3): no Lua module binds it,
    // so it stays out of the bindings, the stub and the stock files alike.
    let catalog = Catalog::load_dir_except(&schema_dir, &[OWNER_ONLY_DIR])
        .map_err(|error| format!("cannot load the schema in {}: {error}", schema_dir.display()))?;
    // A derived event is a signal of its object, so the catalog the bindings, the stub
    // and the drift test read must include it.
    let derived_path = manifest.join(DERIVED_FILE);
    println!("cargo:rerun-if-changed={}", derived_path.display());
    let derived_events =
        derived::load(&derived_path).map_err(|error| format!("cannot load the derived events: {error}"))?;
    let catalog = catalog
        .with_derived(&derived_events)
        .map_err(|error| format!("cannot add the derived events to the schema: {error}"))?;

    check_reserved(&catalog)?;
    let host = load_host_members(&manifest.join(HOST_MEMBERS_FILE))?;
    check_names(&catalog, &host)?;
    check_param_names(&catalog)?;
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
    fs::write(out_dir.join("host_members.rs"), rust_host_members(&host))
        .map_err(|error| format!("cannot write host_members.rs: {error}"))?;
    fs::write(out_dir.join("gnoblin.d.lua"), lua_stub(&catalog, &host))
        .map_err(|error| format!("cannot write gnoblin.d.lua: {error}"))?;
    fs::write(
        out_dir.join("stock_files.rs"),
        rust_stock_files(&schema_files(&schema_dir)?),
    )
    .map_err(|error| format!("cannot write stock_files.rs: {error}"))?;
    fs::write(out_dir.join("derived_events.rs"), rust_derived_events(&derived_events))
        .map_err(|error| format!("cannot write derived_events.rs: {error}"))?;
    // The tree classes come from the full catalog: the owner-only namespace is excluded above,
    // and the shell's table is the one generated place it is read.
    let full = Catalog::load_dir(&schema_dir)
        .map_err(|error| format!("cannot load the schema in {}: {error}", schema_dir.display()))?;
    fs::write(out_dir.join("tree_classes.rs"), rust_tree_classes(&full)?)
        .map_err(|error| format!("cannot write tree_classes.rs: {error}"))?;
    Ok(())
}

/// The Rust table of the events the host raises, read by `src/derived.rs`.
fn rust_derived_events(events: &[derived::DerivedEvent]) -> String {
    let mut text = String::from(
        "// Generated by crates/gnoblin-lua/build.rs from crates/gnoblin-lua/derived_events.toml.\n\
         // Do not edit: change the manifest and rebuild.\n\n\
         /// The events the host raises from a property change.\n\
         pub(crate) static DERIVED_EVENTS: &[RaisedEvent] = &[\n",
    );
    for event in events {
        text.push_str("    RaisedEvent {\n");
        let _ = writeln!(text, "        object: {},", text_literal(&event.object));
        let _ = writeln!(text, "        signal: {},", text_literal(&event.signal));
        let _ = writeln!(text, "        property: {},", text_literal(&event.property));
        let _ = writeln!(text, "        when: {},", event.when);
        text.push_str("        fields: &[\n");
        for field in &event.fields {
            let from = match &field.from {
                derived::DerivedSource::Object => "DerivedSource::Object".to_owned(),
                derived::DerivedSource::Property(name) => {
                    format!("DerivedSource::Property({})", text_literal(name))
                }
            };
            let _ = writeln!(
                text,
                "            RaisedField {{ name: {}, from: {from} }},",
                text_literal(&field.name)
            );
        }
        text.push_str("        ],\n    },\n");
    }
    text.push_str("];\n");
    text
}

/// The namespace the owner-only tree is declared in (`schema/tree/`).
const TREE_NAMESPACE: &str = "tree";

/// The Rust table of the tree classes the shell is given, read by `src/tree_core.rs`.
///
/// Built from the full catalog, owner-only `tree` namespace included, so the classes reach the
/// shell and never the public stub. A class's own members are the properties, signals and
/// methods its base does not already declare identically. A property of a type no wire carries
/// (bytes, a list that is not of doubles) is left out.
fn rust_tree_classes(catalog: &Catalog) -> Result<String, String> {
    let tree = Namespace::parse(TREE_NAMESPACE).map_err(|reason| format!("namespace `{TREE_NAMESPACE}`: {reason}"))?;
    // The protocol message object shares the namespace's name, and a record is a value on the
    // wire, not an identity: neither is a class.
    let objects: BTreeMap<&str, &ObjectType> = catalog
        .objects(&tree)
        .filter(|object| object.name != TREE_NAMESPACE && !object.record)
        .map(|object| (object.name.as_str(), object))
        .collect();
    for object in objects.values() {
        let Some(base) = object.base.as_deref() else { continue };
        if !objects.contains_key(base) {
            return Err(format!(
                "`{}` inherits from `{base}`, which the tree does not declare",
                object.name
            ));
        }
    }

    // A base comes before its subclasses: `TreeCore::shared` requires it.
    let mut order: Vec<&ObjectType> = Vec::new();
    while order.len() < objects.len() {
        let ready = objects.values().copied().find(|object| {
            let unplaced = !order.iter().any(|done| done.name == object.name);
            let base_placed = match &object.base {
                None => true,
                Some(base) => order.iter().any(|done| &done.name == base),
            };
            unplaced && base_placed
        });
        let Some(ready) = ready else {
            return Err("the tree classes inherit from one another in a cycle".to_owned());
        };
        order.push(ready);
    }

    let mut text = String::from(
        "// Generated by crates/gnoblin-lua/build.rs from the `tree` schema namespace.\n\
         // The tree classes the shell is given, a base before its subclasses.\n\
         pub fn tree_classes() -> Vec<ClassSpec> {\n    vec![\n",
    );
    for object in order {
        let base = object.base.as_deref().and_then(|name| objects.get(name).copied());
        let base_props: &[Property] = base.map(|base| base.properties.as_slice()).unwrap_or(&[]);
        let inherited_methods: Vec<&str> =
            base.map_or_else(Vec::new, |base| base.methods.iter().map(|m| m.name.as_str()).collect());
        let inherited_signals: Vec<&str> =
            base.map_or_else(Vec::new, |base| base.signals.iter().map(|s| s.name.as_str()).collect());

        let mut props: Vec<(String, &'static str, bool, Option<String>)> = Vec::new();
        for property in &object.properties {
            if base_props.iter().any(|inherited| same_declaration(inherited, property)) {
                continue;
            }
            let Some(kind) = tree_kind(&property.ty) else { continue };
            let default = property
                .default
                .as_ref()
                .map(|default| lua_default(default, kind, &object.name, &property.name))
                .transpose()?;
            props.push((property.name.clone(), kind, property.readonly, default));
        }
        if object.name == "node" {
            // The root class's one synthetic member: the mirror derives a node's children.
            props.push(("children".to_owned(), "Nodes", true, None));
        }
        let methods = own_names(object.methods.iter().map(|m| m.name.as_str()), &inherited_methods);
        let signals = own_names(object.signals.iter().map(|s| s.name.as_str()), &inherited_signals);

        text.push_str("        ClassSpec {\n");
        text.push_str(&format!(
            "            name: {:?}.to_owned(),\n",
            lua_class_name(&object.name)
        ));
        let base_expr = match &object.base {
            Some(name) => format!("Some({:?}.to_owned())", lua_class_name(name)),
            None => "None".to_owned(),
        };
        text.push_str(&format!("            base: {base_expr},\n"));
        text.push_str("            props: vec![\n");
        for (name, kind, readonly, default) in &props {
            let default = match default {
                Some(expr) => format!("Some({expr})"),
                None => "None".to_owned(),
            };
            text.push_str(&format!(
                "                PropSpec {{ name: {name:?}.to_owned(), kind: TreeValueKind::{kind}, readonly: {readonly}, default: {default} }},\n"
            ));
        }
        text.push_str("            ],\n");
        text.push_str(&format!("            signals: {},\n", string_list(&signals)));
        text.push_str(&format!("            methods: {},\n", string_list(&methods)));
        text.push_str("            slow: Vec::new(),\n        },\n");
    }
    text.push_str("    ]\n}\n");
    Ok(text)
}

/// The Lua name of a tree class: its schema name with the first letter capitalised.
fn lua_class_name(schema_name: &str) -> String {
    let mut chars = schema_name.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// Whether a subclass's declaration of a property matches its base's, in which case the
/// subclass inherits it rather than declaring it.
fn same_declaration(base: &Property, own: &Property) -> bool {
    base.name == own.name && base.ty == own.ty && base.readonly == own.readonly && base.default == own.default
}

/// The `TreeValueKind` variant a catalog type is generated as, or `None` for a type no wire carries.
fn tree_kind(ty: &Type) -> Option<&'static str> {
    Some(match ty {
        Type::Bool => "Bool",
        Type::Int => "Int",
        Type::Uint => "Uint",
        Type::Double => "Double",
        Type::Text => "Text",
        Type::ObjectRef(_) => "Node",
        Type::List(inner) if **inner == Type::Double => "ListDouble",
        Type::List(inner) if **inner == Type::Text => "ListText",
        Type::List(inner) if matches!(&**inner, Type::ObjectRef(name) if name == "effect") => "ListEffect",
        Type::Bytes | Type::List(_) => return None,
    })
}

/// The `LuaValue` expression of a default, converted to the property's kind.
fn lua_default(default: &DefaultValue, kind: &str, class: &str, name: &str) -> Result<String, String> {
    let mismatch = || format!("{class}.{name}: the default {default:?} does not fit {kind}");
    Ok(match (default, kind) {
        (DefaultValue::Bool(value), "Bool") => format!("LuaValue::Bool({value})"),
        (DefaultValue::Int(value), "Int") => format!("LuaValue::Int({value})"),
        (DefaultValue::Uint(value), "Uint") => format!("LuaValue::Uint({value})"),
        (DefaultValue::Double(value), "Double") => format!("LuaValue::Double({value:?})"),
        // A whole-number default of a double property is the same double.
        (DefaultValue::Int(value), "Double") => format!("LuaValue::Double({:?})", *value as f64),
        (DefaultValue::Uint(value), "Double") => format!("LuaValue::Double({:?})", *value as f64),
        (DefaultValue::Text(value), "Text") => format!("LuaValue::Text({value:?}.to_owned())"),
        (DefaultValue::List(items), "ListDouble") => {
            let mut values = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    DefaultValue::Double(value) => values.push(*value),
                    DefaultValue::Int(value) => values.push(*value as f64),
                    _ => return Err(mismatch()),
                }
            }
            let literals: Vec<String> = values.iter().map(|value| format!("{value:?}")).collect();
            format!("LuaValue::List(vec![{}])", literals.join(", "))
        }
        (DefaultValue::List(items), "ListText") if items.is_empty() => "LuaValue::Texts(Vec::new())".to_owned(),
        (DefaultValue::List(items), "ListEffect") if items.is_empty() => "LuaValue::Effects(Vec::new())".to_owned(),
        _ => return Err(mismatch()),
    })
}

/// The names of `own` that `inherited` does not already declare, in declaration order.
fn own_names<'a>(own: impl Iterator<Item = &'a str>, inherited: &[&str]) -> Vec<&'a str> {
    own.filter(|name| !inherited.contains(name)).collect()
}

/// A `Vec<String>` expression holding the given names.
fn string_list(names: &[&str]) -> String {
    if names.is_empty() {
        return "Vec::new()".to_owned();
    }
    let items: Vec<String> = names.iter().map(|name| format!("{name:?}.to_owned()")).collect();
    format!("vec![{}]", items.join(", "))
}

/// The checked-in schema files, in file-name order, as `(name, text)`.
///
/// Order is part of the contract: `Catalog` records the load order, so a build that read the
/// directory in whatever order it came back would produce a different catalog from the same tree.
fn schema_files(dir: &Path) -> Result<Vec<(String, String)>, String> {
    let mut paths = Vec::new();
    collect_toml(dir, &mut paths)?;
    // The tree is owner-socket only (design/node-core.md section 2.3): no Lua module binds it.
    paths.retain(|path| !path.starts_with(dir.join(OWNER_ONLY_DIR)));
    paths.sort();
    if paths.is_empty() {
        return Err(format!("no .toml schema files in {}", dir.display()));
    }
    let mut files = Vec::with_capacity(paths.len());
    for path in paths {
        let name = path
            .strip_prefix(dir)
            .ok()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("{} is not a UTF-8 schema path", path.display()))?;
        let text = fs::read_to_string(&path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        files.push((name.to_owned(), text));
    }
    Ok(files)
}

/// Push every `.toml` file under `dir` onto `paths`, descending into subdirectories.
fn collect_toml(dir: &Path, paths: &mut Vec<std::path::PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect_toml(&path, paths)?;
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("toml") {
            paths.push(path);
        }
    }
    Ok(())
}

/// The Rust table of the checked-in schema files, which `check.rs` parses into the stock catalog.
fn rust_stock_files(files: &[(String, String)]) -> String {
    let mut text = String::from(
        "// Generated by crates/gnoblin-lua/build.rs from crates/gnoblin-schema/schema/*.toml.\n\
         // Do not edit: change the schema and rebuild.\n\n\
         /// The checked-in schema files, in file-name order, as `(name, text)`.\n\
         pub const STOCK_FILES: &[(&str, &str)] = &[\n",
    );
    for (name, body) in files {
        text.push_str(&format!("    ({name:?}, {body:?}),\n"));
    }
    text.push_str("];\n");
    text
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

/// Refuse a schema whose object types would take a name the Lua tables already have.
///
/// The core objects are keys of the `gnoblin` table, beside the declared host members,
/// so a type named `timers` would replace the timer API. An extension namespace is
/// reached through its own module table, which has no declared members, so only
/// duplicate keys can clash there.
fn check_names(catalog: &Catalog, host: &[HostMember]) -> Result<(), String> {
    let host_names: Vec<&str> = host
        .iter()
        .filter(|member| member.object.is_none())
        .map(|member| member.name.as_str())
        .collect();
    check_host_objects(catalog, host)?;
    for namespace in catalog.namespaces() {
        let mut owners = Vec::new();
        for object in catalog.objects(&namespace) {
            owners.push((
                format!("{}.{}", namespace.as_str(), object.name),
                object.name.to_string(),
            ));
        }
        if namespace.as_str() == OWN_NAMESPACE {
            check_lua_names(OWN_NAMESPACE, &owners, &host_names)?;
        } else {
            check_lua_names(&format!("require(\"{}\")", namespace.as_str()), &owners, &[])?;
        }
    }
    Ok(())
}

/// Refuse an object-scoped member whose object is not an object type of the own namespace.
/// A member declared on a missing object would have no table to install on.
fn check_host_objects(catalog: &Catalog, host: &[HostMember]) -> Result<(), String> {
    for member in host {
        let Some(object) = &member.object else {
            continue;
        };
        let declared = catalog.namespaces().into_iter().any(|namespace| {
            namespace.as_str() == OWN_NAMESPACE
                && catalog.objects(&namespace).any(|candidate| candidate.name == *object)
        });
        if !declared {
            return Err(format!(
                "host member `{}` is declared on `{object}`, which is not an object type in the `{OWN_NAMESPACE}` namespace",
                member.name
            ));
        }
    }
    Ok(())
}

/// Refuse a method parameter whose name is not a Lua identifier. The stub writes each name
/// into a LuaCATS `---@param` and a Lua parameter list, so a keyword such as `local` would
/// make the stub unreadable to the language server and to `stylua`.
fn check_param_names(catalog: &Catalog) -> Result<(), String> {
    for namespace in catalog.namespaces() {
        for object in catalog.objects(&namespace) {
            for method in &object.methods {
                for parameter in &method.params {
                    if !is_lua_identifier(&parameter.name) {
                        return Err(format!(
                            "{}.{}:{} takes a parameter `{}`, which is not a Lua identifier; rename it in the schema",
                            namespace.as_str(),
                            object.name,
                            method.name,
                            parameter.name
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

/// One host-only member declaration file, as `host_members.toml` is written.
#[derive(Deserialize)]
struct HostFile {
    member: Vec<HostMember>,
}

/// One member: a function, or a table whose `members` are its children.
#[derive(Deserialize)]
struct HostMember {
    name: String,
    kind: Kind,
    doc: String,
    #[serde(default)]
    params: Vec<Param>,
    #[serde(default)]
    returns: Vec<String>,
    /// The object type this function belongs to, when it is a method of that object's
    /// class table rather than a `gnoblin` member. Only a top-level function may name one.
    #[serde(default)]
    object: Option<String>,
    #[serde(default)]
    members: Vec<HostMember>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Function,
    Table,
}

/// A typed parameter, spelled as LuaCATS.
#[derive(Deserialize)]
struct Param {
    name: String,
    #[serde(rename = "type")]
    ty: String,
}

/// Read the host-only members. A manifest the bindings could not be built from is refused
/// here, at build time, instead of surfacing as a broken `gnoblin` table at startup.
fn load_host_members(path: &Path) -> Result<Vec<HostMember>, String> {
    println!("cargo:rerun-if-changed={}", path.display());
    let text = fs::read_to_string(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let file: HostFile = toml::from_str(&text).map_err(|error| format!("cannot parse {}: {error}", path.display()))?;
    validate_host_members(&file.member, None)?;
    Ok(file.member)
}

/// Refuse a name that is not a Lua identifier, a name declared twice, a function with
/// members, or a table without them. `parent` is the dotted path of the table above.
fn validate_host_members(members: &[HostMember], parent: Option<&str>) -> Result<(), String> {
    // A name is unique within its object, or among the top-level members when it has none.
    let mut seen: Vec<(Option<&str>, &str)> = Vec::new();
    for member in members {
        let path = match parent {
            Some(parent) => format!("{parent}.{}", member.name),
            None => member.name.clone(),
        };
        if member.object.is_some() && (parent.is_some() || !matches!(member.kind, Kind::Function)) {
            return Err(format!(
                "host member `{path}` names an object, so it must be a top-level function"
            ));
        }
        if !is_lua_identifier(&member.name) {
            return Err(format!("host member `{path}` is not a Lua identifier"));
        }
        if seen.contains(&(member.object.as_deref(), member.name.as_str())) {
            return Err(format!("host member `{path}` is declared twice"));
        }
        seen.push((member.object.as_deref(), member.name.as_str()));
        match member.kind {
            Kind::Function if !member.members.is_empty() => {
                return Err(format!("host member `{path}` is a function, so it cannot have members"));
            }
            Kind::Function => {}
            Kind::Table if member.members.is_empty() => {
                return Err(format!("host member `{path}` is a table with no members"));
            }
            Kind::Table if !member.params.is_empty() || !member.returns.is_empty() => {
                return Err(format!(
                    "host member `{path}` is a table, so it cannot take params or returns"
                ));
            }
            Kind::Table => validate_host_members(&member.members, Some(&path))?,
        }
    }
    Ok(())
}

/// Whether `name` is a plain Lua identifier that is not a keyword, so `gnoblin.<name>` is a
/// valid key and a valid stub line.
fn is_lua_identifier(name: &str) -> bool {
    const KEYWORDS: &[&str] = &[
        "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in", "local", "nil",
        "not", "or", "repeat", "return", "then", "true", "until", "while",
    ];
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !KEYWORDS.contains(&name)
}

/// The Rust tables `src/bindings.rs` includes. `HOST_NAMES` lists the top-level names, which
/// the unknown-key error reports; `HOST_MEMBERS` is the declared tree the startup check
/// compares against the installed `gnoblin` table.
fn rust_host_members(host: &[HostMember]) -> String {
    let top: Vec<&HostMember> = host.iter().filter(|member| member.object.is_none()).collect();
    let mut text = String::from("// Generated by build.rs from host_members.toml. Do not edit.\n\n");
    text.push_str("pub(crate) const HOST_NAMES: &[&str] = &[\n");
    for member in &top {
        let _ = writeln!(text, "    {:?},", member.name);
    }
    text.push_str("];\n\n");
    text.push_str("pub(crate) const HOST_MEMBERS: &[HostMemberSpec] = &[\n");
    for member in &top {
        write_spec(&mut text, member, 1);
    }
    text.push_str("];\n\n");
    // The object-scoped members, grouped by the object whose class table they install on.
    text.push_str("pub(crate) const OBJECT_HOST_MEMBERS: &[(&str, &[HostMemberSpec])] = &[\n");
    for object in object_names(host) {
        let _ = writeln!(text, "    ({object:?}, &[");
        for member in host.iter().filter(|member| member.object.as_deref() == Some(object)) {
            write_spec(&mut text, member, 2);
        }
        text.push_str("    ]),\n");
    }
    text.push_str("];\n");
    text
}

/// The objects the host members are declared on, each once, in declaration order.
fn object_names(host: &[HostMember]) -> Vec<&str> {
    let mut names: Vec<&str> = Vec::new();
    for object in host.iter().filter_map(|member| member.object.as_deref()) {
        if !names.contains(&object) {
            names.push(object);
        }
    }
    names
}

/// Write one declared member as a `HostMemberSpec` literal, nested to match its tree.
fn write_spec(text: &mut String, member: &HostMember, depth: usize) {
    let indent = "    ".repeat(depth);
    let _ = writeln!(text, "{indent}HostMemberSpec {{");
    let _ = writeln!(text, "{indent}    name: {:?},", member.name);
    let _ = writeln!(text, "{indent}    members: &[");
    for child in &member.members {
        write_spec(text, child, depth + 1);
    }
    let _ = writeln!(text, "{indent}    ],");
    let _ = writeln!(text, "{indent}}},");
}

/// Write the LuaCATS text for `members`, which live under `prefix` in the stub.
fn stub_host_members(text: &mut String, members: &[HostMember], prefix: &str) {
    for member in members.iter().filter(|member| member.object.is_none()) {
        match member.kind {
            Kind::Function => stub_function(text, member, &format!("{prefix}.{}", member.name)),
            Kind::Table => {
                let _ = writeln!(text, "--- {}", member.doc);
                stub_host_members(text, &member.members, &format!("{prefix}.{}", member.name));
            }
        }
    }
}

/// Write the namespace functions declared on `object`. They live on the class table, which the
/// runtime reaches as `prefix` (for example `gnoblin.session`), so the stub names them by that path.
fn stub_object_host_members(text: &mut String, host: &[HostMember], object: &str, prefix: &str) {
    for member in host.iter().filter(|member| member.object.as_deref() == Some(object)) {
        stub_function(text, member, &format!("{prefix}.{}", member.name));
    }
}

/// Write one function's LuaCATS declaration: its doc, typed params and returns, and a stub
/// body that names its key under `prefix`.
fn stub_function(text: &mut String, member: &HostMember, qualified: &str) {
    let _ = writeln!(text, "--- {}", member.doc);
    for param in &member.params {
        let _ = writeln!(text, "---@param {} {}", param.name, param.ty);
    }
    for ret in &member.returns {
        let _ = writeln!(text, "---@return {ret}");
    }
    let names: Vec<&str> = member.params.iter().map(|param| param.name.as_str()).collect();
    push_function_decl(text, qualified, &names);
    text.push('\n');
}

/// Write `function NAME(PARAMS) end` as stylua lays it out: on one line when that fits in 120
/// columns, otherwise one parameter per line with `end` on its own line.
fn push_function_decl(text: &mut String, name: &str, params: &[&str]) {
    let one_line = format!("function {name}({}) end", params.join(", "));
    if one_line.len() <= 120 {
        let _ = writeln!(text, "{one_line}");
        return;
    }
    let _ = writeln!(text, "function {name}(");
    for (index, param) in params.iter().enumerate() {
        let separator = if index + 1 < params.len() { "," } else { "" };
        let _ = writeln!(text, "    {param}{separator}");
    }
    text.push_str(")\nend\n");
}

/// Start a new top-level block after a blank line, without doubling one already there.
fn blank_line(text: &mut String) {
    if !text.ends_with("\n\n") {
        text.push('\n');
    }
}

/// Ask cargo to rerun when any schema file changes, including the `tree/` subdirectory.
fn watch(dir: &Path) -> Result<(), String> {
    println!("cargo:rerun-if-changed={}", dir.display());
    let entries = fs::read_dir(dir).map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            watch(&path)?;
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("toml") {
            println!("cargo:rerun-if-changed={}", path.display());
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
                    "            MethodSpec {{ name: {}, function: {}, params: &[",
                    text_literal(&method.name),
                    method.function
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
fn lua_stub(catalog: &Catalog, host: &[HostMember]) -> String {
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
    text.push_str("---@field code string|nil the schema error code after a failure\n");
    text.push_str(
        "--- Callbacks receive `(value, error)`; `error` is nil on success and has `code` and `message` on failure.\n",
    );
    text.push_str("local Operation = {}\n");
    text.push_str("---@param fn fun(value: any, error: table|nil)\n");
    text.push_str("---@return gnoblin.Subscription\n");
    text.push_str("function Operation:on_complete(fn) end\n");
    text.push_str("---@param fn fun(value: any, error: table|nil): any\n");
    text.push_str("---@return gnoblin.Operation\n");
    text.push_str("function Operation:map(fn) end\n");
    text.push_str("---@param seconds number\n");
    text.push_str("---@return gnoblin.Operation\n");
    text.push_str("function Operation:timeout(seconds) end\n");
    text.push_str("--- Stop waiting; false means the operation already finished.\n");
    text.push_str("---@return boolean\n");
    text.push_str("function Operation:cancel() end\n\n");
    text.push_str("--- One signal handler registration.\n");
    text.push_str("---@class gnoblin.Subscription\n");
    text.push_str("local Subscription = {}\n");
    text.push_str("--- Stop delivering the event.\n");
    text.push_str("function Subscription:unsubscribe() end\n\n");
    text.push_str("--- One timer on the caller's clock.\n");
    text.push_str("---@class gnoblin.Timer\n");
    text.push_str("local Timer = {}\n");
    text.push_str("--- Cancel the timer.\n");
    text.push_str("function Timer:cancel() end\n\n");
    text.push_str("--- A child process the runtime started.\n");
    text.push_str("---@class gnoblin.Process\n");
    text.push_str("---@field id integer\n");
    text.push_str("---@field pid integer|nil\n");
    text.push_str("---@field argv string[]\n");
    text.push_str("---@field alive boolean\n");
    text.push_str("---@field status string\n");
    text.push_str("---@field exit_code integer|nil\n");
    text.push_str("---@field signal string|nil\n");
    text.push_str("---@field revision integer\n");
    text.push_str("local Process = {}\n");
    text.push_str("--- The exit of the child, answered as an operation.\n");
    text.push_str("---@return gnoblin.Operation\n");
    text.push_str("function Process:wait() end\n");
    text.push_str("--- Send a signal to the child's process group; the default is term.\n");
    text.push_str("---@param signal? string\n");
    text.push_str("---@return gnoblin.Operation\n");
    text.push_str("function Process:kill(signal) end\n\n");
    text.push_str("--- Stop the process when it has ended, or hear the change of one field.\n");
    text.push_str("---@param event string\n");
    text.push_str("---@param callback fun(process: gnoblin.Process)\n");
    text.push_str("---@return gnoblin.Subscription\n");
    text.push_str("function Process:on(event, callback) end\n");
    text.push_str("--- Hear each change of one field (pid, status, exit_code or signal).\n");
    text.push_str("---@param field string\n");
    text.push_str("---@param callback fun(process: gnoblin.Process)\n");
    text.push_str("---@return gnoblin.Subscription\n");
    text.push_str("function Process:watch(field, callback) end\n");
    stub_host_members(&mut text, host, "gnoblin");

    for namespace in catalog.namespaces() {
        for object in catalog.objects(&namespace) {
            stub_object(&mut text, &namespace, object, host);
        }
    }
    stub_modules(&mut text, catalog);
    text
}

/// The names a script reaches objects through: core types on `gnoblin`, and one
/// `require`d module per extension namespace, each with one collection per type.
fn stub_modules(text: &mut String, catalog: &Catalog) {
    let mut overloads = String::new();
    for namespace in catalog.namespaces() {
        let is_own = namespace.as_str() == OWN_NAMESPACE;
        let holder = if is_own {
            OWN_NAMESPACE.to_owned()
        } else {
            format!("{}Module", namespace.as_str())
        };
        blank_line(text);
        let _ = writeln!(text, "---@class {holder}");
        for object in catalog.objects(&namespace) {
            let _ = writeln!(
                text,
                "---@field {} {}.{}Collection",
                object.name,
                namespace.as_str(),
                object.name
            );
        }
        if !is_own {
            let _ = writeln!(
                overloads,
                "---@overload fun(name: \"{}\"): {holder}",
                namespace.as_str()
            );
        }
    }
    text.push_str(&overloads);
    text.push_str("---Load a Lua module by dotted name. `a.b` is `a/b.lua` under the configuration directory, then under the shipped modules. A `gnoblin.` name loads from the shipped modules only.\n");
    text.push_str("---@param name string\n");
    text.push_str("---@return any\n");
    text.push_str("function require(name) end\n");
}

/// One object's class, properties, methods and signals.
fn stub_object(text: &mut String, namespace: &Namespace, object: &ObjectType, host: &[HostMember]) {
    let class = format!("{}.{}", namespace.as_str(), object.name);
    blank_line(text);
    let _ = writeln!(text, "---@class {class}");
    for property in &object.properties {
        let _ = writeln!(text, "---@field {} {}", property.name, lua_type(&property.ty));
    }
    if object.name == "window" {
        // Lua-only: the host answers `window.process` from its own process records, so the
        // schema does not declare it.
        let _ = writeln!(text, "---@field process gnoblin.Process|nil");
    }
    let _ = writeln!(text, "local {} = {{}}", object.name);
    // A namespace function is declared on the class table, with no receiver, by the host members.
    for method in object.methods.iter().filter(|method| !method.function) {
        for parameter in &method.params {
            let _ = writeln!(text, "---@param {} {}", parameter.name, lua_type(&parameter.ty));
        }
        let _ = writeln!(text, "---@return gnoblin.Operation");
        let names: Vec<&str> = method.params.iter().map(|parameter| parameter.name.as_str()).collect();
        push_function_decl(text, &format!("{}:{}", object.name, method.name), &names);
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
    stub_object_host_members(text, host, &object.name.to_string(), &class);
    // The collection a script reaches instances through: one per object type.
    let collection = format!("{class}Collection");
    let _ = writeln!(text, "---@class {collection}");
    let _ = writeln!(text, "---@field by_id fun(id: integer): {class}|nil");
    let _ = writeln!(text, "---@field all fun(): {class}[]");
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
