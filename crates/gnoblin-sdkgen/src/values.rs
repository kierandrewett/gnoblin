//! The value classes of a namespace, as every language that renders them sees them.
//!
//! A record of a local namespace is a value class: a class with fields, defaults and a constructor in each language (a class with an
//! option bag in JavaScript and TypeScript, a `dataclass` in Python, a struct with `Default` in Rust). A record that is the family of a
//! tagged value (a `filter` that is a blur, a grade or a shader) is not a class itself: each of its variants is, and the family is the
//! type that holds any of them (a union in TypeScript and Python, an enum in Rust).
//!
//! This module is the one place that decides which value classes a namespace has, what they are called and which fields they hold, so
//! the three renderers cannot differ on a name or a field. It also settles how a variant meets the family's record on the wire, which
//! is the record's own fields in the record's own order: the renderers write the encoding and the decoding in their own syntax from
//! the [`CarrySlot`]s this module gives them, and write no class by hand.

use gnoblin_schema::{DefaultValue, Export, ObjectKind, ObjectType, Property, TaggedVariant, Type};

use super::text::{camel, pascal, python_ident, raw_ident, snake};

/// One value class: a plain record, or a variant of a family.
pub(super) struct ValueClass<'a> {
    /// The schema name (`shadow`, `blur_filter`).
    pub name: &'a str,
    /// The names the schema gives it in a language, where the conversion is not it.
    pub export: &'a Export,
    /// The fields, in the order the schema declares them: a record's own, or a variant's own.
    pub properties: &'a [Property],
    /// For a variant: the family and the variant.
    pub of: Option<(&'a ObjectType, &'a TaggedVariant)>,
    /// The names a script may be tempted to give that are not fields, each with its reason: a record's own, none for a variant.
    pub refuses: &'a [(String, String)],
    /// The value is frozen where the language can freeze it: a record's own, never a variant's.
    pub immutable: bool,
}

impl<'a> ValueClass<'a> {
    /// The class's name in JavaScript and TypeScript.
    pub fn js(&self) -> String {
        js_class(self.name, self.export)
    }

    /// The class's name in Python.
    pub fn py(&self) -> String {
        py_class(self.name, self.export)
    }

    /// The type's name in Rust.
    pub fn rust(&self) -> String {
        rust_class(self.name, self.export)
    }
}

/// The value classes among `objects`: every record that is not a family, and every variant of a family, in the order of the objects.
pub(super) fn value_classes<'a>(objects: impl IntoIterator<Item = &'a ObjectType>) -> Vec<ValueClass<'a>> {
    let mut classes = Vec::new();
    for object in objects {
        if object.kind != ObjectKind::Record {
            continue;
        }
        if object.variants.is_empty() {
            classes.push(ValueClass {
                name: &object.name,
                export: &object.export,
                properties: &object.properties,
                refuses: &object.refuses,
                immutable: object.immutable,
                of: None,
            });
        }
        for variant in &object.variants {
            classes.push(ValueClass {
                name: &variant.name,
                export: &variant.export,
                properties: &variant.properties,
                refuses: &[],
                immutable: false,
                of: Some((object, variant)),
            });
        }
    }
    classes
}

/// Whether the record is the family of a tagged value.
pub(super) fn is_family(object: &ObjectType) -> bool {
    object.kind == ObjectKind::Record && !object.variants.is_empty()
}

pub(super) fn js_class(name: &str, export: &Export) -> String {
    export.js.clone().unwrap_or_else(|| pascal(name))
}

pub(super) fn py_class(name: &str, export: &Export) -> String {
    export.py.clone().unwrap_or_else(|| pascal(name))
}

pub(super) fn rust_class(name: &str, export: &Export) -> String {
    export.rust.clone().unwrap_or_else(|| pascal(name))
}

/// The name of a field in JavaScript.
pub(super) fn js_field(property: &Property) -> String {
    property
        .extras
        .export
        .js
        .clone()
        .unwrap_or_else(|| camel(&property.name))
}

/// The name of a field in Python.
pub(super) fn py_field(property: &Property) -> String {
    python_ident(
        &property
            .extras
            .export
            .py
            .clone()
            .unwrap_or_else(|| snake(&property.name)),
    )
}

/// The name of a field in Rust.
pub(super) fn rust_field(property: &Property) -> String {
    raw_ident(
        &property
            .extras
            .export
            .rust
            .clone()
            .unwrap_or_else(|| snake(&property.name)),
    )
}

