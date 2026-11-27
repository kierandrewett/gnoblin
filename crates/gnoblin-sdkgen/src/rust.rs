//! The Rust target: what `gnoblin-core` would otherwise spell by hand, twice.
//!
//! The Rust SDK is the thinnest of the SDKs: it calls the ABI table directly, so there is no binding layer to generate. What
//! it does carry is every name and shape the schema and the ABI already say:
//!
//! - `schema.rs`: the catalog hash and, for every class, its name, its properties, methods and signals as `&str` constants,
//!   so a shell names a member by a constant the compiler checks and not by a string that drifts;
//! - `classes.rs`: a typed wrapper over [`Node`](https://docs.rs) for every class that derives from `node` (`Window`,
//!   `Seat`, `View`, ...) with a getter and a setter for every property the SDK can read as a Rust type, and a call for every
//!   method;
//! - `errors.rs`: `ErrorKind`, one variant per `E_*` code of the ABI, with the conversions from a code and from the symbol
//!   the std library puts at the front of its error text.
//!
//! What stays hand-written is the ergonomic layer: the composite accessors (`Window::geometry`), the trampolines and
//! closures of `Gnoblin`, the typed events and the macros. A generated type has none of those; it is only names and shapes.

use gnoblin_schema::{ObjectType, Property, Type};

use super::model::{Const, Library, SdkSpec};
use super::text::{CodeWriter, pascal, rust_ident, snake};
use super::{Comment, File, Target};

/// The Rust SDK's generated module.
pub struct RustTarget;

const PATH: &str = "rust-core/generated.rs";

/// Properties the hand-written `Node` already exposes with a stronger type: its identity and its place in the tree. A wrapper
/// that generated `id()` would shadow `Node::id()` through `Deref` and return a different type, so they are not generated.
const NODE_OWNED: &[&str] = &["id", "parent"];

/// Members of a class that the hand-written part of the SDK defines with its own meaning. `Window::focus` gives the keyboard to
/// the window through the seat; the schema's `focus` method asks the shell's focus policy and is reachable as `Node::call`.
const HAND_WRITTEN: &[(&str, &str)] = &[("window", "focus")];

impl Target for RustTarget {
    fn name(&self) -> &'static str {
        "rust"
    }

    fn generate(&self, spec: &SdkSpec) -> Result<Vec<File>, String> {
        // One file, because `include!` takes no inner attribute or inner documentation at the top of a file: each module's
        // text keeps its own inside an inline module.
        let mut text = String::new();
        text.push_str("// Everything the schema and the ABI say about the SDK, as Rust: the names of the classes and their members,\n");
        text.push_str("// the typed node wrappers and the error kinds.\n\n");
        for (visibility, name, body) in [
            ("pub(crate)", "classes", classes(spec)),
            ("pub(crate)", "errors", errors(spec.core())?),
            ("pub", "schema", schema(spec)),
        ] {
            text.push_str(&format!("{visibility} mod {name} {{\n{body}}}\n\n"));
        }
        text.push_str(&module(spec));
        Ok(vec![File {
            path: PATH.to_owned(),
            comment: Some(Comment::Slashes),
            text,
        }])
    }
}

/// The re-exports that end the file.
fn module(spec: &SdkSpec) -> String {
    let mut code = CodeWriter::new("    ");
    let mut classes: Vec<String> = spec
        .node_classes(false)
        .iter()
        .map(|object| pascal(&object.name))
        .collect();
    classes.sort();
    code.line(format!("pub use classes::{{{}}};", classes.join(", ")));
    code.line("pub use errors::ErrorKind;");
    code.finish()
}

// ---------------------------------------------------------------------------------------------------------------------
// schema.rs

