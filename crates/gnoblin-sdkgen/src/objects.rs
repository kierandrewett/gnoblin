//! Target `objects`: the catalog of each namespace a library serves, as the static tables the library is built from.
//!
//! For `gnoblin.model` and `gnoblin.std` this writes `rust-objects/{model,std}_catalog.rs`: a `CATALOG` (`gnoblin_object::catalog`) whose
//! classes are ordered by name, whose members are a base class's first (so a base member has the same id in every derived class), and
//! whose enums are classes of kind `CLASS_ENUM` with one `MEMBER_VALUE` member per value. `CATALOG_HASH` is the namespace's
//! fingerprint. A library includes the file for its namespace and implements the classes it names.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use gnoblin_schema::{Catalog, DefaultValue, Method, Namespace, ObjectKind, ObjectType, Property, Signal, Type};

use super::{Comment, File, SdkSpec, Target};

/// Writes the static catalogs.
pub struct ObjectsTarget;

impl Target for ObjectsTarget {
    fn name(&self) -> &'static str {
        "objects"
    }

    fn generate(&self, spec: &SdkSpec) -> Result<Vec<File>, String> {
        let mut files = Vec::new();
        for namespace in spec.objects.namespaces() {
            let stem = match namespace.as_str() {
                "gnoblin.model" => "model",
                "gnoblin.std" => "std",
                other => return Err(format!("no library serves the namespace `{other}`")),
            };
            files.push(File {
                path: format!("rust-objects/{stem}_catalog.rs"),
                comment: Some(Comment::Slashes),
                text: render(&spec.objects, &namespace)?,
            });
        }
        Ok(files)
    }
}

/// One member of a class, before it is written.
enum Member<'a> {
    Property(&'a Property),
    Method(&'a Method),
    Signal(&'a Signal),
}

impl Member<'_> {
    fn name(&self) -> &str {
        match self {
            Member::Property(property) => &property.name,
            Member::Method(method) => &method.name,
            Member::Signal(signal) => &signal.name,
        }
    }
}

/// The members of `object` as ids number them: its base class's first, then its own.
fn flatten<'a>(by_name: &BTreeMap<&str, &'a ObjectType>, object: &'a ObjectType) -> Vec<Member<'a>> {
    let mut list = match object.base.as_deref() {
        Some(base) if !base.contains('.') => by_name.get(base).map(|base| flatten(by_name, base)).unwrap_or_default(),
        _ => Vec::new(),
    };
    let known: BTreeSet<String> = list.iter().map(|member| member.name().to_owned()).collect();
    list.extend(
        object
            .properties
            .iter()
            .filter(|property| !known.contains(&property.name))
            .map(Member::Property),
    );
    list.extend(
        object
            .methods
            .iter()
            .filter(|method| !known.contains(&method.name))
            .map(Member::Method),
    );
    list.extend(
        object
            .signals
            .iter()
            .filter(|signal| !known.contains(&signal.name))
            .map(Member::Signal),
    );
    list
}

/// The `TypeDef` statics of a catalog, one per distinct type.
struct Types<'a> {
    catalog: &'a Catalog,
    namespace: &'a Namespace,
    statics: Vec<String>,
    index: BTreeMap<String, usize>,
}

impl Types<'_> {
    /// The index of the static for `ty`, writing it (and its element type) if it is new.
    fn of(&mut self, ty: &Type, secret: bool) -> usize {
        let key = format!("{ty}{}", if secret { "!secret" } else { "" });
        if let Some(index) = self.index.get(&key) {
            return *index;
        }
        let (kind, class, elem) = match ty {
            Type::Bool => ("VALUE_BOOL", String::new(), None),
            Type::Int => ("VALUE_INT", String::new(), None),
            Type::Uint => ("VALUE_UINT", String::new(), None),
            Type::Double => ("VALUE_DOUBLE", String::new(), None),
            Type::Text if secret => ("VALUE_SECRET", String::new(), None),
            Type::Text => ("VALUE_TEXT", String::new(), None),
            Type::Bytes => ("VALUE_BYTES", String::new(), None),
            Type::ObjectRef(name) => {
                let (home, bare) = name
                    .rsplit_once('.')
                    .unwrap_or((self.namespace.as_str(), name.as_str()));
                let record = Namespace::parse(home).ok().is_some_and(|home| {
                    self.catalog
                        .objects(&home)
                        .any(|object| object.name == bare && object.record)
                });
                (if record { "VALUE_RECORD" } else { "VALUE_OBJECT" }, name.clone(), None)
            }
            Type::Enum(name) => ("VALUE_ENUM", name.clone(), None),
            Type::Callback(name) => ("VALUE_CLOSURE", name.clone(), None),
            Type::List(inner) => ("VALUE_LIST", String::new(), Some(self.of(inner, false))),
        };
        let index = self.statics.len();
        let elem = elem.map_or_else(|| "None".to_owned(), |elem| format!("Some(&T{elem})"));
        self.statics.push(format!(
            "static T{index}: TypeDef = TypeDef {{ kind: abi::{kind}, class: {class:?}, elem: {elem} }};"
        ));
        self.index.insert(key, index);
        index
    }

    fn none(&mut self) -> usize {
        self.of_none()
    }

    fn of_none(&mut self) -> usize {
        if let Some(index) = self.index.get("none") {
            return *index;
        }
        let index = self.statics.len();
        self.statics
            .push(format!("static T{index}: TypeDef = TypeDef::plain(abi::VALUE_NONE);"));
        self.index.insert("none".to_owned(), index);
        index
    }
}

