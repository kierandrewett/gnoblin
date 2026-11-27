//! Target `reference`: `reference.json`, the machine-readable reference of the object model.
//!
//! One file with every namespace a library serves, and in each every class, property, method, signal, enum, record and callback, with what
//! the schema says about it (description, `since`, range, unit, default, the error codes it raises, an example) and how each language
//! spells it. The reference of the website is built from this file, so it needs nothing the schema does not say.
//!
//! The declarations are the lines the generated SDK files have: the target renders the TypeScript declarations, the Python stubs and the
//! Rust API with a sink, and each of them tells the sink what it wrote for each member as it writes it (`declarations.rs`). Nothing here
//! spells a type or a signature by a rule of its own.
//!
//! ```text
//! { "format": 1,
//!   "languages": [ { "id", "label", ... } ],
//!   "namespaces": [ {
//!       "name", "version", "fingerprint", "package": { "typescript", "python", "rust" },
//!       "counts": { "objects", "properties", "methods", "signals", "enums", "parameters", "callbacks" },
//!       "areas": [ { "id", "title", "file", "classes", "enums", "callbacks", "augments" } ],
//!       "classes": [ { "name", "names", "area", "kind", "base", "since", "description", "import", "example",
//!                      "properties", "methods", "signals", "variants" } ],
//!       "enums": [ { "name", "names", "area", "since", "description", "values": [ { "name", "id", "names", "description", "declarations" } ] } ],
//!       "callbacks": [ ... ],
//!       "augments": [ { "name", "target", ... the members of a class } ] } ] }
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use gnoblin_schema::{
    Augment, CallbackDecl, Catalog, DefaultValue, EnumDecl, Field, Method, Namespace, Notes, ObjectType, Property, Schema,
    SchemaExample, Signal, TaggedVariant, Type, augment_class_name,
};

use crate::declarations::Declarations;
use crate::describe::{DocLanguageExt, number};
use crate::docs::{DocLanguage, NamespaceDocs, base_of};
use crate::object_script::{ScriptView, package_of};
use crate::objects::stem_of;
use crate::text::pascal;
use crate::{COMMAND, File, SdkSpec, Target, object_api, object_js, object_py, values};

/// The format version of the file.
pub const FORMAT: u32 = 1;

/// A JSON value, written in a fixed order so two renders are byte-identical.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Json {
    Null,
    Bool(bool),
    /// A number, as it is written.
    Num(String),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    fn str(text: impl Into<String>) -> Json {
        Json::Str(text.into())
    }

    fn num(value: impl std::fmt::Display) -> Json {
        Json::Num(value.to_string())
    }

    fn obj(fields: Vec<(&str, Json)>) -> Json {
        Json::Obj(fields.into_iter().map(|(key, value)| (key.to_owned(), value)).collect())
    }

    fn strings<'a>(items: impl IntoIterator<Item = &'a str>) -> Json {
        Json::Arr(items.into_iter().map(Json::str).collect())
    }

    fn opt_str(text: Option<&str>) -> Json {
        text.map_or(Json::Null, Json::str)
    }

    /// The text of the value, indented by two spaces.
    pub(crate) fn pretty(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0);
        out.push('\n');
        out
    }

    fn write(&self, out: &mut String, level: usize) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
            Json::Num(text) => out.push_str(text),
            Json::Str(text) => quote(out, text),
            Json::Arr(items) if items.is_empty() => out.push_str("[]"),
            Json::Arr(items) => {
                out.push_str("[\n");
                for (index, item) in items.iter().enumerate() {
                    indent(out, level + 1);
                    item.write(out, level + 1);
                    out.push_str(if index + 1 == items.len() { "\n" } else { ",\n" });
                }
                indent(out, level);
                out.push(']');
            }
            Json::Obj(fields) if fields.is_empty() => out.push_str("{}"),
            Json::Obj(fields) => {
                out.push_str("{\n");
                for (index, (key, value)) in fields.iter().enumerate() {
                    indent(out, level + 1);
                    quote(out, key);
                    out.push_str(": ");
                    value.write(out, level + 1);
                    out.push_str(if index + 1 == fields.len() { "\n" } else { ",\n" });
                }
                indent(out, level);
                out.push('}');
            }
        }
    }
}

fn indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push_str("  ");
    }
}

fn quote(out: &mut String, text: &str) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if (ch as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", ch as u32);
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
}

/// Writes `reference.json`.
pub struct ReferenceTarget;

impl Target for ReferenceTarget {
    fn name(&self) -> &'static str {
        "reference"
    }

    fn generate(&self, spec: &SdkSpec) -> Result<Vec<File>, String> {
        Ok(vec![File {
            path: "reference/reference.json".to_owned(),
            comment: None,
            text: reference(spec)?.pretty(),
        }])
    }
}

