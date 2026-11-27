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
            let stem = stem_of(&namespace)?;
            files.push(File {
                path: format!("rust-objects/{stem}_catalog.rs"),
                comment: Some(Comment::Slashes),
                text: render(&spec.objects, &namespace)?,
            });
        }
        Ok(files)
    }
}

/// The file stem of the namespace a library serves, which names its generated files (`rust-objects/<stem>_catalog.rs`).
pub(super) fn stem_of(namespace: &Namespace) -> Result<&'static str, String> {
    match namespace.as_str() {
        "gnoblin.model" => Ok("model"),
        "gnoblin.std" => Ok("std"),
        other => Err(format!("no library serves the namespace `{other}`")),
    }
}

/// The path of the crate module that holds the typed classes of a namespace, for a reference from another library's SDK.
pub(super) fn crate_path_of(namespace: &Namespace) -> Result<&'static str, String> {
    match namespace.as_str() {
        "gnoblin.model" => Ok("::gnoblin_core::objects"),
        other => Err(format!(
            "no SDK crate is named for the namespace `{other}`: a library may name only the core's classes"
        )),
    }
}

/// The handle class of another namespace that a type names (`object:gnoblin.model.client`): its namespace and its declaration.
///
/// A parameter, a result, a property, a record field, a signal field or a callback's argument may name a handle of another library's
/// catalog. Going in, the value is that library's handle and the library that takes it checks it against its own table; coming out, it is
/// a `VALUE_FOREIGN_OBJECT` the generated code reads through the other library's classes. A class that is not a handle is refused (a record
/// is a value, a service has no instance, an interface a script implements).
pub(super) fn foreign_handle<'a>(catalog: &'a Catalog, reference: &str) -> Result<(Namespace, &'a ObjectType), String> {
    let (home, bare) = reference
        .rsplit_once('.')
        .ok_or_else(|| format!("`{reference}` is not a class of another namespace"))?;
    let home = Namespace::parse(home).map_err(|_| format!("`{reference}`: `{home}` is not a namespace"))?;
    let class = catalog
        .schemas()
        .iter()
        .filter(|schema| schema.namespace == home)
        .flat_map(|schema| schema.objects.iter())
        .find(|candidate| candidate.name == bare)
        .ok_or_else(|| format!("`{reference}`: namespace `{home}` has no class `{bare}`"))?;
    if class.kind != ObjectKind::Handle {
        return Err(format!(
            "`{reference}` is a {} of another namespace: only a handle of another library can be named by a type of this one",
            class.kind.as_str()
        ));
    }
    Ok((home, class))
}

/// The classes of other namespaces that `namespace` names anywhere in its types, once each and in name order: what the generated module
/// must import before it can check a parameter against them.
pub(super) fn foreign_handles<'a>(
    catalog: &'a Catalog,
    namespace: &'a Namespace,
) -> Result<Vec<(Namespace, &'a ObjectType)>, String> {
    let mut named: BTreeSet<&str> = BTreeSet::new();
    let mut note = |ty: &'a Type| {
        named.extend(ty.referenced_objects().into_iter().filter(|name| name.contains('.')));
    };
    for object in catalog.objects(namespace) {
        for (_, ty) in object.typed_members() {
            note(ty);
        }
        for method in &object.methods {
            if let Some(returns) = &method.returns {
                note(returns);
            }
        }
    }
    for callback in catalog.callbacks(namespace) {
        for field in &callback.params {
            note(&field.ty);
        }
        if let Some(returns) = &callback.returns {
            note(returns);
        }
    }
    named
        .into_iter()
        .map(|reference| foreign_handle(catalog, reference))
        .collect()
}

