//! The TypeScript target.
//!
//! TypeScript has no separate declaration step in this repository: the library under `data/js/gnoblin-core` and
//! `data/js/gnoblin-std` is TypeScript source that the shell strips and runs, and `gnoblinctl types init` hands the same
//! files to an editor, so a generated `.ts` file is both the declaration and the code. What is generated:
//!
//! - `data/js/gnoblin-core/generated/schema.ts`: the catalog hash, an interface of properties, of methods and of signals for
//!   every node class (`WindowProperties`, `WindowMethods`, `WindowSignals`, each extending its base class's), and
//!   `TREE_CLASSES`, the same facts as a table a script can read at run time (kind, read-only, animated, default);
//! - `data/js/gnoblin-core/generated/abi.ts`: the core ABI's constants as `as const` objects with their value types;
//! - `data/js/gnoblin-std/generated/`: the std library's constants, records, callback types and classes over the native
//!   `__gnoblinStd` object (see `std_ts`).
//!
//! The hand-written part is the ergonomic layer on top (`Client`, `Collection` listeners and the rest of `gnoblin-std`'s
//! public API), which is the subject of the migration plan in `design/modular-framework.md` section 25.

use std::collections::BTreeSet;

use gnoblin_schema::{DefaultValue, ObjectType, Property, Type};

use super::model::{ConstGroup, SdkSpec};
use super::text::{CodeWriter, pascal, ts_ident};
use super::{Comment, File, Target};

/// The TypeScript SDK's generated modules.
pub struct TypeScriptTarget;

const CORE_DIR: &str = "data/js/gnoblin-core/generated";
const STD_DIR: &str = "data/js/gnoblin-std/generated";

impl Target for TypeScriptTarget {
    fn name(&self) -> &'static str {
        "typescript"
    }

    fn owned_dirs(&self) -> &'static [&'static str] {
        &[CORE_DIR, STD_DIR]
    }

    fn generate(&self, spec: &SdkSpec) -> Result<Vec<File>, String> {
        let file = |dir: &str, name: &str, text: String| File {
            path: format!("{dir}/{name}"),
            comment: Some(Comment::Slashes),
            text,
        };
        let mut files = vec![
            file(CORE_DIR, "schema.ts", schema(spec)),
            file(CORE_DIR, "abi.ts", abi(spec)),
            file(CORE_DIR, "index.ts", core_index()),
        ];
        files.extend(super::typescript_std::files(spec)?);
        Ok(files)
    }
}

fn core_index() -> String {
    let mut code = CodeWriter::new("    ");
    code.line("export * from \"gnoblin-core/generated/abi\";");
    code.line("export * from \"gnoblin-core/generated/schema\";");
    code.finish()
}

// ---------------------------------------------------------------------------------------------------------------------
// types

/// The TypeScript type of a schema type. A reference to a node class is a handle; a reference to anything else is a record.
pub fn ts_type(ty: &Type, nodes: &BTreeSet<String>) -> String {
    match ty {
        Type::Bool => "boolean".to_owned(),
        Type::Int | Type::Uint | Type::Double => "number".to_owned(),
        Type::Text => "string".to_owned(),
        Type::Bytes => "Uint8Array".to_owned(),
        Type::ObjectRef(name) if nodes.contains(name) => "NodeHandle | null".to_owned(),
        Type::ObjectRef(_) => "Record<string, unknown>".to_owned(),
        Type::List(inner) => match inner.as_ref() {
            Type::ObjectRef(name) if nodes.contains(name) => "NodeHandle[]".to_owned(),
            Type::ObjectRef(_) => "Record<string, unknown>[]".to_owned(),
            other => format!("{}[]", ts_type(other, nodes)),
        },
    }
}

/// An interface of `members` that extends `base`. One with nothing to add is an alias of its base (or of the empty object):
/// an empty interface is what `@typescript-eslint/no-empty-object-type` exists to reject.
fn interface(code: &mut CodeWriter, doc: &str, name: &str, base: Option<String>, members: &[String]) {
    code.blank();
    code.line(format!("/** {doc} */"));
    if members.is_empty() {
        code.line(format!(
            "export type {name} = {};",
            base.unwrap_or_else(|| "Record<never, never>".to_owned())
        ));
        return;
    }
    let extends = base.map(|base| format!(" extends {base}")).unwrap_or_default();
    code.block(
        format!("export interface {name}{extends} {{"),
        |code| {
            for member in members {
                code.line(member);
            }
        },
        "}",
    );
}

/// The schema's own spelling of a type, as the run-time table records it.
fn kind_name(ty: &Type) -> String {
    ty.to_string()
}

