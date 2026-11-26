//! Target `object-api`: the typed Rust API of each namespace a library serves, over the object protocol's client.
//!
//! For `gnoblin.model` and `gnoblin.std` this writes `rust-objects/{model,std}_api.rs`, which the SDK crate of the library includes
//! (`gnoblin_core::objects`, `gnoblin_std::objects`). What it renders, by the kind a catalog gives a class:
//!
//! - an enum is a Rust enum whose discriminants are the ids the protocol numbers its values with;
//! - a record is a struct of its properties in catalog order. A property with a schema default is the type itself and starts at that
//!   default; one without is an `Option` that starts as `None` (a record leaves such a field out when it does not say it);
//! - a handle or a service is a struct over an [`Object`](gnoblin_object::client::Object), released when it is dropped, with a
//!   constructor (`new`, or `get` for a service), a getter and a setter for each property, a method for each method (a function of
//!   the class is an associated function that takes the namespace's `Objects`), and `on_<signal>` and `on_<property>_changed`
//!   listeners that end when the returned guard is dropped. A property that holds an object or a record reads as an `Option`, because
//!   the catalog does not say it is always there;
//! - an interface is a struct of handlers, one boxed closure for each method (an `Option` of one for a method the library has a
//!   native default for), and `implement` makes the object the library calls them through;
//! - a `callback` type is a Rust closure, wherever a property, a constructor or a method takes one.
//!
//! `Objects` is the handle on one library for one generation. The ids of a class are looked up by name the first time a call needs
//! them, so a shell built against an older catalog works against a newer library and fails only on a name that is missing.
//!
//! A parameter may name a handle of the other library's catalog (`object:gnoblin.model.client`): it is a reference to the typed class of
//! that library's SDK crate (`&::gnoblin_core::objects::Client`), whose value is sent as it is. Any other member whose type names a
//! class of another namespace is refused with a message that says so: a typed reference across two libraries needs both libraries'
//! `Objects`, which no catalog needs yet.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use gnoblin_schema::{
    CallbackDecl, Catalog, DefaultValue, EnumDecl, Field, Method, Namespace, ObjectKind, ObjectType, Property, Signal,
    Type,
};

use super::objects::{
    Member, augment_finds, augments_of, check_foreign, crate_path_of, ctor_properties, finds_of, foreign_handle,
    foreign_handles, options_type_name, slots_of, stem_of,
};
use super::text::{pascal, rust_ident, snake};
use super::values::{self, Absent, CarrySlot, ValueClass, absent, is_family};
use super::{Comment, File, SdkSpec, Target};

/// Writes the typed Rust API.
pub struct ObjectApiTarget;

impl Target for ObjectApiTarget {
    fn name(&self) -> &'static str {
        "object-api"
    }

    fn generate(&self, spec: &SdkSpec) -> Result<Vec<File>, String> {
        let mut files = Vec::new();
        for namespace in spec.objects.namespaces() {
            let stem = stem_of(&namespace)?;
            files.push(File {
                path: format!("rust-objects/{stem}_api.rs"),
                comment: Some(Comment::Slashes),
                text: render(&spec.objects, &namespace)?,
            });
        }
        Ok(files)
    }
}

/// Fills `@Name@` placeholders: the generated Rust is full of braces, which `format!` would make unreadable.
fn fill(template: &str, pairs: &[(&str, &str)]) -> String {
    let mut text = template.to_owned();
    for (name, value) in pairs {
        text = text.replace(&format!("@{name}@"), value);
    }
    text
}

/// What a class of the namespace is, for the types that name it.
struct Renderer<'a> {
    catalog: &'a Catalog,
    namespace: &'a Namespace,
    objects: BTreeMap<&'a str, &'a ObjectType>,
    enums: BTreeMap<&'a str, &'a EnumDecl>,
    callbacks: BTreeMap<&'a str, &'a CallbackDecl>,
}

fn class_name(object: &ObjectType) -> String {
    values::rust_class(&object.name, &object.export)
}

fn member_ident(name: &str, export: &Option<String>) -> String {
    rust_ident(&export.clone().unwrap_or_else(|| snake(name)))
}

/// The name a property has before it is made an identifier: the helpers `set_<name>` and `on_<name>_changed` are built from it.
fn property_stem(property: &Property) -> String {
    property
        .extras
        .export
        .rust
        .clone()
        .unwrap_or_else(|| snake(&property.name))
}

fn property_ident(property: &Property) -> String {
    values::rust_field(property)
}

