//! Reading the TOML schema format into the model, and checking every rule.
//!
//! ## Why TOML
//!
//! A schema file is a small tree of tables, and TOML writes that tree without a
//! parser of our own: `[[object]]`, then `[[object.property]]` beneath it, and so
//! on. Three reasons picked it over the alternatives:
//!
//! - comments. Every member exists for a reason, and the reason belongs beside
//!   it. JSON cannot carry one, so a JSON schema turns into an undocumented one.
//! - one spelling in the tree. Gnoblin already writes configuration as TOML, so
//!   contributors and documentation do not learn a second file syntax.
//! - a small, audited parser. A bespoke schema language would need its own
//!   parser, error messages and editor support for no gain.
//!
//! ## Rules
//!
//! A file declares one namespace at one version. Every object, member and field
//! records `since`, the version that added it. The parser fails, listing every
//! problem, when:
//!
//! - two objects, members or fields share a name (`since` cannot tell them
//!   apart);
//! - a `since` is missing, is not three numbers, or is newer than the schema
//!   version;
//! - a name does not match its pattern, or a type names nothing known;
//! - a field's `since` is earlier than the member that introduced it;
//! - a `default` does not match its property's type;
//! - a `base` is not a class name in the same namespace.
//!
//! Object references are resolved by [`crate::Catalog`], because a namespace may
//! span several files.
//!
//! ## Keys added by node-core
//!
//! - `base = "<class>"` on an object: the class it derives from, in the same
//!   namespace. The derived class gains the base's properties, methods and
//!   signals, base members first. A property may redeclare a base property only
//!   with the same type; no object may inherit from itself, directly or through
//!   a chain.
//! - `default = <value>` on a property: the value a read sees before anything has
//!   set it. It must match the property's type.
//! - `readonly = true` on a property: a `set` from a client is refused.
//! - `animated = true` on a property: a `set` may carry a duration in seconds.
//! - `optional = true` on a method parameter: a call may leave the parameter out, and the method's own comment says what
//!   a missing one means. A required parameter after an optional one is refused, so the generated signatures can write
//!   the optional ones as trailing parameters; a signal field is never optional.
//!
//! - `always = true` on a property of an object type: the library always has an object to answer (the roots of `gnoblin`:
//!   `gnoblin.input`, `gnoblin.clients`), so a generated SDK declares the read without an absent case (no `| undefined`, no
//!   `Option`). It is a declaration for the SDKs and changes nothing on the wire: the property is the same value, and no
//!   hash of the catalog covers it.
//!
//! `readonly`, `animated`, `optional` and `always` are booleans and default to `false`. Every key a
//! schema file has not declared is refused, so a misspelt key cannot pass silently.
//!
//! ## Keys of a local namespace
//!
//! A file that says `local = true` declares a namespace a library serves through the object protocol, and may use the keys below.
//! A file that does not is the compositor's schema, and using one is refused, so the wire never grows a type it cannot carry.
//!
//! - `kind = "handle" | "record" | "interface" | "service"` on an object (`record = true` stays as an alias of `"record"`).
//! - `ctor = ["name", ..]` on an object: the properties its constructor takes; every other writable property is set after.
//! - `retained = true` on an object: an instance stays until it is destroyed, whatever the shell releases.
//! - `export = { js = "..", py = "..", rust = ".." }` on an object or a member: a name that differs from the mechanical conversion.
//! - `[[enum]]` with `values = [..]`, and `[[callback]]` with `[[callback.param]]` and `returns`: the types `enum:<name>` and
//!   `callback:<name>`. `optional = true` on a callback: the function may answer nothing (a TypeScript `void`, a Python `None`), which
//!   the library reads as the answer the callback's comment gives.
//! - `returns = "<type>"` on a method, `required = true` and `default = "native"` on a method of an interface.
//! - `optional = true` on a method: its result (an `object:` type) may be none, as `undefined` in TypeScript, `None` in Python and
//!   `Option::None` in Rust. The library answers a nothing value; without the key a missing member is an error.
//! - `python = { dunder = "__len__" }` on a method: the Python special method the member is.
//! - `tag = "<property>"` on a record, with a `[[object.variant]]` for each value of that enum property: the record is the family of a
//!   tagged value (a filter that is a blur, a grade or a shader), and each variant is a value class of its own in every language.
//!   A variant has a `name`, the `tag` value it stands for and `[[object.variant.property]]` entries, each with a `field` that says
//!   where the family record carries it (`amount`, or `shader.source` for a field of the record a family field holds). The wire
//!   carries the family's record, so a variant adds no class to the object table. A script may also give a variant as an object with
//!   one key, its tag: `{ blur: 24 }` for a variant of one property, `{ shader: { source, params } }` for the others.
//! - `number = "<field>"` on a record that is not a family: a bare number stands for the record with only that field
//!   (`corner_radius` says `all`, so `12` is every corner).
//! - `optional = true` on a property: a record may leave the field out (the same key a method parameter has). A field with neither a
//!   default nor this is always there.
//! - `find = "Kind"` on a method of a service that returns a list of objects and takes one optional record, its filter: a shell reads the
//!   service's `find(kind, filter)` as a call of this method, and an unknown kind is refused with the kinds that work. Each method names
//!   a kind of its own.
//! - `inert = "reason"` on a member: accepted and not applied. `abi = "fn:<name>"`: served by an existing typed table function.
//! - `base = "<namespace>.<class>"`: a class of another namespace's catalog; the members are not copied into the derived class.
//! - `[[augment]]` with `target = "core.<class>"`, and `[[augment.property]]` and `[[augment.method]]` blocks: members a library above the
//!   core adds to a class of the core's catalog (`gnoblin.model`), which the core never reads. A property is `readonly`, and a method is a
//!   method of the object. The catalog refuses a target the core lacks and a member name the target has, and lowers the members of every
//!   augment of one target into the service class `<class>_augment` of the augmenting namespace ([`crate::Augment`]). `receiver = true`
//!   makes each member read on, or called on, one object of the target class, which the library is handed as the first parameter.

use std::collections::BTreeSet;

use serde::Deserialize;

use crate::error::SchemaError;
use crate::model::{
    Augment, CallbackDecl, DefaultValue, EnumDecl, ErrorDecl, Export, Extras, Field, Method, Namespace, ObjectKind,
    ObjectType, Property, Schema, Signal, TaggedVariant, Type, is_class_ref, is_error_code, is_lower_name,
};
use gnoblin_types::Version;

/// What every check of a file needs: the schema's version and whether a library serves the namespace.
#[derive(Clone, Copy)]
struct Scope {
    version: Option<Version>,
    local: bool,
}