/// The reference model of every namespace the libraries serve.
pub(crate) fn reference(spec: &SdkSpec) -> Result<Json, String> {
    let mut namespaces = Vec::new();
    for namespace in spec.objects.namespaces() {
        namespaces.push(namespace_json(spec, &namespace)?);
    }
    Ok(Json::obj(vec![
        ("format", Json::num(FORMAT)),
        (
            "generated",
            Json::str(format!(
                "GENERATED by gnoblin-sdkgen (target: reference) at build time. Nothing here is edited: change the schema; `{COMMAND} --out DIR` writes a readable copy."
            )),
        ),
        ("catalog", Json::str(format!("0x{:016x}", spec.catalog_hash))),
        (
            "languages",
            Json::Arr(vec![
                Json::obj(vec![
                    ("id", Json::str("typescript")),
                    ("label", Json::str("TypeScript")),
                    ("result", Json::Null),
                ]),
                Json::obj(vec![
                    ("id", Json::str("python")),
                    ("label", Json::str("Python")),
                    ("result", Json::Null),
                ]),
                Json::obj(vec![
                    ("id", Json::str("rust")),
                    ("label", Json::str("Rust")),
                    (
                        "result",
                        Json::obj(vec![
                            ("type", Json::str("Result<T, Fail>")),
                            ("import", Json::str("use gnoblin_core::Fail;")),
                        ]),
                    ),
                ]),
            ]),
        ),
        ("namespaces", Json::Arr(namespaces)),
    ]))
}

/// What the file of a schema is called: its name without the extension.
fn area_of(schema: &Schema) -> String {
    Path::new(&schema.origin)
        .file_stem()
        .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned())
}

/// The Rust crate and module that hold the typed classes of a namespace.
fn rust_crate(namespace: &Namespace) -> Result<&'static str, String> {
    Ok(match stem_of(namespace)? {
        "model" => "gnoblin_core",
        _ => "gnoblin_std",
    })
}

/// What a namespace's reference is built from.
struct ReferenceContext<'a> {
    spec: &'a SdkSpec,
    namespace: &'a Namespace,
    docs: &'a NamespaceDocs,
    objects: BTreeMap<&'a str, &'a ObjectType>,
    ts: ScriptView<'a>,
    py: ScriptView<'a>,
    rust: object_api::Spelling<'a>,
    ts_lines: &'a Declarations,
    py_lines: &'a Declarations,
    rust_lines: &'a Declarations,
}

fn namespace_json(spec: &SdkSpec, namespace: &Namespace) -> Result<Json, String> {
    let docs = spec.docs.namespace(namespace);
    let ts_lines = Declarations::default();
    let py_lines = Declarations::default();
    let rust_lines = Declarations::default();
    // The three targets write their files with a sink, and the model reads what they said.
    object_js::render_dts(
        &ScriptView::new(&spec.objects, namespace)
            .with_docs(docs)
            .with_declarations(&ts_lines),
    )?;
    object_py::render(
        &ScriptView::new(&spec.objects, namespace)
            .with_docs(docs)
            .with_flavor(DocLanguage::Python)
            .with_declarations(&py_lines),
        true,
    )?;
    object_api::render_described(&spec.objects, namespace, docs, Some(&rust_lines))?;

    let context = ReferenceContext {
        spec,
        namespace,
        docs,
        objects: spec
            .objects
            .objects(namespace)
            .map(|object| (object.name.as_str(), object))
            .collect(),
        ts: ScriptView::new(&spec.objects, namespace).with_docs(docs),
        py: ScriptView::new(&spec.objects, namespace)
            .with_docs(docs)
            .with_flavor(DocLanguage::Python),
        rust: object_api::Spelling::new(&spec.objects, namespace, docs)?,
        ts_lines: &ts_lines,
        py_lines: &py_lines,
        rust_lines: &rust_lines,
    };
    context.json()
}