fn default_literal(value: &DefaultValue) -> String {
    match value {
        DefaultValue::Bool(value) => value.to_string(),
        DefaultValue::Int(value) => value.to_string(),
        DefaultValue::Uint(value) => value.to_string(),
        DefaultValue::Double(value) => {
            if value.fract() == 0.0 && value.is_finite() {
                format!("{value:.1}").trim_end_matches(".0").to_owned()
            } else {
                format!("{value}")
            }
        }
        DefaultValue::Text(value) => format!("{value:?}"),
        DefaultValue::List(items) => format!("[{}]", items.iter().map(default_literal).collect::<Vec<_>>().join(", ")),
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// schema.ts

/// The members a class adds to its base's.
fn own_properties<'a>(object: &'a ObjectType, base: Option<&ObjectType>) -> Vec<&'a Property> {
    object
        .properties
        .iter()
        .filter(|property| !property.is_sensitive())
        .filter(|property| {
            base.is_none_or(|base| !base.properties.iter().any(|inherited| inherited.name == property.name))
        })
        .collect()
}

fn schema(spec: &SdkSpec) -> String {
    let objects = spec.node_classes(true);
    let nodes = node_class_names(spec);
    let mut code = CodeWriter::new("    ");
    code.line("// What the schema says about the nodes of the compositor's tree, as types and as a table.");
    code.blank();
    code.line("/** A node of the tree, by id. */");
    code.line("export type NodeHandle = number;");
    code.blank();
    code.line("/** A fingerprint of the catalog this file was generated from: `gnoblin_catalog_hash` of a core built from the same schema. */");
    code.line(format!("export const CATALOG_HASH = \"0x{:016x}\";", spec.catalog_hash));
    code.blank();
    code.line("/** One property of a class, as the schema declares it. */");
    code.block(
        "export interface PropertyInfo {",
        |code| {
            code.line("/** The schema's type: `bool`, `int`, `uint`, `double`, `string`, `bytes`, `object:<class>` or `list<...>`. */");
            code.line("readonly type: string;");
            code.line("readonly readonly: boolean;");
            code.line("readonly animated: boolean;");
            code.line("/** The value before any write, when the schema gives one. */");
            code.line("readonly default?: unknown;");
        },
        "}",
    );
    for object in &objects {
        let base = object
            .base
            .as_deref()
            .and_then(|name| objects.iter().find(|other| other.name == name));
        let class = pascal(&object.name);
        let base_name = |suffix: &str| base.map(|base| format!("{}{suffix}", pascal(&base.name)));
        let properties: Vec<String> = own_properties(object, base)
            .into_iter()
            .map(|property| {
                let modifier = if property.readonly { "readonly " } else { "" };
                format!(
                    "{modifier}{}: {};",
                    member(&property.name),
                    ts_type(&property.ty, &nodes)
                )
            })
            .collect();
        interface(
            &mut code,
            &format!("The properties of a `{}` node.", object.name),
            &format!("{class}Properties"),
            base_name("Properties"),
            &properties,
        );
        let methods: Vec<String> = object
            .methods
            .iter()
            .filter(|method| {
                !(method.function
                    || method.is_sensitive()
                    || base.is_some_and(|base| base.methods.iter().any(|inherited| inherited.name == method.name)))
            })
            .map(|method| {
                let params: Vec<String> = method
                    .params
                    .iter()
                    .map(|param| {
                        let optional = if param.optional { "?" } else { "" };
                        format!("{}{optional}: {}", ts_ident(&param.name), ts_type(&param.ty, &nodes))
                    })
                    .collect();
                format!("{}({}): void;", member(&method.name), params.join(", "))
            })
            .collect();
        interface(
            &mut code,
            &format!("The methods of a `{}` node.", object.name),
            &format!("{class}Methods"),
            base_name("Methods"),
            &methods,
        );
        let signals: Vec<String> = object
            .signals
            .iter()
            .filter(|signal| {
                !(signal.is_sensitive()
                    || base.is_some_and(|base| base.signals.iter().any(|inherited| inherited.name == signal.name)))
            })
            .map(|signal| {
                let fields: Vec<String> = signal
                    .fields
                    .iter()
                    .map(|field| format!("{}: {}", ts_ident(&field.name), ts_type(&field.ty, &nodes)))
                    .collect();
                format!("{}: [{}];", member(&signal.name), fields.join(", "))
            })
            .collect();
        interface(
            &mut code,
            &format!("The signals of a `{}` node: each name and its payload.", object.name),
            &format!("{class}Signals"),
            base_name("Signals"),
            &signals,
        );
    }
    code.blank();
    code.line("/** Every node class by its schema name. */");
    code.block(
        "export interface TreeClasses {",
        |code| {
            for object in &objects {
                let class = pascal(&object.name);
                code.line(format!(
                    "{}: {{ properties: {class}Properties; methods: {class}Methods; signals: {class}Signals }};",
                    member(&object.name)
                ));
            }
        },
        "}",
    );
    code.blank();
    code.line("/** The schema name of a node class. */");
    code.line("export type TreeClassName = keyof TreeClasses;");
    code.blank();
    code.line("/** Every node class: its base and the property table, so a script can ask what a class has without a node of it. */");
    code.block(
        "export const TREE_CLASSES: { readonly [Name in TreeClassName]: { readonly base: TreeClassName | null; readonly properties: { readonly [property: string]: PropertyInfo } } } = {",
        |code| {
            for object in &objects {
                let base = object
                    .base
                    .as_deref()
                    .map_or_else(|| "null".to_owned(), |name| format!("{name:?}"));
                code.line(format!("{}: {{", member(&object.name)));
                code.indent();
                code.line(format!("base: {base},"));
                code.line("properties: {");
                code.indent();
                for property in object.properties.iter().filter(|property| !property.is_sensitive()) {
                    let mut entry = format!(
                        "type: {:?}, readonly: {}, animated: {}",
                        kind_name(&property.ty),
                        property.readonly,
                        property.animated
                    );
                    if let Some(default) = &property.default {
                        entry.push_str(&format!(", default: {}", default_literal(default)));
                    }
                    code.line(format!("{}: {{ {entry} }},", member(&property.name)));
                }
                code.dedent();
                code.line("},");
                code.dedent();
                code.line("},");
            }
        },
        "};",
    );
    code.finish()
}