fn schema(spec: &SdkSpec) -> String {
    let mut code = CodeWriter::new("    ");
    code.line("//! The schema catalog's names, as constants.");
    code.line("//!");
    code.line("//! `CLASS` is the name `Gnoblin::class` takes (a bare name for the `tree` namespace, `namespace.object` for the rest).");
    code.line("//! The members are the names `Node::get`, `Node::set`, `Node::call` and the subscriptions take.");
    code.blank();
    // `tree::tree` is the schema's own spelling: the `tree` class of the `tree` namespace.
    code.line("#![allow(clippy::module_inception)]");
    code.blank();
    code.line("/// A fingerprint of the catalog this SDK was generated from. `gnoblin_core::catalog_hash()` is the installed core's: equal means the");
    code.line("/// core and this SDK describe the same classes, properties, methods and signals.");
    code.line(format!("pub const CATALOG_HASH: u64 = 0x{:016x};", spec.catalog_hash));
    for namespace in spec.catalog.namespaces() {
        code.blank();
        code.line(format!("/// The `{}` namespace.", namespace.as_str()));
        code.block(
            format!("pub mod {} {{", rust_ident(namespace.as_str())),
            |code| {
                for object in spec.catalog.objects(&namespace) {
                    let class_name = if namespace.as_str() == "tree" {
                        object.name.clone()
                    } else {
                        format!("{}.{}", namespace.as_str(), object.name)
                    };
                    code.line(format!("/// The `{}` class.", object.name));
                    code.block(
                        format!("pub mod {} {{", rust_ident(&object.name)),
                        |code| {
                            code.line("/// The name `Gnoblin::class` takes for this class.");
                            code.line(format!("pub const CLASS: &str = {class_name:?};"));
                            names(
                                code,
                                "property",
                                object.properties.iter().map(|property| property.name.as_str()),
                            );
                            names(
                                code,
                                "method",
                                object
                                    .methods
                                    .iter()
                                    .filter(|method| !method.function)
                                    .map(|method| method.name.as_str()),
                            );
                            names(
                                code,
                                "function",
                                object
                                    .methods
                                    .iter()
                                    .filter(|method| method.function)
                                    .map(|method| method.name.as_str()),
                            );
                            names(code, "signal", object.signals.iter().map(|signal| signal.name.as_str()));
                            signal_fields(code, &object.signals);
                        },
                        "}",
                    );
                }
            },
            "}",
        );
    }
    code.finish()
}

/// A `pub mod <kind> { pub const NAME: &str = "name"; }` block; nothing when there are no names.
fn names<'a>(code: &mut CodeWriter, kind: &str, names: impl Iterator<Item = &'a str>) {
    let names: Vec<&str> = names.collect();
    if names.is_empty() {
        return;
    }
    code.line(format!("/// The class's {kind}s."));
    code.block(
        format!("pub mod {kind} {{"),
        |code| {
            for name in names {
                code.line(format!("/// `{name}`."));
                code.line(format!("pub const {}: &str = {name:?};", name.to_ascii_uppercase()));
            }
        },
        "}",
    );
}

/// The position of every field of every signal that has some, in the payload `SignalEvent::value` indexes: the schema's field order,
/// which is the order the core builds a payload in. Nothing when no signal of the class carries a payload.
fn signal_fields(code: &mut CodeWriter, signals: &[gnoblin_schema::Signal]) {
    let carrying: Vec<&gnoblin_schema::Signal> = signals.iter().filter(|signal| !signal.fields.is_empty()).collect();
    if carrying.is_empty() {
        return;
    }
    code.line("/// The fields of the class's signals: the position of each in the payload `SignalEvent::value` takes.");
    code.block(
        "pub mod fields {",
        |code| {
            for signal in carrying {
                code.line(format!("/// The fields of `{}`.", signal.name));
                code.block(
                    format!("pub mod {} {{", rust_ident(&signal.name)),
                    |code| {
                        for (position, field) in signal.fields.iter().enumerate() {
                            code.line(format!("/// `{}`: {}.", field.name, type_phrase(&field.ty)));
                            code.line(format!(
                                "pub const {}: usize = {position};",
                                field.name.to_ascii_uppercase()
                            ));
                        }
                    },
                    "}",
                );
            }
        },
        "}",
    );
}