impl<'a> ReferenceContext<'a> {
    fn json(&self) -> Result<Json, String> {
        let namespace = self.namespace;
        let schemas: Vec<&Schema> = self
            .spec
            .objects
            .schemas()
            .iter()
            .filter(|schema| &schema.namespace == namespace)
            .collect();
        let stem = stem_of(namespace)?;
        let mut areas = Vec::new();
        let mut classes = Vec::new();
        let mut enums = Vec::new();
        let mut callbacks = Vec::new();
        let mut augments = Vec::new();
        let mut counts = Counts::default();
        for schema in schemas {
            let id = area_of(schema);
            let mut area_classes = Vec::new();
            let mut area_augments = Vec::new();
            for object in schema.objects.iter().filter(|object| object.augment_of.is_none()) {
                area_classes.push(Json::str(object.name.as_str()));
                classes.push(self.class_json(&id, object, &mut counts)?);
            }
            for augment in &schema.augments {
                let name = augment_class_name(&augment.class);
                area_augments.push(Json::str(name.as_str()));
                augments.push(self.augment_json(&id, augment, &name)?);
            }
            for declared in &schema.enums {
                enums.push(self.enum_json(&id, declared));
                counts.enums += 1;
            }
            for declared in &schema.callbacks {
                callbacks.push(self.callback_json(&id, declared));
                counts.callbacks += 1;
            }
            areas.push(Json::obj(vec![
                ("id", Json::str(id.as_str())),
                ("title", Json::opt_str(schema.title.as_deref())),
                ("file", Json::str(format!("{id}.toml"))),
                ("classes", Json::Arr(area_classes)),
                (
                    "enums",
                    Json::strings(schema.enums.iter().map(|declared| declared.name.as_str())),
                ),
                (
                    "callbacks",
                    Json::strings(schema.callbacks.iter().map(|declared| declared.name.as_str())),
                ),
                ("augments", Json::Arr(area_augments)),
            ]));
        }
        let rust = rust_crate(namespace)?;
        Ok(Json::obj(vec![
            ("name", Json::str(namespace.as_str())),
            (
                "version",
                Json::str(
                    self.spec
                        .objects
                        .version(namespace)
                        .map_or_else(String::new, |version| version.to_string()),
                ),
            ),
            (
                "fingerprint",
                Json::str(format!("0x{:016x}", self.spec.objects.namespace_fingerprint(namespace))),
            ),
            (
                "package",
                Json::obj(vec![
                    (
                        "typescript",
                        Json::obj(vec![
                            ("package", Json::str(package_of(namespace)?)),
                            (
                                "module",
                                Json::str(format!("{}/generated/objects", package_of(namespace)?)),
                            ),
                        ]),
                    ),
                    (
                        "python",
                        Json::obj(vec![("module", Json::str(format!("gnoblin.objects.{stem}")))]),
                    ),
                    (
                        "rust",
                        Json::obj(vec![
                            ("crate", Json::str(rust)),
                            ("module", Json::str(format!("{rust}::objects"))),
                        ]),
                    ),
                ]),
            ),
            ("counts", counts.json()),
            ("areas", Json::Arr(areas)),
            ("classes", Json::Arr(classes)),
            ("enums", Json::Arr(enums)),
            ("callbacks", Json::Arr(callbacks)),
            ("augments", Json::Arr(augments)),
        ]))
    }

    /// The names of a class in each language, and the line that imports it.
    fn class_names(&self, object: &ObjectType) -> Result<(Json, Json), String> {
        let (ts, py, rust) = (
            values::js_class(&object.name, &object.export),
            values::py_class(&object.name, &object.export),
            values::rust_class(&object.name, &object.export),
        );
        let package = package_of(self.namespace)?;
        let stem = stem_of(self.namespace)?;
        let crate_name = rust_crate(self.namespace)?;
        // The root of each language's package re-exports the classes: `gnoblin` for the core, `gnoblin.std` for the std library.
        let python_module = if stem == "model" {
            "gnoblin".to_owned()
        } else {
            format!("gnoblin.{stem}")
        };
        let import = Json::obj(vec![
            (
                "typescript",
                Json::str(format!("import {{ {ts} }} from \"{package}\";")),
            ),
            ("python", Json::str(format!("from {python_module} import {py}"))),
            ("rust", Json::str(format!("use {crate_name}::{rust};"))),
        ]);
        Ok((names(&ts, &py, &rust), import))
    }

