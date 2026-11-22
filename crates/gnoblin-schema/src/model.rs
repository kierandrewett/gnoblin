//! The validated object model, and the catalog that checks schema files against
//! each other.
//!
//! Everything here is produced by [`crate::parse`]; the fields are public so
//! generators can read them without accessor noise, but a caller should treat a
//! parsed schema as immutable. The validation invariants (a member is never
//! newer than its schema, a name is unique in its object) hold only for values
//! that came from the parser or from [`Catalog::new`].

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use crate::error::SchemaError;
use gnoblin_types::Version;

/// A schema namespace: `gnoblin` for the compositor's own objects, or a dotted
/// extension name such as `example.ext`.
///
/// The namespace is the first field of every IPC frame, so it sorts as text and
/// has to be stable across releases.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Namespace(String);

impl Namespace {
    /// The namespace as it appears on the wire.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Parse a dotted namespace of lower-case segments.
    pub fn parse(text: &str) -> Result<Namespace, String> {
        if text.is_empty() {
            return Err("namespace is empty".to_owned());
        }
        for segment in text.split('.') {
            if !is_lower_name(segment) {
                return Err(format!("namespace segment `{segment}` must match [a-z][a-z0-9_]*"));
            }
        }
        Ok(Namespace(text.to_owned()))
    }
}

impl fmt::Display for Namespace {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// The type of a property, parameter or signal field.
///
/// The set is small on purpose: it covers the first slice of the object model
/// and every addition is a schema and generator change. How a type appears on
/// the wire is the generator's business (`xtask` maps these onto Protocol
/// Buffers), so this enum stays format-neutral.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    /// A boolean.
    Bool,
    /// A signed 64-bit integer.
    Int,
    /// An unsigned 64-bit integer, used for identities and counts.
    Uint,
    /// An IEEE 754 double.
    Double,
    /// A UTF-8 string.
    Text,
    /// Opaque octets.
    Bytes,
    /// The identity of another object type: one of the same namespace (`window`), or of another namespace's catalog, named in full
    /// (`gnoblin.model.client`).
    ObjectRef(String),
    /// A homogeneous sequence of the inner type.
    List(Box<Type>),
    /// A member of a closed set of named values the namespace declares (`[[enum]]`). Local namespaces only.
    Enum(String),
    /// A closure a shell passes in, declared by `[[callback]]`: named parameters and a result. Local namespaces only.
    Callback(String),
}

impl Type {
    /// Parse the short form used in schema files: `string`, `list<object:window>`.
    pub fn parse(text: &str) -> Result<Type, String> {
        let text = text.trim();
        if let Some(inner) = text.strip_prefix("list<").and_then(|rest| rest.strip_suffix('>')) {
            return Ok(Type::List(Box::new(Type::parse(inner)?)));
        }
        match text {
            "bool" => Ok(Type::Bool),
            "int" => Ok(Type::Int),
            "uint" => Ok(Type::Uint),
            "double" => Ok(Type::Double),
            "string" => Ok(Type::Text),
            "bytes" => Ok(Type::Bytes),
            other => {
                if let Some(name) = other.strip_prefix("object:") {
                    return if is_class_ref(name) {
                        Ok(Type::ObjectRef(name.to_owned()))
                    } else {
                        Err(format!(
                            "`{other}` is not an object type; expected [a-z][a-z0-9_]* (dotted for another namespace) after `object:`"
                        ))
                    };
                }
                if let Some(name) = other.strip_prefix("enum:") {
                    return if is_class_ref(name) {
                        Ok(Type::Enum(name.to_owned()))
                    } else {
                        Err(format!("`{other}` is not an enum type; expected a name after `enum:`"))
                    };
                }
                if let Some(name) = other.strip_prefix("callback:") {
                    return if is_lower_name(name) {
                        Ok(Type::Callback(name.to_owned()))
                    } else {
                        Err(format!(
                            "`{other}` is not a callback type; expected a name after `callback:`"
                        ))
                    };
                }
                Err(format!("unknown type `{other}`"))
            }
        }
    }

    /// Every object type this type refers to, at any depth.
    pub fn referenced_objects(&self) -> Vec<&str> {
        let mut references = Vec::new();
        self.collect_references(&mut references, |ty| match ty {
            Type::ObjectRef(name) => Some(name),
            _ => None,
        });
        references
    }

    /// Every enum this type refers to, at any depth.
    pub fn referenced_enums(&self) -> Vec<&str> {
        let mut references = Vec::new();
        self.collect_references(&mut references, |ty| match ty {
            Type::Enum(name) => Some(name),
            _ => None,
        });
        references
    }

    /// Every callback this type refers to, at any depth.
    pub fn referenced_callbacks(&self) -> Vec<&str> {
        let mut references = Vec::new();
        self.collect_references(&mut references, |ty| match ty {
            Type::Callback(name) => Some(name),
            _ => None,
        });
        references
    }