/// The members of the base class of `object` when it belongs to another namespace, in the ids they have there, and that namespace.
/// A class of one library may extend a class the other serves; the derived catalog lists these members first.
pub(super) fn foreign_base<'a>(catalog: &'a Catalog, object: &ObjectType) -> Option<(Namespace, Vec<Member<'a>>)> {
    let (home, bare) = object.base.as_deref()?.rsplit_once('.')?;
    let home = Namespace::parse(home).ok()?;
    let by_name: BTreeMap<&'a str, &'a ObjectType> = catalog
        .schemas()
        .iter()
        .filter(|schema| schema.namespace == home)
        .flat_map(|schema| schema.objects.iter())
        .map(|candidate| (candidate.name.as_str(), candidate))
        .collect();
    let members = flatten(&by_name, by_name.get(bare)?);
    Some((home, members))
}

/// The kinds a service's `find` knows and the method that lists each, in the order the catalog declares them. A class that holds an
/// augment has no `find` of its own: its kinds are added to the `find` of the core class it extends ([`augment_finds`]).
pub(super) fn finds_of(object: &ObjectType) -> Vec<(&str, &gnoblin_schema::Method)> {
    if object.augment_of.is_some() {
        return Vec::new();
    }
    augment_finds(object)
}

/// The kinds `object`'s methods list with `find = "Kind"`, whatever the class: for a class that holds an augment, the kinds it adds to
/// the core class's `find`.
pub(super) fn augment_finds(object: &ObjectType) -> Vec<(&str, &gnoblin_schema::Method)> {
    object
        .methods
        .iter()
        .filter_map(|method| method.extras.find.as_deref().map(|kind| (kind, method)))
        .collect()
}

/// Whether `method` of a class that holds a receiver augment is the setter `set_<name>` of one of its writable properties, which a script
/// generator installs as that property's setter and not as a method.
pub(super) fn is_augment_setter(object: &ObjectType, method: &str) -> bool {
    method
        .strip_prefix("set_")
        .is_some_and(|property| object.augment_setters.iter().any(|name| name == property))
}

/// The checks a class with `assign = true` needs: no member of the class (or of the core class an augment extends) is named `set`, and
/// `set(props, options)` commits through the root's `request_transition`, which an augment of the core's `gnoblin` in the same namespace
/// must add.
pub(super) fn check_assign<'a>(
    object: &ObjectType,
    target: Option<&ObjectType>,
    mut objects: impl Iterator<Item = &'a ObjectType>,
) -> Result<(), String> {
    let named_set = |class: &ObjectType| {
        class.properties.iter().any(|property| property.name == "set")
            || class.methods.iter().any(|method| method.name == "set")
    };
    if named_set(object) || target.is_some_and(named_set) {
        return Err(format!(
            "`{}` has `assign = true` and a member named `set`, which `assign` adds",
            object.name
        ));
    }
    let requests = objects.any(|other| {
        other.augment_of.as_deref() == Some("gnoblin")
            && other.methods.iter().any(|method| method.name == "request_transition")
    });
    if requests {
        Ok(())
    } else {
        Err(format!(
            "`{}` has `assign = true`, and its namespace adds no `request_transition` to the core's `gnoblin`",
            object.name
        ))
    }
}

/// The writable properties `set(props, options)` of `object` takes: the object's own, and for the class that holds an augment, its
/// target's. A receiver augment's own writable properties are not properties of its class: they are its `augment_setters`.
pub(super) fn assign_fields<'a>(
    object: &'a ObjectType,
    target: Option<&'a ObjectType>,
) -> Vec<&'a gnoblin_schema::Property> {
    let own = object.properties.iter().filter(|property| !property.readonly);
    let extended = target
        .into_iter()
        .flat_map(|target| target.properties.iter().filter(|property| !property.readonly));
    own.chain(extended).collect()
}

/// The classes of `objects` that hold the members of an augment, each with the class of the core's catalog it extends, in the order of their
/// names. The core catalog is read to name the class a member is installed on, and is never changed by it.
pub(super) fn augments_of<'a>(
    catalog: &'a Catalog,
    objects: &BTreeMap<&'a str, &'a ObjectType>,
) -> Result<Vec<(&'a ObjectType, &'a ObjectType)>, String> {
    let mut found = Vec::new();
    for object in objects.values().filter(|object| object.augment_of.is_some()) {
        let class = object.augment_of.as_deref().unwrap_or_default();
        let target = catalog
            .schemas()
            .iter()
            .filter(|schema| schema.namespace.as_str() == gnoblin_schema::CORE_NAMESPACE)
            .flat_map(|schema| schema.objects.iter())
            .find(|candidate| candidate.name == class)
            .ok_or_else(|| {
                format!(
                    "the class `{}` holds the members of an augment of `core.{class}`, which the core catalog lacks",
                    object.name
                )
            })?;
        found.push((*object, target));
    }
    Ok(found)
}