    /// The description of the member `member` of `class`, from the class or the nearest base that has one.
    fn member_doc(&self, class: &str, member: &str) -> &'a str {
        self.docs.member(class, member, |name| base_of(&self.objects, name))
    }

    fn class_json(&self, area: &str, object: &ObjectType, counts: &mut Counts) -> Result<Json, String> {
        counts.objects += 1;
        let (names, import) = self.class_names(object)?;
        let own = own_members(&self.spec.objects, object, self.namespace);
        let mut properties = Vec::new();
        for property in &own.properties {
            counts.properties += 1;
            properties.push(self.property_json(object, property)?);
        }
        let mut methods = Vec::new();
        for method in &own.methods {
            counts.methods += 1;
            counts.parameters += method.params.len();
            methods.push(self.method_json(object, method)?);
        }
        let mut signals = Vec::new();
        for signal in &own.signals {
            counts.signals += 1;
            signals.push(self.signal_json(object, signal)?);
        }
        let mut variants = Vec::new();
        for variant in &object.variants {
            variants.push(self.variant_json(object, variant)?);
        }
        Ok(Json::obj(vec![
            ("name", Json::str(object.name.as_str())),
            ("names", names),
            ("area", Json::str(area)),
            ("kind", Json::str(object.kind.as_str())),
            ("base", Json::opt_str(object.base.as_deref())),
            ("since", Json::str(object.since.to_string())),
            ("description", Json::str(self.docs.class(&object.name))),
            ("secret", Json::Bool(object.secret)),
            ("import", import),
            ("example", example_json(object.notes.example.as_ref())),
            ("properties", Json::Arr(properties)),
            ("methods", Json::Arr(methods)),
            ("signals", Json::Arr(signals)),
            ("variants", Json::Arr(variants)),
        ]))
    }

    /// What a library adds to a class of the core: a pseudo class that holds the members, named for the class it extends.
    fn augment_json(&self, area: &str, augment: &Augment, name: &str) -> Result<Json, String> {
        let object = self.objects.get(name).ok_or_else(|| {
            format!(
                "`{}` has no class `{name}` for its augment of `{}`",
                self.namespace, augment.target
            )
        })?;
        let mut properties = Vec::new();
        for property in &augment.properties {
            properties.push(self.property_json(object, property)?);
        }
        let mut methods = Vec::new();
        let mut parameters = 0;
        for method in &augment.methods {
            parameters += method.params.len();
            methods.push(self.method_json(object, method)?);
        }
        Ok(Json::obj(vec![
            ("name", Json::str(name)),
            ("target", Json::str(augment.class.as_str())),
            ("area", Json::str(area)),
            ("since", Json::str(augment.since.to_string())),
            (
                "counts",
                Json::obj(vec![
                    ("properties", Json::num(properties.len())),
                    ("methods", Json::num(methods.len())),
                    ("parameters", Json::num(parameters)),
                ]),
            ),
            ("properties", Json::Arr(properties)),
            ("methods", Json::Arr(methods)),
        ]))
    }

    /// The type of a value in each language: the schema's spelling, the three declarations' and the classes and enums it names.
    fn type_json(&self, ty: &Type, ts: &str, py: &str, rust: &str) -> Json {
        let mut refs = Vec::new();
        for name in ty.referenced_objects() {
            refs.push(Json::obj(vec![("kind", Json::str("class")), ("name", Json::str(name))]));
        }
        for name in ty.referenced_enums() {
            refs.push(Json::obj(vec![("kind", Json::str("enum")), ("name", Json::str(name))]));
        }
        for name in ty.referenced_callbacks() {
            refs.push(Json::obj(vec![
                ("kind", Json::str("callback")),
                ("name", Json::str(name)),
            ]));
        }
        Json::obj(vec![
            ("schema", Json::str(ty.to_string())),
            ("typescript", Json::str(ts)),
            ("python", Json::str(py)),
            ("rust", Json::str(rust)),
            ("refs", Json::Arr(refs)),
        ])
    }

    fn lines(&self, class: &str, member: &str) -> Json {
        Json::obj(vec![
            ("typescript", strings(self.ts_lines.get(class, member))),
            ("python", strings(self.py_lines.get(class, member))),
            ("rust", strings(self.rust_lines.get(class, member))),
        ])
    }

    fn property_json(&self, object: &ObjectType, property: &Property) -> Result<Json, String> {
        let names = names(
            &values::js_field(property),
            &values::py_field(property),
            &values::rust_field(property),
        );
        let ty = self.type_json(
            &property.ty,
            &object_js::spelling::property(&self.ts, property)?,
            &object_py::spelling::property(&self.py, object, property)?,
            &self.rust.property(property)?,
        );
        Ok(Json::obj(vec![
            ("name", Json::str(property.name.as_str())),
            ("names", names),
            ("since", Json::str(property.since.to_string())),
            ("description", Json::str(self.member_doc(&object.name, &property.name))),
            ("type", ty),
            ("readonly", Json::Bool(property.readonly)),
            ("optional", Json::Bool(property.optional)),
            ("always", Json::Bool(property.always)),
            ("animated", Json::Bool(property.animated)),
            ("secret", Json::Bool(property.secret)),
            ("default", property.default.as_ref().map_or(Json::Null, default_json)),
            ("range", range_json(&property.notes)),
            ("unit", Json::opt_str(property.notes.unit.as_deref())),
            (
                "raises",
                Json::strings(property.notes.raises.iter().map(String::as_str)),
            ),
            ("example", example_json(property.notes.example.as_ref())),
            ("declarations", self.lines(&object.name, &property.name)),
        ]))
    }

    /// A parameter or a field of a signal or a callback.
    fn field_json(&self, field: &Field, description: &str) -> Result<Json, String> {
        let ts = object_js::spelling::input(&self.ts, field)?;
        let py = object_py::spelling::input(&self.py, field)?;
        let rust = self.rust.input(field)?;
        let names = names(
            &DocLanguage::TypeScript.param_name(&field.name),
            &DocLanguage::Python.param_name(&field.name),
            &DocLanguage::Rust.param_name(&field.name),
        );
        Ok(Json::obj(vec![
            ("name", Json::str(field.name.as_str())),
            ("names", names),
            ("since", Json::str(field.since.to_string())),
            ("description", Json::str(description)),
            ("type", self.type_json(&field.ty, &ts, &py, &rust)),
            ("optional", Json::Bool(field.optional)),
            ("secret", Json::Bool(field.secret)),
            ("default", field.default.as_ref().map_or(Json::Null, default_json)),
            ("range", range_json(&field.notes)),
            ("unit", Json::opt_str(field.notes.unit.as_deref())),
        ]))
    }

    /// Whether the class `name` is an operation, or extends one.
    fn is_operation(&self, name: &str) -> bool {
        let mut class = name;
        loop {
            if class == "operation" {
                return true;
            }
            match self.objects.get(class).and_then(|object| object.base.as_deref()) {
                Some(base) => class = base,
                None => return false,
            }
        }
    }

    fn method_json(&self, object: &ObjectType, method: &Method) -> Result<Json, String> {
        let names = names(
            &object_js::member_name(&method.name, &method.extras.export.js),
            &object_py::member_name(&method.name, &method.extras.export.py),
            &object_api::member_ident(&method.name, &method.extras.export.rust),
        );
        let mut params = Vec::new();
        for param in &method.params {
            params.push(self.field_json(param, self.child_doc(object, &method.name, param))?);
        }
        let returns = match &method.returns {
            None => Json::Null,
            Some(ty) => Json::obj(vec![
                (
                    "type",
                    self.type_json(
                        ty,
                        &object_js::spelling::returns(&self.ts, method)?,
                        &object_py::spelling::returns(&self.py, method)?,
                        &self.rust.returns(method)?,
                    ),
                ),
                ("optional", Json::Bool(method.optional)),
            ]),
        };
        let operation = matches!(&method.returns, Some(Type::ObjectRef(name)) if self.is_operation(name));
        Ok(Json::obj(vec![
            ("name", Json::str(method.name.as_str())),
            ("names", names),
            ("since", Json::str(method.since.to_string())),
            ("description", Json::str(self.member_doc(&object.name, &method.name))),
            ("static", Json::Bool(method.function)),
            ("required", Json::Bool(method.required)),
            ("secret", Json::Bool(method.secret)),
            ("params", Json::Arr(params)),
            ("returns", returns),
            ("operation", Json::Bool(operation)),
            ("raises", Json::strings(method.notes.raises.iter().map(String::as_str))),
            ("example", example_json(method.notes.example.as_ref())),
            ("declarations", self.lines(&object.name, &method.name)),
        ]))
    }

    fn child_doc(&self, object: &ObjectType, member: &str, child: &Field) -> &'a str {
        self.docs
            .child(&object.name, member, &child.name, |name| base_of(&self.objects, name))
    }

    fn signal_json(&self, object: &ObjectType, signal: &Signal) -> Result<Json, String> {
        let (ts, py, rust) = (
            object_js::member_name(&signal.name, &signal.extras.export.js),
            object_py::member_name(&signal.name, &signal.extras.export.py),
            format!(
                "on_{}",
                object_api::member_ident(&signal.name, &signal.extras.export.rust)
            ),
        );
        let mut payload = Vec::new();
        for field in &signal.fields {
            payload.push(self.field_json(field, self.child_doc(object, &signal.name, field))?);
        }
        Ok(Json::obj(vec![
            ("name", Json::str(signal.name.as_str())),
            ("since", Json::str(signal.since.to_string())),
            ("description", Json::str(self.member_doc(&object.name, &signal.name))),
            ("secret", Json::Bool(signal.secret)),
            ("payload", Json::Arr(payload)),
            (
                "listener",
                Json::obj(vec![
                    (
                        "typescript",
                        Json::obj(vec![("method", Json::str("on")), ("event", Json::str(ts))]),
                    ),
                    (
                        "python",
                        Json::obj(vec![("method", Json::str("on")), ("event", Json::str(py))]),
                    ),
                    ("rust", Json::obj(vec![("method", Json::str(rust))])),
                ]),
            ),
            ("example", example_json(signal.notes.example.as_ref())),
            ("declarations", self.lines(&object.name, &signal.name)),
        ]))
    }

    fn variant_json(&self, object: &ObjectType, variant: &TaggedVariant) -> Result<Json, String> {
        let mut fields = Vec::new();
        for property in &variant.properties {
            fields.push(Json::obj(vec![
                ("name", Json::str(property.name.as_str())),
                ("field", Json::opt_str(property.field.as_deref())),
                (
                    "description",
                    Json::str(self.docs.variant_property(&object.name, &variant.name, &property.name)),
                ),
                ("type", Json::str(property.ty.to_string())),
                ("default", property.default.as_ref().map_or(Json::Null, default_json)),
                ("range", range_json(&property.notes)),
                ("unit", Json::opt_str(property.notes.unit.as_deref())),
            ]));
        }
        Ok(Json::obj(vec![
            ("name", Json::str(variant.name.as_str())),
            ("tag", Json::str(variant.tag.as_str())),
            ("since", Json::str(variant.since.to_string())),
            ("description", Json::str(self.docs.variant(&object.name, &variant.name))),
            ("properties", Json::Arr(fields)),
        ]))
    }

    fn enum_json(&self, area: &str, declared: &EnumDecl) -> Json {
        let mut values = Vec::new();
        for (index, value) in declared.values.iter().enumerate() {
            values.push(Json::obj(vec![
                ("name", Json::str(value.as_str())),
                ("id", Json::num(index + 1)),
                (
                    "names",
                    names(&format!("\"{value}\""), &object_py::enum_member(value), &pascal(value)),
                ),
                ("description", Json::str(self.docs.enum_value(&declared.name, value))),
                ("declarations", self.lines(&declared.name, value)),
            ]));
        }
        let name = pascal(&declared.name);
        Json::obj(vec![
            ("name", Json::str(declared.name.as_str())),
            ("names", names(&name, &name, &name)),
            ("area", Json::str(area)),
            ("since", Json::str(declared.since.to_string())),
            ("description", Json::str(self.docs.enum_doc(&declared.name))),
            ("values", Json::Arr(values)),
        ])
    }

    fn callback_json(&self, area: &str, declared: &CallbackDecl) -> Json {
        let params: Vec<Json> = declared
            .params
            .iter()
            .map(|param| {
                Json::obj(vec![
                    ("name", Json::str(param.name.as_str())),
                    ("type", Json::str(param.ty.to_string())),
                    (
                        "description",
                        Json::str(self.docs.callback_param(&declared.name, &param.name)),
                    ),
                ])
            })
            .collect();
        Json::obj(vec![
            ("name", Json::str(declared.name.as_str())),
            ("area", Json::str(area)),
            ("since", Json::str(declared.since.to_string())),
            ("description", Json::str(self.docs.callback(&declared.name))),
            ("params", Json::Arr(params)),
            (
                "returns",
                declared
                    .returns
                    .as_ref()
                    .map_or(Json::Null, |ty| Json::str(ty.to_string())),
            ),
            ("optional", Json::Bool(declared.optional)),
        ])
    }
}

