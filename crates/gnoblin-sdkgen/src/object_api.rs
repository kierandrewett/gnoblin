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
//! A member whose type names a class of another namespace is refused with a message that says so: a typed reference across two
//! libraries needs both libraries' `Objects`, which no catalog needs yet.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use gnoblin_schema::{
    CallbackDecl, Catalog, DefaultValue, EnumDecl, Field, Method, Namespace, ObjectKind, ObjectType, Property, Signal,
    Type,
};

use super::objects::{Member, flatten, foreign_base, stem_of};
use super::text::{pascal, rust_ident, snake};
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

/// A member to render, and whether another library's class owns it.
struct ClassMember<'a> {
    member: Member<'a>,
    foreign: bool,
}

fn class_name(object: &ObjectType) -> String {
    object.export.rust.clone().unwrap_or_else(|| pascal(&object.name))
}

fn member_ident(name: &str, export: &Option<String>) -> String {
    rust_ident(&export.clone().unwrap_or_else(|| snake(name)))
}

fn property_ident(property: &Property) -> String {
    member_ident(&property.name, &property.extras.export.rust)
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

    /// The class `name` names, or an error saying why the typed API cannot render a reference to it.
    fn class(&self, name: &str) -> Result<&'a ObjectType, String> {
        if name.contains('.') {
            return Err(format!(
                "`{name}` is a class of another namespace: a typed reference across two libraries is not rendered, because it needs both \
                 libraries' `Objects`"
            ));
        }
        self.objects
            .get(name)
            .copied()
            .ok_or_else(|| format!("`{name}` is not a class of `{}`", self.namespace))
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
            Type::ObjectRef(name) => format!("&{}", class_name(self.class(name)?)),
            Type::List(inner) => format!("&[{}]", self.owned(inner)?),
            Type::Callback(name) => self.closure_type(name)?,
            other => self.owned(other)?,
        })
    }

    /// The closure type of a callback declaration.
    fn closure_type(&self, name: &str) -> Result<String, String> {
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
        Ok(format!(
            "impl FnMut({}) -> go::Res<{returns}> + 'static",
            params.join(", ")
        ))
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

    fn record_source(&self, object: &ObjectType) -> Result<String, String> {
        let name = class_name(object);
        if !object.methods.is_empty() || !object.signals.is_empty() {
            return Err(format!(
                "the record `{}` declares members that are not properties",
                object.name
            ));
        }
        let mut declared = Vec::new();
        let mut starts = Vec::new();
        let mut required: Vec<(String, String)> = Vec::new();
        let mut writes = Vec::new();
        let mut reads = Vec::new();
        for (index, property) in object.properties.iter().enumerate() {
            let field = property_ident(property);
            let owned = self.owned(&property.ty)?;
            // A field with a default starts at it; one marked `optional` is an `Option` that starts as `None`; any other is always there,
            // so a value of the record is made with it (`new`) and a read that finds it missing is an error.
            let (ty, start, read) = match (&property.default, property.optional) {
                (Some(default), _) => {
                    let default = self.default_expr(&property.ty, default)?;
                    let read = format!("go::typed::field(cx, fields, {index}, || {default})?");
                    (owned, default, read)
                }
                (None, true) => (
                    format!("Option<{owned}>"),
                    "None".to_owned(),
                    format!("go::typed::field(cx, fields, {index}, || None)?"),
                ),
                (None, false) => {
                    required.push((field.clone(), owned.clone()));
                    (owned, field.clone(), format!("go::typed::arg(cx, fields, {index})?"))
                }
            };
            declared.push(format!("    pub {field}: {ty},"));
            starts.push(format!("            {field}: {start},"));
            writes.push(format!("go::typed::ToVal::to_val(&self.{field})"));
            reads.push(format!("            {field}: {read},"));
        }
        let plain = object.properties.iter().all(|property| self.plain(&property.ty, 0));
        let mut text = String::new();
        let _ = writeln!(text, "/// The record `{}`.", object.name);
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
        Ok(@Name@ {
@Reads@
        })
    }
}