    fn collect_references<'a>(&'a self, references: &mut Vec<&'a str>, pick: fn(&'a Type) -> Option<&'a String>) {
        if let Some(name) = pick(self) {
            references.push(name);
        }
        if let Type::List(inner) = self {
            inner.collect_references(references, pick);
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Bool => formatter.write_str("bool"),
            Type::Int => formatter.write_str("int"),
            Type::Uint => formatter.write_str("uint"),
            Type::Double => formatter.write_str("double"),
            Type::Text => formatter.write_str("string"),
            Type::Bytes => formatter.write_str("bytes"),
            Type::ObjectRef(name) => write!(formatter, "object:{name}"),
            Type::List(inner) => write!(formatter, "list<{inner}>"),
            Type::Enum(name) => write!(formatter, "enum:{name}"),
            Type::Callback(name) => write!(formatter, "callback:{name}"),
        }
    }
}

/// What an object is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectKind {
    /// An identity: an instance has a lifetime and is passed by reference.
    Handle,
    /// A value: a reference to it is the properties of that value. A record declares no methods or signals.
    Record,
    /// Members the shell implements and the library calls.
    Interface,
    /// One instance per generation, with functions.
    Service,
}

impl ObjectKind {
    /// The word schema files use.
    pub fn as_str(self) -> &'static str {
        match self {
            ObjectKind::Handle => "handle",
            ObjectKind::Record => "record",
            ObjectKind::Interface => "interface",
            ObjectKind::Service => "service",
        }
    }
}

/// The names a member or an object has in a language, where the mechanical conversion of its schema name is not it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Export {
    /// The name in JavaScript and TypeScript.
    pub js: Option<String>,
    /// The name in Python.
    pub py: Option<String>,
    /// The name in Rust.
    pub rust: Option<String>,
}

/// What only the members of a local namespace carry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Extras {
    /// The member is accepted and not applied; the text says why.
    pub inert: Option<String>,
    /// The existing typed table function that serves the member, written `fn:<name>`.
    pub abi: Option<String>,
    /// The member's name in each language when it differs from the mechanical conversion.
    pub export: Export,
    /// A method that lists the live objects of one kind, and the name of that kind (`Window`): a shell reads it as `find(kind, filter)`
    /// on the object that has the method. Every method of an object that carries one names a kind of its own.
    pub find: Option<String>,
}

/// A closed set of named values: `[[enum]]`. The value's id is its place in `values`, counted from 1.
#[derive(Clone, Debug)]
pub struct EnumDecl {
    /// The name as declared.
    pub name: String,
    /// The version that added the enum.
    pub since: Version,
    /// The value names, in order. A later version only appends.
    pub values: Vec<String>,
}

/// A closure a shell passes in: `[[callback]]`.
#[derive(Clone, Debug)]
pub struct CallbackDecl {
    /// The name as declared.
    pub name: String,
    /// The version that added the callback.
    pub since: Version,
    /// The parameters, in order.
    pub params: Vec<Field>,
    /// What the closure answers, when it does.
    pub returns: Option<Type>,
}

/// One schema file: a namespace at one version, with its objects and error codes.
#[derive(Clone, Debug)]
pub struct Schema {
    /// The file the schema was read from, used in errors and generated headers.
    pub origin: String,
    /// The namespace the schema declares.
    pub namespace: Namespace,
    /// The schema version, as written in the schema file.
    pub version: Version,
    /// A library serves the namespace, not the compositor: the wire never carries it and the `gnoblin` catalog hash does not
    /// include it.
    pub local: bool,
    /// The object types declared, in file order.
    pub objects: Vec<ObjectType>,
    /// The error codes declared, in file order.
    pub errors: Vec<ErrorDecl>,
    /// The enums declared, in file order.
    pub enums: Vec<EnumDecl>,
    /// The callbacks declared, in file order.
    pub callbacks: Vec<CallbackDecl>,
}

/// An object type: an identity with typed properties, methods and signals.
#[derive(Clone, Debug)]
pub struct ObjectType {
    /// The object's name as declared in the schema.
    pub name: String,
    /// The base class this object extends, as declared. Its members are already
    /// resolved into this object, so readers never walk the chain.
    pub base: Option<String>,
    /// The version that added the object.
    pub since: Version,
    /// Marks the whole object secret, as if every member were.
    pub secret: bool,
    /// A record is a value, not an identity: a reference to it is the properties of
    /// that value on the wire, not an object id. A record declares no methods or signals.
    pub record: bool,
    /// What the object is. `record` is true exactly when this is [`ObjectKind::Record`].
    pub kind: ObjectKind,
    /// The properties a constructor takes, in order, when the object has constructor arguments of its own; `None` means every
    /// writable property, by name.
    pub ctor: Option<Vec<String>>,
    /// An instance stays until it is destroyed, whatever the shell releases (a widget on screen).
    pub retained: bool,
    /// The object's name in each language when it differs from the mechanical conversion.
    pub export: Export,
    /// For a record that is the family of a tagged value: the enum property whose value says which variant a value is.
    pub tag: Option<String>,
    /// For a family: its variants, one for each value of the `tag` enum, in the enum's order. A variant is a value class of its own in
    /// each language, and the family is the type that holds any of them; on the wire a variant is the family's record.
    pub variants: Vec<TaggedVariant>,
    /// Typed properties of the object.
    pub properties: Vec<Property>,
    /// Calls a client can make on the object.
    pub methods: Vec<Method>,
    /// Notifications the object sends to its clients.
    pub signals: Vec<Signal>,
}