/// A property or method name as an object key or interface member: quoted only when it is not an identifier.
fn member(name: &str) -> String {
    let identifier = name
        .chars()
        .enumerate()
        .all(|(index, c)| c == '_' || c == '$' || c.is_ascii_alphabetic() || (index > 0 && c.is_ascii_digit()));
    if identifier {
        name.to_owned()
    } else {
        format!("{name:?}")
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// abi.ts

/// One constant group as an `as const` object and the type of its values.
pub fn constants(code: &mut CodeWriter, group: &ConstGroup) {
    let name = group.type_name();
    code.blank();
    code.line(format!("/** `{}*` constants of the ABI. */", group.prefix));
    code.block(
        format!("export const {name} = {{"),
        |code| {
            for member in &group.members {
                if let Some(first) = member.doc.first() {
                    code.line(format!("/** {} */", first.replace("*/", "* /")));
                }
                code.line(format!("{}: {},", member_key(&member.short), member.value));
            }
        },
        "} as const;",
    );
    code.line(format!("export type {name} = (typeof {name})[keyof typeof {name}];"));
}

fn member_key(short: &str) -> String {
    if short.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("_{short}")
    } else {
        short.to_owned()
    }
}

fn abi(spec: &SdkSpec) -> String {
    let core = spec.core();
    let mut code = CodeWriter::new("    ");
    code.line("// The core ABI's constants. A value read from the core or written to it is one of these.");
    code.blank();
    code.line(format!("export const ABI_MAJOR = {};", core.abi.0));
    code.line(format!("export const ABI_MINOR = {};", core.abi.1));
    code.line(format!("export const SONAME = {:?};", core.soname));
    for group in &core.groups {
        constants(&mut code, group);
    }
    if !core.loose.is_empty() {
        code.blank();
        for constant in &core.loose {
            code.line(format!("export const {} = {};", constant.name, constant.value));
        }
    }
    code.finish()
}

/// Names of the node classes as a set, for target code that needs to know whether a reference is to one.
pub fn node_class_names(spec: &SdkSpec) -> BTreeSet<String> {
    spec.node_classes(true)
        .iter()
        .map(|object| object.name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn spec() -> SdkSpec {
        SdkSpec::load(Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("workspace root")).expect("spec loads")
    }

    #[test]
    fn classes_extend_their_bases_and_declare_only_what_they_add() {
        let text = schema(&spec());
        assert!(text.contains("export interface NodeProperties {"));
        assert!(text.contains("export interface WindowProperties extends NodeProperties {"));
        assert!(text.contains("export interface ScreenProperties extends ViewProperties {"));
        assert!(text.contains("readonly title: string;"));
        assert!(text.contains("maximised: boolean;"));
        assert!(text.contains("close(): void;"));
    }

    #[test]
    fn the_run_time_table_names_every_class() {
        let spec = spec();
        let text = schema(&spec);
        for object in spec.node_classes(true) {
            assert!(
                text.contains(&format!("{}: {{ properties:", member(&object.name))),
                "{}",
                object.name
            );
        }
        assert!(text.contains("title: { type: \"string\", readonly: true, animated: false"));
    }

    #[test]
    fn abi_constants_are_const_objects_with_a_value_type() {
        let text = abi(&spec());
        assert!(text.contains("export const ErrorKind = {"));
        assert!(text.contains("NO_SUCH_NODE: -8,"));
        assert!(text.contains("export type ErrorKind = (typeof ErrorKind)[keyof typeof ErrorKind];"));
    }
}