/// One member of a class, before it is written.
pub(super) enum Member<'a> {
    Property(&'a Property),
    Method(&'a Method),
    Signal(&'a Signal),
}

impl Member<'_> {
    pub(super) fn name(&self) -> &str {
        match self {
            Member::Property(property) => &property.name,
            Member::Method(method) => &method.name,
            Member::Signal(signal) => &signal.name,
        }
    }
}

/// The members of `object` as ids number them: its base class's first, then its own.
pub(super) fn flatten<'a>(by_name: &BTreeMap<&str, &'a ObjectType>, object: &'a ObjectType) -> Vec<Member<'a>> {
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

/// A member of a class as its id numbers it, and whether another library's class owns it.
pub(super) struct ClassMember<'a> {
    pub(super) member: Member<'a>,
    pub(super) foreign: bool,
}

/// The members of `object` as ids number them, with whether another library owns each: the members of a base class of another
/// namespace first (the derived catalog lists them with the ids they have there), then its own, base class first.
pub(super) fn slots_of<'a>(
    catalog: &'a Catalog,
    by_name: &BTreeMap<&str, &'a ObjectType>,
    object: &'a ObjectType,
) -> Vec<ClassMember<'a>> {
    let mut slots: Vec<ClassMember<'a>> = foreign_base(catalog, object)
        .map(|(_, members)| members)
        .unwrap_or_default()
        .into_iter()
        .map(|member| ClassMember { member, foreign: true })
        .collect();
    slots.extend(
        flatten(by_name, object)
            .into_iter()
            .map(|member| ClassMember { member, foreign: false }),
    );
    slots
}

/// Refuses a member of a base class of another namespace whose types name that namespace's classes: a typed reference across two
/// libraries needs both libraries' handles, which no catalog needs yet.
pub(super) fn check_foreign(class: &str, slot: &ClassMember<'_>) -> Result<(), String> {
    if !slot.foreign {
        return Ok(());
    }
    let types: Vec<&Type> = match &slot.member {
        Member::Property(property) => vec![&property.ty],
        Member::Method(method) => method
            .params
            .iter()
            .map(|field| &field.ty)
            .chain(method.returns.as_ref())
            .collect(),
        Member::Signal(signal) => signal.fields.iter().map(|field| &field.ty).collect(),
    };
    for ty in types {
        if !(ty.referenced_objects().is_empty()
            && ty.referenced_enums().is_empty()
            && ty.referenced_callbacks().is_empty())
        {
            return Err(format!(
                "`{class}` extends a class of another namespace, and its member `{}` has the type `{ty}` of that namespace; a typed \
                 reference across two libraries is not rendered",
                slot.member.name()
            ));
        }
    }
    Ok(())
}

/// The properties a constructor takes: the positional ones, and the ones the optional record after them carries.
pub(super) struct CtorProperties<'a> {
    /// The properties the catalog's `ctor` names, in its order.
    pub(super) positional: Vec<&'a Property>,
    /// Every other writable property of the class's own members, in catalog order. One of a base class that another library serves is
    /// left out: that library makes the base object, and the record is the properties this class's library takes.
    pub(super) options: Vec<&'a Property>,
}