impl ObjectType {
    /// True when any frame carrying this object is secret.
    pub fn is_sensitive(&self) -> bool {
        self.secret
            || self.properties.iter().any(Property::is_sensitive)
            || self.methods.iter().any(Method::is_sensitive)
            || self.signals.iter().any(Signal::is_sensitive)
    }

    /// The object as retired may see it: `None` when the whole object is secret, otherwise
    /// a copy without every property, method and signal that is itself sensitive.
    ///
    /// A method or signal that carries a secret parameter or field is removed whole,
    /// because its signature cannot be exposed without the secret.
    pub fn without_secrets(&self) -> Option<ObjectType> {
        if self.secret {
            return None;
        }
        Some(ObjectType {
            name: self.name.clone(),
            base: self.base.clone(),
            since: self.since,
            secret: false,
            record: self.record,
            kind: self.kind,
            ctor: self.ctor.clone(),
            retained: self.retained,
            export: self.export.clone(),
            tag: self.tag.clone(),
            variants: self.variants.clone(),
            properties: self
                .properties
                .iter()
                .filter(|property| !property.is_sensitive())
                .cloned()
                .collect(),
            methods: self
                .methods
                .iter()
                .filter(|method| !method.is_sensitive())
                .cloned()
                .collect(),
            signals: self
                .signals
                .iter()
                .filter(|signal| !signal.is_sensitive())
                .cloned()
                .collect(),
        })
    }

    /// Every typed member with a description, for validation messages.
    pub fn typed_members(&self) -> Vec<(String, &Type)> {
        let mut members = Vec::new();
        for property in &self.properties {
            members.push((
                format!("property `{}` of object `{}`", property.name, self.name),
                &property.ty,
            ));
        }
        for method in &self.methods {
            for parameter in &method.params {
                members.push((
                    format!("parameter `{}` of method `{}`", parameter.name, method.name),
                    &parameter.ty,
                ));
            }
        }
        for signal in &self.signals {
            for field in &signal.fields {
                members.push((format!("field `{}` of signal `{}`", field.name, signal.name), &field.ty));
            }
        }
        members
    }
}

/// One variant of a tagged family: a value class that is the family's record with one value of its `tag` property and the fields its own
/// properties carry. It is not a class of the namespace: the object table, the wire and the catalog's class ids know only the family.
#[derive(Clone, Debug)]
pub struct TaggedVariant {
    /// The class name as declared; the variant's name in each language is its conversion, or `export`.
    pub name: String,
    /// The value of the family's `tag` enum that says a value is this variant.
    pub tag: String,
    /// The version that added the variant.
    pub since: Version,
    /// The variant's name in each language when it differs from the mechanical conversion.
    pub export: Export,
    /// The variant's own properties, in order. Each says in `field` where the family record carries it.
    pub properties: Vec<Property>,
}

/// A property's default, read as the property's declared type. An object
/// reference or bytes has no TOML literal, so those types take no default.
#[derive(Clone, Debug, PartialEq)]
pub enum DefaultValue {
    /// A boolean.
    Bool(bool),
    /// A signed integer.
    Int(i64),
    /// An unsigned integer.
    Uint(u64),
    /// An IEEE 754 double.
    Double(f64),
    /// A UTF-8 string.
    Text(String),
    /// A list, each item a default of the inner type.
    List(Vec<DefaultValue>),
}

/// A typed property published in a property diff.
#[derive(Clone, Debug)]
pub struct Property {
    /// The property name as declared.
    pub name: String,
    /// The value type of the property.
    pub ty: Type,
    /// The version that added the property.
    pub since: Version,
    /// True when the value must be redacted from unauthorised readers.
    pub secret: bool,
    /// The value the property holds before any write, when the schema gives one.
    pub default: Option<DefaultValue>,
    /// True when a `set` on the property is refused `read-only`.
    pub readonly: bool,
    /// True when a `set` may carry `seconds` to animate the change; without it the set is refused `not-animated`.
    pub animated: bool,
    /// True when a record may leave the field out because it has no value to say (a delay that is none, a flag an event of another kind
    /// does not use). A field with neither a default nor this is always there. Local namespaces only, and not with a default.
    pub optional: bool,
    /// For a property of a variant: where the family record carries the value, as a field of the family (`amount`) or a field of a
    /// record that field holds (`shader.source`). The property's own name is what a script calls it, and may differ.
    pub field: Option<String>,
    /// What only a local namespace's property carries: `inert`, `abi` and the names in each language.
    pub extras: Extras,
}

impl Property {
    /// True when the property's value never leaves the compositor unredacted.
    pub fn is_sensitive(&self) -> bool {
        self.secret
    }
}

