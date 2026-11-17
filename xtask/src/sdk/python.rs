//! The Python target.
//!
//! Python is a scripting language over the same two tables, so what it needs generated is the typed surface, not a binding
//! layer in Python: the native module (`gnoblin._core`, written on the Rust SDK) moves values by name, and the classes here
//! say which names exist and what they hold.
//!
//! - `classes.py` / `classes.pyi`: one class per node class of the schema (`Window`, `Seat`, ...), derived from `Node` the way
//!   the schema derives them, with a `Property`, `Method` or `Signal` descriptor for every member the class adds. The `.pyi`
//!   gives a type checker the real attribute types (`window.title` is a `str`, `window.maximised` assigns a `bool`).
//! - `abi.py`: the ABI's constants as `IntEnum` and `IntFlag`, and one exception class per `E_*` code.
//! - `std.py`: the std library's classes, records and constants, over the native `gnoblin._std` module.
//!
//! The generic descriptors (`Node`, `Property`, `Method`, `Signal`) are hand-written in `data/python/gnoblin/_base.py`: they are
//! the part that talks to the runtime, and they are the same for every class.

use std::collections::BTreeSet;

use gnoblin_schema::{Method, ObjectType, Property, Type};

use super::model::{Const, ConstGroup, SdkSpec};
use super::text::{CodeWriter, pascal, python_ident};
use super::{Comment, File, Target};

/// The Python SDK's generated package.
pub struct PythonTarget;

const DIR: &str = "data/python/gnoblin/_generated";

impl Target for PythonTarget {
    fn name(&self) -> &'static str {
        "python"
    }

    fn owned_dirs(&self) -> &'static [&'static str] {
        &[DIR]
    }

    fn generate(&self, spec: &SdkSpec) -> Result<Vec<File>, String> {
        let python = |name: &str, text: String| File {
            path: format!("{DIR}/{name}"),
            comment: Some(Comment::Hash),
            text,
        };
        let mut files = vec![
            python("__init__.py", init()),
            python("classes.py", classes(spec, false)),
            python("classes.pyi", classes(spec, true)),
            python("abi.py", abi(spec)),
        ];
        files.extend(super::python_std::files(spec)?);
        Ok(files)
    }
}

/// The Python name of a node class: `node` is `TreeNode`, so it does not hide the runtime's own `Node`, the handle every class
/// of the tree derives from (an unknown class is a plain `Node`).
fn py_class(name: &str) -> String {
    if name == "node" {
        "TreeNode".to_owned()
    } else {
        pascal(name)
    }
}

fn init() -> String {
    let mut code = CodeWriter::new("    ");
    code.line(
        "\"\"\"Written by `cargo xtask gen-sdks`: the typed classes, the ABI constants and the std bindings.\"\"\"",
    );
    code.finish()
}

// ---------------------------------------------------------------------------------------------------------------------
// types

/// A Python annotation for a schema type, as source text. A reference to a node class is the class's name (a forward
/// reference, so it is quoted where it appears in a runtime expression).
pub fn annotation(ty: &Type, node_classes: &BTreeSet<String>) -> String {
    match ty {
        Type::Bool => "bool".to_owned(),
        Type::Int | Type::Uint => "int".to_owned(),
        Type::Double => "float".to_owned(),
        Type::Text => "str".to_owned(),
        Type::Bytes => "bytes".to_owned(),
        Type::ObjectRef(name) if node_classes.contains(name) => format!("{} | None", py_class(name)),
        Type::ObjectRef(_) => "dict[str, Any]".to_owned(),
        Type::List(inner) => match inner.as_ref() {
            Type::ObjectRef(name) if node_classes.contains(name) => format!("list[{}]", py_class(name)),
            Type::ObjectRef(_) => "list[dict[str, Any]]".to_owned(),
            other => format!("list[{}]", annotation(other, node_classes)),
        },
    }
}

/// The type argument of a descriptor: a builtin class where there is one, a string otherwise.
fn descriptor_type(ty: &Type, node_classes: &BTreeSet<String>) -> String {
    match ty {
        Type::Bool | Type::Int | Type::Uint | Type::Double | Type::Text | Type::Bytes => annotation(ty, node_classes),
        other => format!("{:?}", annotation(other, node_classes)),
    }
}

fn node_class_names(spec: &SdkSpec) -> BTreeSet<String> {
    spec.node_classes(true)
        .iter()
        .map(|object| object.name.clone())
        .collect()
}

// ---------------------------------------------------------------------------------------------------------------------
// classes.py and classes.pyi

/// The members a class adds to its base: the base's are inherited in Python as they are in the schema.
struct Own<'a> {
    properties: Vec<&'a Property>,
    methods: Vec<&'a Method>,
    signals: Vec<&'a gnoblin_schema::Signal>,
}