/// The properties the constructor of `object` takes, split into the positional ones and the options record's.
pub(super) fn ctor_properties<'a>(
    object: &ObjectType,
    slots: &[ClassMember<'a>],
) -> Result<CtorProperties<'a>, String> {
    let properties = || {
        slots.iter().filter_map(|slot| match &slot.member {
            Member::Property(property) => Some((*property, slot.foreign)),
            _ => None,
        })
    };
    let mut positional = Vec::new();
    for wanted in object.ctor.iter().flatten() {
        let (property, _) = properties()
            .find(|(property, _)| &property.name == wanted)
            .ok_or_else(|| {
                format!(
                    "`{}`: the constructor names `{wanted}`, which is not a property",
                    object.name
                )
            })?;
        positional.push(property);
    }
    let options = properties()
        .filter(|(property, foreign)| {
            !foreign && !property.readonly && !positional.iter().any(|taken| taken.name == property.name)
        })
        .map(|(property, _)| property)
        .collect();
    Ok(CtorProperties { positional, options })
}

/// The name of the type that carries the options of a constructor: the class's name and `Options` (`WorkspaceOptions`), or the class's name
/// and `CreateOptions` where `taken` says a type of the catalog already has the first (the record `binding_options` beside the class
/// `binding`).
pub(super) fn options_type_name(class: &str, taken: impl Fn(&str) -> bool) -> Result<String, String> {
    [format!("{class}Options"), format!("{class}CreateOptions")]
        .into_iter()
        .find(|name| !taken(name))
        .ok_or_else(|| {
            format!("`{class}`: both `{class}Options` and `{class}CreateOptions` name another type of the catalog")
        })
}

/// The `TypeDef` statics of a catalog, one per distinct type.
struct Types<'a> {
    catalog: &'a Catalog,
    namespace: Namespace,
    statics: Vec<String>,
    index: BTreeMap<String, usize>,
}