impl<'a> Renderer<'a> {
    fn new(catalog: &'a Catalog, namespace: &'a Namespace) -> Result<Renderer<'a>, String> {
        let objects: BTreeMap<&str, &ObjectType> = catalog
            .objects(namespace)
            .map(|object| (object.name.as_str(), object))
            .collect();
        let enums = catalog
            .enums(namespace)
            .map(|declared| (declared.name.as_str(), declared))
            .collect();
        let callbacks = catalog
            .callbacks(namespace)
            .map(|declared| (declared.name.as_str(), declared))
            .collect();
        let gen_ = Renderer {
            catalog,
            namespace,
            objects,
            enums,
            callbacks,
        };
        let mut names = BTreeSet::new();
        for name in gen_
            .objects
            .values()
            .map(|object| class_name(object))
            .chain(gen_.enums.keys().map(|name| pascal(name)))
        {
            if !names.insert(name.clone()) {
                return Err(format!(
                    "`{namespace}`: two types would both be called `{name}` in Rust"
                ));
            }
        }
        Ok(gen_)
    }

    /// The class `name` names in this namespace, or an error saying why there is none. A class of another namespace is reached through
    /// [`Self::foreign_path`] instead.
    fn class(&self, name: &str) -> Result<&'a ObjectType, String> {
        if name.contains('.') {
            return Err(format!(
                "`{name}` is a class of another namespace: it has no declaration in `{}`",
                self.namespace
            ));
        }
        self.objects
            .get(name)
            .copied()
            .ok_or_else(|| format!("`{name}` is not a class of `{}`", self.namespace))
    }

    /// The path of the typed class of a handle of another namespace, which a parameter may name (`::gnoblin_core::objects::Client`).
    fn foreign_path(&self, name: &str) -> Result<String, String> {
        let (home, class) = foreign_handle(self.catalog, name)?;
        Ok(format!("{}::{}", crate_path_of(&home)?, class_name(class)))
    }

    fn enum_name(&self, name: &str) -> Result<String, String> {
        if name.contains('.') || !self.enums.contains_key(name) {
            return Err(format!("`{name}` is not an enum of `{}`", self.namespace));
        }
        Ok(pascal(name))
    }

    /// A type as a value that is read or kept.
    fn owned(&self, ty: &Type) -> Result<String, String> {
        Ok(match ty {
            Type::Bool => "bool".to_owned(),
            Type::Int => "i64".to_owned(),
            Type::Uint => "u64".to_owned(),
            Type::Double => "f64".to_owned(),
            Type::Text => "String".to_owned(),
            Type::Bytes => "go::typed::Octets".to_owned(),
            Type::ObjectRef(name) if name.contains('.') => self.foreign_path(name)?,
            Type::ObjectRef(name) => class_name(self.class(name)?),
            Type::Enum(name) => self.enum_name(name)?,
            Type::List(inner) => format!("Vec<{}>", self.owned(inner)?),
            Type::Callback(name) => {
                return Err(format!(
                    "`callback:{name}` can be given to the library, not read back from it"
                ));
            }
        })
    }

    /// A type as a parameter.
    fn param(&self, ty: &Type) -> Result<String, String> {
        Ok(match ty {
            Type::Text => "&str".to_owned(),
            Type::Bytes => "&go::typed::Octets".to_owned(),
            Type::ObjectRef(name) if name.contains('.') => format!("&{}", self.foreign_path(name)?),
            Type::ObjectRef(name) => format!("&{}", class_name(self.class(name)?)),
            Type::List(inner) => format!("&[{}]", self.owned(inner)?),
            Type::Callback(name) => self.closure_type(name)?,
            other => self.owned(other)?,
        })
    }

    /// The closure type of a callback declaration, as a parameter takes it.
    fn closure_type(&self, name: &str) -> Result<String, String> {
        Ok(format!("impl {}", self.closure_bound(name)?))
    }

    /// The bound a closure of a callback declaration meets.
    fn closure_bound(&self, name: &str) -> Result<String, String> {
        let declared = self
            .callbacks
            .get(name)
            .ok_or_else(|| format!("`{name}` is not a callback of `{}`", self.namespace))?;
        let params: Vec<String> = declared
            .params
            .iter()
            .map(|field| self.field_type(field))
            .collect::<Result<_, _>>()?;
        let returns = match &declared.returns {
            Some(ty) => self.owned(ty)?,
            None => "()".to_owned(),
        };
        Ok(format!("FnMut({}) -> go::Res<{returns}> + 'static", params.join(", ")))
    }

    /// A field or an argument as the value a closure or a signal listener receives.
    fn field_type(&self, field: &Field) -> Result<String, String> {
        let owned = self.owned(&field.ty)?;
        Ok(if field.optional {
            format!("Option<{owned}>")
        } else {
            owned
        })
    }

    /// A property read back from the library: an object or a record may be absent.
    fn getter_type(&self, ty: &Type) -> Result<String, String> {
        let owned = self.owned(ty)?;
        Ok(if matches!(ty, Type::ObjectRef(_)) {
            format!("Option<{owned}>")
        } else {
            owned
        })
    }

    /// True when a value of the type can be cloned, compared and printed: it holds no reference to an object.
    fn plain(&self, ty: &Type, depth: u32) -> bool {
        match ty {
            Type::ObjectRef(name) => {
                depth < 8
                    && self.class(name).is_ok_and(|class| {
                        class.kind == ObjectKind::Record
                            && class
                                .properties
                                .iter()
                                .all(|property| self.plain(&property.ty, depth + 1))
                    })
            }
            Type::List(inner) => self.plain(inner, depth + 1),
            Type::Callback(_) => false,
            _ => true,
        }
    }

    /// The Rust expression for a schema default.
    fn default_expr(&self, ty: &Type, default: &DefaultValue) -> Result<String, String> {
        Ok(match (default, ty) {
            (DefaultValue::Bool(value), Type::Bool) => value.to_string(),
            (DefaultValue::Int(value), Type::Int) => format!("{value}_i64"),
            (DefaultValue::Int(value), Type::Uint) => format!("{value}_u64"),
            (DefaultValue::Int(value), Type::Double) => format!("{value}.0_f64"),
            (DefaultValue::Uint(value), Type::Uint) => format!("{value}_u64"),
            (DefaultValue::Uint(value), Type::Int) => format!("{value}_i64"),
            (DefaultValue::Uint(value), Type::Double) => format!("{value}.0_f64"),
            (DefaultValue::Double(value), Type::Double) => format!("{value:?}_f64"),
            (DefaultValue::Text(value), Type::Text) => format!("{value:?}.to_owned()"),
            (DefaultValue::Text(value), Type::Enum(name)) => format!("{}::{}", self.enum_name(name)?, pascal(value)),
            (DefaultValue::List(items), Type::List(inner)) => {
                let items: Vec<String> = items
                    .iter()
                    .map(|item| self.default_expr(inner, item))
                    .collect::<Result<_, _>>()?;
                format!("vec![{}]", items.join(", "))
            }
            (other, ty) => return Err(format!("the default {other:?} does not fit the type `{ty}`")),
        })
    }

    /// The Val a parameter becomes, as an expression. `objects` is the namespace handle in scope.
    fn arg_expr(&self, name: &str, ty: &Type, optional: bool) -> Result<String, String> {
        match ty {
            Type::Callback(callback) => {
                let make = format!("make_{}", snake(callback));
                self.closure_type(callback)?;
                Ok(if optional {
                    format!("match {name} {{ Some(run) => {make}(objects, run), None => go::Val::None }}")
                } else {
                    format!("{make}(objects, {name})")
                })
            }
            _ => Ok(format!("go::typed::ToVal::to_val(&{name})")),
        }
    }

    // ---- the parts of the file ----------------------------------------------------------------------------------------------

    fn enum_source(&self, declared: &EnumDecl) -> String {
        let name = pascal(&declared.name);
        let variants: Vec<String> = declared.values.iter().map(|value| pascal(value)).collect();
        let mut text = String::new();
        let _ = writeln!(text, "/// The enum `{}`.", declared.name);
        text.push_str("#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]\n");
        let _ = writeln!(text, "pub enum {name} {{");
        for (index, variant) in variants.iter().enumerate() {
            let _ = writeln!(text, "    {variant} = {},", index + 1);
        }
        text.push_str("}\n");
        let all: Vec<String> = variants.iter().map(|variant| format!("{name}::{variant}")).collect();
        let names: Vec<String> = variants
            .iter()
            .zip(&declared.values)
            .map(|(variant, value)| format!("{name}::{variant} => {value:?},"))
            .collect();
        let reads: Vec<String> = variants
            .iter()
            .enumerate()
            .map(|(index, variant)| format!("{} => Ok({name}::{variant}),", index + 1))
            .collect();
        text.push_str(&fill(
            r#"impl @Name@ {
    /// Every value, in the order the catalog declares them.
    pub const ALL: &'static [@Name@] = &[@All@];

    /// The value's name in the catalog.
    pub const fn as_str(self) -> &'static str {
        match self {
            @Names@
        }
    }
}

impl go::typed::ToVal for @Name@ {
    fn to_val(&self) -> go::Val {
        go::Val::Enum { class: 0, value: *self as u32 }
    }
}

impl go::typed::FromVal<Objects> for @Name@ {
    fn from_val(_: &Objects, value: &go::Val) -> go::Res<@Name@> {
        match value.enum_value()? {
            @Reads@
            other => Err(go::Fail::type_error(format!("{other} is not a value of `@Schema@`"))),
        }
    }
}

"#,
            &[
                ("Name", &name),
                ("All", &all.join(", ")),
                ("Names", &names.join("\n            ")),
                ("Reads", &reads.join("\n            ")),
                ("Schema", &declared.name),
            ],
        ));
        text
    }

    /// The types of a record: one struct, or for the family of a tagged value, a struct for each variant and the enum that holds any.
    fn record_source(&self, object: &ObjectType) -> Result<String, String> {
        if !object.methods.is_empty() || !object.signals.is_empty() {
            return Err(format!(
                "the record `{}` declares members that are not properties",
                object.name
            ));
        }
        let mut text = String::new();
        for class in values::value_classes([object]) {
            text.push_str(&self.class_source(&class)?);
        }
        if is_family(object) {
            text.push_str(&self.family_source(object)?);
        }
        Ok(text)
    }

    /// Whether a value of the type is `Copy`, so it is passed on without a `clone`.
    fn is_copy(ty: &Type) -> bool {
        matches!(ty, Type::Bool | Type::Int | Type::Uint | Type::Double | Type::Enum(_))
    }

    /// `expr` as an owned value of a field of type `ty`.
    fn taken(ty: &Type, expr: &str) -> String {
        if Self::is_copy(ty) {
            expr.to_owned()
        } else {
            format!("{expr}.clone()")
        }
    }

    /// The Rust expression for the value a field of the record `record` holds, when a variant's property `own` carries it.
    fn carried_value(own: &Property, inner: &Property) -> String {
        let value = Self::taken(&own.ty, &format!("self.{}", values::rust_field(own)));
        if inner.optional && !own.optional {
            format!("Some({value})")
        } else {
            value
        }
    }

    /// A value class as a struct: its fields in the schema's order, each started at its default, and the conversion to and from the
    /// library's record (for a variant, the family's record).
    fn class_source(&self, class: &ValueClass<'_>) -> Result<String, String> {
        let name = class.rust();
        let mut declared = Vec::new();
        let mut starts = Vec::new();
        let mut required: Vec<(String, String)> = Vec::new();
        // The fields a read makes, and what the read needs to have before them (a record a variant carries part of).
        let mut reads = Vec::new();
        let mut setup = Vec::new();
        let mut writes = Vec::new();
        let mut guard = String::new();
        let slots = match class.of {
            Some((family, variant)) => Some(values::slots(&|record| self.class(record), family, variant)?),
            None => None,
        };
        for (index, property) in class.properties.iter().enumerate() {
            let field = values::rust_field(property);
            let owned = self.owned(&property.ty)?;
            // A field with a default starts at it; one marked `optional` is an `Option` that starts as `None`; any other is always there,
            // so a value of the record is made with it (`new`) and a read that finds it missing is an error.
            let (ty, start) = match absent(property) {
                Absent::Default(default) => (owned, self.default_expr(&property.ty, default)?),
                Absent::None => (format!("Option<{owned}>"), "None".to_owned()),
                Absent::Required => {
                    required.push((field.clone(), owned.clone()));
                    (owned, field.clone())
                }
            };
            declared.push(format!("    pub {field}: {ty},"));
            starts.push(format!("            {field}: {start},"));
            if slots.is_none() {
                writes.push(format!("go::typed::ToVal::to_val(&self.{field})"));
                reads.push(format!("            {field}: {},", self.read_whole(property, index)?));
            }
        }
        if let (Some(slots), Some((family, variant))) = (&slots, class.of) {
            let tag_at = slots
                .iter()
                .position(|slot| matches!(slot.slot, CarrySlot::Tag(_)))
                .ok_or_else(|| format!("the family `{}` has no `tag` property", family.name))?;
            let Type::Enum(tag_enum) = &slots[tag_at].field.ty else {
                return Err(format!("the `tag` of the family `{}` is not an enum", family.name));
            };
            let tag_type = self.enum_name(tag_enum)?;
            let tag_value = format!("{tag_type}::{}", pascal(&variant.tag));
            guard = format!(
                "        let kind: {tag_type} = go::typed::arg(cx, fields, {tag_at})?;\n        if kind != {tag_value} {{\n            return Err(go::Fail::type_error(format!(\"expected a `{}` {}, got `{{}}`\", kind.as_str())));\n        }}\n",
                variant.tag, family.name
            );
            for (index, slot) in slots.iter().enumerate() {
                match &slot.slot {
                    CarrySlot::Tag(_) => writes.push(format!("go::typed::ToVal::to_val(&{tag_value})")),
                    CarrySlot::Whole(own) => {
                        writes.push(format!("go::typed::ToVal::to_val(&self.{})", values::rust_field(own)));
                        reads.push(format!(
                            "            {}: {},",
                            values::rust_field(own),
                            self.read_whole(own, index)?
                        ));
                    }
                    CarrySlot::Part { record, parts } => {
                        let part = format!("part_{}", snake(&slot.field.name));
                        let record_name = class_name(record);
                        setup.push(format!(
                            "        let {part}: {record_name} = go::typed::arg(cx, fields, {index})?;\n"
                        ));
                        let mut literal = Vec::new();
                        for inner in &record.properties {
                            let value = match parts.iter().find(|carried| carried.inner.name == inner.name) {
                                Some(carried) => Self::carried_value(carried.own, inner),
                                None => match absent(inner) {
                                    Absent::Default(default) => self.default_expr(&inner.ty, default)?,
                                    Absent::None => "None".to_owned(),
                                    Absent::Required => {
                                        return Err(format!(
                                            "the variant `{}` leaves out `{}.{}`, which is always there",
                                            variant.name, record.name, inner.name
                                        ));
                                    }
                                },
                            };
                            literal.push(format!("{}: {value}", values::rust_field(inner)));
                        }
                        writes.push(format!(
                            "go::typed::ToVal::to_val(&{record_name} {{ {} }})",
                            literal.join(", ")
                        ));
                        for carried in parts {
                            let held = format!("{part}.{}", values::rust_field(carried.inner));
                            let value = match (absent(carried.own), carried.inner.optional) {
                                (Absent::Default(default), true) => {
                                    format!(
                                        "{held}.unwrap_or_else(|| {})",
                                        self.default_expr(&carried.own.ty, default)?
                                    )
                                }
                                (Absent::Required, true) => format!(
                                    "{held}.ok_or_else(|| go::Fail::type_error(\"`{}.{}` is missing\"))?",
                                    record.name, carried.inner.name
                                ),
                                _ => held,
                            };
                            reads.push(format!("            {}: {value},", values::rust_field(carried.own)));
                        }
                    }
                    CarrySlot::Empty => writes.push("go::Val::None".to_owned()),
                }
            }
        }
        let plain = class.properties.iter().all(|property| self.plain(&property.ty, 0));
        let mut text = String::new();
        match class.of {
            None => {
                let _ = writeln!(text, "/// The record `{}`.", class.name);
            }
            Some((family, variant)) => {
                let _ = writeln!(
                    text,
                    "/// The `{}` variant of the record `{}`.",
                    variant.tag, family.name
                );
            }
        }
        if plain {
            text.push_str("#[derive(Clone, Debug, PartialEq)]\n");
        }
        let starts = starts.join("\n");
        let start = if required.is_empty() {
            fill(
                "impl Default for @Name@ {\n    fn default() -> @Name@ {\n        @Name@ {\n@Starts@\n        }\n    }\n}\n",
                &[("Name", &name), ("Starts", &starts)],
            )
        } else {
            let params: Vec<String> = required.iter().map(|(field, ty)| format!("{field}: {ty}")).collect();
            fill(
                "impl @Name@ {\n    /// A value with the fields the record always has; the others start at their defaults or none.\n    pub fn new(@Params@) -> @Name@ {\n        @Name@ {\n@Starts@\n        }\n    }\n}\n",
                &[("Name", &name), ("Params", &params.join(", ")), ("Starts", &starts)],
            )
        };
        // A variant's writes are in the family record's order, whatever the order of the variant's own fields.
        let writes = writes.join(", ");
        text.push_str(&fill(
            r#"pub struct @Name@ {
@Declared@
}

@Start@
impl go::typed::ToVal for @Name@ {
    fn to_val(&self) -> go::Val {
        go::Val::Record { class: 0, fields: vec![@Writes@] }
    }
}

impl go::typed::FromVal<Objects> for @Name@ {
    fn from_val(cx: &Objects, value: &go::Val) -> go::Res<@Name@> {
        let fields = value.record()?;
@Guard@@Setup@        Ok(@Name@ {
@Reads@
        })
    }
}

"#,
            &[
                ("Name", &name),
                ("Declared", &declared.join("\n")),
                ("Start", &start),
                ("Writes", &writes),
                ("Guard", &guard),
                ("Setup", &setup.concat()),
                ("Reads", &reads.join("\n")),
            ],
        ));
        Ok(text)
    }

    /// The Rust expression that reads `property` from the field `index` of a record.
    fn read_whole(&self, property: &Property, index: usize) -> Result<String, String> {
        Ok(match absent(property) {
            Absent::Default(default) => format!(
                "go::typed::field(cx, fields, {index}, || {})?",
                self.default_expr(&property.ty, default)?
            ),
            Absent::None => format!("go::typed::field(cx, fields, {index}, || None)?"),
            Absent::Required => format!("go::typed::arg(cx, fields, {index})?"),
        })
    }

    /// The enum that holds any variant of a family: a script's value is one of them, and the library's record says by its tag which.
    fn family_source(&self, family: &ObjectType) -> Result<String, String> {
        let name = class_name(family);
        let tag = family
            .properties
            .iter()
            .position(|property| Some(property.name.as_str()) == family.tag.as_deref())
            .ok_or_else(|| format!("the family `{}` has no `tag` property", family.name))?;
        let Type::Enum(tag_enum) = &family.properties[tag].ty else {
            return Err(format!("the `tag` of the family `{}` is not an enum", family.name));
        };
        let tag_type = self.enum_name(tag_enum)?;
        let mut members = Vec::new();
        let mut converts = Vec::new();
        let mut kinds = Vec::new();
        let mut writes = Vec::new();
        let mut reads = Vec::new();
        let mut plain = true;
        for variant in &family.variants {
            let member = pascal(&variant.tag);
            let class = values::rust_class(&variant.name, &variant.export);
            plain &= variant.properties.iter().all(|property| self.plain(&property.ty, 0));
            members.push(format!(
                "    /// The `{}` variant.\n    {member}({class}),",
                variant.tag
            ));
            converts.push(format!(
                "impl From<{class}> for {name} {{\n    fn from(value: {class}) -> {name} {{\n        {name}::{member}(value)\n    }}\n}}\n"
            ));
            kinds.push(format!("{name}::{member}(_) => {tag_type}::{member},"));
            writes.push(format!("{name}::{member}(value) => go::typed::ToVal::to_val(value),"));
            reads.push(format!(
                "{tag_type}::{member} => <{class} as go::typed::FromVal<Objects>>::from_val(cx, value).map({name}::{member}),"
            ));
        }
        let mut text = String::new();
        let _ = writeln!(text, "/// The record `{}`: any one of its variants.", family.name);
        if plain {
            text.push_str("#[derive(Clone, Debug, PartialEq)]\n");
        }
        text.push_str(&fill(
            r#"pub enum @Name@ {
@Members@
}

impl @Name@ {
    /// The value of `@Tag@` that says which variant this is.
    pub const fn kind(&self) -> @TagType@ {
        match self {
            @Kinds@
        }
    }
}

@Converts@
impl go::typed::ToVal for @Name@ {
    fn to_val(&self) -> go::Val {
        match self {
            @Writes@
        }
    }
}

impl go::typed::FromVal<Objects> for @Name@ {
    fn from_val(cx: &Objects, value: &go::Val) -> go::Res<@Name@> {
        let fields = value.record()?;
        let kind: @TagType@ = go::typed::arg(cx, fields, @TagAt@)?;
        match kind {
            @Reads@
        }
    }
}

"#,
            &[
                ("Name", &name),
                ("Members", &members.join("\n")),
                ("Tag", &family.properties[tag].name),
                ("TagType", &tag_type),
                ("TagAt", &tag.to_string()),
                ("Kinds", &kinds.join("\n            ")),
                ("Converts", &converts.join("\n")),
                ("Writes", &writes.join("\n            ")),
                ("Reads", &reads.join("\n            ")),
            ],
        ));
        Ok(text)
    }

    fn handle_source(&self, object: &'a ObjectType) -> Result<(String, String), String> {
        let name = class_name(object);
        let ids_name = format!("{name}Ids");
        let slots = slots_of(self.catalog, &self.objects, object);
        for slot in &slots {
            check_foreign(&object.name, slot)?;
        }
        let has_instance_members = object.kind == ObjectKind::Handle
            || slots.iter().any(|slot| match &slot.member {
                Member::Method(method) => !method.function,
                _ => true,
            });

        // The ids, resolved by name the first time a call needs them.
        let mut id_fields = Vec::new();
        let mut id_reads = Vec::new();
        for slot in &slots {
            let (prefix, how, member) = match &slot.member {
                Member::Property(property) => ("p", "property", &property.name),
                Member::Method(method) => ("m", "method", &method.name),
                Member::Signal(signal) => ("s", "signal", &signal.name),
            };
            let field = format!("{prefix}_{}", snake(member));
            id_fields.push(format!("    {field}: go::MemberId,"));
            id_reads.push(format!(
                "            {field}: go::typed::{how}(api, class, {member:?})?,"
            ));
        }
        let ids = fill(
            r#"#[derive(Debug)]
struct @Ids@ {
    class: go::ClassId,
@Fields@
}

impl @Ids@ {
    fn of(objects: &Objects) -> go::Res<&@Ids@> {
        objects.ids.@Field@.get(|| {
            let api = &objects.api;
            let class = api.class(@Schema@)?;
            Ok(@Ids@ {
                class,
@Reads@
            })
        })
    }
}

"#,
            &[
                ("Ids", &ids_name),
                ("Fields", &id_fields.join("\n")),
                ("Field", &rust_ident(&snake(&object.name))),
                ("Schema", &format!("{:?}", object.name)),
                ("Reads", &id_reads.join("\n")),
            ],
        );

        let mut taken: BTreeSet<String> = ["new", "new_with", "get", "handle", "destroy", "objects"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let mut claim = |ident: &str| -> Result<(), String> {
            if taken.insert(ident.to_owned()) {
                Ok(())
            } else {
                Err(format!(
                    "`{}`: two members would both be `{ident}` in Rust",
                    object.name
                ))
            }
        };

        let mut methods = String::new();
        let mut options_text = String::new();
        // The constructor.
        if has_instance_members {
            let new_name = if object.kind == ObjectKind::Service {
                "get"
            } else {
                "new"
            };
            let ctor = ctor_properties(object, &slots)?;
            let mut params = Vec::new();
            let mut inits = Vec::new();
            for property in &ctor.positional {
                let ident = property_ident(property);
                params.push(format!("{ident}: {}", self.param(&property.ty)?));
                inits.push(format!(
                    "(ids.p_{}, {})",
                    snake(&property.name),
                    self.arg_expr(&ident, &property.ty, false)?
                ));
            }
            let params = if params.is_empty() {
                String::new()
            } else {
                format!(", {}", params.join(", "))
            };
            methods.push_str(&fill(
                r#"    /// Makes the object.
    ///
    /// # Errors
    ///
    /// The library refuses the arguments.
    pub fn @New@(objects: &Objects@Params@) -> go::Res<@Name@> {
        let ids = @Ids@::of(objects)?;
        let object = go::client::Object::create(objects.api, ids.class, &[@Inits@]).map_err(|fail| go::typed::at_class(fail, @NameLit@, @NewLit@))?;
        Ok(@Name@ { object, objects: objects.clone() })
    }

    /// The reference this value holds, to pass to a call or to keep past it.
    pub fn handle(&self) -> &go::client::Object {
        &self.object
    }

    /// Ends the object and its children, whoever else holds it.
    ///
    /// # Errors
    ///
    /// The object is already gone.
    pub fn destroy(&self) -> go::Res<()> {
        self.object.destroy().map_err(|fail| self.object.at(fail, @NameLit@, "destroy"))
    }

"#,
                &[
                    ("New", new_name),
                    ("Params", &params),
                    ("Name", &name),
                    ("Ids", &ids_name),
                    ("Inits", &inits.join(", ")),
                    ("NameLit", &format!("{name:?}")),
                    ("NewLit", &format!("{new_name:?}")),
                ],
            ));
            if object.kind != ObjectKind::Service && !ctor.options.is_empty() {
                let (declared, new_with) = self.options_source(object, &ids_name, &params, &inits, &ctor.options)?;
                options_text = declared;
                methods.push_str(&new_with);
            }
        }

        for slot in &slots {
            match &slot.member {
                Member::Property(property) => {
                    self.property_source(&mut methods, &name, &ids_name, property, &mut claim)?;
                }
                Member::Method(method) => self.method_source(&mut methods, &name, &ids_name, method, &mut claim)?,
                Member::Signal(signal) => self.signal_source(&mut methods, &name, &ids_name, signal, &mut claim)?,
            }
        }

        let finds = finds_of(object);
        let mut found_impls = String::new();
        if has_instance_members && !finds.is_empty() {
            claim("find")?;
            let kinds: Vec<String> = finds.iter().map(|(kind, _)| format!("`{kind}`")).collect();
            let Some(Type::List(first)) = finds.first().and_then(|(_, method)| method.returns.as_ref()) else {
                return Err(format!("`{}`: the first finder does not return a list", object.name));
            };
            let first = self.owned(first)?;
            let _ = write!(
                methods,
                "    /// The live objects of the kind `T` that match `filter`, in list order, or every one when it is left out. The kinds are {}: `find::<{first}>(None)`.\n    ///\n    /// # Errors\n    ///\n    /// The object is gone, or the library refuses the filter.\n    pub fn find<T: Finds<{name}>>(&self, filter: Option<&T::Filter>) -> go::Res<Vec<T>> {{\n        T::find_in(self, filter)\n    }}\n\n",
                kinds.join(", ")
            );
            for (kind, method) in &finds {
                let Some(Type::List(inner)) = &method.returns else {
                    return Err(format!(
                        "`{}`: the finder of `{kind}` does not return a list",
                        object.name
                    ));
                };
                let _ = write!(
                    found_impls,
                    "impl Finds<{name}> for {found} {{\n    type Filter = {filter};\n    const KIND: &'static str = {kind:?};\n\n    fn find_in(service: &{name}, filter: Option<&{filter}>) -> go::Res<Vec<{found}>> {{\n        service.{ident}(filter)\n    }}\n}}\n\n",
                    found = self.owned(inner)?,
                    filter = self.owned(&method.params[0].ty)?,
                    ident = member_ident(&method.name, &method.extras.export.rust),
                );
            }
        }

        let mut text = options_text;
        let _ = writeln!(text, "/// The class `{}`.", object.name);
        if has_instance_members {
            // A call that takes a base class takes what derives from it through these: the same object, held as the base.
            let mut above = object.base.as_deref();
            while let Some(base) = above.filter(|base| !base.contains('.')) {
                let Some(base_object) = self.objects.get(base) else {
                    break;
                };
                let _ = write!(
                    methods,
                    "    /// This object as a `{base}`, to pass where a `{base}` is taken.\n    ///\n    /// # Errors\n    ///\n    /// The object is gone.\n    pub fn as_{}(&self) -> go::Res<{}> {{\n        go::typed::FromVal::from_val(&self.objects, &self.object.val())\n    }}\n\n",
                    snake(base),
                    class_name(base_object)
                );
                above = base_object.base.as_deref();
            }
            text.push_str(&fill(
                r#"#[derive(Debug)]
pub struct @Name@ {
    object: go::client::Object,
    objects: Objects,
}

impl @Name@ {
@Methods@}

impl go::typed::ToVal for @Name@ {
    fn to_val(&self) -> go::Val {
        self.object.val()
    }
}

/// One object, whichever value holds it: two deliveries of it are equal.
impl PartialEq for @Name@ {
    fn eq(&self, other: &@Name@) -> bool {
        self.object == other.object
    }
}

impl Eq for @Name@ {}

impl ::std::hash::Hash for @Name@ {
    fn hash<H: ::std::hash::Hasher>(&self, state: &mut H) {
        self.object.hash(state);
    }
}

impl go::typed::FromVal<Objects> for @Name@ {
    fn from_val(cx: &Objects, value: &go::Val) -> go::Res<@Name@> {
        Ok(@Name@ { object: go::client::Object::from_val(cx.api, value)?, objects: cx.clone() })
    }
}

"#,
                &[("Name", &name), ("Methods", &methods)],
            ));
            text.push_str(&found_impls);
        } else {
            text.push_str(&fill(
                "#[derive(Clone, Copy, Debug)]\npub struct @Name@;\n\nimpl @Name@ {\n@Methods@}\n\n",
                &[("Name", &name), ("Methods", &methods)],
            ));
        }
        Ok((ids, text))
    }

    /// The name of the struct that holds the options of the constructor of `object`.
    fn options_type(&self, object: &ObjectType) -> Result<String, String> {
        options_type_name(&class_name(object), |name| {
            self.objects.values().any(|other| {
                class_name(other) == name
                    || other
                        .variants
                        .iter()
                        .any(|variant| values::rust_class(&variant.name, &variant.export) == name)
            }) || self.enums.keys().any(|declared| pascal(declared) == name)
        })
    }

    /// The struct that carries the options of the constructor of `object`, and `new_with`, the constructor that takes it after the
    /// positional properties (`params` and `inits` are the ones `new` takes).
    fn options_source(
        &self,
        object: &ObjectType,
        ids_name: &str,
        params: &str,
        inits: &[String],
        options: &[&Property],
    ) -> Result<(String, String), String> {
        let name = class_name(object);
        let options_name = self.options_type(object)?;
        if params.contains(" options:") {
            return Err(format!(
                "`{}`: a constructor parameter is called `options`, which is the name of the options struct's parameter",
                object.name
            ));
        }
        let mut fields = Vec::new();
        let mut writes = Vec::new();
        let mut moves = false;
        for property in options {
            let ident = property_ident(property);
            let (ty, source) = match &property.ty {
                // A closure is moved into the value the library holds; any other value is borrowed, so an object it names is still held
                // when the library reads the create call.
                Type::Callback(callback) => {
                    moves = true;
                    (
                        format!("Box<dyn {}>", self.closure_bound(callback)?),
                        format!("options.{ident}.take()"),
                    )
                }
                other => (self.owned(other)?, format!("&options.{ident}")),
            };
            fields.push(format!(
                "    /// The property `{}`.\n    pub {ident}: Option<{ty}>,",
                property.name
            ));
            writes.push(format!(
                "        if let Some(value) = {source} {{\n            inits.push((ids.p_{}, {}));\n        }}",
                snake(&property.name),
                self.arg_expr("value", &property.ty, false)?
            ));
        }
        let declared = fill(
            r#"/// The properties `@Name@::new_with` sets in the call that makes the object, besides its positional ones. Each `Some` field is sent
/// with the create call, so a value the library refuses fails the call and leaves no object, and a `None` field is not sent, so
/// that property keeps the library's default: `@Options@ { field: Some(value), ..Default::default() }`.
#[derive(Default)]
pub struct @Options@ {
@Fields@
}

"#,
            &[
                ("Name", &name),
                ("Options", &options_name),
                ("Fields", &fields.join("\n")),
            ],
        );
        let new_with = fill(
            r#"    /// Makes the object with `options` set in the same call.
    ///
    /// # Errors
    ///
    /// The library refuses an argument or an option, and no object is made.
    pub fn new_with(objects: &Objects@Params@, @Mut@options: @Options@) -> go::Res<@Name@> {
        let ids = @Ids@::of(objects)?;
        let mut inits = vec![@Inits@];
@Writes@
        let object = go::client::Object::create(objects.api, ids.class, &inits).map_err(|fail| go::typed::at_class(fail, @NameLit@, "new"))?;
        Ok(@Name@ { object, objects: objects.clone() })
    }

"#,
            &[
                ("Params", params),
                ("Mut", if moves { "mut " } else { "" }),
                ("Options", &options_name),
                ("Name", &name),
                ("Ids", ids_name),
                ("Inits", &inits.join(", ")),
                ("Writes", &writes.join("\n")),
                ("NameLit", &format!("{name:?}")),
            ],
        );
        Ok((declared, new_with))
    }

    fn property_source(
        &self,
        out: &mut String,
        class: &str,
        ids: &str,
        property: &Property,
        claim: &mut impl FnMut(&str) -> Result<(), String>,
    ) -> Result<(), String> {
        let ident = property_ident(property);
        let id = format!("ids.p_{}", snake(&property.name));
        let schema = &property.name;
        if let Type::Callback(_) = property.ty {
            let setter = format!("set_{}", property_stem(property));
            claim(&setter)?;
            let param = self.param(&property.ty)?;
            let value = self.arg_expr("run", &property.ty, false)?;
            let _ = write!(
                out,
                "    /// Sets the closure `{schema}`.\n    ///\n    /// # Errors\n    ///\n    /// The library refuses it.\n    pub fn {setter}(&self, run: {param}) -> go::Res<()> {{\n        let objects = &self.objects;\n        let ids = {ids}::of(objects)?;\n        let value = {value};\n        objects.api.set(self.object.handle(), {id}, &value).map_err(|fail| self.object.at(fail, {class:?}, {schema:?}))\n    }}\n\n"
            );
            return Ok(());
        }
        claim(&ident)?;
        let getter = self.getter_type(&property.ty)?;
        let _ = write!(
            out,
            "    /// The property `{schema}`.\n    ///\n    /// # Errors\n    ///\n    /// The object is gone, or the library cannot read it.\n    pub fn {ident}(&self) -> go::Res<{getter}> {{\n        let ids = {ids}::of(&self.objects)?;\n        let value = self.objects.api.get(self.object.handle(), {id}).map_err(|fail| self.object.at(fail, {class:?}, {schema:?}))?;\n        go::typed::FromVal::from_val(&self.objects, &value)\n    }}\n\n"
        );
        if !property.readonly {
            let setter = format!("set_{}", property_stem(property));
            claim(&setter)?;
            let param = self.param(&property.ty)?;
            let _ = write!(
                out,
                "    /// Sets the property `{schema}`.\n    ///\n    /// # Errors\n    ///\n    /// The value is refused.\n    pub fn {setter}(&self, value: {param}) -> go::Res<()> {{\n        let ids = {ids}::of(&self.objects)?;\n        self.objects.api.set(self.object.handle(), {id}, &go::typed::ToVal::to_val(&value)).map_err(|fail| self.object.at(fail, {class:?}, {schema:?}))\n    }}\n\n"
            );
            if property.animated {
                let timed = format!("set_{}_timed", property_stem(property));
                claim(&timed)?;
                let _ = write!(
                    out,
                    "    /// Sets the property `{schema}` over a time.\n    ///\n    /// # Errors\n    ///\n    /// The value is refused.\n    pub fn {timed}(&self, value: {param}, timing: &go::Timing) -> go::Res<()> {{\n        let ids = {ids}::of(&self.objects)?;\n        self.objects.api.set_timed(self.object.handle(), {id}, &go::typed::ToVal::to_val(&value), timing).map_err(|fail| self.object.at(fail, {class:?}, {schema:?}))\n    }}\n\n"
                );
            }
        }
        let watch = format!("on_{}_changed", property_stem(property));
        claim(&watch)?;
        let owned = self.owned(&property.ty)?;
        let _ = write!(
            out,
            "    /// Calls `run` from the library's turn after the property `{schema}` changes, until the guard is dropped.\n    ///\n    /// # Errors\n    ///\n    /// The object is gone.\n    pub fn {watch}(&self, mut run: impl FnMut({owned}) + 'static) -> go::Res<go::typed::Listener> {{\n        let ids = {ids}::of(&self.objects)?;\n        let objects = self.objects.clone();\n        let reg = self.objects.api.listen(self.object.handle(), {id}, move |_, args| {{\n            run(go::typed::arg(&objects, args, 0)?);\n            Ok(go::Val::None)\n        }}).map_err(|fail| self.object.at(fail, {class:?}, {schema:?}))?;\n        Ok(go::typed::Listener::new(self.objects.api, reg))\n    }}\n\n"
        );
        Ok(())
    }

    fn params_source(&self, fields: &[Field]) -> Result<(String, Vec<String>), String> {
        let mut params = Vec::new();
        let mut args = Vec::new();
        for field in fields {
            let ident = rust_ident(&snake(&field.name));
            let param = self.param(&field.ty)?;
            params.push(format!(
                ", {ident}: {}",
                if field.optional {
                    format!("Option<{param}>")
                } else {
                    param
                }
            ));
            args.push(self.arg_expr(&ident, &field.ty, field.optional)?);
        }
        Ok((params.concat(), args))
    }

    fn method_source(
        &self,
        out: &mut String,
        class: &str,
        ids: &str,
        method: &Method,
        claim: &mut impl FnMut(&str) -> Result<(), String>,
    ) -> Result<(), String> {
        let ident = member_ident(&method.name, &method.extras.export.rust);
        claim(&ident)?;
        let (params, args) = self.params_source(&method.params)?;
        let returns = match &method.returns {
            Some(ty) if method.optional => format!("Option<{}>", self.owned(ty)?),
            Some(ty) => self.owned(ty)?,
            None => "()".to_owned(),
        };
        let schema = &method.name;
        let args = args.join(", ");
        if method.function {
            let _ = write!(
                out,
                "    /// The function `{schema}`.\n    ///\n    /// # Errors\n    ///\n    /// The library refuses the arguments.\n    pub fn {ident}(objects: &Objects{params}) -> go::Res<{returns}> {{\n        let ids = {ids}::of(objects)?;\n        let value = objects.api.call_static(ids.class, ids.m_{name}, &[{args}]).map_err(|fail| go::typed::at_class(fail, {class:?}, {schema:?}))?;\n        go::typed::FromVal::from_val(objects, &value)\n    }}\n\n",
                name = snake(schema),
            );
        } else {
            let _ = write!(
                out,
                "    /// The method `{schema}`.\n    ///\n    /// # Errors\n    ///\n    /// The object is gone, or the library refuses the arguments.\n    pub fn {ident}(&self{params}) -> go::Res<{returns}> {{\n        let objects = &self.objects;\n        let ids = {ids}::of(objects)?;\n        let value = objects.api.call(self.object.handle(), ids.m_{name}, &[{args}]).map_err(|fail| self.object.at(fail, {class:?}, {schema:?}))?;\n        go::typed::FromVal::from_val(objects, &value)\n    }}\n\n",
                name = snake(schema),
            );
        }
        Ok(())
    }

    /// The extension trait that gives the core's typed class `target` the members of the class `class`, which holds an augment: the trait is
    /// the library's and is implemented for the core's class, so a shell sees the members only when it uses this library's prelude. A member
    /// is read or called through the service of this library, made for the generation the core's object belongs to. A property is always
    /// there, so it reads as the value and not as an `Option`.
    fn augment_source(&self, class: &ObjectType, target: &ObjectType) -> Result<String, String> {
        let service = class_name(class);
        let core = format!("::gnoblin_core::objects::{}", class_name(target));
        let trait_name = format!("{}StdExt", class_name(target));
        let mut declared = String::new();
        let mut defined = String::new();
        for property in &class.properties {
            let ident = property_ident(property);
            let owned = self.owned(&property.ty)?;
            let _ = write!(
                declared,
                "    /// The property `{}`.\n    ///\n    /// # Errors\n    ///\n    /// The generation has retired, or the library cannot read it.\n    fn {ident}(&self) -> go::Res<{owned}>;\n\n",
                property.name
            );
            let read = if matches!(property.ty, Type::ObjectRef(_)) {
                format!(
                    "{service}::get(&super::std_objects(self.handle().api())?)?.{ident}()?.ok_or_else(|| go::Fail::err(\"the library gave no `{}`\"))",
                    property.name
                )
            } else {
                format!("{service}::get(&super::std_objects(self.handle().api())?)?.{ident}()")
            };
            let _ = write!(
                defined,
                "    fn {ident}(&self) -> go::Res<{owned}> {{\n        {read}\n    }}\n\n"
            );
        }
        // A receiver augment's methods take the object they act on first, which the trait's `self` is.
        let receiver = class.augment_getters.is_some();
        for method in &class.methods {
            let ident = member_ident(&method.name, &method.extras.export.rust);
            let own_params = if receiver {
                &method.params[1..]
            } else {
                &method.params[..]
            };
            let (params, _) = self.params_source(own_params)?;
            let returns = match &method.returns {
                Some(ty) if method.optional => format!("Option<{}>", self.owned(ty)?),
                Some(ty) => self.owned(ty)?,
                None => "()".to_owned(),
            };
            let given: Vec<String> = receiver
                .then(|| "self".to_owned())
                .into_iter()
                .chain(own_params.iter().map(|field| rust_ident(&snake(&field.name))))
                .collect();
            let kind = if class
                .augment_getters
                .as_ref()
                .is_some_and(|getters| getters.contains(&method.name))
            {
                "property"
            } else {
                "method"
            };
            let _ = write!(
                declared,
                "    /// The {kind} `{}`.\n    ///\n    /// # Errors\n    ///\n    /// The generation has retired, or the library refuses the arguments.\n    fn {ident}(&self{params}) -> go::Res<{returns}>;\n\n",
                method.name
            );
            let _ = write!(
                defined,
                "    fn {ident}(&self{params}) -> go::Res<{returns}> {{\n        {service}::get(&super::std_objects(self.handle().api())?)?.{ident}({})\n    }}\n\n",
                given.join(", ")
            );
        }
        // A finder the augment declares (`find = "Kind"`) is a kind of the core class's `find::<T>`: the kind is this library's class, so
        // the impl of the core's `Finds` lives here.
        let mut finds = String::new();
        for (kind, method) in augment_finds(class) {
            let Some(Type::List(inner)) = &method.returns else {
                return Err(format!(
                    "`{}`: the finder of `{kind}` does not return a list",
                    class.name
                ));
            };
            let _ = write!(
                finds,
                "impl ::gnoblin_core::objects::Finds<{core}> for {found} {{\n    type Filter = {filter};\n    const KIND: &'static str = {kind:?};\n\n    fn find_in(service: &{core}, filter: Option<&{filter}>) -> go::Res<Vec<{found}>> {{\n        {trait_name}::{ident}(service, filter)\n    }}\n}}\n\n",
                found = self.owned(inner)?,
                filter = self.owned(&method.params[0].ty)?,
                ident = member_ident(&method.name, &method.extras.export.rust),
            );
        }
        Ok(format!(
            "/// The members `{ns}` adds to the core's `{name}`: `use gnoblin_std::prelude::*` brings them in, and a shell that never does has none.\n///\n/// Each is read through the library's service for the generation the object belongs to, so a member never binds to a library of another\n/// generation.\npub trait {trait_name} {{\n{declared}}}\n\nimpl {trait_name} for {core} {{\n{defined}}}\n\n{finds}",
            ns = self.namespace,
            name = class_name(target),
        ))
    }

    fn signal_source(
        &self,
        out: &mut String,
        class: &str,
        ids: &str,
        signal: &Signal,
        claim: &mut impl FnMut(&str) -> Result<(), String>,
    ) -> Result<(), String> {
        let ident = format!("on_{}", member_ident(&signal.name, &signal.extras.export.rust));
        claim(&ident)?;
        let types: Vec<String> = signal
            .fields
            .iter()
            .map(|field| self.field_type(field))
            .collect::<Result<_, _>>()?;
        let reads: Vec<String> = (0..signal.fields.len())
            .map(|index| format!("go::typed::arg(&objects, args, {index})?"))
            .collect();
        let schema = &signal.name;
        // A signal with no fields reads nothing, so it needs neither the namespace handle nor the arguments.
        let (keep, args) = if signal.fields.is_empty() {
            ("", "_args")
        } else {
            ("        let objects = self.objects.clone();\n", "args")
        };
        let _ = write!(
            out,
            "    /// Calls `run` from the library's turn each time the signal `{schema}` is told, until the guard is dropped.\n    ///\n    /// # Errors\n    ///\n    /// The object is gone.\n    pub fn {ident}(&self, mut run: impl FnMut({types}) + 'static) -> go::Res<go::typed::Listener> {{\n        let ids = {ids}::of(&self.objects)?;\n{keep}        let reg = self.objects.api.listen(self.object.handle(), ids.s_{name}, move |_, {args}| {{\n            run({reads});\n            Ok(go::Val::None)\n        }}).map_err(|fail| self.object.at(fail, {class:?}, {schema:?}))?;\n        Ok(go::typed::Listener::new(self.objects.api, reg))\n    }}\n\n",
            types = types.join(", "),
            reads = reads.join(", "),
            name = snake(schema),
        );
        Ok(())
    }

    fn interface_source(&self, object: &'a ObjectType) -> Result<(String, String), String> {
        let name = class_name(object);
        let ids_name = format!("{name}Ids");
        if !object.properties.is_empty() || !object.signals.is_empty() {
            return Err(format!(
                "the interface `{}` declares members that are not methods",
                object.name
            ));
        }
        let methods: Vec<&Method> = object.methods.iter().collect();
        let mut id_fields = Vec::new();
        let mut id_reads = Vec::new();
        let mut handlers = Vec::new();
        let mut moves = Vec::new();
        let mut copies = Vec::new();
        let mut marks = Vec::new();
        let mut arms = Vec::new();
        for method in &methods {
            let schema = &method.name;
            let ident = member_ident(schema, &method.extras.export.rust);
            let field = format!("m_{}", snake(schema));
            id_fields.push(format!("    {field}: go::MemberId,"));
            id_reads.push(format!(
                "            {field}: go::typed::method(api, class, {schema:?})?,"
            ));
            let params: Vec<String> = method
                .params
                .iter()
                .map(|field| self.field_type(field))
                .collect::<Result<_, _>>()?;
            let returns = match &method.returns {
                Some(ty) => self.owned(ty)?,
                None => "()".to_owned(),
            };
            let closure = format!("Box<dyn FnMut({}) -> go::Res<{returns}>>", params.join(", "));
            let optional = !method.required;
            handlers.push(format!(
                "    /// The method `{schema}`{}.\n    pub {ident}: {},",
                if optional {
                    ", which the library has a default for"
                } else {
                    ""
                },
                if optional {
                    format!("Option<{closure}>")
                } else {
                    closure
                }
            ));
            let local = format!("h_{}", snake(schema));
            moves.push(format!("{ident}: mut {local}"));
            copies.push(format!("let {field} = ids.{field};"));
            let reads: Vec<String> = (0..method.params.len())
                .map(|index| format!("go::typed::arg(objects, args, {index})?"))
                .collect();
            let reads = reads.join(", ");
            if optional {
                marks.push(format!("if {local}.is_some() {{ implemented.push(ids.{field}); }}"));
                arms.push(format!(
                    "if member == {field} {{ return match &mut {local} {{ Some(run) => {{ let answer = run({reads})?; Ok(go::typed::ToVal::to_val(&answer)) }}, None => Err(go::Fail::unsupported(\"`{schema}` is not implemented\")) }}; }}"
                ));
            } else {
                marks.push(format!("implemented.push(ids.{field});"));
                arms.push(format!(
                    "if member == {field} {{ let answer = {local}({reads})?; return Ok(go::typed::ToVal::to_val(&answer)); }}"
                ));
            }
        }
        let handlers_name = format!("{name}Handlers");
        let mut ids = fill(
            r#"#[derive(Debug)]
struct @Ids@ {
    class: go::ClassId,
@Fields@
}

impl @Ids@ {
    fn of(objects: &Objects) -> go::Res<&@Ids@> {
        objects.ids.@Field@.get(|| {
            let api = &objects.api;
            let class = api.class(@Schema@)?;
            Ok(@Ids@ {
                class,
@Reads@
            })
        })
    }
}

"#,
            &[
                ("Ids", &ids_name),
                ("Fields", &id_fields.join("\n")),
                ("Field", &rust_ident(&snake(&object.name))),
                ("Schema", &format!("{:?}", object.name)),
                ("Reads", &id_reads.join("\n")),
            ],
        );
        ids.push('\n');
        let text = fill(
            r#"/// The handlers of the interface `@Schema@`.
pub struct @Handlers@ {
@Declared@
}

impl @Handlers@ {
    /// Makes the object the library calls these handlers through. The library calls them from its turn, never inside a call of yours.
    ///
    /// # Errors
    ///
    /// The library refuses the object.
    pub fn implement(self, objects: &Objects) -> go::Res<@Name@> {
        let ids = @Ids@::of(objects)?;
        let @Handlers@ { @Moves@ } = self;
        let mut implemented = Vec::new();
        @Marks@
        @Copies@
        let objects_for_args = objects.clone();
        let obj = objects.api.implement(ids.class, &implemented, move |member, args| {
            let objects = &objects_for_args;
            @Arms@
            Err(go::Fail::unsupported("the library called a method this interface does not have"))
        }).map_err(|fail| go::typed::at_class(fail, @NameLit@, "implement"))?;
        Ok(@Name@ { object: go::client::Object::adopt(objects.api, ids.class, obj) })
    }
}

/// An object that implements the interface `@Schema@`.
#[derive(Debug)]
pub struct @Name@ {
    object: go::client::Object,
}

impl @Name@ {
    /// The reference this value holds.
    pub fn handle(&self) -> &go::client::Object {
        &self.object
    }
}

impl go::typed::ToVal for @Name@ {
    fn to_val(&self) -> go::Val {
        self.object.val()
    }
}

/// One object, whichever value holds it: two deliveries of it are equal.
impl PartialEq for @Name@ {
    fn eq(&self, other: &@Name@) -> bool {
        self.object == other.object
    }
}

impl Eq for @Name@ {}

impl ::std::hash::Hash for @Name@ {
    fn hash<H: ::std::hash::Hasher>(&self, state: &mut H) {
        self.object.hash(state);
    }
}

impl go::typed::FromVal<Objects> for @Name@ {
    fn from_val(cx: &Objects, value: &go::Val) -> go::Res<@Name@> {
        Ok(@Name@ { object: go::client::Object::from_val(cx.api, value)? })
    }
}

"#,
            &[
                ("Schema", &object.name),
                ("Name", &name),
                ("NameLit", &format!("{name:?}")),
                ("Handlers", &handlers_name),
                ("Ids", &ids_name),
                ("Declared", &handlers.join("\n")),
                ("Moves", &moves.join(", ")),
                ("Marks", &marks.join("\n        ")),
                ("Copies", &copies.join("\n        ")),
                ("Arms", &arms.join("\n            ")),
            ],
        );
        Ok((ids, text))
    }

    /// A function that wraps a Rust closure as the value of a callback type.
    fn callback_source(&self, declared: &CallbackDecl) -> Result<String, String> {
        let closure = self.closure_type(&declared.name)?;
        let reads: Vec<String> = (0..declared.params.len())
            .map(|index| format!("go::typed::arg(&objects, args, {index})?"))
            .collect();
        // A callback with no parameters reads nothing: it keeps neither the objects nor the arguments.
        let template = if declared.params.is_empty() {
            r#"fn make_@Name@(_objects: &Objects, mut run: @Closure@) -> go::Val {
    go::typed::callback(move |_args| run().map(|answer| go::typed::ToVal::to_val(&answer)))
}

"#
        } else {
            r#"fn make_@Name@(objects: &Objects, mut run: @Closure@) -> go::Val {
    let objects = objects.clone();
    go::typed::callback(move |args| run(@Reads@).map(|answer| go::typed::ToVal::to_val(&answer)))
}

"#
        };
        Ok(fill(
            template,
            &[
                ("Name", &snake(&declared.name)),
                ("Closure", &closure),
                ("Reads", &reads.join(", ")),
            ],
        ))
    }
}