fn own<'a>(object: &'a ObjectType, base: Option<&ObjectType>) -> Own<'a> {
    let inherited = |name: &str, kind: char| match (base, kind) {
        (Some(base), 'p') => base.properties.iter().any(|member| member.name == name),
        (Some(base), 'm') => base.methods.iter().any(|member| member.name == name),
        (Some(base), _) => base.signals.iter().any(|member| member.name == name),
        (None, _) => false,
    };
    Own {
        properties: object
            .properties
            .iter()
            .filter(|property| !property.is_sensitive() && !inherited(&property.name, 'p'))
            .collect(),
        methods: object
            .methods
            .iter()
            .filter(|method| !method.function && !method.is_sensitive() && !inherited(&method.name, 'm'))
            .collect(),
        signals: object
            .signals
            .iter()
            .filter(|signal| !signal.is_sensitive() && !inherited(&signal.name, 's'))
            .collect(),
    }
}

fn classes(spec: &SdkSpec, stub: bool) -> String {
    let node_classes = node_class_names(spec);
    let objects = spec.node_classes(true);
    let mut code = CodeWriter::new("    ");
    if stub {
        code.line("from typing import Any, Callable");
        code.blank();
        code.line("from .._base import Method, Node as _Node, Property, Signal");
        code.blank();
        code.line("CATALOG_HASH: int");
        code.line("__all__: list[str]");
    } else {
        code.line(
            "\"\"\"One class per node class of the schema, with a descriptor for every member the class adds.\"\"\"",
        );
        code.blank();
        code.line("from __future__ import annotations");
        code.blank();
        code.line("from typing import Any");
        code.blank();
        code.line("from .._base import Method, Node as _Node, Property, Signal");
        code.blank();
        code.line(format!("CATALOG_HASH = 0x{:016x}", spec.catalog_hash));
        let names: Vec<String> = objects
            .iter()
            .map(|object| format!("{:?}", py_class(&object.name)))
            .collect();
        code.line(format!("__all__ = [\"CATALOG_HASH\", {}]", names.join(", ")));
    }
    for object in &objects {
        let base = object
            .base
            .as_deref()
            .and_then(|name| objects.iter().find(|other| other.name == name));
        code.blank();
        class(&mut code, object, base, &node_classes, stub);
    }
    code.finish()
}

fn class(
    code: &mut CodeWriter,
    object: &ObjectType,
    base: Option<&ObjectType>,
    node_classes: &BTreeSet<String>,
    stub: bool,
) {
    let members = own(object, base);
    let name = py_class(&object.name);
    let parent = base.map_or_else(|| "_Node".to_owned(), |base| py_class(&base.name));
    code.blank();
    code.line(format!("class {name}({parent}):"));
    code.indent();
    if !stub {
        code.line(format!("\"\"\"A node of class `{}`.\"\"\"", object.name));
        code.blank();
        code.line(format!("__class_name__ = {:?}", object.name));
    }
    let mut empty = true;
    for property in &members.properties {
        empty = false;
        let attribute = python_ident(&property.name);
        if stub {
            let ty = annotation(&property.ty, node_classes);
            code.line("@property");
            code.line(format!("def {attribute}(self) -> {ty}: ..."));
            if !property.readonly {
                code.line(format!("@{attribute}.setter"));
                code.line(format!("def {attribute}(self, value: {ty}) -> None: ..."));
            }
        } else {
            let mut arguments = vec![
                format!("{:?}", property.name),
                descriptor_type(&property.ty, node_classes),
            ];
            if property.readonly {
                arguments.push("readonly=True".to_owned());
            }
            if property.animated {
                arguments.push("animated=True".to_owned());
            }
            code.line(format!("{attribute} = Property({})", arguments.join(", ")));
        }
    }
    for method in &members.methods {
        empty = false;
        let attribute = python_ident(&method.name);
        if stub {
            let mut parameters = String::from("self");
            for param in &method.params {
                let ty = annotation(&param.ty, node_classes);
                if param.optional {
                    parameters.push_str(&format!(", {}: {ty} | None = None", python_ident(&param.name)));
                } else {
                    parameters.push_str(&format!(", {}: {ty}", python_ident(&param.name)));
                }
            }
            parameters.push_str(", *, on_answer: Callable[..., None] | None = None");
            code.line(format!("def {attribute}({parameters}) -> None: ..."));
        } else {
            let params: Vec<String> = method
                .params
                .iter()
                .map(|param| {
                    let ty = descriptor_type(&param.ty, node_classes);
                    if param.optional {
                        format!("({:?}, {ty}, True)", param.name)
                    } else {
                        format!("({:?}, {ty})", param.name)
                    }
                })
                .collect();
            code.line(format!(
                "{attribute} = Method({:?}, ({}))",
                method.name,
                tuple_items(&params)
            ));
        }
    }
    for signal in &members.signals {
        empty = false;
        let attribute = python_ident(&signal.name);
        if stub {
            code.line(format!("{attribute}: Signal"));
        } else {
            let fields: Vec<String> = signal
                .fields
                .iter()
                .map(|field| format!("({:?}, {})", field.name, descriptor_type(&field.ty, node_classes)))
                .collect();
            code.line(format!(
                "{attribute} = Signal({:?}, ({}))",
                signal.name,
                tuple_items(&fields)
            ));
        }
    }
    if empty && stub {
        code.line("...");
    }
    code.dedent();
}