"#,
            &[
                ("Name", &name),
                ("Declared", &declared.join("\n")),
                ("Start", &start),
                ("Writes", &writes.join(", ")),
                ("Reads", &reads.join("\n")),
            ],
        ));
        Ok(text)
    }

    /// The members of a class as ids number them, with whether another library owns each.
    fn slots(&self, object: &'a ObjectType) -> Vec<ClassMember<'a>> {
        let mut slots: Vec<ClassMember<'a>> = foreign_base(self.catalog, object)
            .map(|(_, members)| members)
            .unwrap_or_default()
            .into_iter()
            .map(|member| ClassMember { member, foreign: true })
            .collect();
        slots.extend(
            flatten(&self.objects, object)
                .into_iter()
                .map(|member| ClassMember { member, foreign: false }),
        );
        slots
    }

    /// Refuses a member of a base class of another namespace whose types name that namespace's classes.
    fn check_foreign(&self, class: &str, slot: &ClassMember<'_>) -> Result<(), String> {
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

    fn handle_source(&self, object: &'a ObjectType) -> Result<(String, String), String> {
        let name = class_name(object);
        let ids_name = format!("{name}Ids");
        let slots = self.slots(object);
        for slot in &slots {
            self.check_foreign(&object.name, slot)?;
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

        let mut taken: BTreeSet<String> = ["new", "get", "handle", "destroy", "objects"]
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
        // The constructor.
        if has_instance_members {
            let new_name = if object.kind == ObjectKind::Service {
                "get"
            } else {
                "new"
            };
            let ctor_names: Vec<String> = object.ctor.clone().unwrap_or_default();
            let mut params = Vec::new();
            let mut inits = Vec::new();
            for ctor in &ctor_names {
                let property = slots
                    .iter()
                    .find_map(|slot| match &slot.member {
                        Member::Property(property) if &property.name == ctor => Some(*property),
                        _ => None,
                    })
                    .ok_or_else(|| {
                        format!(
                            "`{}`: the constructor names `{ctor}`, which is not a property",
                            object.name
                        )
                    })?;
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
        let object = go::client::Object::create(objects.api, ids.class, &[@Inits@])?;
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
        self.object.destroy()
    }

"#,
                &[
                    ("New", new_name),
                    ("Params", &params),
                    ("Name", &name),
                    ("Ids", &ids_name),
                    ("Inits", &inits.join(", ")),
                ],
            ));
        }

        for slot in &slots {
            match &slot.member {
                Member::Property(property) => {
                    self.property_source(&mut methods, &name, &ids_name, property, &mut claim)?;
                }
                Member::Method(method) => self.method_source(&mut methods, &ids_name, method, &mut claim)?,
                Member::Signal(signal) => self.signal_source(&mut methods, &ids_name, signal, &mut claim)?,
            }
        }

        let mut text = String::new();
        let _ = writeln!(text, "/// The class `{}`.", object.name);
        if has_instance_members {
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

impl go::typed::FromVal<Objects> for @Name@ {
    fn from_val(cx: &Objects, value: &go::Val) -> go::Res<@Name@> {
        Ok(@Name@ { object: go::client::Object::from_val(cx.api, value)?, objects: cx.clone() })
    }
}

"#,
                &[("Name", &name), ("Methods", &methods)],
            ));
        } else {
            text.push_str(&fill(
                "#[derive(Clone, Copy, Debug)]\npub struct @Name@;\n\nimpl @Name@ {\n@Methods@}\n\n",
                &[("Name", &name), ("Methods", &methods)],
            ));
        }
        Ok((ids, text))
    }

    fn property_source(
        &self,
        out: &mut String,
        _class: &str,
        ids: &str,
        property: &Property,
        claim: &mut impl FnMut(&str) -> Result<(), String>,
    ) -> Result<(), String> {
        let ident = property_ident(property);
        let id = format!("ids.p_{}", snake(&property.name));
        let schema = &property.name;
        if let Type::Callback(_) = property.ty {
            let setter = format!("set_{ident}");
            claim(&setter)?;
            let param = self.param(&property.ty)?;
            let value = self.arg_expr("run", &property.ty, false)?;
            let _ = write!(
                out,
                "    /// Sets the closure `{schema}`.\n    ///\n    /// # Errors\n    ///\n    /// The library refuses it.\n    pub fn {setter}(&self, run: {param}) -> go::Res<()> {{\n        let objects = &self.objects;\n        let ids = {ids}::of(objects)?;\n        let value = {value};\n        objects.api.set(self.object.handle(), {id}, &value)\n    }}\n\n"
            );
            return Ok(());
        }
        claim(&ident)?;
        let getter = self.getter_type(&property.ty)?;
        let _ = write!(
            out,
            "    /// The property `{schema}`.\n    ///\n    /// # Errors\n    ///\n    /// The object is gone, or the library cannot read it.\n    pub fn {ident}(&self) -> go::Res<{getter}> {{\n        let ids = {ids}::of(&self.objects)?;\n        let value = self.objects.api.get(self.object.handle(), {id})?;\n        go::typed::FromVal::from_val(&self.objects, &value)\n    }}\n\n"
        );
        if !property.readonly {
            let setter = format!("set_{ident}");
            claim(&setter)?;
            let param = self.param(&property.ty)?;
            let _ = write!(
                out,
                "    /// Sets the property `{schema}`.\n    ///\n    /// # Errors\n    ///\n    /// The value is refused.\n    pub fn {setter}(&self, value: {param}) -> go::Res<()> {{\n        let ids = {ids}::of(&self.objects)?;\n        self.objects.api.set(self.object.handle(), {id}, &go::typed::ToVal::to_val(&value))\n    }}\n\n"
            );
            if property.animated {
                let timed = format!("set_{ident}_timed");
                claim(&timed)?;
                let _ = write!(
                    out,
                    "    /// Sets the property `{schema}` over a time.\n    ///\n    /// # Errors\n    ///\n    /// The value is refused.\n    pub fn {timed}(&self, value: {param}, timing: &go::Timing) -> go::Res<()> {{\n        let ids = {ids}::of(&self.objects)?;\n        self.objects.api.set_timed(self.object.handle(), {id}, &go::typed::ToVal::to_val(&value), timing)\n    }}\n\n"
                );
            }
        }
        let watch = format!("on_{ident}_changed");
        claim(&watch)?;
        let owned = self.owned(&property.ty)?;
        let _ = write!(
            out,
            "    /// Calls `run` from the library's turn after the property `{schema}` changes, until the guard is dropped.\n    ///\n    /// # Errors\n    ///\n    /// The object is gone.\n    pub fn {watch}(&self, mut run: impl FnMut({owned}) + 'static) -> go::Res<go::typed::Listener> {{\n        let ids = {ids}::of(&self.objects)?;\n        let objects = self.objects.clone();\n        let reg = self.objects.api.listen(self.object.handle(), {id}, move |_, args| {{\n            run(go::typed::arg(&objects, args, 0)?);\n            Ok(go::Val::None)\n        }})?;\n        Ok(go::typed::Listener::new(self.objects.api, reg))\n    }}\n\n"
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
        ids: &str,
        method: &Method,
        claim: &mut impl FnMut(&str) -> Result<(), String>,
    ) -> Result<(), String> {
        let ident = member_ident(&method.name, &method.extras.export.rust);
        claim(&ident)?;
        let (params, args) = self.params_source(&method.params)?;
        let returns = match &method.returns {
            Some(ty) => self.owned(ty)?,
            None => "()".to_owned(),
        };
        let schema = &method.name;
        let args = args.join(", ");
        if method.function {
            let _ = write!(
                out,
                "    /// The function `{schema}`.\n    ///\n    /// # Errors\n    ///\n    /// The library refuses the arguments.\n    pub fn {ident}(objects: &Objects{params}) -> go::Res<{returns}> {{\n        let ids = {ids}::of(objects)?;\n        let value = objects.api.call_static(ids.class, ids.m_{name}, &[{args}])?;\n        go::typed::FromVal::from_val(objects, &value)\n    }}\n\n",
                name = snake(schema),
            );
        } else {
            let _ = write!(
                out,
                "    /// The method `{schema}`.\n    ///\n    /// # Errors\n    ///\n    /// The object is gone, or the library refuses the arguments.\n    pub fn {ident}(&self{params}) -> go::Res<{returns}> {{\n        let objects = &self.objects;\n        let ids = {ids}::of(objects)?;\n        let value = objects.api.call(self.object.handle(), ids.m_{name}, &[{args}])?;\n        go::typed::FromVal::from_val(objects, &value)\n    }}\n\n",
                name = snake(schema),
            );
        }
        Ok(())
    }

    fn signal_source(
        &self,
        out: &mut String,
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
            "    /// Calls `run` from the library's turn each time the signal `{schema}` is told, until the guard is dropped.\n    ///\n    /// # Errors\n    ///\n    /// The object is gone.\n    pub fn {ident}(&self, mut run: impl FnMut({types}) + 'static) -> go::Res<go::typed::Listener> {{\n        let ids = {ids}::of(&self.objects)?;\n{keep}        let reg = self.objects.api.listen(self.object.handle(), ids.s_{name}, move |_, {args}| {{\n            run({reads});\n            Ok(go::Val::None)\n        }})?;\n        Ok(go::typed::Listener::new(self.objects.api, reg))\n    }}\n\n",
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
        })?;
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

impl go::typed::FromVal<Objects> for @Name@ {
    fn from_val(cx: &Objects, value: &go::Val) -> go::Res<@Name@> {
        Ok(@Name@ { object: go::client::Object::from_val(cx.api, value)? })
    }
}

"#,
            &[
                ("Schema", &object.name),
                ("Name", &name),
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
    text.push_str(&body);
    text.push_str(&ids);
    Ok(text)
}