/// A call a client can make on an object.
///
/// A method returns an `Operation` in the object model; the reply carries the
/// operation and the result arrives as a signal, so the schema records request
/// parameters only.
#[derive(Clone, Debug)]
pub struct Method {
    /// The method name as declared.
    pub name: String,
    /// The version that added the method.
    pub since: Version,
    /// True when the method is secret regardless of its parameters.
    pub secret: bool,
    /// True when the method is a namespace function (`gnoblin.<object>.<name>()`), not an
    /// instance method: the host installs it on the object's class table with no receiver.
    pub function: bool,
    /// Request parameters, in declaration order.
    pub params: Vec<Field>,
    /// What the method answers, when it does (local namespaces; a method of the compositor's schema answers with an operation).
    pub returns: Option<Type>,
    /// The result may be none (`returns` is an object): `undefined`, `None` or `Option::None` in the generated SDKs.
    pub optional: bool,
    /// For an interface: the shell must implement the method.
    pub required: bool,
    /// For an interface: the library runs its own default when the shell does not implement the method.
    pub native_default: bool,
    /// The Python special method the member is (`__len__`), where a language spells the member that way.
    pub dunder: Option<String>,
    /// What only a local namespace's method carries: `inert`, `abi` and the names in each language.
    pub extras: Extras,
}

impl Method {
    /// True when any frame carrying this method is secret.
    ///
    /// A reply to `request.answer` may echo nothing, but the request
    /// carries the password, so the envelope is flagged for the whole exchange.
    pub fn is_sensitive(&self) -> bool {
        self.secret || self.params.iter().any(|parameter| parameter.secret)
    }
}

/// A notification the compositor sends to clients.
#[derive(Clone, Debug)]
pub struct Signal {
    /// The signal name as declared.
    pub name: String,
    /// The version that added the signal.
    pub since: Version,
    /// True when the signal is secret regardless of its fields.
    pub secret: bool,
    /// Payload fields, in declaration order.
    pub fields: Vec<Field>,
    /// What only a local namespace's signal carries: `inert`, `abi` and the names in each language.
    pub extras: Extras,
}

impl Signal {
    /// True when any frame carrying this signal is secret.
    pub fn is_sensitive(&self) -> bool {
        self.secret || self.fields.iter().any(|field| field.secret)
    }
}

/// A named, typed value: a method parameter or a signal field.
#[derive(Clone, Debug)]
pub struct Field {
    /// The field name as declared.
    pub name: String,
    /// The value type of the field.
    pub ty: Type,
    /// The version that added the field; defaults to the member's version.
    pub since: Version,
    /// True when the value must be redacted from unauthorised readers.
    pub secret: bool,
    /// True when a call may leave the parameter out; the method's own comment says what a missing one means. Only a
    /// method parameter is ever optional, and the optional ones are the last of a method's.
    pub optional: bool,
}

/// A declared error code a namespace can return in an error frame.
///
/// Named `ErrorDecl` and not `ErrorCode` because the two are different things: this
/// is the schema's *declaration* of a code (its name and the version that added
/// it), while [`gnoblin_types::ErrorCode`] is the kebab-case *value* a frame
/// carries. One is a member of a contract, the other is what travels.
#[derive(Clone, Debug)]
pub struct ErrorDecl {
    /// Kebab-case name, for example `provider-gone`.
    pub name: String,
    /// The version that added the code.
    pub since: Version,
}

/// Every schema file in one tree, checked against each other.
///
/// Namespace-wide facts only exist at this level: a namespace may span several
/// files, so version agreement, unique object names and resolvable object
/// references are catalog checks, not per-file ones.
#[derive(Clone, Debug)]
pub struct Catalog {
    schemas: Vec<Schema>,
}