fn names(typescript: &str, python: &str, rust: &str) -> Json {
    Json::obj(vec![
        ("typescript", Json::str(typescript)),
        ("python", Json::str(python)),
        ("rust", Json::str(rust)),
    ])
}

fn strings(lines: Vec<String>) -> Json {
    Json::Arr(lines.into_iter().map(Json::Str).collect())
}

fn range_json(notes: &Notes) -> Json {
    notes.range.map_or(Json::Null, |range| {
        Json::obj(vec![
            ("min", Json::Num(number(range.min))),
            ("max", Json::Num(number(range.max))),
        ])
    })
}

fn example_json(example: Option<&SchemaExample>) -> Json {
    example.map_or(Json::Null, |example| {
        Json::obj(vec![
            ("typescript", Json::opt_str(example.typescript.as_deref())),
            ("python", Json::opt_str(example.python.as_deref())),
            ("rust", Json::opt_str(example.rust.as_deref())),
        ])
    })
}

fn default_json(default: &DefaultValue) -> Json {
    match default {
        DefaultValue::Bool(flag) => Json::Bool(*flag),
        DefaultValue::Int(value) => Json::num(value),
        DefaultValue::Uint(value) => Json::num(value),
        DefaultValue::Double(value) => Json::Num(number(*value)),
        DefaultValue::Text(text) => Json::str(text.as_str()),
        DefaultValue::List(items) => Json::Arr(items.iter().map(default_json).collect()),
    }
}