fn default_source(default: Option<&DefaultValue>, ty: &Type) -> String {
    match (default, ty) {
        (Some(DefaultValue::Bool(value)), _) => format!("DefaultDef::Bool({value})"),
        (Some(DefaultValue::Int(value)), _) => format!("DefaultDef::Int({value})"),
        (Some(DefaultValue::Uint(value)), _) => format!("DefaultDef::Uint({value})"),
        (Some(DefaultValue::Double(value)), _) => format!("DefaultDef::Double({value:?}_f64)"),
        (Some(DefaultValue::Text(name)), Type::Enum(_)) => format!("DefaultDef::Enum({name:?})"),
        (Some(DefaultValue::Text(value)), _) => format!("DefaultDef::Text({value:?})"),
        _ => "DefaultDef::None".to_owned(),
    }
}

fn args_source(types: &mut Types<'_>, fields: &[gnoblin_schema::Field]) -> String {
    let mut args = Vec::new();
    let mut kinds = Vec::new();
    for field in fields {
        let index = types.of(&field.ty, field.secret);
        args.push(format!(
            "ArgDef {{ name: {:?}, ty: &T{index}, optional: {} }}",
            field.name, field.optional
        ));
        kinds.push(format!("T{index}.kind"));
    }
    let names: Vec<&str> = fields.iter().map(|field| field.name.as_str()).collect();
    format!(
        "args: &[{}], arg_kinds: &[{}], arg_names: {:?}",
        args.join(", "),
        kinds.join(", "),
        names.join(" ")
    )
}

fn member_source(types: &mut Types<'_>, member: &Member<'_>) -> String {
    match member {
        Member::Property(property) => {
            let ty = types.of(&property.ty, property.secret);
            let mut flags = Vec::new();
            if property.animated {
                flags.push("abi::MEMBER_ANIMATED");
            }
            if property.readonly {
                flags.push("abi::MEMBER_READONLY");
            }
            if property.secret {
                flags.push("abi::MEMBER_SECRET");
            }
            if matches!(property.ty, Type::Callback(_)) {
                flags.push("abi::MEMBER_CALLBACK");
            }
            if property.extras.inert.is_some() {
                flags.push("abi::MEMBER_INERT");
            }
            format!(
                "MemberDef {{ name: {:?}, kind: abi::MEMBER_PROPERTY, flags: {}, ty: &T{ty}, args: &[], arg_kinds: &[], arg_names: \"\", default: {}, abi: {:?} }}",
                property.name,
                flags_source(&flags),
                default_source(property.default.as_ref(), &property.ty),
                property.extras.abi.as_deref().unwrap_or("")
            )
        }
        Member::Method(method) => {
            let ty = match &method.returns {
                Some(returns) => types.of(returns, false),
                None => types.none(),
            };
            let mut flags = Vec::new();
            if method.function {
                flags.push("abi::MEMBER_STATIC");
            }
            if method.required {
                flags.push("abi::MEMBER_REQUIRED");
            }
            if method.native_default {
                flags.push("abi::MEMBER_NATIVE_DEFAULT");
            }
            if method.secret {
                flags.push("abi::MEMBER_SECRET");
            }
            if method.extras.inert.is_some() {
                flags.push("abi::MEMBER_INERT");
            }
            let args = args_source(types, &method.params);
            format!(
                "MemberDef {{ name: {:?}, kind: abi::MEMBER_METHOD, flags: {}, ty: &T{ty}, {args}, default: DefaultDef::None, abi: {:?} }}",
                method.name,
                flags_source(&flags),
                method.extras.abi.as_deref().unwrap_or("")
            )
        }
        Member::Signal(signal) => {
            let ty = types.none();
            let flags = if signal.secret { "abi::MEMBER_SECRET" } else { "0" };
            let args = args_source(types, &signal.fields);
            format!(
                "MemberDef {{ name: {:?}, kind: abi::MEMBER_SIGNAL, flags: {flags}, ty: &T{ty}, {args}, default: DefaultDef::None, abi: {:?} }}",
                signal.name,
                signal.extras.abi.as_deref().unwrap_or("")
            )
        }
    }
}