/// Shape of one schema file, exactly as `toml` deserializes it.
///
/// Unknown keys are rejected: a typo like `propertys` would otherwise produce a
/// schema that silently lacks a member, and the generated code would silently
/// lack a field.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSchema {
    namespace: String,
    version: String,
    #[serde(default)]
    local: bool,
    // Schema files name these arrays in the singular, like the other tables.
    #[serde(default, rename = "object")]
    objects: Vec<RawObject>,
    #[serde(default, rename = "error")]
    errors: Vec<RawError>,
    #[serde(default, rename = "enum")]
    enums: Vec<RawEnum>,
    #[serde(default, rename = "callback")]
    callbacks: Vec<RawCallback>,
    #[serde(default, rename = "augment")]
    augments: Vec<RawAugment>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAugment {
    target: String,
    since: Option<String>,
    #[serde(default)]
    receiver: bool,
    #[serde(default)]
    assign: bool,
    #[serde(default, rename = "property")]
    properties: Vec<RawProperty>,
    #[serde(default, rename = "method")]
    methods: Vec<RawMethod>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawObject {
    name: String,
    since: Option<String>,
    #[serde(default)]
    secret: bool,
    #[serde(default)]
    record: bool,
    #[serde(default)]
    base: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    ctor: Option<Vec<String>>,
    #[serde(default)]
    retained: bool,
    #[serde(default)]
    assign: bool,
    #[serde(default)]
    export: Option<RawExport>,
    #[serde(default)]
    tag: Option<String>,
    #[serde(default)]
    number: Option<String>,
    #[serde(default, rename = "variant")]
    variants: Vec<RawVariant>,
    #[serde(default, rename = "property")]
    properties: Vec<RawProperty>,
    #[serde(default, rename = "method")]
    methods: Vec<RawMethod>,
    #[serde(default, rename = "signal")]
    signals: Vec<RawSignal>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawVariant {
    name: String,
    tag: String,
    since: Option<String>,
    #[serde(default)]
    export: Option<RawExport>,
    #[serde(default, rename = "property")]
    properties: Vec<RawProperty>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawExport {
    js: Option<String>,
    py: Option<String>,
    rust: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPython {
    dunder: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProperty {
    name: String,
    #[serde(rename = "type")]
    ty: Option<String>,
    since: Option<String>,
    #[serde(default)]
    secret: bool,
    #[serde(default)]
    readonly: bool,
    #[serde(default)]
    animated: bool,
    #[serde(default)]
    optional: bool,
    #[serde(default)]
    always: bool,
    default: Option<toml::Value>,
    #[serde(default)]
    field: Option<String>,
    #[serde(default)]
    inert: Option<String>,
    #[serde(default)]
    abi: Option<String>,
    #[serde(default)]
    export: Option<RawExport>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMethod {
    name: String,
    since: Option<String>,
    #[serde(default)]
    secret: bool,
    /// A namespace function: the host reaches it as `gnoblin.<object>.<name>()`, not as a
    /// method on each instance. The wire serves it as a method of the object all the same.
    #[serde(default)]
    function: bool,
    #[serde(default, rename = "param")]
    params: Vec<RawField>,
    #[serde(default)]
    returns: Option<String>,
    /// The result may be none: `undefined`, `None`, `Option::None`.
    #[serde(default)]
    optional: bool,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    default: Option<String>,
    #[serde(default)]
    python: Option<RawPython>,
    #[serde(default)]
    inert: Option<String>,
    #[serde(default)]
    abi: Option<String>,
    #[serde(default)]
    export: Option<RawExport>,
    #[serde(default)]
    find: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSignal {
    name: String,
    since: Option<String>,
    #[serde(default)]
    secret: bool,
    #[serde(default, rename = "field")]
    fields: Vec<RawField>,
    #[serde(default)]
    inert: Option<String>,
    #[serde(default)]
    abi: Option<String>,
    #[serde(default)]
    export: Option<RawExport>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawField {
    name: String,
    #[serde(rename = "type")]
    ty: Option<String>,
    since: Option<String>,
    #[serde(default)]
    secret: bool,
    #[serde(default)]
    optional: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawError {
    name: String,
    since: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEnum {
    name: String,
    since: Option<String>,
    #[serde(default)]
    values: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCallback {
    name: String,
    since: Option<String>,
    #[serde(default, rename = "param")]
    params: Vec<RawField>,
    #[serde(default)]
    returns: Option<String>,
    #[serde(default)]
    optional: bool,
}

/// Parse and validate one schema file.
///
/// `origin` is the file name or path used in messages and in generated output.
pub fn parse_schema(origin: &str, text: &str) -> Result<Schema, SchemaError> {
    let raw: RawSchema = toml::from_str(text).map_err(|error| SchemaError::Toml {
        source: origin.to_owned(),
        message: error.to_string(),
    })?;

    let mut problems = Vec::new();
    let version = match Version::parse(&raw.version) {
        Ok(version) => Some(version),
        Err(error) => {
            problems.push(format!("schema version: {error}"));
            None
        }
    };
    let namespace = match Namespace::parse(&raw.namespace) {
        Ok(namespace) => Some(namespace),
        Err(reason) => {
            problems.push(format!("namespace: {reason}"));
            None
        }
    };
    let scope = Scope {
        version,
        local: raw.local,
    };

    let errors = parse_errors(&raw.errors, version, &mut problems);
    let objects = parse_objects(&raw.objects, scope, &mut problems);
    let enums = parse_enums(&raw.enums, scope, &mut problems);
    let callbacks = parse_callbacks(&raw.callbacks, scope, &mut problems);
    let augments = parse_augments(&raw.augments, scope, &mut problems);

    if !problems.is_empty() {
        return Err(SchemaError::Invalid {
            source: origin.to_owned(),
            problems,
        });
    }
    match (namespace, version) {
        (Some(namespace), Some(version)) => Ok(Schema {
            origin: origin.to_owned(),
            namespace,
            version,
            local: raw.local,
            objects,
            errors,
            enums,
            callbacks,
            augments,
        }),
        // Unreachable in practice: a bad namespace or version already pushed a
        // problem, so the branch above returned. Reported rather than panicked so
        // a future edit cannot turn a schema mistake into a crash.
        _ => Err(SchemaError::Invalid {
            source: origin.to_owned(),
            problems: vec!["a schema needs a namespace and a version".to_owned()],
        }),
    }
}

/// Refuses a key that only a local namespace may use.
fn local_only(scope: Scope, problems: &mut Vec<String>, owner: &str, key: &str, present: bool) {
    if present && !scope.local {
        problems.push(format!(
            "{owner}: `{key}` is a key of a namespace a library serves: the file needs `local = true`"
        ));
    }
}

/// A kind a `find` names: `Window`, `Workspace`.
fn is_kind_name(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|first| first.is_ascii_uppercase()) && chars.all(|c| c.is_ascii_alphanumeric())
}

fn export_of(raw: Option<&RawExport>) -> Export {
    raw.map_or_else(Export::default, |raw| Export {
        js: raw.js.clone(),
        py: raw.py.clone(),
        rust: raw.rust.clone(),
    })
}

/// The keys every kind of member shares.
fn extras_of(
    scope: Scope,
    problems: &mut Vec<String>,
    owner: &str,
    inert: Option<&String>,
    abi: Option<&String>,
    export: Option<&RawExport>,
) -> Extras {
    local_only(scope, problems, owner, "inert", inert.is_some());
    local_only(scope, problems, owner, "abi", abi.is_some());
    local_only(scope, problems, owner, "export", export.is_some());
    if let Some(abi) = abi
        && !abi.strip_prefix("fn:").is_some_and(is_lower_name)
    {
        problems.push(format!("{owner}: `abi` must be `fn:<table function>`, got `{abi}`"));
    }
    Extras {
        inert: inert.cloned(),
        abi: abi.cloned(),
        export: export_of(export),
        find: None,
    }
}

fn parse_errors(raw_errors: &[RawError], version: Option<Version>, problems: &mut Vec<String>) -> Vec<ErrorDecl> {
    let mut errors = Vec::new();
    let mut seen = BTreeSet::new();
    for raw_error in raw_errors {
        let owner = format!("error code `{}`", raw_error.name);
        if !is_error_code(&raw_error.name) {
            problems.push(format!("{owner}: name must match [a-z][a-z0-9-]*"));
        }
        if !seen.insert(raw_error.name.as_str()) {
            problems.push(format!("duplicate {owner}"));
        }
        let Some(since) = since_of(problems, raw_error.since.as_deref(), &owner, version) else {
            continue;
        };
        errors.push(ErrorDecl {
            name: raw_error.name.clone(),
            since,
        });
    }
    errors
}

fn parse_enums(raw_enums: &[RawEnum], scope: Scope, problems: &mut Vec<String>) -> Vec<EnumDecl> {
    let mut enums = Vec::new();
    for raw_enum in raw_enums {
        let owner = format!("enum `{}`", raw_enum.name);
        local_only(scope, problems, &owner, "[[enum]]", true);
        if !is_lower_name(&raw_enum.name) {
            problems.push(format!("{owner}: name must match [a-z][a-z0-9_]*"));
        }
        let mut seen = BTreeSet::new();
        for value in &raw_enum.values {
            if !is_enum_value(value) {
                problems.push(format!("{owner}: value `{value}` must match [a-z][a-z0-9_-]*"));
            }
            if !seen.insert(value.as_str()) {
                problems.push(format!("{owner}: duplicate value `{value}`"));
            }
        }
        if raw_enum.values.is_empty() {
            problems.push(format!("{owner}: an enum needs at least one value"));
        }
        let Some(since) = since_of(problems, raw_enum.since.as_deref(), &owner, scope.version) else {
            continue;
        };
        enums.push(EnumDecl {
            name: raw_enum.name.clone(),
            since,
            values: raw_enum.values.clone(),
        });
    }
    enums
}

fn parse_callbacks(raw_callbacks: &[RawCallback], scope: Scope, problems: &mut Vec<String>) -> Vec<CallbackDecl> {
    let mut callbacks = Vec::new();
    for raw_callback in raw_callbacks {
        let owner = format!("callback `{}`", raw_callback.name);
        local_only(scope, problems, &owner, "[[callback]]", true);
        if !is_lower_name(&raw_callback.name) {
            problems.push(format!("{owner}: name must match [a-z][a-z0-9_]*"));
        }
        let Some(since) = since_of(problems, raw_callback.since.as_deref(), &owner, scope.version) else {
            continue;
        };
        let params = parse_fields(
            &raw_callback.params,
            "parameter",
            &owner,
            since,
            scope.version,
            problems,
        );
        let returns = raw_callback
            .returns
            .as_deref()
            .and_then(|text| type_of(problems, Some(text), &format!("{owner}: `returns`")));
        if raw_callback.optional && returns.is_none() {
            problems.push(format!(
                "{owner}: `optional` lets a function answer nothing, so it needs a `returns`"
            ));
        }
        callbacks.push(CallbackDecl {
            name: raw_callback.name.clone(),
            since,
            params,
            returns,
            optional: raw_callback.optional,
        });
    }
    callbacks
}

/// The augments of a local namespace: the members a library adds to a class of the core's catalog. The catalog checks the target against
/// the core's classes and the members against the target's; here a member is read the way an object's is, with the rules an augment adds:
/// a property is read-only (the library serves it and no language writes it) and a method is a method of the object, not a function.
fn parse_augments(raw_augments: &[RawAugment], scope: Scope, problems: &mut Vec<String>) -> Vec<Augment> {
    let mut augments = Vec::new();
    for raw_augment in raw_augments {
        let owner = format!("augment `{}`", raw_augment.target);
        local_only(scope, problems, &owner, "[[augment]]", true);
        let Some(class) = raw_augment
            .target
            .strip_prefix("core.")
            .filter(|class| is_lower_name(class))
        else {
            problems.push(format!(
                "{owner}: `target` must be `core.<class>`, a class of the core catalog, written [a-z][a-z0-9_]*"
            ));
            continue;
        };
        let Some(since) = since_of(problems, raw_augment.since.as_deref(), &owner, scope.version) else {
            continue;
        };
        let mut members = BTreeSet::new();
        let properties = parse_properties(
            &raw_augment.properties,
            &owner,
            since,
            scope,
            &mut members,
            false,
            problems,
        );
        // A receiver augment's writable property is written through its setter method `set_<name>(target, value)`; a property of an
        // augment that acts on no one object has nothing to write through.
        for property in properties
            .iter()
            .filter(|property| !property.readonly && !raw_augment.receiver)
        {
            problems.push(format!(
                "{owner}: property `{}` must be read-only (`readonly = true`): only a `receiver = true` augment has writable properties",
                property.name
            ));
        }
        let methods = parse_methods(
            &raw_augment.methods,
            &owner,
            since,
            ObjectKind::Service,
            scope,
            &mut members,
            problems,
        );
        for method in methods.iter().filter(|method| method.function) {
            problems.push(format!(
                "{owner}: method `{}` is a `function`: an augment adds methods of the object it extends",
                method.name
            ));
        }
        augments.push(Augment {
            target: raw_augment.target.clone(),
            class: class.to_owned(),
            since,
            receiver: raw_augment.receiver,
            assign: raw_augment.assign,
            properties,
            methods,
        });
    }
    augments
}

fn parse_objects(raw_objects: &[RawObject], scope: Scope, problems: &mut Vec<String>) -> Vec<ObjectType> {
    let mut objects = Vec::new();
    let mut seen_objects = BTreeSet::new();
    for raw_object in raw_objects {
        let owner = format!("object `{}`", raw_object.name);
        if !is_lower_name(&raw_object.name) {
            problems.push(format!("{owner}: name must match [a-z][a-z0-9_]*"));
        }
        if !seen_objects.insert(raw_object.name.as_str()) {
            problems.push(format!("duplicate {owner}"));
        }
        let Some(since) = since_of(problems, raw_object.since.as_deref(), &owner, scope.version) else {
            continue;
        };

        local_only(scope, problems, &owner, "kind", raw_object.kind.is_some());
        local_only(scope, problems, &owner, "ctor", raw_object.ctor.is_some());
        local_only(scope, problems, &owner, "retained", raw_object.retained);
        local_only(scope, problems, &owner, "export", raw_object.export.is_some());
        let kind = match raw_object.kind.as_deref() {
            None if raw_object.record => ObjectKind::Record,
            None | Some("handle") => ObjectKind::Handle,
            Some("record") => ObjectKind::Record,
            Some("interface") => ObjectKind::Interface,
            Some("service") => ObjectKind::Service,
            Some(other) => {
                problems.push(format!(
                    "{owner}: `kind` must be handle, record, interface or service, got `{other}`"
                ));
                ObjectKind::Handle
            }
        };
        if raw_object.record && kind != ObjectKind::Record {
            problems.push(format!(
                "{owner}: `record = true` and `kind = \"{}\"` disagree",
                kind.as_str()
            ));
        }

        // One namespace of member names inside an object: a dump, a generated
        // binding and a subscriber all address `object.member`, so a property and
        // a method may not share a name.
        let mut members = BTreeSet::new();
        let properties = parse_properties(
            &raw_object.properties,
            &owner,
            since,
            scope,
            &mut members,
            false,
            problems,
        );
        let methods = parse_methods(&raw_object.methods, &owner, since, kind, scope, &mut members, problems);
        let signals = parse_signals(&raw_object.signals, &owner, since, scope, &mut members, problems);

        if let Some(base) = raw_object.base.as_deref().filter(|base| !is_class_ref(base)) {
            problems.push(format!(
                "{owner}: base `{base}` must match [a-z][a-z0-9_]* (dotted for another namespace's class)"
            ));
        }
        if kind == ObjectKind::Record && (!raw_object.methods.is_empty() || !raw_object.signals.is_empty()) {
            problems.push(format!("{owner}: a record declares no methods or signals"));
        }
        if kind == ObjectKind::Interface && !raw_object.signals.is_empty() {
            problems.push(format!("{owner}: an interface declares no signals"));
        }
        local_only(scope, problems, &owner, "tag", raw_object.tag.is_some());
        local_only(
            scope,
            problems,
            &owner,
            "[[object.variant]]",
            !raw_object.variants.is_empty(),
        );
        if kind != ObjectKind::Record && (raw_object.tag.is_some() || !raw_object.variants.is_empty()) {
            problems.push(format!("{owner}: `tag` and variants belong to a record"));
        }
        if raw_object.tag.is_some() != !raw_object.variants.is_empty() {
            problems.push(format!(
                "{owner}: a family of variants needs both its `tag` and at least one `[[object.variant]]`"
            ));
        }
        let variants = parse_variants(&raw_object.variants, &owner, since, scope, problems);
        local_only(scope, problems, &owner, "number", raw_object.number.is_some());
        if let Some(field) = &raw_object.number {
            let numeric = properties
                .iter()
                .find(|property| &property.name == field)
                .is_some_and(|property| matches!(property.ty, Type::Int | Type::Uint | Type::Double));
            if kind != ObjectKind::Record || raw_object.tag.is_some() || !numeric {
                problems.push(format!(
                    "{owner}: `number = \"{field}\"` names a number field of a record that is not a family of variants"
                ));
            }
        }
        local_only(scope, problems, &owner, "assign", raw_object.assign);
        if raw_object.assign && kind != ObjectKind::Handle {
            problems.push(format!(
                "{owner}: `assign = true` gives a handle `set(props, options)`, and this object is a {}",
                kind.as_str()
            ));
        }
        objects.push(ObjectType {
            name: raw_object.name.clone(),
            base: raw_object.base.clone(),
            since,
            secret: raw_object.secret,
            record: kind == ObjectKind::Record,
            kind,
            ctor: raw_object.ctor.clone(),
            retained: raw_object.retained,
            assign: raw_object.assign,
            augment_of: None,
            augment_getters: None,
            augment_setters: Vec::new(),
            export: export_of(raw_object.export.as_ref()),
            tag: raw_object.tag.clone(),
            number: raw_object.number.clone(),
            variants,
            properties,
            methods,
            signals,
        });
    }
    objects
}

/// The variants of a family record. The catalog checks them against the family's `tag` and fields, which a file may not hold whole.
fn parse_variants(
    raw_variants: &[RawVariant],
    object: &str,
    object_since: Version,
    scope: Scope,
    problems: &mut Vec<String>,
) -> Vec<TaggedVariant> {
    let mut variants = Vec::new();
    let mut names = BTreeSet::new();
    for raw_variant in raw_variants {
        let owner = format!("variant `{}` of {object}", raw_variant.name);
        if !is_lower_name(&raw_variant.name) {
            problems.push(format!("{owner}: name must match [a-z][a-z0-9_]*"));
        }
        if !names.insert(raw_variant.name.as_str()) {
            problems.push(format!("{object}: duplicate variant `{}`", raw_variant.name));
        }
        local_only(scope, problems, &owner, "export", raw_variant.export.is_some());
        let Some(since) = member_since(
            problems,
            raw_variant.since.as_deref(),
            &owner,
            object_since,
            scope.version,
        ) else {
            continue;
        };
        let mut members = BTreeSet::new();
        let properties = parse_properties(
            &raw_variant.properties,
            &owner,
            since,
            scope,
            &mut members,
            true,
            problems,
        );
        variants.push(TaggedVariant {
            name: raw_variant.name.clone(),
            tag: raw_variant.tag.clone(),
            since,
            export: export_of(raw_variant.export.as_ref()),
            properties,
        });
    }
    variants
}

/// Read a property's `default` as its declared type. TOML has no literal for an
/// object reference or for bytes, so those types take no default.
fn default_value(ty: &Type, value: &toml::Value) -> Result<DefaultValue, String> {
    match (ty, value) {
        (Type::Bool, toml::Value::Boolean(flag)) => Ok(DefaultValue::Bool(*flag)),
        (Type::Int, toml::Value::Integer(number)) => Ok(DefaultValue::Int(*number)),
        (Type::Uint, toml::Value::Integer(number)) => u64::try_from(*number)
            .map(DefaultValue::Uint)
            .map_err(|_| format!("default `{number}` is negative, so it is not a `uint`")),
        (Type::Double, toml::Value::Float(number)) => Ok(DefaultValue::Double(*number)),
        (Type::Double, toml::Value::Integer(number)) => Ok(DefaultValue::Double(*number as f64)),
        (Type::Text, toml::Value::String(text)) => Ok(DefaultValue::Text(text.clone())),
        // An enum's default is the name of one of its values; the catalog checks the name.
        (Type::Enum(_), toml::Value::String(name)) => Ok(DefaultValue::Text(name.clone())),
        (Type::List(inner), toml::Value::Array(items)) => items
            .iter()
            .map(|item| default_value(inner, item))
            .collect::<Result<Vec<_>, _>>()
            .map(DefaultValue::List),
        (Type::Bytes | Type::Any | Type::ObjectRef(_) | Type::Callback(_), _) => {
            Err(format!("default is refused: `{ty}` has no TOML literal"))
        }
        _ => Err(format!("default `{value}` is not a `{ty}`")),
    }
}

fn parse_properties(
    raw_properties: &[RawProperty],
    object: &str,
    object_since: Version,
    scope: Scope,
    members: &mut BTreeSet<String>,
    in_variant: bool,
    problems: &mut Vec<String>,
) -> Vec<Property> {
    let mut properties = Vec::new();
    for raw_property in raw_properties {
        let owner = format!("property `{}` of {object}", raw_property.name);
        match (in_variant, &raw_property.field) {
            (true, None) => problems.push(format!(
                "{owner}: a property of a variant needs `field`, the family field that carries it"
            )),
            (false, Some(_)) => problems.push(format!("{owner}: `field` belongs to a property of a variant")),
            _ => {}
        }
        if !members.insert(raw_property.name.clone()) {
            problems.push(format!("{object}: duplicate member name `{}`", raw_property.name));
        }
        if !is_lower_name(&raw_property.name) {
            problems.push(format!("{owner}: name must match [a-z][a-z0-9_]*"));
        }
        let extras = extras_of(
            scope,
            problems,
            &owner,
            raw_property.inert.as_ref(),
            raw_property.abi.as_ref(),
            raw_property.export.as_ref(),
        );
        local_only(scope, problems, &owner, "optional", raw_property.optional);
        if raw_property.optional && raw_property.default.is_some() {
            problems.push(format!(
                "{owner}: a property with a default is always there, so it is not `optional`"
            ));
        }
        let Some(ty) = type_of(problems, raw_property.ty.as_deref(), &owner) else {
            continue;
        };
        if raw_property.always && !matches!(ty, Type::ObjectRef(_)) {
            problems.push(format!(
                "{owner}: `always` is for a property of an object type, which a read may otherwise answer with none"
            ));
        }
        if raw_property.always && (raw_property.optional || in_variant) {
            problems.push(format!(
                "{owner}: `always` says the read has an object, so it is not `optional` and not a field of a variant"
            ));
        }
        let Some(since) = member_since(
            problems,
            raw_property.since.as_deref(),
            &owner,
            object_since,
            scope.version,
        ) else {
            continue;
        };
        let default = match raw_property
            .default
            .as_ref()
            .map(|value| default_value(&ty, value))
            .transpose()
        {
            Ok(default) => default,
            Err(reason) => {
                problems.push(format!("{owner}: {reason}"));
                continue;
            }
        };
        properties.push(Property {
            name: raw_property.name.clone(),
            ty,
            since,
            secret: raw_property.secret,
            default,
            readonly: raw_property.readonly,
            animated: raw_property.animated,
            optional: raw_property.optional,
            always: raw_property.always,
            field: raw_property.field.clone(),
            extras,
        });
    }
    properties
}

fn parse_methods(
    raw_methods: &[RawMethod],
    object: &str,
    object_since: Version,
    kind: ObjectKind,
    scope: Scope,
    members: &mut BTreeSet<String>,
    problems: &mut Vec<String>,
) -> Vec<Method> {
    let mut methods: Vec<Method> = Vec::new();
    for raw_method in raw_methods {
        let owner = format!("method `{}` of {object}", raw_method.name);
        if !members.insert(raw_method.name.clone()) {
            problems.push(format!("{object}: duplicate member name `{}`", raw_method.name));
        }
        if !is_lower_name(&raw_method.name) {
            problems.push(format!("{owner}: name must match [a-z][a-z0-9_]*"));
        }
        let mut extras = extras_of(
            scope,
            problems,
            &owner,
            raw_method.inert.as_ref(),
            raw_method.abi.as_ref(),
            raw_method.export.as_ref(),
        );
        local_only(scope, problems, &owner, "find", raw_method.find.is_some());
        extras.find = raw_method.find.clone();
        local_only(scope, problems, &owner, "returns", raw_method.returns.is_some());
        local_only(scope, problems, &owner, "optional", raw_method.optional);
        local_only(scope, problems, &owner, "required", raw_method.required);
        local_only(scope, problems, &owner, "default", raw_method.default.is_some());
        local_only(scope, problems, &owner, "python", raw_method.python.is_some());
        if (raw_method.required || raw_method.default.is_some()) && kind != ObjectKind::Interface {
            problems.push(format!(
                "{owner}: `required` and `default` belong to a method of an interface"
            ));
        }
        if raw_method.required && raw_method.default.is_some() {
            problems.push(format!("{owner}: a required method has no default to run"));
        }
        if let Some(default) = raw_method.default.as_deref()
            && default != "native"
        {
            problems.push(format!("{owner}: `default` can only be \"native\", got `{default}`"));
        }
        let Some(since) = member_since(
            problems,
            raw_method.since.as_deref(),
            &owner,
            object_since,
            scope.version,
        ) else {
            continue;
        };
        let params = parse_fields(&raw_method.params, "parameter", &owner, since, scope.version, problems);
        let returns = raw_method
            .returns
            .as_deref()
            .and_then(|text| type_of(problems, Some(text), &format!("{owner}: `returns`")));
        if kind == ObjectKind::Interface {
            // The shell answers an interface's method: `optional` lets it answer nothing, which the library reads as the method's
            // comment says (a binding's `on_press` that answers nothing handled the press).
            if raw_method.optional && returns.is_none() {
                problems.push(format!(
                    "{owner}: `optional` lets the shell's method answer nothing, so it needs a `returns`"
                ));
            }
        } else if raw_method.optional && !matches!(returns, Some(Type::ObjectRef(_))) {
            problems.push(format!("{owner}: `optional` needs `returns` to be an `object:` type"));
        }
        if let Some(found) = &raw_method.find {
            if !is_kind_name(found) {
                problems.push(format!(
                    "{owner}: `find` names a kind that starts with a capital letter and has letters and digits only, got `{found}`"
                ));
            }
            if methods
                .iter()
                .any(|earlier| earlier.extras.find.as_deref() == Some(found.as_str()))
            {
                problems.push(format!(
                    "{owner}: another method of {object} already finds the kind `{found}`"
                ));
            }
            let lists_objects = matches!(&returns, Some(Type::List(inner)) if matches!(**inner, Type::ObjectRef(_)));
            let one_query = matches!(
                params.as_slice(),
                [query] if query.optional && matches!(query.ty, Type::ObjectRef(_))
            );
            if kind != ObjectKind::Service || !lists_objects || !one_query || raw_method.function {
                problems.push(format!(
                    "{owner}: `find` belongs to a method of a service that returns a list of objects and takes one optional record as its filter"
                ));
            }
        }
        methods.push(Method {
            name: raw_method.name.clone(),
            since,
            secret: raw_method.secret,
            function: raw_method.function,
            params,
            returns,
            optional: raw_method.optional,
            required: raw_method.required,
            native_default: raw_method.default.is_some(),
            dunder: raw_method.python.as_ref().and_then(|python| python.dunder.clone()),
            extras,
        });
    }
    methods
}

fn parse_signals(
    raw_signals: &[RawSignal],
    object: &str,
    object_since: Version,
    scope: Scope,
    members: &mut BTreeSet<String>,
    problems: &mut Vec<String>,
) -> Vec<Signal> {
    let mut signals = Vec::new();
    for raw_signal in raw_signals {
        let owner = format!("signal `{}` of {object}", raw_signal.name);
        if !members.insert(raw_signal.name.clone()) {
            problems.push(format!("{object}: duplicate member name `{}`", raw_signal.name));
        }
        if !is_lower_name(&raw_signal.name) {
            problems.push(format!("{owner}: name must match [a-z][a-z0-9_]*"));
        }
        let extras = extras_of(
            scope,
            problems,
            &owner,
            raw_signal.inert.as_ref(),
            raw_signal.abi.as_ref(),
            raw_signal.export.as_ref(),
        );
        let Some(since) = member_since(
            problems,
            raw_signal.since.as_deref(),
            &owner,
            object_since,
            scope.version,
        ) else {
            continue;
        };
        let fields = parse_fields(&raw_signal.fields, "field", &owner, since, scope.version, problems);
        signals.push(Signal {
            name: raw_signal.name.clone(),
            since,
            secret: raw_signal.secret,
            fields,
            extras,
        });
    }
    signals
}

fn parse_fields(
    raw_fields: &[RawField],
    role: &str,
    member: &str,
    member_version: Version,
    version: Option<Version>,
    problems: &mut Vec<String>,
) -> Vec<Field> {
    let mut fields = Vec::new();
    let mut seen = BTreeSet::new();
    for raw_field in raw_fields {
        let owner = format!("{role} `{}` of {member}", raw_field.name);
        if !seen.insert(raw_field.name.as_str()) {
            problems.push(format!("{member}: duplicate {role} `{}`", raw_field.name));
        }
        if !is_lower_name(&raw_field.name) {
            problems.push(format!("{owner}: name must match [a-z][a-z0-9_]*"));
        }
        let Some(ty) = type_of(problems, raw_field.ty.as_deref(), &owner) else {
            continue;
        };
        // A field without `since` was added with the member that carries it.
        let since = match raw_field.since.as_deref() {
            None => member_version,
            Some(text) => {
                let Some(since) = member_since(problems, Some(text), &owner, member_version, version) else {
                    continue;
                };
                since
            }
        };
        if raw_field.optional && role != "parameter" {
            problems.push(format!("{owner}: only a method parameter can be `optional`"));
        }
        if !raw_field.optional && fields.last().is_some_and(|before: &Field| before.optional) {
            problems.push(format!(
                "{owner}: a required {role} cannot follow an optional one, so the optional ones come last"
            ));
        }
        fields.push(Field {
            name: raw_field.name.clone(),
            ty,
            since,
            secret: raw_field.secret,
            optional: raw_field.optional,
        });
    }
    fields
}

fn type_of(problems: &mut Vec<String>, raw: Option<&str>, owner: &str) -> Option<Type> {
    let Some(raw) = raw else {
        problems.push(format!("{owner}: missing `type`"));
        return None;
    };
    match Type::parse(raw) {
        Ok(ty) => Some(ty),
        Err(reason) => {
            problems.push(format!("{owner}: {reason}"));
            None
        }
    }
}

/// Read a `since`, check it is a full version, and check it is not from the
/// future.
fn since_of(problems: &mut Vec<String>, raw: Option<&str>, owner: &str, version: Option<Version>) -> Option<Version> {
    let Some(raw) = raw else {
        problems.push(format!("{owner}: missing `since`"));
        return None;
    };
    let since = match Version::parse(raw) {
        Ok(since) => since,
        Err(error) => {
            problems.push(format!("{owner}: {error}"));
            return None;
        }
    };
    if let Some(version) = version
        && since > version
    {
        problems.push(format!(
            "{owner}: `since` {since} is newer than the schema version {version}"
        ));
        return None;
    }
    Some(since)
}

/// A member's `since`, which may not predate the object that contains it.
fn member_since(
    problems: &mut Vec<String>,
    raw: Option<&str>,
    owner: &str,
    object_since: Version,
    version: Option<Version>,
) -> Option<Version> {
    let since = since_of(problems, raw, owner, version)?;
    if since < object_since {
        problems.push(format!(
            "{owner}: `since` {since} is earlier than its object's `since` {object_since}"
        ));
        return None;
    }
    Some(since)
}

/// True for the name of an enum value: a member name that may also contain hyphens, as a value spelt `follow-mouse` does.
fn is_enum_value(text: &str) -> bool {
    let mut bytes = text.bytes();
    matches!(bytes.next(), Some(first) if first.is_ascii_lowercase())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-')
}

#[cfg(test)]
mod tests {
    use crate::Catalog;
    use crate::model::Type;
    use crate::parse::parse_schema;

    /// A minimal valid file, with one placeholder that a test can replace.
    fn schema_text(members: &str) -> String {
        format!(
            "namespace = \"test\"\nversion = \"0.2.0\"\n\n[[object]]\nname = \"thing\"\nsince = \"0.2.0\"\n{members}"
        )
    }

    fn problems(members: &str) -> Vec<String> {
        match parse_schema("test.toml", &schema_text(members)) {
            Ok(schema) => panic!("expected a rejection, parsed {schema:?}"),
            Err(crate::SchemaError::Invalid { problems, .. }) => problems,
            Err(other) => panic!("expected validation problems, got {other}"),
        }
    }

    #[test]
    fn parses_a_minimal_schema() {
        let schema = parse_schema(
            "test.toml",
            &schema_text(
                "\n[[object.property]]\nname = \"version\"\ntype = \"string\"\nsince = \"0.2.0\"\n\
                 \n[[object.method]]\nname = \"refresh\"\nsince = \"0.2.0\"\n\
                 \n[[object.signal]]\nname = \"ready\"\nsince = \"0.2.0\"\n",
            ),
        )
        .expect("schema");
        assert_eq!(schema.namespace.as_str(), "test");
        assert_eq!(schema.version.to_string(), "0.2.0");
        assert_eq!(schema.objects.len(), 1);
        let object = &schema.objects[0];
        assert_eq!(object.name, "thing");
        assert_eq!(object.properties[0].ty, Type::Text);
        assert_eq!(object.methods[0].name, "refresh");
        assert!(!object.methods[0].is_sensitive());
        assert_eq!(object.signals[0].name, "ready");
    }

    #[test]
    fn a_secret_field_makes_the_member_sensitive() {
        let schema = parse_schema(
            "test.toml",
            &schema_text(
                "\n[[object.method]]\nname = \"auth_response\"\nsince = \"0.2.0\"\n\
                 \n[[object.method.param]]\nname = \"response\"\ntype = \"string\"\nsecret = true\n",
            ),
        )
        .expect("schema");
        let method = &schema.objects[0].methods[0];
        assert!(method.is_sensitive());
        assert!(schema.objects[0].is_sensitive());
        // The parameter inherits the member's `since` when it omits one.
        assert_eq!(method.params[0].since.to_string(), "0.2.0");
    }

    #[test]
    fn an_optional_parameter_is_read_and_must_come_last() {
        let schema = parse_schema(
            "test.toml",
            &schema_text(
                "\n[[object.method]]\nname = \"apply\"\nsince = \"0.2.0\"\n\
                 \n[[object.method.param]]\nname = \"a\"\ntype = \"int\"\n\
                 \n[[object.method.param]]\nname = \"b\"\ntype = \"int\"\noptional = true\n",
            ),
        )
        .expect("schema");
        let params = &schema.objects[0].methods[0].params;
        assert!(!params[0].optional && params[1].optional);

        let found = problems(
            "\n[[object.method]]\nname = \"apply\"\nsince = \"0.2.0\"\n\
             \n[[object.method.param]]\nname = \"a\"\ntype = \"int\"\noptional = true\n\
             \n[[object.method.param]]\nname = \"b\"\ntype = \"int\"\n",
        );
        assert!(
            found
                .iter()
                .any(|problem| problem.contains("cannot follow an optional one")),
            "{found:?}"
        );
    }

    #[test]
    fn a_signal_field_cannot_be_optional() {
        let found = problems(
            "\n[[object.signal]]\nname = \"ready\"\nsince = \"0.2.0\"\n\
             \n[[object.signal.field]]\nname = \"a\"\ntype = \"int\"\noptional = true\n",
        );
        assert!(
            found
                .iter()
                .any(|problem| problem.contains("only a method parameter can be `optional`")),
            "{found:?}"
        );
    }

    #[test]
    fn a_field_may_not_predate_its_member() {
        let found = problems("\n[[object.property]]\nname = \"title\"\ntype = \"string\"\nsince = \"0.1.0\"\n");
        assert!(
            found.iter().any(|problem| problem.contains("earlier than its object")),
            "{found:?}"
        );
    }

    #[test]
    fn rejects_duplicate_member_names() {
        let found = problems(
            "\n[[object.property]]\nname = \"title\"\ntype = \"string\"\nsince = \"0.2.0\"\n\
             \n[[object.method]]\nname = \"title\"\nsince = \"0.2.0\"\n",
        );
        assert!(
            found
                .iter()
                .any(|problem| problem.contains("duplicate member name `title`")),
            "{found:?}"
        );
    }

    #[test]
    fn rejects_a_missing_since() {
        let found = problems("\n[[object.property]]\nname = \"title\"\ntype = \"string\"\n");
        assert!(
            found
                .iter()
                .any(|problem| problem.contains("property `title` of object `thing`: missing `since`")),
            "{found:?}"
        );
    }

    #[test]
    fn rejects_a_version_that_is_not_three_numbers() {
        let found = problems("\n[[object.property]]\nname = \"title\"\ntype = \"string\"\nsince = \"0.2\"\n");
        assert!(
            found
                .iter()
                .any(|problem| problem.contains("`0.2` is not a MAJOR.MINOR.PATCH")),
            "{found:?}"
        );
    }

    #[test]
    fn rejects_a_since_from_the_future() {
        let found = problems("\n[[object.property]]\nname = \"title\"\ntype = \"string\"\nsince = \"0.3.0\"\n");
        assert!(
            found
                .iter()
                .any(|problem| problem.contains("newer than the schema version 0.2.0")),
            "{found:?}"
        );
    }

    #[test]
    fn rejects_an_unknown_type_and_key() {
        let found = problems("\n[[object.property]]\nname = \"title\"\ntype = \"text\"\nsince = \"0.2.0\"\n");
        assert!(
            found.iter().any(|problem| problem.contains("unknown type `text`")),
            "{found:?}"
        );

        let error = parse_schema(
            "test.toml",
            "namespace = \"test\"\nversion = \"0.2.0\"\n\n[[object]]\nname = \"thing\"\nsince = \"0.2.0\"\npropertys = []\n",
        )
        .expect_err("unknown key");
        assert!(error.to_string().contains("unknown field"), "{error}");
    }

    #[test]
    fn rejects_a_reference_to_a_missing_object() {
        let schema = parse_schema(
            "test.toml",
            &schema_text("\n[[object.property]]\nname = \"parent\"\ntype = \"object:group\"\nsince = \"0.2.0\"\n"),
        )
        .expect("schema parses; references are a catalog check");
        let error = Catalog::new(vec![schema]).expect_err("unresolved reference");
        assert!(error.to_string().contains("object `group`"), "{error}");
    }

    #[test]
    fn resolves_a_reference_across_files_of_one_namespace() {
        let window = parse_schema(
            "window.toml",
            "namespace = \"test\"\nversion = \"0.2.0\"\n\n[[object]]\nname = \"window\"\nsince = \"0.2.0\"\n",
        )
        .expect("window");
        let group = parse_schema(
            "group.toml",
            "namespace = \"test\"\nversion = \"0.2.0\"\n\n[[object]]\nname = \"group\"\nsince = \"0.2.0\"\n\
             \n[[object.property]]\nname = \"active\"\ntype = \"object:window\"\nsince = \"0.2.0\"\n",
        )
        .expect("group");
        let catalog = Catalog::new(vec![window, group]).expect("catalog");
        let namespaces = catalog.namespaces();
        assert_eq!(namespaces[0].as_str(), "test");
        assert_eq!(catalog.objects(&namespaces[0]).count(), 2);
    }

    #[test]
    fn rejects_two_versions_for_one_namespace() {
        let older = parse_schema(
            "older.toml",
            "namespace = \"test\"\nversion = \"0.2.0\"\n\n[[object]]\nname = \"window\"\nsince = \"0.2.0\"\n",
        )
        .expect("older");
        let newer = parse_schema(
            "newer.toml",
            "namespace = \"test\"\nversion = \"0.3.0\"\n\n[[object]]\nname = \"group\"\nsince = \"0.3.0\"\n",
        )
        .expect("newer");
        let error = Catalog::new(vec![older, newer]).expect_err("version disagreement");
        assert!(error.to_string().contains("is version 0.2.0"), "{error}");
    }

    #[test]
    fn the_checked_in_schemas_parse_and_validate() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("schema");
        let catalog = Catalog::load_dir(&dir).expect("checked-in schema files");
        let namespaces: Vec<String> = catalog.namespaces().iter().map(ToString::to_string).collect();
        assert_eq!(namespaces, vec!["gnoblin".to_owned(), "tree".to_owned()]);
        let gnoblin = catalog.namespaces()[0].clone();
        assert_eq!(catalog.version(&gnoblin).expect("version").to_string(), "0.2.0");
        let objects: Vec<&str> = catalog.objects(&gnoblin).map(|object| object.name.as_str()).collect();
        assert_eq!(
            objects,
            vec![
                "config",
                "eis",
                "idle",
                "operation",
                "output",
                "portal_ui",
                "portal_outcome",
                "request",
                "session",
                "settings",
                "window",
                "workspace",
            ]
        );
    }

    #[test]
    fn the_example_extension_fixture_parses_outside_the_stock_catalog() {
        let text = include_str!("../tests/fixtures/extensions/example-ext.toml");
        let schema = super::parse_schema("example-ext.toml", text).expect("fixture parses");
        assert_eq!(schema.namespace.as_str(), "example.ext");
        assert_eq!(schema.objects.len(), 2);
        let catalog = Catalog::new(vec![schema]).expect("fixture validates alone");
        assert_eq!(catalog.namespaces().len(), 1);
    }

    /// A record of a local namespace, with the properties `members` declares.
    fn local_record(members: &str) -> String {
        format!(
            "namespace = \"test\"\nversion = \"0.2.0\"\nlocal = true\n\n[[object]]\nname = \"span\"\nsince = \"0.2.0\"\nkind = \"record\"\n{members}"
        )
    }

    #[test]
    fn an_optional_property_is_a_key_of_a_local_namespace_and_never_has_a_default() {
        let property = "\n[[object.property]]\nname = \"closed\"\ntype = \"bool\"\nsince = \"0.2.0\"\n";
        let schema = parse_schema("test.toml", &local_record(&format!("{property}optional = true\n"))).expect("schema");
        assert!(schema.objects[0].properties[0].optional);
        let schema = parse_schema("test.toml", &local_record(property)).expect("schema");
        assert!(
            !schema.objects[0].properties[0].optional,
            "a property is always there unless it says otherwise"
        );

        let Err(crate::SchemaError::Invalid { problems: refused, .. }) = parse_schema(
            "test.toml",
            &local_record(&format!("{property}optional = true\ndefault = false\n")),
        ) else {
            panic!("a default and `optional` together are refused");
        };
        assert!(
            refused.iter().any(|problem| problem.contains("is always there")),
            "{refused:?}"
        );

        let found = problems(&format!("{property}optional = true\n"));
        assert!(
            found
                .iter()
                .any(|problem| problem.contains("`optional`") && problem.contains("local = true")),
            "the compositor's own files do not use it: {found:?}"
        );
    }

    /// A handle of a local namespace with one method `find` that declares `extra` after its `returns`.
    fn local_find(returns: &str, extra: &str, kind: &str) -> String {
        format!(
            "namespace = \"test\"\nversion = \"0.2.0\"\nlocal = true\n\n[[object]]\nname = \"thing\"\nsince = \"0.2.0\"\nkind = \"{kind}\"\n\
             \n[[object.method]]\nname = \"find\"\nsince = \"0.2.0\"\nreturns = \"{returns}\"\n{extra}"
        )
    }

    fn refused(text: &str) -> Vec<String> {
        match parse_schema("test.toml", text) {
            Ok(schema) => panic!("expected a rejection, parsed {schema:?}"),
            Err(crate::SchemaError::Invalid { problems, .. }) => problems,
            Err(other) => panic!("expected validation problems, got {other}"),
        }
    }

    #[test]
    fn an_optional_result_is_read_only_for_a_method_that_answers_an_object() {
        let schema =
            parse_schema("test.toml", &local_find("object:thing", "optional = true\n", "handle")).expect("schema");
        assert!(schema.objects[0].methods[0].optional);
        let schema = parse_schema("test.toml", &local_find("object:thing", "", "handle")).expect("schema");
        assert!(
            !schema.objects[0].methods[0].optional,
            "a result is always there unless it says otherwise"
        );

        let found = refused(&local_find("int", "optional = true\n", "handle"));
        assert!(
            found
                .iter()
                .any(|problem| problem.contains("`optional` needs `returns`")),
            "{found:?}"
        );
        // On an interface the shell answers, and `optional` lets its method answer nothing, of any type the method returns.
        let schema = parse_schema("test.toml", &local_find("bool", "optional = true\n", "interface")).expect("schema");
        assert!(schema.objects[0].methods[0].optional);
        let found = refused(&format!(
            "{}\n[[object.method]]\nname = \"silent\"\nsince = \"0.2.0\"\noptional = true\n",
            local_find("bool", "", "interface")
        ));
        assert!(
            found
                .iter()
                .any(|problem| problem.contains("answer nothing, so it needs a `returns`")),
            "{found:?}"
        );
        let found = problems("\n[[object.method]]\nname = \"find\"\nsince = \"0.2.0\"\noptional = true\n");
        assert!(
            found
                .iter()
                .any(|problem| problem.contains("`optional`") && problem.contains("local = true")),
            "the compositor's own files do not use it: {found:?}"
        );
    }
    /// A namespace with a record `query`, a handle `thing` and a service `root` whose methods are `methods`.
    fn finder(kind: &str, methods: &str) -> String {
        format!(
            "namespace = \"test\"\nversion = \"0.2.0\"\nlocal = true\n\n\
             [[object]]\nname = \"query\"\nsince = \"0.2.0\"\nkind = \"record\"\n\n\
             [[object]]\nname = \"thing\"\nsince = \"0.2.0\"\nkind = \"handle\"\n\n\
             [[object]]\nname = \"root\"\nsince = \"0.2.0\"\nkind = \"{kind}\"\n\n{methods}"
        )
    }

    /// One method `name` that returns `returns` and takes the optional record `query`, with `extra` after its keys.
    fn finding(name: &str, returns: &str, extra: &str) -> String {
        format!(
            "[[object.method]]\nname = \"{name}\"\nsince = \"0.2.0\"\nreturns = \"{returns}\"\n{extra}\n\
             [[object.method.param]]\nname = \"query\"\ntype = \"object:query\"\noptional = true\n\n"
        )
    }

    #[test]
    fn a_find_kind_belongs_to_a_service_method_that_lists_objects_and_names_a_kind_once() {
        let good = format!(
            "{}{}",
            finding("find_things", "list<object:thing>", "find = \"Thing\"\n"),
            finding("find_others", "list<object:thing>", "find = \"Other\"\n")
        );
        let schema = parse_schema("test.toml", &finder("service", &good)).expect("schema");
        let root = schema
            .objects
            .iter()
            .find(|object| object.name == "root")
            .expect("root");
        let kinds: Vec<_> = root
            .methods
            .iter()
            .map(|method| method.extras.find.as_deref())
            .collect();
        assert_eq!(kinds, [Some("Thing"), Some("Other")]);

        let plain = finding("find_things", "list<object:thing>", "");
        let schema = parse_schema("test.toml", &finder("service", &plain)).expect("schema");
        assert_eq!(
            schema.objects[2].methods[0].extras.find, None,
            "a method names no kind unless it says so"
        );

        let twice = format!(
            "{}{}",
            finding("find_things", "list<object:thing>", "find = \"Thing\"\n"),
            finding("find_more", "list<object:thing>", "find = \"Thing\"\n")
        );
        let found = refused(&finder("service", &twice));
        assert!(
            found
                .iter()
                .any(|problem| problem.contains("already finds the kind `Thing`")),
            "{found:?}"
        );

        let lower = finding("find_things", "list<object:thing>", "find = \"thing\"\n");
        let found = refused(&finder("service", &lower));
        assert!(
            found
                .iter()
                .any(|problem| problem.contains("starts with a capital letter")),
            "{found:?}"
        );

        let one = finding("find_things", "object:thing", "find = \"Thing\"\n");
        let found = refused(&finder("service", &one));
        assert!(
            found
                .iter()
                .any(|problem| problem.contains("returns a list of objects")),
            "{found:?}"
        );

        let on_handle = finding("find_things", "list<object:thing>", "find = \"Thing\"\n");
        let found = refused(&finder("handle", &on_handle));
        assert!(
            found.iter().any(|problem| problem.contains("method of a service")),
            "{found:?}"
        );

        let found = problems("\n[[object.method]]\nname = \"find\"\nsince = \"0.2.0\"\nfind = \"Thing\"\n");
        assert!(
            found
                .iter()
                .any(|problem| problem.contains("`find`") && problem.contains("local = true")),
            "the compositor's own files do not use it: {found:?}"
        );
    }
}