impl Catalog {
    /// Build a catalog from parsed schemas.
    pub fn new(mut schemas: Vec<Schema>) -> Result<Catalog, SchemaError> {
        let mut problems = Vec::new();
        let mut versions: BTreeMap<&str, (Version, &str)> = BTreeMap::new();
        let mut objects: BTreeMap<&str, BTreeMap<&str, &str>> = BTreeMap::new();
        let mut errors: BTreeMap<&str, BTreeMap<&str, &str>> = BTreeMap::new();
        let mut enums: BTreeMap<&str, BTreeMap<&str, &str>> = BTreeMap::new();
        let mut callbacks: BTreeMap<&str, BTreeMap<&str, &str>> = BTreeMap::new();

        for schema in &schemas {
            let namespace = schema.namespace.as_str();
            match versions.get(namespace) {
                Some((version, origin)) if *version != schema.version => problems.push(format!(
                    "namespace `{namespace}` is version {version} in {origin} but {} in {}",
                    schema.version, schema.origin
                )),
                Some(_) => {}
                None => {
                    versions.insert(namespace, (schema.version, schema.origin.as_str()));
                }
            }
            let namespace_objects = objects.entry(namespace).or_default();
            for object in &schema.objects {
                if namespace_objects
                    .insert(object.name.as_str(), schema.origin.as_str())
                    .is_some()
                {
                    problems.push(format!(
                        "object `{}` is declared more than once in namespace `{namespace}`",
                        object.name
                    ));
                }
            }
            let namespace_errors = errors.entry(namespace).or_default();
            for error in &schema.errors {
                if namespace_errors
                    .insert(error.name.as_str(), schema.origin.as_str())
                    .is_some()
                {
                    problems.push(format!(
                        "error code `{}` is declared more than once in namespace `{namespace}`",
                        error.name
                    ));
                }
            }
            let namespace_enums = enums.entry(namespace).or_default();
            for declared in &schema.enums {
                if namespace_enums
                    .insert(declared.name.as_str(), schema.origin.as_str())
                    .is_some()
                {
                    problems.push(format!(
                        "enum `{}` is declared more than once in namespace `{namespace}`",
                        declared.name
                    ));
                }
            }
            let namespace_callbacks = callbacks.entry(namespace).or_default();
            for declared in &schema.callbacks {
                if namespace_callbacks
                    .insert(declared.name.as_str(), schema.origin.as_str())
                    .is_some()
                {
                    problems.push(format!(
                        "callback `{}` is declared more than once in namespace `{namespace}`",
                        declared.name
                    ));
                }
            }
        }

        for schema in &schemas {
            let namespace = schema.namespace.as_str();
            let mut typed: Vec<(String, &Type)> = Vec::new();
            for object in &schema.objects {
                typed.extend(object.typed_members());
                for method in &object.methods {
                    if let Some(returns) = &method.returns {
                        typed.push((
                            format!("result of method `{}` of object `{}`", method.name, object.name),
                            returns,
                        ));
                    }
                }
            }
            for callback in &schema.callbacks {
                for parameter in &callback.params {
                    typed.push((
                        format!("parameter `{}` of callback `{}`", parameter.name, callback.name),
                        &parameter.ty,
                    ));
                }
                if let Some(returns) = &callback.returns {
                    typed.push((format!("result of callback `{}`", callback.name), returns));
                }
            }
            for (member, ty) in typed {
                for reference in ty.referenced_objects() {
                    let (home, name) = split_class_ref(namespace, reference);
                    if !objects.get(home).is_some_and(|known| known.contains_key(name)) {
                        problems.push(format!(
                            "{member} refers to object `{reference}`, which namespace `{home}` does not declare"
                        ));
                    }
                }
                for reference in ty.referenced_enums() {
                    let (home, name) = split_class_ref(namespace, reference);
                    if !schema.local {
                        problems.push(format!("{member} is an enum: only a local namespace has enums"));
                    } else if !enums.get(home).is_some_and(|known| known.contains_key(name)) {
                        problems.push(format!(
                            "{member} refers to enum `{reference}`, which namespace `{home}` does not declare"
                        ));
                    }
                }
                for reference in ty.referenced_callbacks() {
                    if !schema.local {
                        problems.push(format!("{member} is a callback: only a local namespace has callbacks"));
                    } else if !callbacks
                        .get(namespace)
                        .is_some_and(|known| known.contains_key(reference))
                    {
                        problems.push(format!(
                            "{member} refers to callback `{reference}`, which namespace `{namespace}` does not declare"
                        ));
                    }
                }
            }
        }

        crate::variants::check(&schemas, &mut problems);

        if let Err(inheritance) = crate::inherit::resolve(&mut schemas) {
            problems.extend(inheritance);
        }

        if !problems.is_empty() {
            return Err(SchemaError::Invalid {
                source: "schema catalog".to_owned(),
                problems,
            });
        }
        Ok(Catalog { schemas })
    }

    /// Read every `.toml` file in `dir`, in file-name order.
    ///
    /// File-name order is part of the contract: generated output must not depend
    /// on directory iteration order, or it would change between two builds of the
    /// same schema.
    pub fn load_dir(dir: &Path) -> Result<Catalog, SchemaError> {
        Self::load_dir_except(dir, &[])
    }

    /// Read every `.toml` file in `dir` except those under the `skip` subdirectories,
    /// in file-name order. A skipped subdirectory is a schema the caller must not
    /// expose, such as the owner-socket-only tree.
    pub fn load_dir_except(dir: &Path, skip: &[&str]) -> Result<Catalog, SchemaError> {
        Self::load_dirs(&[dir], skip)
    }

    /// Read the schema files of several directories as one catalog, the files of each in name order and the directories in the
    /// order given. The catalogs a library serves (`crates/gnoblin-core-abi/schema`, `crates/gnoblin-std-abi/schema`) are loaded
    /// together so a class of one can name a class of the other; `skip` names subdirectories to leave out of every directory.
    pub fn load_dirs(dirs: &[&Path], skip: &[&str]) -> Result<Catalog, SchemaError> {
        Self::load_dirs_with(dirs, skip, &[])
    }