fn flags_source(flags: &[&str]) -> String {
    if flags.is_empty() {
        "0".to_owned()
    } else {
        flags.join(" | ")
    }
}

fn render(catalog: &Catalog, namespace: &Namespace) -> Result<String, String> {
    let objects: BTreeMap<&str, &ObjectType> = catalog
        .objects(namespace)
        .map(|object| (object.name.as_str(), object))
        .collect();
    let enums: BTreeMap<&str, &gnoblin_schema::EnumDecl> = catalog
        .enums(namespace)
        .map(|declared| (declared.name.as_str(), declared))
        .collect();
    if let Some(name) = enums.keys().find(|name| objects.contains_key(*name)) {
        return Err(format!("`{name}` is both a class and an enum of `{namespace}`"));
    }
    let mut types = Types {
        catalog,
        namespace,
        statics: Vec::new(),
        index: BTreeMap::new(),
    };
    let mut classes: BTreeMap<String, String> = BTreeMap::new();
    for (name, object) in &objects {
        let kind = match object.kind {
            ObjectKind::Handle => "CLASS_HANDLE",
            ObjectKind::Record => "CLASS_RECORD",
            ObjectKind::Interface => "CLASS_INTERFACE",
            ObjectKind::Service => "CLASS_SERVICE",
        };
        let members: Vec<String> = flatten(&objects, object)
            .iter()
            .map(|member| member_source(&mut types, member))
            .collect();
        classes.insert(
            (*name).to_owned(),
            format!(
                "ClassDef {{ name: {name:?}, kind: abi::{kind}, flags: {}, base: {:?}, members: &[\n        {}\n    ] }}",
                if object.retained { "abi::CLASS_FLAG_RETAINED" } else { "0" },
                object.base.as_deref().unwrap_or(""),
                members.join(",\n        ")
            ),
        );
    }
    for (name, declared) in &enums {
        let none = types.none();
        let members: Vec<String> = declared
            .values
            .iter()
            .map(|value| {
                format!(
                    "MemberDef {{ name: {value:?}, kind: abi::MEMBER_VALUE, flags: 0, ty: &T{none}, args: &[], arg_kinds: &[], arg_names: \"\", default: DefaultDef::None, abi: \"\" }}"
                )
            })
            .collect();
        classes.insert(
            (*name).to_owned(),
            format!(
                "ClassDef {{ name: {name:?}, kind: abi::CLASS_ENUM, flags: 0, base: \"\", members: &[\n        {}\n    ] }}",
                members.join(",\n        ")
            ),
        );
    }
    let mut text = String::new();
    let _ = writeln!(
        text,
        "use gnoblin_core_abi as abi;\nuse gnoblin_object::catalog::{{ArgDef, CatalogDef, ClassDef, DefaultDef, MemberDef, TypeDef}};\n"
    );
    let _ = writeln!(
        text,
        "/// The hash of the `{namespace}` catalog.\npub const CATALOG_HASH: u64 = {:#018x};\n",
        catalog.namespace_fingerprint(namespace)
    );
    for source in &types.statics {
        let _ = writeln!(text, "{source}");
    }
    let _ = writeln!(
        text,
        "\n/// The catalog of `{namespace}`.\npub static CATALOG: CatalogDef = CatalogDef {{\n    namespace: {:?},\n    hash: CATALOG_HASH,\n    classes: &[\n    {}\n    ],\n}};",
        namespace.as_str(),
        classes.into_values().collect::<Vec<_>>().join(",\n    ")
    );
    Ok(text)
}