/// A schema type in the words a doc comment uses.
fn type_phrase(ty: &Type) -> String {
    match ty {
        Type::Bool => "a boolean".to_owned(),
        Type::Int => "a signed integer".to_owned(),
        Type::Uint => "an unsigned integer".to_owned(),
        Type::Double => "a number".to_owned(),
        Type::Text => "text".to_owned(),
        Type::Bytes => "bytes".to_owned(),
        Type::Any => "any value".to_owned(),
        Type::ObjectRef(name) => format!("a `{name}`"),
        Type::Enum(name) => format!("a value of `{name}`"),
        Type::Callback(name) => format!("a `{name}` closure"),
        Type::List(inner) => format!("a list of {}", type_phrase(inner)),
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// classes.rs

/// How the SDK reads and writes a property as a Rust type.
struct RustType {
    /// The type a getter returns.
    get: &'static str,
    /// The expression that reads it, with `{p}` for the property name, `{n}` for the node and `{r}` for a reference to it.
    read: &'static str,
    /// The type a setter takes.
    set: &'static str,
    /// The expression that turns the setter's `value` into a `Value`.
    write: &'static str,
}

fn rust_type(ty: &Type) -> Option<RustType> {
    Some(match ty {
        Type::Bool => RustType {
            get: "bool",
            read: "{n}.get_bool({p})",
            set: "bool",
            write: "value",
        },
        Type::Double => RustType {
            get: "f64",
            read: "{n}.get_f64({p})",
            set: "f64",
            write: "value",
        },
        Type::Int => RustType {
            get: "i64",
            read: "read_int({r}, {p})",
            set: "i64",
            write: "value",
        },
        Type::Uint => RustType {
            get: "u64",
            read: "read_uint({r}, {p})",
            set: "u64",
            write: "value",
        },
        Type::Text => RustType {
            get: "String",
            read: "{n}.get_text({p})",
            set: "&str",
            write: "value",
        },
        Type::ObjectRef(_) => RustType {
            get: "Option<Node>",
            read: "{n}.get_node({p})",
            set: "Option<&Node>",
            write: "Value::Node(value.map_or(NodeId::NIL, Node::id))",
        },
        Type::List(inner) => match inner.as_ref() {
            Type::Double => RustType {
                get: "Vec<f64>",
                read: "read_numbers({r}, {p})",
                set: "&[f64]",
                write: "Value::List(value.to_vec())",
            },
            Type::Text => RustType {
                get: "Vec<String>",
                read: "read_texts({r}, {p})",
                set: "&[String]",
                write: "Value::Texts(value.to_vec())",
            },
            _ => return None,
        },
        Type::Bytes | Type::Any | Type::Enum(_) | Type::Callback(_) => return None,
    })
}

/// The Rust type of a method parameter, and the expression that makes a `Value` of it.
fn param_type(ty: &Type) -> (&'static str, &'static str) {
    match ty {
        Type::Bool => ("bool", "{v}.into()"),
        Type::Double => ("f64", "{v}.into()"),
        Type::Int => ("i64", "{v}.into()"),
        Type::Uint => ("u64", "{v}.into()"),
        Type::Text => ("&str", "{v}.into()"),
        Type::ObjectRef(_) => ("NodeId", "{v}.into()"),
        Type::List(inner) if **inner == Type::Double => ("&[f64]", "{v}.to_vec().into()"),
        _ => ("Value", "{v}"),
    }
}

fn classes(spec: &SdkSpec) -> String {
    let mut code = CodeWriter::new("    ");
    code.line("//! Typed wrappers over `Node`, one per class of the tree that derives from `node`.");
    code.line("//!");
    code.line("//! A wrapper holds a `Node` and dereferences to it, so everything a node does a window does. What it adds is a getter and a");
    code.line("//! setter for every property the SDK can read as a Rust type, and a call for every method of the class. A property of");
    code.line("//! another kind (a list of effects, bytes) is read and written with `Node::get` and `Node::set`. The members come from");
    code.line("//! the schema, so a new property appears here with the next run of the generator and a removed one fails the build");
    code.line("//! of the shell that used it.");
    code.blank();
    code.line("#![allow(dead_code)]");
    code.blank();
    code.line("use std::ops::Deref;");
    code.blank();
    code.line("use crate::error::{CoreError, ErrorKind, Result};");
    code.line("use crate::gnoblin::Gnoblin;");
    code.line("use crate::owned::Value;");
    code.line("use crate::tree::Node;");
    code.line("use crate::value::NodeId;");
    code.blank();
    helpers(&mut code);
    // What `node` declares is written once, on `Node` itself, so every node (and every wrapper, through `Deref`) has it.
    let node = spec.node_classes(true).into_iter().find(|object| object.name == "node");
    let node = node.as_ref().expect("the tree namespace declares `node`");
    code.line("impl Node {");
    code.indent();
    members(&mut code, node, &NODE, &[], &[]);
    code.dedent();
    code.line("}");
    code.blank();
    let node_properties: Vec<&str> = node.properties.iter().map(|property| property.name.as_str()).collect();
    let node_methods: Vec<&str> = node.methods.iter().map(|method| method.name.as_str()).collect();
    for object in spec.node_classes(false) {
        class(&mut code, &object, &node_properties, &node_methods);
    }
    code.finish()
}

fn helpers(code: &mut CodeWriter) {
    code.line("fn wrong(property: &str, value: &Value, wanted: &str) -> CoreError {");
    code.line("    CoreError::new(ErrorKind::Type, format!(\"`{property}` is {value:?}, not {wanted}\"))");
    code.line("}");
    code.blank();
    code.line("fn read_int(node: &Node, property: &str) -> Result<i64> {");
    code.line("    match node.get(property)? {");
    code.line("        Value::Int(value) => Ok(value),");
    code.line("        Value::Uint(value) => i64::try_from(value).map_err(|_| wrong(property, &Value::Uint(value), \"a signed integer\")),");
    code.line("        other => Err(wrong(property, &other, \"an integer\")),");
    code.line("    }");
    code.line("}");
    code.blank();
    code.line("fn read_uint(node: &Node, property: &str) -> Result<u64> {");
    code.line("    match node.get(property)? {");
    code.line("        Value::Uint(value) => Ok(value),");
    code.line("        Value::Int(value) => u64::try_from(value).map_err(|_| wrong(property, &Value::Int(value), \"an unsigned integer\")),");
    code.line("        other => Err(wrong(property, &other, \"an unsigned integer\")),");
    code.line("    }");
    code.line("}");
    code.blank();
    code.line("fn read_numbers(node: &Node, property: &str) -> Result<Vec<f64>> {");
    code.line("    match node.get(property)? {");
    code.line("        Value::List(list) => Ok(list),");
    code.line("        other => Err(wrong(property, &other, \"a list of numbers\")),");
    code.line("    }");
    code.line("}");
    code.blank();
    code.line("fn read_texts(node: &Node, property: &str) -> Result<Vec<String>> {");
    code.line("    match node.get(property)? {");
    code.line("        Value::Texts(list) => Ok(list),");
    code.line("        other => Err(wrong(property, &other, \"a list of texts\")),");
    code.line("    }");
    code.line("}");
    code.blank();
}

fn getter_name(property: &Property) -> String {
    let name = snake(&property.name);
    if property.ty == Type::Bool && !name.starts_with("is_") && !name.starts_with("has_") {
        format!("is_{name}")
    } else {
        rust_ident(&name)
    }
}

fn class(code: &mut CodeWriter, object: &ObjectType, node_properties: &[&str], node_methods: &[&str]) {
    let type_name = pascal(&object.name);
    code.line(format!(
        "/// A node of class `{}`: {}.",
        object.name,
        class_summary(object)
    ));
    code.line("#[derive(Clone, Debug, PartialEq, Eq)]");
    code.line(format!("pub struct {type_name}(pub(crate) Node);"));
    code.blank();
    code.block(
        format!("impl Deref for {type_name} {{"),
        |code| {
            code.line("type Target = Node;");
            code.blank();
            code.block("fn deref(&self) -> &Node {", |code| code.line("&self.0"), "}");
        },
        "}",
    );
    code.blank();
    code.block(
        format!("impl {type_name} {{"),
        |code| {
            code.line("/// The schema name of the class, as `Gnoblin::class` takes it.");
            code.line(format!("pub const CLASS: &'static str = {:?};", object.name));
            code.blank();
            code.line("pub(crate) fn new(g: Gnoblin, id: NodeId) -> Self {");
            code.line(format!("    {type_name}(Node::new(g, id))"));
            code.line("}");
            code.blank();
            code.line(format!(
                "/// The `{}` made of `node`, if `node` is of that class.",
                object.name
            ));
            code.line("///");
            code.line("/// # Errors");
            code.line("///");
            code.line("/// The node is of another class (`Type`), or is gone.");
            code.line("pub fn from_node(node: Node) -> Result<Self> {");
            code.line("    let class = node.gnoblin().class(Self::CLASS)?;");
            code.line("    if node.class()? == class {");
            code.line(format!("        Ok({type_name}(node))"));
            code.line("    } else {");
            code.line("        Err(CoreError::new(");
            code.line("            ErrorKind::Type,");
            code.line("            format!(");
            code.line("                \"node {} is a {}, not a {}\",");
            code.line("                node.id().0,");
            code.line("                node.class_name().unwrap_or_default(),");
            code.line("                Self::CLASS");
            code.line("            ),");
            code.line("        ))");
            code.line("    }");
            code.line("}");
            code.blank();
            code.line("/// The node this wrapper holds.");
            code.line("pub fn node(&self) -> &Node {");
            code.line("    &self.0");
            code.line("}");
            members(code, object, &WRAPPER, node_properties, node_methods);
        },
        "}",
    );
    code.blank();
}

fn class_summary(object: &ObjectType) -> String {
    match &object.base {
        Some(base) => format!("`{base}` and its members, then the class's own"),
        None => "its members".to_owned(),
    }
}

/// How generated code names the node it acts on: a wrapper holds it in field 0, `Node` is it.
struct Recv {
    n: &'static str,
    r: &'static str,
}

const WRAPPER: Recv = Recv {
    n: "self.0",
    r: "&self.0",
};
const NODE: Recv = Recv { n: "self", r: "self" };

/// The accessors and calls of `object`, except the members named in `skip_properties` and `skip_methods`.
fn members(code: &mut CodeWriter, object: &ObjectType, recv: &Recv, skip_properties: &[&str], skip_methods: &[&str]) {
    for property in &object.properties {
        if NODE_OWNED.contains(&property.name.as_str())
            || property.is_sensitive()
            || skip_properties.contains(&property.name.as_str())
        {
            continue;
        }
        accessors(code, object, property, recv);
    }
    for method in &object.methods {
        if method.function
            || method.is_sensitive()
            || skip_methods.contains(&method.name.as_str())
            || HAND_WRITTEN.contains(&(object.name.as_str(), method.name.as_str()))
        {
            continue;
        }
        call(code, method, recv);
    }
}

fn accessors(code: &mut CodeWriter, object: &ObjectType, property: &Property, recv: &Recv) {
    let Some(kind) = rust_type(&property.ty) else { return };
    let quoted = format!("{:?}", property.name);
    let getter = getter_name(property);
    let flags = match (property.readonly, property.animated) {
        (true, _) => "read-only",
        (false, true) => "animated",
        (false, false) => "read-write",
    };
    code.blank();
    code.line(format!(
        "/// The `{}` property (`{}`, {flags}).",
        property.name, property.ty
    ));
    code.line("///");
    code.line("/// # Errors");
    code.line("///");
    code.line("/// The node is gone (`NoSuchNode`) or the generation has retired (`Retired`).");
    code.line(format!("pub fn {getter}(&self) -> Result<{}> {{", kind.get));
    code.line(format!(
        "    {}",
        kind.read
            .replace("{p}", &quoted)
            .replace("{n}", recv.n)
            .replace("{r}", recv.r)
    ));
    code.line("}");
    if property.readonly {
        return;
    }
    let setter = rust_ident(&format!("set_{}", snake(&property.name)));
    if HAND_WRITTEN.contains(&(object.name.as_str(), setter.as_str())) {
        return;
    }
    code.blank();
    code.line(format!(
        "/// Writes the `{}` property; the write leaves with the turn's other writes.",
        property.name
    ));
    code.line("///");
    code.line("/// # Errors");
    code.line("///");
    code.line("/// The node is gone, the generation has retired, or the compositor refuses the value.");
    code.line(format!("pub fn {setter}(&self, value: {}) -> Result<()> {{", kind.set));
    code.line(format!("    {}.set({quoted}, {})", recv.n, kind.write));
    code.line("}");
    if property.animated {
        let animated = rust_ident(&format!("set_{}_animated", snake(&property.name)));
        code.blank();
        code.line(format!(
            "/// Writes the `{}` property so that the compositor plays the change over `seconds`.",
            property.name
        ));
        code.line("///");
        code.line("/// # Errors");
        code.line("///");
        code.line("/// As for the plain setter.");
        code.line(format!(
            "pub fn {animated}(&self, value: {}, seconds: f64) -> Result<()> {{",
            kind.set
        ));
        code.line(format!(
            "    {}.set_animated({quoted}, {}, seconds)",
            recv.n, kind.write
        ));
        code.line("}");
    }
}

fn call(code: &mut CodeWriter, method: &gnoblin_schema::Method, recv: &Recv) {
    let name = rust_ident(&snake(&method.name));
    let mut signature = String::new();
    for param in &method.params {
        let (ty, _) = param_type(&param.ty);
        let ty = if param.optional {
            format!("Option<{ty}>")
        } else {
            ty.to_owned()
        };
        signature.push_str(&format!(", {}: {ty}", rust_ident(&snake(&param.name))));
    }
    code.blank();
    code.line(format!("/// Calls the `{}` method.", method.name));
    code.line("///");
    code.line("/// # Errors");
    code.line("///");
    code.line("/// The node is gone, the generation has retired, or the arguments do not fit the method.");
    code.line(format!("pub fn {name}(&self{signature}) -> Result<()> {{"));
    let required: Vec<String> = method
        .params
        .iter()
        .filter(|param| !param.optional)
        .map(|param| {
            let (_, make) = param_type(&param.ty);
            make.replace("{v}", &rust_ident(&snake(&param.name)))
        })
        .collect();
    let optional = method.params.iter().any(|param| param.optional);
    if optional {
        code.line(format!("    let mut args: Vec<Value> = vec![{}];", required.join(", ")));
        for param in method.params.iter().filter(|param| param.optional) {
            let variable = rust_ident(&snake(&param.name));
            let (_, make) = param_type(&param.ty);
            code.line(format!("    if let Some({variable}) = {variable} {{"));
            code.line(format!("        args.push({});", make.replace("{v}", &variable)));
            code.line("    }");
        }
        code.line(format!("    {}.call({:?}, &args)", recv.n, method.name));
    } else {
        code.line(format!(
            "    {}.call({:?}, &[{}])",
            recv.n,
            method.name,
            required.join(", ")
        ));
    }
    code.line("}");
}

// ---------------------------------------------------------------------------------------------------------------------
// errors.rs

fn variant(constant: &Const) -> String {
    match constant.short.as_str() {
        "ERR" => "Failed".to_owned(),
        other => pascal(&other.to_ascii_lowercase()),
    }
}

fn errors(core: &Library) -> Result<String, String> {
    let group = core
        .groups
        .iter()
        .find(|group| group.name == "Error")
        .ok_or("the core ABI has no E_* constants")?;
    let codes: Vec<&Const> = group.members.iter().filter(|constant| constant.short != "OK").collect();
    let mut code = CodeWriter::new("    ");
    code.line("//! The error kinds: one per `E_*` code of the core ABI.");
    code.blank();
    code.line("use gnoblin_core_abi as abi;");
    code.blank();
    code.line("/// Why a call failed. Mirrors the `E_*` codes; a code a newer core adds that this SDK does not know is [`Failed`].");
    code.line("///");
    code.line("/// [`Failed`]: ErrorKind::Failed");
    code.line("#[derive(Clone, Copy, Debug, PartialEq, Eq)]");
    code.line("#[non_exhaustive]");
    code.block(
        "pub enum ErrorKind {",
        |code| {
            for constant in &codes {
                code.doc("///", &constant.doc);
                code.line(format!("{},", variant(constant)));
            }
        },
        "}",
    );
    code.blank();
    code.block(
        "impl ErrorKind {",
        |code| {
            code.line("/// The kind for a negative `E_*` code.");
            code.block(
                "pub fn from_code(code: i32) -> ErrorKind {",
                |code| {
                    code.block(
                        "match code {",
                        |code| {
                            for constant in codes.iter().filter(|constant| variant(constant) != "Failed") {
                                code.line(format!(
                                    "abi::E_{} => ErrorKind::{},",
                                    constant.short,
                                    variant(constant)
                                ));
                            }
                            code.line("_ => ErrorKind::Failed,");
                        },
                        "}",
                    );
                },
                "}",
            );
            code.blank();
            code.line(
                "/// The kind named by an `E_*` symbol, as the std library writes it at the front of an error's text",
            );
            code.line("/// (`E_REFUSED: the client is a dialog`). An unknown symbol is `Failed`.");
            code.block(
                "pub fn from_symbol(symbol: &str) -> ErrorKind {",
                |code| {
                    code.block(
                        "match symbol {",
                        |code| {
                            for constant in codes.iter().filter(|constant| variant(constant) != "Failed") {
                                code.line(format!("\"E_{}\" => ErrorKind::{},", constant.short, variant(constant)));
                            }
                            code.line("_ => ErrorKind::Failed,");
                        },
                        "}",
                    );
                },
                "}",
            );
            code.blank();
            code.line("/// The `E_*` code this kind stands for.");
            code.block(
                "pub fn code(self) -> i32 {",
                |code| {
                    code.block(
                        "match self {",
                        |code| {
                            for constant in &codes {
                                code.line(format!(
                                    "ErrorKind::{} => abi::E_{},",
                                    variant(constant),
                                    constant.short
                                ));
                            }
                        },
                        "}",
                    );
                },
                "}",
            );
            code.blank();
            code.line(
                "/// The `E_*` symbol of this kind, which the std library writes at the front of an error's text.",
            );
            code.block(
                "pub fn symbol(self) -> &'static str {",
                |code| {
                    code.block(
                        "match self {",
                        |code| {
                            for constant in &codes {
                                code.line(format!("ErrorKind::{} => \"E_{}\",", variant(constant), constant.short));
                            }
                        },
                        "}",
                    );
                },
                "}",
            );
        },
        "}",
    );
    Ok(code.finish())
}

#[cfg(test)]
mod tests {

    use super::*;

    fn spec() -> SdkSpec {
        SdkSpec::load(&crate::workspace_root()).expect("spec loads")
    }

    #[test]
    fn the_node_classes_are_the_ones_that_derive_from_node() {
        let spec = spec();
        let classes = spec.node_classes(false);
        let names: Vec<&str> = classes.iter().map(|object| object.name.as_str()).collect();
        for expected in ["window", "seat", "view", "screen", "camera"] {
            assert!(names.contains(&expected), "{expected} in {names:?}");
        }
        assert!(!names.contains(&"node"));
        assert!(!names.contains(&"op"), "a protocol message is not a node");
    }

    #[test]
    fn the_module_re_exports_every_generated_class() {
        let spec = spec();
        let module = module(&spec);
        for object in spec.node_classes(false) {
            assert!(
                module.contains(&pascal(&object.name)),
                "{} is not re-exported by mod.rs",
                object.name
            );
        }
    }

    #[test]
    fn the_window_wrapper_has_the_accessors_the_hand_written_one_had() {
        let text = classes(&spec());
        for needed in [
            "pub fn title(&self) -> Result<String>",
            "pub fn app_id(&self) -> Result<String>",
            "pub fn is_focused(&self) -> Result<bool>",
            "pub fn is_maximised(&self) -> Result<bool>",
            "pub fn set_maximised(&self, value: bool)",
            "pub fn set_opacity(&self, value: f64)",
            "pub fn set_visible(&self, value: bool)",
            "pub fn close(&self) -> Result<()>",
            "pub fn kill(&self) -> Result<()>",
        ] {
            assert!(text.contains(needed), "missing: {needed}");
        }
        assert!(!text.contains("pub fn id(&self)"), "id is Node's");
        assert!(!text.contains("pub fn parent(&self)"), "parent is Node's");
    }

    #[test]
    fn error_kinds_cover_every_code_and_keep_the_failed_default() {
        let spec = spec();
        let text = errors(spec.core()).expect("errors render");
        assert!(text.contains("Failed,"));
        assert!(text.contains("abi::E_NO_SUCH_NODE => ErrorKind::NoSuchNode"));
        assert!(text.contains("\"E_REFUSED\" => ErrorKind::Refused"));
        assert!(!text.contains("E_OK"));
    }
}