/// The typed API of `namespace`.
/// The trait a service's generic `find` is written over: a kind of object the service can list, and the filter that kind takes.
const FINDS: &str = r#"/// A kind of object the service `S` can list: `S::find::<Self>(filter)`. A kind a service does not list has no impl, so asking
/// for it is refused when the shell is built, with the service named.
pub trait Finds<S>: Sized {
    /// The record the kind is filtered by.
    type Filter;
    /// The kind's name, as a script writes it (`"Window"`).
    const KIND: &'static str;
    /// Lists the objects of the kind that match `filter`, or every one when it is left out.
    ///
    /// # Errors
    ///
    /// The service is gone, or the library refuses the filter.
    fn find_in(service: &S, filter: Option<&Self::Filter>) -> go::Res<Vec<Self>>;
}

"#;

pub(super) fn render(catalog: &Catalog, namespace: &Namespace) -> Result<String, String> {
    let gen_ = Renderer::new(catalog, namespace)?;
    let mut body = String::new();
    let mut ids = String::new();
    let mut fields = Vec::new();
    for declared in gen_.enums.values() {
        body.push_str(&gen_.enum_source(declared));
    }
    for declared in gen_.callbacks.values() {
        body.push_str(&gen_.callback_source(declared)?);
    }
    for object in gen_.objects.values() {
        match object.kind {
            ObjectKind::Record => body.push_str(&gen_.record_source(object)?),
            ObjectKind::Handle | ObjectKind::Service => {
                let (class_ids, text) = gen_.handle_source(object)?;
                ids.push_str(&class_ids);
                fields.push((rust_ident(&snake(&object.name)), format!("{}Ids", class_name(object))));
                body.push_str(&text);
            }
            ObjectKind::Interface => {
                let (class_ids, text) = gen_.interface_source(object)?;
                ids.push_str(&class_ids);
                fields.push((rust_ident(&snake(&object.name)), format!("{}Ids", class_name(object))));
                body.push_str(&text);
            }
        }
    }
    for (class, target) in augments_of(catalog, &gen_.objects)? {
        body.push_str(&gen_.augment_source(class, target)?);
    }
    // A class of another library that this one hands out is read through that library's typed API, for the generation of the value.
    for (home, class) in foreign_handles(catalog, namespace)? {
        let path = format!("{}::{}", crate_path_of(&home)?, class_name(class));
        let _ = write!(
            body,
            "impl go::typed::FromVal<Objects> for {path} {{\n    fn from_val(cx: &Objects, value: &go::Val) -> go::Res<{path}> {{\n        let home = {}::for_generation(cx.api.ctx())?;\n        go::typed::FromVal::from_val(&home, &go::typed::foreign_in(value, {:?})?)\n    }}\n}}\n\n",
            crate_path_of(&home)?,
            home.as_str()
        );
    }
    let ids_fields: Vec<String> = fields
        .iter()
        .map(|(field, ty)| format!("    {field}: go::typed::Lazy<{ty}>,"))
        .collect();
    let mut text = fill(
        r#"// The typed API of `@Namespace@` over the object protocol's client. See `gnoblin-sdkgen`'s `object_api` for what each kind of class becomes.

use ::gnoblin_object as go;

/// The ids of every class, looked up by name the first time a call needs them.
#[derive(Debug, Default)]
struct Ids {
@IdFields@
}

/// The typed API of `@Namespace@` for one generation: one library's object table and the ids resolved from it. Cheap to clone.
#[derive(Clone, Debug)]
pub struct Objects {
    api: go::client::Api,
    ids: ::std::rc::Rc<Ids>,
}

impl Objects {
    /// The typed API over `api`, which is the table of the library that serves `@Namespace@`.
    pub fn new(api: go::client::Api) -> Objects {
        Objects { api, ids: ::std::rc::Rc::default() }
    }

    /// The untyped protocol this API is written over.
    pub fn api(&self) -> go::client::Api {
        self.api
    }
}

"#,
        &[("Namespace", namespace.as_str()), ("IdFields", &ids_fields.join("\n"))],
    );
    if body.contains("impl Finds<") {
        text.push_str(FINDS);
    }
    text.push_str(&body);
    text.push_str(&ids);
    Ok(text)
}