    /// [`Catalog::load_dirs`] with schema files that are not on disk: each is an origin (the path the file would have, which names its
    /// lock) and its text, read after the files of the directories. A library whose classes are written from a table of its own (the
    /// built-in layouts) gives the table's schema this way.
    pub fn load_dirs_with(dirs: &[&Path], skip: &[&str], generated: &[(&str, &str)]) -> Result<Catalog, SchemaError> {
        let mut paths = Vec::new();
        for dir in dirs {
            let mut found = Vec::new();
            Self::collect_toml(dir, &mut found)?;
            let skipped: Vec<PathBuf> = skip.iter().map(|name| dir.join(name)).collect();
            found.retain(|path| !skipped.iter().any(|root| path.starts_with(root)));
            found.sort();
            paths.extend(found);
        }

        let mut problems = Vec::new();
        let mut schemas = Vec::new();
        for path in &paths {
            let origin = path.display().to_string();
            let text = match std::fs::read_to_string(path) {
                Ok(text) => text,
                Err(source) => {
                    problems.push(
                        SchemaError::Read {
                            path: path.clone(),
                            source,
                        }
                        .to_string(),
                    );
                    continue;
                }
            };
            match crate::parse::parse_schema(&origin, &text) {
                Ok(schema) => schemas.push(schema),
                Err(SchemaError::Invalid {
                    problems: file_problems,
                    ..
                }) => problems.extend(file_problems),
                Err(error) => problems.push(error.to_string()),
            }
        }

        let source = dirs
            .iter()
            .map(|dir| dir.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let given = generated.len();
        for (origin, text) in generated {
            match crate::parse::parse_schema(origin, text) {
                Ok(schema) => schemas.push(schema),
                Err(SchemaError::Invalid {
                    problems: file_problems,
                    ..
                }) => problems.extend(file_problems),
                Err(error) => problems.push(error.to_string()),
            }
        }
        if paths.is_empty() && given == 0 {
            problems.push("no .toml schema files found".to_owned());
        }
        if !problems.is_empty() {
            return Err(SchemaError::Invalid { source, problems });
        }
        Catalog::new(schemas)
    }

    /// Push every `.toml` file under `dir` onto `paths`, descending into subdirectories.
    fn collect_toml(dir: &Path, paths: &mut Vec<std::path::PathBuf>) -> Result<(), SchemaError> {
        let entries = std::fs::read_dir(dir).map_err(|source| SchemaError::Read {
            path: dir.to_path_buf(),
            source,
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| SchemaError::Read {
                path: dir.to_path_buf(),
                source,
            })?;
            let path = entry.path();
            if path.is_dir() {
                Self::collect_toml(&path, paths)?;
            } else if path.extension().and_then(|extension| extension.to_str()) == Some("toml") {
                paths.push(path);
            }
        }
        Ok(())
    }

    /// The files in the catalog, in load order.
    pub fn schemas(&self) -> &[Schema] {
        &self.schemas
    }

    /// Every namespace, sorted.
    pub fn namespaces(&self) -> Vec<Namespace> {
        let mut namespaces: BTreeSet<Namespace> = BTreeSet::new();
        for schema in &self.schemas {
            namespaces.insert(schema.namespace.clone());
        }
        namespaces.into_iter().collect()
    }

    /// The version a namespace is declared at, which [`Catalog::new`] has
    /// already checked is the same in every file that declares it.
    pub fn version(&self, namespace: &Namespace) -> Option<Version> {
        self.schemas
            .iter()
            .find(|schema| &schema.namespace == namespace)
            .map(|schema| schema.version)
    }

    /// Every object type in a namespace, in declaration order.
    pub fn objects<'a>(&'a self, namespace: &'a Namespace) -> impl Iterator<Item = &'a ObjectType> {
        self.schemas
            .iter()
            .filter(move |schema| &schema.namespace == namespace)
            .flat_map(|schema| schema.objects.iter())
    }

    /// Every error code in a namespace, in declaration order.
    pub fn errors<'a>(&'a self, namespace: &'a Namespace) -> impl Iterator<Item = &'a ErrorDecl> {
        self.schemas
            .iter()
            .filter(move |schema| &schema.namespace == namespace)
            .flat_map(|schema| schema.errors.iter())
    }

    /// Every file that declares a namespace, for generated headers.
    pub fn origins<'a>(&'a self, namespace: &'a Namespace) -> impl Iterator<Item = &'a str> {
        self.schemas
            .iter()
            .filter(move |schema| &schema.namespace == namespace)
            .map(|schema| schema.origin.as_str())
    }

    /// Every enum in a namespace, in declaration order.
    pub fn enums<'a>(&'a self, namespace: &'a Namespace) -> impl Iterator<Item = &'a EnumDecl> {
        self.schemas
            .iter()
            .filter(move |schema| &schema.namespace == namespace)
            .flat_map(|schema| schema.enums.iter())
    }

    /// Every callback in a namespace, in declaration order.
    pub fn callbacks<'a>(&'a self, namespace: &'a Namespace) -> impl Iterator<Item = &'a CallbackDecl> {
        self.schemas
            .iter()
            .filter(move |schema| &schema.namespace == namespace)
            .flat_map(|schema| schema.callbacks.iter())
    }

    /// True when a library serves the namespace and the compositor does not.
    pub fn is_local(&self, namespace: &Namespace) -> bool {
        self.schemas
            .iter()
            .any(|schema| &schema.namespace == namespace && schema.local)
    }

    /// A 64-bit fingerprint of everything the generated SDKs read from the catalog: every namespace, class, base, property
    /// (type, default, read-only, animated, secret), method (parameters), signal (fields) and error code.
    ///
    /// The core library reports it as `gnoblin_catalog_hash`, and each generated SDK file records it in its header, so "does
    /// this SDK match the core I am running on" is one integer comparison. It changes when a member is added, removed,
    /// retyped or flagged differently, and not when a comment, the order of the schema files or a `since` version changes.
    /// The hash is FNV-1a over a canonical text, so it is stable across platforms and toolchains.
    pub fn fingerprint(&self) -> u64 {
        use std::fmt::Write as _;
        let mut text = String::new();
        // A namespace a library serves is not the compositor's: its changes never move this hash, which the golden shells and the
        // wire depend on (`namespace_fingerprint` is the hash of those).
        for namespace in self
            .namespaces()
            .into_iter()
            .filter(|namespace| !self.is_local(namespace))
        {
            let _ = writeln!(text, "namespace {}", namespace.as_str());
            for object in self.objects(&namespace) {
                let _ = writeln!(
                    text,
                    "class {} base={} secret={} record={}",
                    object.name,
                    object.base.as_deref().unwrap_or("-"),
                    object.secret,
                    object.record
                );
                for property in &object.properties {
                    let _ = writeln!(
                        text,
                        "p {} {} default={:?} ro={} anim={} secret={}",
                        property.name,
                        property.ty,
                        property.default,
                        property.readonly,
                        property.animated,
                        property.secret
                    );
                }
                for method in &object.methods {
                    let _ = write!(
                        text,
                        "m {} fn={} secret={}",
                        method.name, method.function, method.secret
                    );
                    for param in &method.params {
                        let _ = write!(text, " {}:{}:{}:{}", param.name, param.ty, param.optional, param.secret);
                    }
                    text.push('\n');
                }
                for signal in &object.signals {
                    let _ = write!(text, "s {} secret={}", signal.name, signal.secret);
                    for field in &signal.fields {
                        let _ = write!(text, " {}:{}:{}", field.name, field.ty, field.secret);
                    }
                    text.push('\n');
                }
            }
            for error in self.errors(&namespace) {
                let _ = writeln!(text, "e {}", error.name);
            }
        }
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for byte in text.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }

    /// A 64-bit fingerprint of one namespace's catalog, everything a generated SDK or a library reads from it: kinds, bases,
    /// constructors, every member with its type, flags and defaults, enums, callbacks and errors. A library reports it through its
    /// object table, so a generated SDK and its library can tell whether they were built from the same catalog. Comments, `since` versions and the order of files do not move it.
    pub fn namespace_fingerprint(&self, namespace: &Namespace) -> u64 {
        use std::fmt::Write as _;
        let mut text = format!("namespace {}\n", namespace.as_str());
        let mut classes: Vec<&ObjectType> = self.objects(namespace).collect();
        classes.sort_by(|a, b| a.name.cmp(&b.name));
        for object in classes {
            let _ = writeln!(
                text,
                "class {} kind={} base={} retained={} ctor={:?} export={:?}",
                object.name,
                object.kind.as_str(),
                object.base.as_deref().unwrap_or("-"),
                object.retained,
                object.ctor,
                object.export
            );
            for property in &object.properties {
                let _ = writeln!(
                    text,
                    "p {} {} default={:?} ro={} anim={} secret={} optional={} extras={:?}",
                    property.name,
                    property.ty,
                    property.default,
                    property.readonly,
                    property.animated,
                    property.secret,
                    property.optional,
                    property.extras
                );
            }
            if let Some(tag) = &object.tag {
                let _ = writeln!(text, "tag {tag}");
            }
            for variant in &object.variants {
                let _ = writeln!(
                    text,
                    "variant {} tag={} export={:?}",
                    variant.name, variant.tag, variant.export
                );
                for property in &variant.properties {
                    let _ = writeln!(
                        text,
                        "vp {} {} default={:?} optional={} field={:?} export={:?}",
                        property.name,
                        property.ty,
                        property.default,
                        property.optional,
                        property.field,
                        property.extras.export
                    );
                }
            }
            for method in &object.methods {
                let _ = write!(
                    text,
                    "m {} fn={} secret={} returns={} optional={} required={} native={} dunder={:?} extras={:?}",
                    method.name,
                    method.function,
                    method.secret,
                    method
                        .returns
                        .as_ref()
                        .map_or_else(|| "-".to_owned(), ToString::to_string),
                    method.optional,
                    method.required,
                    method.native_default,
                    method.dunder,
                    method.extras
                );
                for param in &method.params {
                    let _ = write!(text, " {}:{}:{}", param.name, param.ty, param.optional);
                }
                text.push('\n');
            }
            for signal in &object.signals {
                let _ = write!(text, "s {} extras={:?}", signal.name, signal.extras);
                for field in &signal.fields {
                    let _ = write!(text, " {}:{}", field.name, field.ty);
                }
                text.push('\n');
            }
        }
        let mut enums: Vec<&EnumDecl> = self.enums(namespace).collect();
        enums.sort_by(|a, b| a.name.cmp(&b.name));
        for declared in enums {
            let _ = writeln!(text, "enum {} {}", declared.name, declared.values.join(" "));
        }
        let mut callbacks: Vec<&CallbackDecl> = self.callbacks(namespace).collect();
        callbacks.sort_by(|a, b| a.name.cmp(&b.name));
        for declared in callbacks {
            let _ = write!(
                text,
                "callback {} returns={}",
                declared.name,
                declared
                    .returns
                    .as_ref()
                    .map_or_else(|| "-".to_owned(), ToString::to_string)
            );
            for param in &declared.params {
                let _ = write!(text, " {}:{}", param.name, param.ty);
            }
            text.push('\n');
        }
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for byte in text.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }
}

/// True for a class reference: `[a-z][a-z0-9_]*` segments, joined by dots for another namespace's class.
pub fn is_class_ref(text: &str) -> bool {
    !text.is_empty() && text.split('.').all(is_lower_name)
}

/// The namespace and name a reference means: a dotted one names another namespace's class, an undotted one is in `namespace`.
pub(crate) fn split_class_ref<'a>(namespace: &'a str, reference: &'a str) -> (&'a str, &'a str) {
    match reference.rsplit_once('.') {
        Some((home, name)) => (home, name),
        None => (namespace, reference),
    }
}