impl Types<'_> {
    /// The index of the static for `ty`, writing it (and its element type) if it is new.
    fn of(&mut self, ty: &Type, secret: bool) -> usize {
        let key = format!("{ty}{}@{}", if secret { "!secret" } else { "" }, self.namespace);
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
            Type::Any => ("VALUE_ANY", String::new(), None),
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
            if method.optional {
                flags.push("abi::MEMBER_OPTIONAL");
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

pub(super) fn render(catalog: &Catalog, namespace: &Namespace) -> Result<String, String> {
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
        namespace: namespace.clone(),
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
        // The base class may belong to another library: its members come first, with the ids they have there, and the derived class
        // forwards them. Their types are the base namespace's.
        let mut foreign = Vec::new();
        if let Some((home, base_members)) = foreign_base(catalog, object) {
            types.namespace = home;
            foreign = base_members
                .iter()
                .map(|member| member_source(&mut types, member))
                .collect();
            types.namespace = namespace.clone();
        }
        let foreign_members = foreign.len();
        let mut members = foreign;
        members.extend(
            flatten(&objects, object)
                .iter()
                .map(|member| member_source(&mut types, member)),
        );
        classes.insert(
            (*name).to_owned(),
            format!(
                "ClassDef {{ name: {name:?}, kind: abi::{kind}, flags: {}, base: {:?}, foreign_members: {foreign_members}, members: &[\n        {}\n    ] }}",
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
                "ClassDef {{ name: {name:?}, kind: abi::CLASS_ENUM, flags: 0, base: \"\", foreign_members: 0, members: &[\n        {}\n    ] }}",
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use gnoblin_schema::{Catalog, Namespace};

    use super::render;
    use crate::object_api;

    fn write(dir: &Path, name: &str, text: &str) {
        fs::create_dir_all(dir).expect("a directory");
        fs::write(dir.join(name), text).expect("a schema file");
    }

    /// Two namespaces: `lib.a` serves `thing`, and `lib.b` serves `fancy`, which extends it. `extra` is added to `thing`.
    fn catalog(extra: &str) -> (tempfile::TempDir, Catalog) {
        let root = tempfile::tempdir().expect("a directory");
        let a = root.path().join("a");
        let b = root.path().join("b");
        write(
            &a,
            "thing.toml",
            &format!(
                "namespace = \"lib.a\"\nversion = \"0.1.0\"\nlocal = true\n\n[[object]]\nname = \"thing\"\nsince = \"0.1.0\"\nkind = \"handle\"\n\n\
                 [[object.property]]\nname = \"count\"\ntype = \"int\"\nsince = \"0.1.0\"\n\n\
                 [[object.method]]\nname = \"bump\"\nsince = \"0.1.0\"\nreturns = \"int\"\n{extra}"
            ),
        );
        write(
            &b,
            "fancy.toml",
            "namespace = \"lib.b\"\nversion = \"0.1.0\"\nlocal = true\n\n[[object]]\nname = \"fancy\"\nsince = \"0.1.0\"\nkind = \"handle\"\nbase = \"lib.a.thing\"\n\n\
             [[object.property]]\nname = \"label\"\ntype = \"string\"\nsince = \"0.1.0\"\n",
        );
        let catalog = Catalog::load_dirs(&[a.as_path(), b.as_path()], &[]).expect("the catalogs load together");
        (root, catalog)
    }

    #[test]
    fn a_derived_class_can_be_held_as_its_base() {
        let root = tempfile::tempdir().expect("a directory");
        write(
            root.path(),
            "n.toml",
            "namespace = \"lib.n\"\nversion = \"0.1.0\"\nlocal = true\n\n\
             [[object]]\nname = \"node\"\nsince = \"0.1.0\"\nkind = \"handle\"\n\n\
             [[object]]\nname = \"view\"\nsince = \"0.1.0\"\nkind = \"handle\"\nbase = \"node\"\n\n\
             [[object]]\nname = \"sub_view\"\nsince = \"0.1.0\"\nkind = \"handle\"\nbase = \"view\"\n",
        );
        let catalog = Catalog::load_dirs(&[root.path()], &[]).expect("a catalog");
        let ns = Namespace::parse("lib.n").expect("a namespace");
        let typed = object_api::render(&catalog, &ns).expect("the typed API renders");
        assert!(typed.contains("pub fn as_node(&self) -> go::Res<Node>"), "{typed}");
        assert!(typed.contains("pub fn as_view(&self) -> go::Res<View>"), "{typed}");
        assert_eq!(
            typed.matches("pub fn as_node(").count(),
            2,
            "the view and the sub view: {typed}"
        );
    }

    #[test]
    fn a_class_that_extends_another_namespaces_class_lists_the_base_members_first() {
        let (_root, catalog) = catalog("");
        let derived = Namespace::parse("lib.b").expect("a namespace");
        let text = render(&catalog, &derived).expect("the derived catalog renders");
        assert!(
            text.contains("foreign_members: 2"),
            "the base's two members are the base library's:\n{text}"
        );
        let at = |name: &str| {
            text.find(&format!("name: {name:?}, kind: abi::MEMBER"))
                .unwrap_or_else(|| panic!("`{name}` is missing:\n{text}"))
        };
        assert!(
            at("count") < at("bump") && at("bump") < at("label"),
            "base members first, in the base's order:\n{text}"
        );

        let typed = object_api::render(&catalog, &derived).expect("the typed API renders");
        for member in ["pub fn count(", "pub fn bump(", "pub fn label("] {
            assert!(
                typed.contains(member),
                "`{member}` is a method of the derived class:\n{typed}"
            );
        }
        let ids = typed
            .find("p_count: go::typed::property")
            .expect("an id for the base property");
        assert!(
            ids < typed
                .find("p_label: go::typed::property")
                .expect("an id for its own property")
        );
    }

    #[test]
    fn the_typed_api_refuses_a_base_member_that_names_a_class_of_the_other_namespace() {
        let (_root, catalog) =
            catalog("\n[[object.method]]\nname = \"twin\"\nsince = \"0.1.0\"\nreturns = \"object:thing\"\n");
        let derived = Namespace::parse("lib.b").expect("a namespace");
        let refused = object_api::render(&catalog, &derived).expect_err("a typed reference across two libraries");
        assert!(
            refused.contains("twin") && refused.contains("another namespace"),
            "the message names the member and the reason: {refused}"
        );
        // The catalog tables carry the type either way: the library that serves the base checks it.
        render(&catalog, &derived).expect("the tables render");
    }
}