/// The inside of a Python tuple of `items`: a one-element tuple needs its trailing comma.
fn tuple_items(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [only] => format!("{only},"),
        many => many.join(", "),
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// abi.py

/// The Python name of a group's members: the constant's short name, which is already an upper-case identifier.
fn member_name(constant: &Const) -> String {
    if constant.short.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("_{}", constant.short)
    } else {
        constant.short.clone()
    }
}

/// One `IntEnum` or `IntFlag` for a constant group.
pub fn enumeration(code: &mut CodeWriter, group: &ConstGroup, name: &str) {
    let base = if group.flags { "enum.IntFlag" } else { "enum.IntEnum" };
    code.blank();
    code.blank();
    code.line(format!("class {name}({base}):"));
    code.indent();
    code.line(format!("\"\"\"`{}*` constants of the ABI.\"\"\"", group.prefix));
    code.blank();
    for member in &group.members {
        code.line(format!("{} = {}", member_name(member), member.value));
        if let Some(first) = member.doc.first() {
            code.line(format!("\"\"\"{}\"\"\"", first.replace('"', "'").replace('\\', "/")));
        }
    }
    code.dedent();
}

fn abi(spec: &SdkSpec) -> String {
    let core = spec.core();
    let mut code = CodeWriter::new("    ");
    code.line("\"\"\"The core ABI's constants. The exception classes are the runtime's own (`gnoblin.errors`).\"\"\"");
    code.blank();
    code.line("from __future__ import annotations");
    code.blank();
    code.line("import enum");
    code.blank();
    code.line(format!("CATALOG_HASH = 0x{:016x}", spec.catalog_hash));
    code.line(format!("ABI_MAJOR = {}", core.abi.0));
    code.line(format!("ABI_MINOR = {}", core.abi.1));
    code.line(format!("SONAME = {:?}", core.soname));
    for group in &core.groups {
        enumeration(&mut code, group, &group.type_name());
    }
    if !core.loose.is_empty() {
        code.blank();
        for constant in &core.loose {
            code.line(format!("{} = {}", constant.name, constant.value));
        }
    }
    code.finish()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn spec() -> SdkSpec {
        SdkSpec::load(Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("workspace root")).expect("spec loads")
    }

    fn find<'a>(files: &'a [File], name: &str) -> &'a str {
        &files.iter().find(|file| file.path.ends_with(name)).expect("file").text
    }

    #[test]
    fn classes_derive_the_way_the_schema_does() {
        let spec = spec();
        let text = classes(&spec, false);
        assert!(text.contains("class TreeNode(_Node):"));
        assert!(text.contains("class Window(TreeNode):"));
        assert!(text.contains("class Screen(View):"), "screen derives from view");
        let view = text.find("class View(").expect("View");
        let screen = text.find("class Screen(").expect("Screen");
        assert!(view < screen, "a base class is defined before its subclass");
        assert!(text.contains("title = Property(\"title\", str, readonly=True)"));
        assert!(text.contains("x = Property(\"x\", float, animated=True)"));
        assert!(text.contains("parent = Property(\"parent\", \"TreeNode | None\", readonly=True)"));
    }

    #[test]
    fn a_derived_class_adds_only_what_it_does_not_inherit() {
        let spec = spec();
        let text = classes(&spec, false);
        let window = &text[text.find("class Window(").expect("Window")..];
        let window = &window[..window.find("\nclass ").unwrap_or(window.len())];
        assert!(!window.contains("Property(\"opacity\""), "opacity is Node's");
        assert!(window.contains("Property(\"maximised\""));
        assert!(window.contains("Method(\"close\""));
    }

    #[test]
    fn the_stub_types_each_property() {
        let spec = spec();
        let text = classes(&spec, true);
        assert!(text.contains("def title(self) -> str: ..."));
        assert!(text.contains("@maximised.setter"));
        assert!(!text.contains("@title.setter"), "title is read-only");
    }

    #[test]
    fn abi_has_an_enumeration_per_constant_group() {
        let spec = spec();
        let files = PythonTarget.generate(&spec).expect("renders");
        let abi = find(&files, "abi.py");
        assert!(abi.contains("class ErrorKind(enum.IntEnum):"));
        assert!(abi.contains("NO_SUCH_NODE = -8"));
        assert!(abi.contains("ERR = -1"));
    }
}