/// True for a member or object name: `[a-z][a-z0-9_]*`.
pub fn is_lower_name(text: &str) -> bool {
    let mut bytes = text.bytes();
    matches!(bytes.next(), Some(first) if first.is_ascii_lowercase())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// True for an error code: `[a-z][a-z0-9-]*`.
///
/// The rule is `gnoblin-types`': a declaration and a wire value must not be able to
/// disagree about what an error code looks like, and that crate's
/// [`gnoblin_types::ErrorCode`] is what the wire carries.
pub use gnoblin_types::is_error_code;

#[cfg(test)]
mod tests {
    use super::{Namespace, Type, is_error_code, is_lower_name};

    #[test]
    fn checks_lower_names() {
        for name in ["session", "app_id", "window2"] {
            assert!(is_lower_name(name), "{name}");
        }
        for name in ["Session", "app-id", "2window", "", "_private", "app id"] {
            assert!(!is_lower_name(name), "{name}");
        }
    }

    #[test]
    fn checks_error_codes() {
        assert!(is_error_code("provider-gone"));
        assert!(is_error_code("unknown-member"));
        assert!(!is_error_code("provider_gone"));
        assert!(!is_error_code("Provider-gone"));
    }

    #[test]
    fn the_owner_only_tree_is_skipped_and_the_public_schema_is_kept() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("schema");
        let all = super::Catalog::load_dir(&dir).expect("the schema loads");
        let public = super::Catalog::load_dir_except(&dir, &["tree"]).expect("the public schema loads");

        let tree = Namespace::parse("tree").expect("namespace");
        let gnoblin = Namespace::parse("gnoblin").expect("namespace");
        assert!(all.namespaces().contains(&tree), "the full catalog holds the tree");
        assert!(
            !public.namespaces().contains(&tree),
            "the public catalog omits the tree"
        );
        assert!(
            public.namespaces().contains(&gnoblin),
            "the public catalog keeps gnoblin"
        );
        assert!(public.objects(&gnoblin).next().is_some());
    }

    #[test]
    fn the_tree_node_defaults_and_flags_follow_section_two_four() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("schema");
        let catalog = super::Catalog::load_dir(&dir).expect("the schema loads");
        let tree = Namespace::parse("tree").expect("namespace");
        let node = catalog
            .objects(&tree)
            .find(|object| object.name == "node")
            .expect("the node class");
        let property = |name: &str| {
            node.properties
                .iter()
                .find(|property| property.name == name)
                .unwrap_or_else(|| panic!("node has no {name}"))
        };

        // Read-only: the identity and the parent, which only `insert` changes.
        assert!(property("id").readonly && property("parent").readonly);
        // Animatable: a change of the local transform or rectangle may carry `seconds`.
        for name in ["x", "y", "z", "rotation", "scale", "width"] {
            assert!(property(name).animated, "{name} is animated");
        }
        // Defaults: the origin is 0.0 and the scale is the identity.
        assert_eq!(property("x").default, Some(super::DefaultValue::Double(0.0)));
        assert_eq!(property("name").default, Some(super::DefaultValue::Text(String::new())));
        assert!(
            property("id").default.is_none(),
            "id has no default; the compositor assigns it"
        );
    }

    #[test]
    fn checks_namespaces() {
        assert_eq!(
            Namespace::parse("example.ext").expect("namespace").as_str(),
            "example.ext"
        );
        assert_eq!(Namespace::parse("gnoblin").expect("namespace").to_string(), "gnoblin");
        for text in ["", "Example.ext", "example.", "example..ext", "example-ext"] {
            assert!(Namespace::parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn parses_and_renders_types() {
        let cases = [
            ("bool", Type::Bool),
            ("int", Type::Int),
            ("uint", Type::Uint),
            ("double", Type::Double),
            ("string", Type::Text),
            ("bytes", Type::Bytes),
            ("object:window", Type::ObjectRef("window".to_owned())),
            ("list<string>", Type::List(Box::new(Type::Text))),
            (
                "list<object:window>",
                Type::List(Box::new(Type::ObjectRef("window".to_owned()))),
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(Type::parse(text), Ok(expected.clone()), "{text}");
            assert_eq!(expected.to_string(), text);
        }
        for text in ["", "str", "object:Window", "list<>", "list<string"] {
            assert!(Type::parse(text).is_err(), "{text}");
        }
        assert_eq!(
            Type::parse("list<object:window>").expect("type").referenced_objects(),
            vec!["window"]
        );
    }
}