/// How many of each there are in a namespace: what a test checks the file against.
#[derive(Default)]
struct Counts {
    objects: usize,
    properties: usize,
    methods: usize,
    signals: usize,
    enums: usize,
    parameters: usize,
    callbacks: usize,
}

impl Counts {
    fn json(&self) -> Json {
        Json::obj(vec![
            ("objects", Json::num(self.objects)),
            ("properties", Json::num(self.properties)),
            ("methods", Json::num(self.methods)),
            ("signals", Json::num(self.signals)),
            ("enums", Json::num(self.enums)),
            ("parameters", Json::num(self.parameters)),
            ("callbacks", Json::num(self.callbacks)),
        ])
    }
}

/// The members a class declares itself: the catalog resolves a base's members into each class that extends it, and the reference lists
/// each member once, under the class that declares it.
pub(crate) struct OwnMembers<'a> {
    pub(crate) properties: Vec<&'a Property>,
    pub(crate) methods: Vec<&'a Method>,
    pub(crate) signals: Vec<&'a Signal>,
}

/// The class a `base` names, as the catalog holds it: `name` of the class's own namespace, or `namespace.name` of another's.
fn base_class<'a>(catalog: &'a Catalog, namespace: &Namespace, base: &str) -> Option<&'a ObjectType> {
    let (home, name) = match base.rsplit_once('.') {
        Some((home, name)) => (Namespace::parse(home).ok()?, name),
        None => (namespace.clone(), base),
    };
    catalog
        .schemas()
        .iter()
        .filter(|schema| schema.namespace == home)
        .flat_map(|schema| schema.objects.iter())
        .find(|object| object.name == name)
}

