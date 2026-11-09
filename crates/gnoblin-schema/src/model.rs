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
    /// The identity of another object type in the same namespace.
    ObjectRef(String),
    /// A homogeneous sequence of the inner type.
    List(Box<Type>),
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
            other => match other.strip_prefix("object:") {
                Some(name) if is_lower_name(name) => Ok(Type::ObjectRef(name.to_owned())),
                Some(_) => Err(format!(
                    "`{other}` is not an object type; expected [a-z][a-z0-9_]* after `object:`"
                )),
                None => Err(format!("unknown type `{other}`")),
            },
        }
    }

    /// Every object type this type refers to, at any depth.
    pub fn referenced_objects(&self) -> Vec<&str> {
        let mut references = Vec::new();
        self.collect_references(&mut references);
        references
    }

    fn collect_references<'a>(&'a self, references: &mut Vec<&'a str>) {
        match self {
            Type::ObjectRef(name) => references.push(name),
            Type::List(inner) => inner.collect_references(references),
            _ => {}
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
        }
    }
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
    /// The object types declared, in file order.
    pub objects: Vec<ObjectType>,
    /// The error codes declared, in file order.
    pub errors: Vec<ErrorDecl>,
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

    /// The object as Lua may see it: `None` when the whole object is secret, otherwise
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
    /// The schemas as declared, before inheritance is resolved into them.
    declared: Vec<Schema>,
    schemas: Vec<Schema>,
}

impl Catalog {
    /// Build a catalog from parsed schemas.
    pub fn new(mut schemas: Vec<Schema>) -> Result<Catalog, SchemaError> {
        let mut problems = Vec::new();
        let mut versions: BTreeMap<&str, (Version, &str)> = BTreeMap::new();
        let mut objects: BTreeMap<&str, BTreeMap<&str, &str>> = BTreeMap::new();
        let mut errors: BTreeMap<&str, BTreeMap<&str, &str>> = BTreeMap::new();

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
        }

        for schema in &schemas {
            let namespace = schema.namespace.as_str();
            let Some(known) = objects.get(namespace) else {
                continue;
            };
            for object in &schema.objects {
                for (member, ty) in object.typed_members() {
                    for reference in ty.referenced_objects() {
                        if !known.contains_key(reference) {
                            problems.push(format!(
                                "{member} refers to object `{reference}`, which namespace `{namespace}` does not declare"
                            ));
                        }
                    }
                }
            }
        }

        // Resolution runs on a copy: `declared` keeps the schemas as written, so that
        // `with_derived` can add members and rebuild from them without inheriting twice.
        let declared = schemas.clone();
        if let Err(inheritance) = crate::inherit::resolve(&mut schemas) {
            problems.extend(inheritance);
        }

        if !problems.is_empty() {
            return Err(SchemaError::Invalid {
                source: "schema catalog".to_owned(),
                problems,
            });
        }
        Ok(Catalog { schemas, declared })
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
    /// expose, such as the owner-socket-only tree (design/node-core.md section 2.3).
    pub fn load_dir_except(dir: &Path, skip: &[&str]) -> Result<Catalog, SchemaError> {
        let mut paths = Vec::new();
        Self::collect_toml(dir, &mut paths)?;
        let skipped: Vec<PathBuf> = skip.iter().map(|name| dir.join(name)).collect();
        paths.retain(|path| !skipped.iter().any(|root| path.starts_with(root)));
        paths.sort();

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

        let source = dir.display().to_string();
        if paths.is_empty() {
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

    /// The files as declared, before inheritance: what a rebuild starts from, and
    /// what an extension catalog must be built from so inherited members are not
    /// declared twice.
    pub fn declared_schemas(&self) -> &[Schema] {
        &self.declared
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