/// What a value does when it does not say a field.
pub(super) enum Absent<'a> {
    /// The field starts at this.
    Default(&'a DefaultValue),
    /// The field may be left out: it is none.
    None,
    /// The field is always there, and a value is made with it.
    Required,
}

/// What a missing `property` is.
pub(super) fn absent(property: &Property) -> Absent<'_> {
    match (&property.default, property.optional) {
        (Some(default), _) => Absent::Default(default),
        (None, true) => Absent::None,
        (None, false) => Absent::Required,
    }
}

/// How the family's record carries one of its fields when the value is a given variant.
pub(super) enum CarrySlot<'a> {
    /// The tag: the value of the enum that stands for the variant.
    Tag(&'a str),
    /// The whole field, carried by a property of the variant.
    Whole(&'a Property),
    /// A field that holds a record, part of which the variant carries: the record's own field, and the variant's property for it.
    Part {
        record: &'a ObjectType,
        parts: Vec<Carried<'a>>,
    },
    /// The variant carries nothing in the field.
    Empty,
}

/// A variant's property and the field of a record that holds it.
pub(super) struct Carried<'a> {
    pub inner: &'a Property,
    pub own: &'a Property,
}

/// A field of the family and what carries it.
pub(super) struct FamilySlot<'a> {
    pub field: &'a Property,
    pub slot: CarrySlot<'a>,
}

/// The fields of `family`, in the record's order, with what carries each when the value is `variant`. `class` finds a record by name.
///
/// The catalog has checked that every `field` of a variant names something, so a path that does not is a bug in the catalog's checks.
pub(super) fn slots<'a>(
    class: &dyn Fn(&str) -> Result<&'a ObjectType, String>,
    family: &'a ObjectType,
    variant: &'a TaggedVariant,
) -> Result<Vec<FamilySlot<'a>>, String> {
    let tag = family
        .tag
        .as_deref()
        .ok_or_else(|| format!("the family `{}` has no `tag`", family.name))?;
    let mut slots = Vec::new();
    for field in &family.properties {
        if field.name == tag {
            slots.push(FamilySlot {
                field,
                slot: CarrySlot::Tag(&variant.tag),
            });
            continue;
        }
        let mut whole = None;
        let mut parts = Vec::new();
        for own in &variant.properties {
            let path = own.field.as_deref().unwrap_or_default();
            let (first, rest) = path
                .split_once('.')
                .map_or((path, None), |(first, rest)| (first, Some(rest)));
            if first != field.name {
                continue;
            }
            match rest {
                None => whole = Some(own),
                Some(rest) => {
                    let Type::ObjectRef(record) = &field.ty else {
                        return Err(format!("`{path}` goes into `{first}`, which holds no record"));
                    };
                    let record = class(record)?;
                    let inner = record
                        .properties
                        .iter()
                        .find(|candidate| candidate.name == rest)
                        .ok_or_else(|| format!("`{path}` names no field of the record `{}`", record.name))?;
                    parts.push((record, Carried { inner, own }));
                }
            }
        }
        let slot = match (whole, parts.first().map(|(record, _)| *record)) {
            (Some(own), _) => CarrySlot::Whole(own),
            (None, Some(record)) => CarrySlot::Part {
                record,
                parts: parts.into_iter().map(|(_, carried)| carried).collect(),
            },
            (None, None) => CarrySlot::Empty,
        };
        slots.push(FamilySlot { field, slot });
    }
    Ok(slots)
}

#[cfg(test)]
mod tests {
    use gnoblin_schema::{DefaultValue, Type};

    use super::*;
    use crate::object_script::ScriptView;
    use crate::{SdkSpec, object_api, object_js, object_py, workspace_root};

    /// A value as a language writes it, reduced to what the languages must agree on.
    #[derive(Clone, Debug, PartialEq)]
    enum Value {
        Number(f64),
        Text(String),
        Bool(bool),
        List(Vec<Value>),
        /// An enum value: `"blur"`, `FilterKind.BLUR` and `FilterKind::Blur` are one.
        Word(String),
    }

    /// What a field starts as.
    #[derive(Clone, Debug, PartialEq)]
    enum Start {
        Required,
        Optional,
        Default(Value),
    }

    type Fields = Vec<(String, Start)>;

    fn word(text: &str) -> String {
        text.rsplit(['.', ':'])
            .next()
            .unwrap_or(text)
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>()
            .to_ascii_lowercase()
    }