/// The members of `object` that its base does not have.
pub(crate) fn own_members<'a>(catalog: &'a Catalog, object: &'a ObjectType, namespace: &Namespace) -> OwnMembers<'a> {
    let inherited: BTreeSet<&str> = object
        .base
        .as_deref()
        .and_then(|base| base_class(catalog, namespace, base))
        .map(|base| {
            base.properties
                .iter()
                .map(|property| property.name.as_str())
                .chain(base.methods.iter().map(|method| method.name.as_str()))
                .chain(base.signals.iter().map(|signal| signal.name.as_str()))
                .collect()
        })
        .unwrap_or_default();
    OwnMembers {
        properties: object
            .properties
            .iter()
            .filter(|property| !inherited.contains(property.name.as_str()))
            .collect(),
        methods: object
            .methods
            .iter()
            .filter(|method| !inherited.contains(method.name.as_str()))
            .collect(),
        signals: object
            .signals
            .iter()
            .filter(|signal| !inherited.contains(signal.name.as_str()))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::model::MODEL_DIRS;
    use crate::{render, workspace_root};

    fn get<'a>(json: &'a Json, key: &str) -> &'a Json {
        let Json::Obj(fields) = json else {
            panic!("not an object looking for `{key}`");
        };
        &fields
            .iter()
            .find(|(name, _)| name == key)
            .unwrap_or_else(|| panic!("no key `{key}`"))
            .1
    }

    fn items(json: &Json) -> &[Json] {
        match json {
            Json::Arr(items) => items,
            other => panic!("not an array: {other:?}"),
        }
    }

    fn text(json: &Json) -> &str {
        match json {
            Json::Str(text) => text,
            other => panic!("not a string: {other:?}"),
        }
    }

    fn number(json: &Json) -> usize {
        match json {
            Json::Num(text) => text.parse().expect("a count"),
            other => panic!("not a number: {other:?}"),
        }
    }

    /// The tables of a schema file text, counted by the header line each has, with the namespace the file declares.
    fn tables(text: &str, totals: &mut BTreeMap<String, BTreeMap<&'static str, usize>>) {
        let namespace = text
            .lines()
            .find_map(|line| line.strip_prefix("namespace = \"")?.strip_suffix('"'))
            .expect("a namespace")
            .to_owned();
        let entry = totals.entry(namespace).or_default();
        for line in text.lines() {
            let key = match line.trim() {
                "[[object]]" => "objects",
                "[[object.property]]" => "properties",
                "[[object.method]]" => "methods",
                "[[object.method.param]]" => "parameters",
                "[[object.signal]]" => "signals",
                "[[enum]]" => "enums",
                "[[callback]]" => "callbacks",
                _ => continue,
            };
            *entry.entry(key).or_default() += 1;
        }
    }

    #[test]
    fn the_counts_are_the_tables_of_the_schema_files() {
        let root = workspace_root();
        let spec = SdkSpec::load(&root).expect("the catalogs load");
        let mut totals = BTreeMap::new();
        for dir in MODEL_DIRS {
            for entry in fs::read_dir(root.join(dir)).expect("a schema directory") {
                let path = entry.expect("an entry").path();
                if path.extension().is_some_and(|extension| extension == "toml") {
                    tables(&fs::read_to_string(&path).expect("a schema file"), &mut totals);
                }
            }
        }
        // The built-in layouts are the std catalog's file `layouts.toml`, written from the library's table and never on disk.
        tables(&crate::layout_schema::schema_text(), &mut totals);

        let reference = reference(&spec).expect("the reference builds");
        let mut seen = 0;
        for namespace in items(get(&reference, "namespaces")) {
            let name = text(get(namespace, "name"));
            let counts = get(namespace, "counts");
            let expected = totals.get(name).unwrap_or_else(|| panic!("no files for `{name}`"));
            for (kind, count) in expected {
                assert_eq!(number(get(counts, kind)), *count, "{name}: {kind}");
            }
            // The arrays hold what the counts say.
            assert_eq!(
                items(get(namespace, "classes")).len(),
                number(get(counts, "objects")),
                "{name}"
            );
            assert_eq!(
                items(get(namespace, "enums")).len(),
                number(get(counts, "enums")),
                "{name}"
            );
            assert_eq!(
                items(get(namespace, "callbacks")).len(),
                number(get(counts, "callbacks")),
                "{name}"
            );
            seen += 1;
        }
        assert_eq!(seen, 2, "the core and the std library");
    }

    /// The lines of a generated file, trimmed.
    fn lines_of(files: &[crate::TargetFiles], path: &str) -> Vec<String> {
        files
            .iter()
            .flat_map(|target| &target.files)
            .find(|(name, _)| name == path)
            .unwrap_or_else(|| panic!("no file `{path}`"))
            .1
            .lines()
            .map(|line| line.trim().to_owned())
            .collect()
    }

    #[test]
    fn every_declaration_is_a_line_of_the_sdk_file() {
        let spec = SdkSpec::load(&workspace_root()).expect("the catalogs load");
        let files = render(&spec, None).expect("the targets render");
        let reference = reference(&spec).expect("the reference builds");
        let mut checked = 0;
        for namespace in items(get(&reference, "namespaces")) {
            let stem = text(get(namespace, "name"))
                .rsplit('.')
                .next()
                .expect("a stem")
                .to_owned();
            let package = if stem == "model" { "gnoblin-core" } else { "gnoblin-std" };
            let ts = lines_of(&files, &format!("js/{package}/generated/objects.d.ts"));
            let py = lines_of(&files, &format!("python/gnoblin/objects/{stem}.pyi"));
            let rust: Vec<String> = lines_of(&files, &format!("rust-objects/{stem}_api.rs"))
                .iter()
                .map(|line| object_api::public_result(line))
                .collect();
            for class in items(get(namespace, "classes")) {
                let members = ["properties", "methods", "signals"]
                    .into_iter()
                    .flat_map(|key| items(get(class, key)));
                for member in members {
                    let declarations = get(member, "declarations");
                    for line in items(get(declarations, "typescript")) {
                        assert!(ts.iter().any(|have| have == text(line)), "TypeScript: `{}`", text(line));
                        checked += 1;
                    }
                    for line in items(get(declarations, "python")) {
                        assert!(
                            py.iter().any(|have| have.starts_with(text(line))),
                            "Python: `{}`",
                            text(line)
                        );
                        checked += 1;
                    }
                    for line in items(get(declarations, "rust")) {
                        assert!(
                            rust.iter().any(|have| have.starts_with(text(line))),
                            "Rust: `{}`",
                            text(line)
                        );
                        checked += 1;
                    }
                }
            }
            for declared in items(get(namespace, "enums")) {
                for value in items(get(declared, "values")) {
                    let declarations = get(value, "declarations");
                    for line in items(get(declarations, "typescript")) {
                        assert!(
                            ts.iter().any(|have| have.contains(text(line))),
                            "TypeScript: `{}`",
                            text(line)
                        );
                    }
                    for line in items(get(declarations, "python")) {
                        assert!(py.iter().any(|have| have == text(line)), "Python: `{}`", text(line));
                    }
                    for line in items(get(declarations, "rust")) {
                        assert!(rust.iter().any(|have| have == text(line)), "Rust: `{}`", text(line));
                    }
                    checked += 3;
                }
            }
        }
        assert!(checked > 2000, "the reference declares most members: {checked}");
    }

    #[test]
    fn the_rust_result_is_the_public_one() {
        let spec = SdkSpec::load(&workspace_root()).expect("the catalogs load");
        let reference = reference(&spec).expect("the reference builds");
        let core = &items(get(&reference, "namespaces"))[0];
        let class = items(get(core, "classes"))
            .iter()
            .find(|class| text(get(class, "name")) == "mouse")
            .expect("the class mouse");
        let method = items(get(class, "methods"))
            .iter()
            .find(|method| text(get(method, "name")) == "reset")
            .expect("the method reset");
        let rust = items(get(get(method, "declarations"), "rust"));
        assert_eq!(text(&rust[0]), "pub fn reset(&self, name: &str) -> Result<(), Fail>");
        let python = items(get(get(method, "declarations"), "python"));
        assert_eq!(text(&python[0]), "def reset(self, name: str) -> None");
    }
}