    fn same(a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Text(text), Value::Word(name)) | (Value::Word(name), Value::Text(text)) => word(text) == *name,
            (Value::List(a), Value::List(b)) => a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same(a, b)),
            (a, b) => a == b,
        }
    }

    /// A literal of JavaScript, Python or Rust.
    fn value_of(text: &str) -> Value {
        let text = text.trim().trim_end_matches(".to_owned()");
        if let Some(inner) = text
            .strip_prefix("vec![")
            .and_then(|rest| rest.strip_suffix(']'))
            .or_else(|| text.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')))
        {
            let items = inner.split(',').map(str::trim).filter(|item| !item.is_empty());
            return Value::List(items.map(value_of).collect());
        }
        if let Some(rest) = text.strip_prefix('"') {
            return Value::Text(rest.trim_end_matches('"').replace("\\\"", "\"").replace("\\\\", "\\"));
        }
        match text {
            "true" | "True" => return Value::Bool(true),
            "false" | "False" => return Value::Bool(false),
            _ => {}
        }
        let number = text
            .trim_end_matches("_f64")
            .trim_end_matches("_i64")
            .trim_end_matches("_u64");
        number
            .parse::<f64>()
            .map_or_else(|_| Value::Word(word(text)), Value::Number)
    }

    /// The lines of the block that starts at the line `head`, up to the line `close`.
    fn block<'t>(text: &'t str, head: &str, close: &str) -> Vec<&'t str> {
        let mut lines = text.lines().skip_while(|line| *line != head);
        assert!(lines.next().is_some(), "`{head}` is not generated:\n{text}");
        lines.take_while(|line| *line != close).collect()
    }

    fn js_fields(js: &str, name: &str) -> Fields {
        block(js, &format!("export class {name} {{"), "}")
            .into_iter()
            .filter_map(|line| line.trim().strip_prefix("this."))
            .map(|line| {
                let (field, rest) = line.split_once(" = ").expect("an assignment");
                let rest = rest.trim_end_matches(';');
                let start = if rest.starts_with("B.need(") {
                    Start::Required
                } else if let Some((_, default)) = rest.split_once(" ?? ") {
                    Start::Default(value_of(default))
                } else {
                    Start::Optional
                };
                (field.to_owned(), start)
            })
            .collect()
    }

    fn dts_fields(dts: &str, name: &str) -> Vec<String> {
        block(dts, &format!("export class {name} {{"), "}")
            .into_iter()
            .filter(|line| !line.trim().starts_with("constructor"))
            .map(|line| {
                let line = line.trim();
                // A readonly field (a frozen record's) is still the field.
                let line = line.strip_prefix("readonly ").unwrap_or(line);
                line.split(['?', ':']).next().unwrap_or_default().to_owned()
            })
            .collect()
    }

    fn py_fields(py: &str, name: &str) -> Fields {
        block(py, &format!("class {name}:"), "")
            .into_iter()
            .filter(|line| line.starts_with("    ") && !line.trim().starts_with("\"\"\""))
            .map(|line| {
                let (field, rest) = line.trim().split_once(": ").expect("an annotation");
                let start = match rest.split_once(" = ") {
                    None => Start::Required,
                    Some((ty, _)) if ty.ends_with("| None") && rest.ends_with(" = None") => Start::Optional,
                    Some((_, default)) => Start::Default(value_of(
                        default
                            .strip_prefix("dataclasses.field(default_factory=lambda: ")
                            .and_then(|inner| inner.strip_suffix(')'))
                            .unwrap_or(default),
                    )),
                };
                (field.to_owned(), start)
            })
            .collect()
    }

    fn rust_fields(rust: &str, name: &str) -> Fields {
        let declared: Vec<String> = block(rust, &format!("pub struct {name} {{"), "}")
            .into_iter()
            .map(|line| {
                line.trim()
                    .trim_start_matches("pub ")
                    .split(':')
                    .next()
                    .unwrap_or_default()
                    .to_owned()
            })
            .collect();
        let starts: Vec<&str> = block(rust, &format!("        {name} {{"), "        }")
            .into_iter()
            .collect();
        declared
            .into_iter()
            .map(|field| {
                let line = starts
                    .iter()
                    .find_map(|line| line.trim().strip_prefix(&format!("{field}: ")))
                    .unwrap_or_else(|| panic!("`{field}` of `{name}` has no start"))
                    .trim_end_matches(',');
                let start = if line == field {
                    Start::Required
                } else if line == "None" {
                    Start::Optional
                } else {
                    Start::Default(value_of(line))
                };
                (field, start)
            })
            .collect()
    }

    fn expected(class: &ValueClass<'_>, name: impl Fn(&Property) -> String) -> Fields {
        class
            .properties
            .iter()
            .map(|property| {
                let start = match absent(property) {
                    Absent::Required => Start::Required,
                    Absent::None => Start::Optional,
                    Absent::Default(default) => Start::Default(canon(&property.ty, default)),
                };
                (name(property), start)
            })
            .collect()
    }

    fn canon(ty: &Type, default: &DefaultValue) -> Value {
        match (default, ty) {
            (DefaultValue::Bool(value), _) => Value::Bool(*value),
            (DefaultValue::Int(value), _) => Value::Number(*value as f64),
            (DefaultValue::Uint(value), _) => Value::Number(*value as f64),
            (DefaultValue::Double(value), _) => Value::Number(*value),
            (DefaultValue::Text(value), Type::Enum(_)) => Value::Word(word(value)),
            (DefaultValue::Text(value), _) => Value::Text(value.clone()),
            (DefaultValue::List(items), Type::List(inner)) => {
                Value::List(items.iter().map(|item| canon(inner, item)).collect())
            }
            (DefaultValue::List(_), _) => panic!("a list default of a type that is not a list"),
        }
    }

    #[track_caller]
    fn agree(what: &str, got: &Fields, wanted: &Fields) {
        assert_eq!(
            got.len(),
            wanted.len(),
            "{what}: the fields differ\n got {got:?}\nwant {wanted:?}"
        );
        for ((got_name, got_start), (name, start)) in got.iter().zip(wanted) {
            assert_eq!(
                got_name, name,
                "{what}: the fields differ in name or order\n got {got:?}\nwant {wanted:?}"
            );
            let alike = match (got_start, start) {
                (Start::Default(a), Start::Default(b)) => same(a, b),
                (a, b) => a == b,
            };
            assert!(
                alike,
                "{what}: `{name}` starts as {got_start:?}, and the catalog says {start:?}"
            );
        }
    }

    /// F5: every value class of both libraries' catalogs has one name, the same fields in the same order, and the same default for each, in
    /// the JavaScript, the declarations, the Python, the Python stub and the Rust. Every one comes from the model in this file, so a
    /// disagreement is a renderer that stopped reading it.
    #[test]
    fn the_three_languages_agree_on_the_names_fields_and_defaults_of_every_value_class() {
        let spec = SdkSpec::load(&workspace_root()).expect("the specification loads");
        let mut checked = 0;
        for namespace in spec.objects.namespaces() {
            let view = ScriptView::new(&spec.objects, &namespace);
            let js = object_js::render_js(&view).expect("the JavaScript renders");
            let dts = object_js::render_dts(&view).expect("the declarations render");
            let py = object_py::render(&view, false).expect("the Python renders");
            let stub = object_py::render(&view, true).expect("the stub renders");
            let rust = object_api::render(&spec.objects, &namespace).expect("the Rust renders");
            for class in value_classes(view.objects.values().copied()) {
                let what = format!("{namespace}: {}", class.name);
                let (js_name, py_name, rust_name) = (class.js(), class.py(), class.rust());
                let model = expected(&class, js_field);
                agree(&format!("{what} in JavaScript"), &js_fields(&js, &js_name), &model);
                assert_eq!(
                    dts_fields(&dts, &js_name),
                    model.iter().map(|(name, _)| name.clone()).collect::<Vec<_>>(),
                    "{what}: the declarations name other fields than the class"
                );
                assert!(
                    dts.contains(&format!("export interface {js_name}Init {{")),
                    "{what}: no option bag"
                );
                agree(
                    &format!("{what} in Python"),
                    &py_fields(&py, &py_name),
                    &expected(&class, py_field),
                );
                let in_stub: Vec<String> = py_fields_of_stub(&stub, &py_name);
                assert_eq!(
                    in_stub,
                    expected(&class, py_field)
                        .into_iter()
                        .map(|(name, _)| name)
                        .collect::<Vec<_>>(),
                    "{what}: the stub names other fields than the class"
                );
                agree(
                    &format!("{what} in Rust"),
                    &rust_fields(&rust, &rust_name),
                    &expected(&class, rust_field),
                );
                checked += 1;
            }
        }
        assert!(checked > 50, "only {checked} value classes were found");
    }

    fn py_fields_of_stub(stub: &str, name: &str) -> Vec<String> {
        block(stub, &format!("class {name}:"), "")
            .into_iter()
            .filter(|line| line.starts_with("    ") && !line.trim().starts_with("\"\"\""))
            .map(|line| line.trim().split(": ").next().unwrap_or_default().to_owned())
            .collect()
    }

    /// F5 and F8: the style value classes carry the names and defaults of the TypeScript library they replace (`BlurFilter` 12, a grade 1,
    /// a `Border` 1 pixel wide), a variant says its tag in all three languages, and each family's converter has an arm for every variant.
    #[test]
    fn the_style_value_classes_have_the_typescript_names_and_defaults_and_every_variant_is_dispatched() {
        let spec = SdkSpec::load(&workspace_root()).expect("the specification loads");
        let namespace = spec
            .objects
            .namespaces()
            .into_iter()
            .find(|namespace| namespace.as_str() == "gnoblin.std")
            .expect("the std namespace");
        let view = ScriptView::new(&spec.objects, &namespace);
        let js = object_js::render_js(&view).expect("the JavaScript renders");
        let py = object_py::render(&view, false).expect("the Python renders");
        let rust = object_api::render(&spec.objects, &namespace).expect("the Rust renders");
        let classes = value_classes(view.objects.values().copied());
        let by_js: std::collections::BTreeMap<String, &ValueClass<'_>> =
            classes.iter().map(|class| (class.js(), class)).collect();
        for (name, field, default) in [
            ("BlurFilter", "radius", 12.0),
            ("SaturateFilter", "amount", 1.0),
            ("BrightnessFilter", "amount", 1.0),
            ("ContrastFilter", "amount", 1.0),
            ("Border", "width", 1.0),
            ("Shadow", "blur", 12.0),
        ] {
            let class = by_js
                .get(name)
                .unwrap_or_else(|| panic!("`{name}` is not a value class"));
            let got = js_fields(&js, name);
            assert!(
                got.iter()
                    .any(|(got, start)| got == field && same_start(start, default)),
                "{name}.{field} does not start at {default}: {got:?}"
            );
            assert_eq!(
                (class.py(), class.rust()),
                (name.to_owned(), name.to_owned()),
                "one name in every language"
            );
        }
        for (tag, class) in [
            ("blur", "BlurFilter"),
            ("saturate", "SaturateFilter"),
            ("brightness", "BrightnessFilter"),
            ("contrast", "ContrastFilter"),
            ("shader", "ShaderFilter"),
        ] {
            let snake_name = snake(class);
            let enc_js: Vec<&str> = js
                .lines()
                .skip_while(|line| !line.starts_with(&format!("function enc_var_{snake_name}(")))
                .take(2)
                .collect();
            assert!(
                enc_js.iter().any(|line| line.contains(&format!("\"{tag}\""))),
                "{class} does not say `{tag}` in JavaScript"
            );
            let enc_py: Vec<&str> = py
                .lines()
                .skip_while(|line| !line.starts_with(&format!("def _enc_var_{snake_name}(")))
                .take(2)
                .collect();
            assert!(
                enc_py.iter().any(|line| line.contains(&format!("\"{tag}\""))),
                "{class} does not say `{tag}` in Python"
            );
            assert!(
                rust.contains(&format!(
                    "FilterKind::{} => <{class} as go::typed::FromVal<Objects>>::from_val",
                    pascal(tag)
                )),
                "the Rust Filter has no arm for {class}"
            );
            assert!(
                js.contains(&format!("case \"{tag}\": return dec_var_{snake_name}(fields);")),
                "{class} is not decoded in JavaScript"
            );
            assert!(
                py.contains(&format!("return _dec_var_{snake_name}(fields)")),
                "{class} is not decoded in Python"
            );
            assert!(
                js.contains(&format!("record instanceof {class}")),
                "{class} is not encoded in JavaScript"
            );
            assert!(
                py.contains(&format!("isinstance(record, {class})")),
                "{class} is not encoded in Python"
            );
        }
        // A kind the library sends that no class stands for is an error and not a wrong class (F8).
        assert!(js.contains("which this SDK has no class for"));
        assert!(py.contains("which this SDK has no class for"));
        assert!(rust.contains("pub enum Filter {"));
        // The family is not a class of its own, and the old record struct is gone (F6).
        assert!(!by_js.contains_key("Filter"));
    }

    fn same_start(start: &Start, default: f64) -> bool {
        matches!(start, Start::Default(Value::Number(number)) if (*number - default).abs() < f64::EPSILON)
    }
}
