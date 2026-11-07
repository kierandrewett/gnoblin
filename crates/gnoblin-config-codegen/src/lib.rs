//! The configuration schema, and everything generated from it.
//!
//! `crates/gnoblin-config/schema/config.toml` declares every setting once:
//! section, key, type and range, default, accepted-values text, `since`,
//! whether the runtime may override it, and the deprecated spellings that still
//! parse. This crate reads that file and writes:
//!
//! * the Rust document (structs, enums, the alias table, the per-field
//!   validators and the settings catalogue) into `OUT_DIR`, which
//!   `gnoblin-config`'s `build.rs` compiles with `include!`, so a clean checkout
//!   builds with plain `cargo build` and no prior step;
//! * the reference tables and the `gnoblin.configure` Lua stubs, as text for
//!   `cargo xtask gen-config` to write into a gitignored directory.
//!
//! Nothing generated is committed. The generator is a library rather than a
//! module of the build script so `build.rs` and `xtask` run the same code.
//!
//! The generator is also the first line of validation, because a schema mistake
//! should fail the build rather than produce a document that cannot be
//! validated: a default outside its own range, an alias whose canonical spelling
//! is not a variant of its setting, an alias spelling that is already current (so
//! the alias could never fire), a reference to a record declared later, or a hook
//! named on a field (a hook belongs to a whole table) are all errors here.
//!
//! # Adding a setting
//!
//! 1. Add a `[[section.field]]` (or `[[record.field]]`) to `config.toml` with
//!    `key`, `kind`, `since` (a full `x.y.z`), `doc` and, where it applies,
//!    `min`/`max`, `default`, `enum`, `record` or `inherit`.
//! 2. If the value needs a rule no data can state (a cross-field conflict),
//!    name a hook on the enclosing section or record and define it in
//!    `crates/gnoblin-config/src/hooks.rs`. Fields may not name hooks.
//! 3. Build. `build.rs` calls [`generate`], which fails on any schema error
//!    above, so a mistake shows up as a build error, not a runtime one.
//! 4. Run `cargo xtask gen-config` to refresh the reference tables and Lua stubs.
//!
//! Pipeline: `config.toml` is deserialised into the private `Schema` model,
//! checked by `check_schema`, then rendered by `render_rust`, `render_table`,
//! `render_docs`, `render_lua` and `render_settings_json`. Each renderer is a
//! pure function of the checked schema, so the outputs cannot drift apart.
//! Field names are the key with `-` replaced by `_`; Rust keywords get a raw
//! identifier (`r#type`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

/// The layout types every `layout-selector` field generates. They are fixed, not
/// per-schema: a layout is named by text, or referenced by root, module, content
/// hash and plain options, and the compositor resolves that reference itself.
const LAYOUT_TYPES: &str = r#"/// Where a layout module is read from: one of two fixed roots, never a path on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutRoot {
    /// The layouts the compositor ships.
    Shipped,
    /// The layouts in the user's configuration directory.
    User,
}

/// A plain value passed to a layout module as one of its options.
#[derive(Clone, Debug, PartialEq)]
pub enum LayoutOptionValue {
    /// A boolean option.
    Bool(bool),
    /// An integer option.
    Integer(i64),
    /// A number option.
    Float(f64),
    /// A text option.
    Text(String),
}

/// A layout module named by its root, dotted module path, content hash and options.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutReference {
    /// The root the module is read from.
    pub root: LayoutRoot,
    /// The dotted module path under the root, for example `gnoblin.layouts.tile`.
    pub module: String,
    /// The content hash the module must have, as hex.
    pub hash: String,
    /// The options the layout starts with, by name.
    pub options: std::collections::BTreeMap<String, LayoutOptionValue>,
}

/// A layout a setting names: a shipped layout by its name, or a module by reference.
#[derive(Clone, Debug, PartialEq)]
pub enum LayoutSelector {
    /// A shipped layout, by name. This is shorthand for `gnoblin.layouts.<name>`.
    Named(String),
    /// A module reference, verified against its hash before it runs.
    Reference(LayoutReference),
}
"#;

use serde::Deserialize;

/// Everything the generator writes.
#[derive(Clone, Debug, Default)]
pub struct Artifacts {
    /// The Rust module compiled into `gnoblin-config`.
    pub rust: String,
    /// The reference page body: one table of settings per section.
    pub docs: String,
    /// The `gnoblin.configure` Lua type stubs fragment.
    pub lua: String,
    /// Every setting as JSON, for `gnoblinctl` and the docs site.
    pub settings_json: String,
}

/// Read a schema and generate every artifact.
///
/// Every problem found is reported, not just the first: a schema edit should
/// learn about all of its mistakes in one build.
pub fn generate(schema_toml: &str) -> Result<Artifacts, Vec<String>> {
    let schema: Schema = match toml::from_str(schema_toml) {
        Ok(schema) => schema,
        Err(error) => return Err(vec![format!("schema does not parse: {error}")]),
    };
    let mut errors: Vec<String> = Vec::new();
    if schema.format != 1 {
        errors.push(format!("schema format {} is not 1", schema.format));
    }
    check_schema(&schema, &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(Artifacts {
        rust: render_rust(&schema),
        docs: render_docs(&schema),
        lua: render_lua(&schema),
        settings_json: render_settings_json(&schema),
    })
}

/// Rust keywords a field name may collide with.
const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "box", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false",
    "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self",
    "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while", "yield",
];

/// The kinds of value a setting can hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Bool,
    Int,
    Float,
    Text,
    Enum,
    EnumList,
    TextList,
    FloatList,
    Record,
    RecordList,
    RecordMap,
    EnumBoolMap,
    TextListMap,
    LayoutSelector,
}

impl Kind {
    fn parse(text: &str) -> Option<Kind> {
        Some(match text {
            "bool" => Kind::Bool,
            "int" => Kind::Int,
            "float" => Kind::Float,
            "text" => Kind::Text,
            "enum" => Kind::Enum,
            "enum-list" => Kind::EnumList,
            "text-list" => Kind::TextList,
            "float-list" => Kind::FloatList,
            "record" => Kind::Record,
            "record-list" => Kind::RecordList,
            "record-map" => Kind::RecordMap,
            "enum-bool-map" => Kind::EnumBoolMap,
            "text-list-map" => Kind::TextListMap,
            "layout-selector" => Kind::LayoutSelector,
            _ => return None,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Schema {
    format: u32,
    #[serde(default)]
    r#enum: Vec<EnumDecl>,
    #[serde(default)]
    alias: Vec<AliasDecl>,
    #[serde(default)]
    record: Vec<TableDecl>,
    #[serde(default)]
    section: Vec<TableDecl>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnumDecl {
    name: String,
    setting: String,
    doc: String,
    variants: Vec<VariantDecl>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VariantDecl {
    spelling: String,
    doc: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AliasDecl {
    setting: String,
    spelling: String,
    canonical: String,
    removed_in: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TableDecl {
    #[serde(default)]
    key: String,
    name: Option<String>,
    r#type: Option<String>,
    doc: String,
    hook: Option<String>,
    #[serde(default)]
    overridable: bool,
    #[serde(default)]
    field: Vec<FieldDecl>,
    spread: Option<SpreadDecl>,
    /// Kept in the document but not interpreted: no struct and no validator.
    #[serde(default)]
    carried: bool,
}

impl TableDecl {
    fn rust_name(&self) -> &str {
        // A section declares `type`, a record declares `name`; the generator
        // checks that exactly one is present.
        self.r#type.as_deref().or(self.name.as_deref()).unwrap_or_default()
    }

    fn keys_const(&self) -> String {
        format!("{}_KEYS", self.rust_name().to_uppercase())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FieldDecl {
    key: String,
    kind: String,
    #[serde(default, rename = "enum")]
    enum_name: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    record: Option<String>,
    #[serde(default)]
    min: Option<f64>,
    #[serde(default)]
    max: Option<f64>,
    #[serde(default)]
    exclusive_max: bool,
    #[serde(default)]
    length: Option<usize>,
    #[serde(default)]
    nonempty: bool,
    #[serde(default)]
    first_nonempty: bool,
    #[serde(default)]
    inherit: bool,
    #[serde(default)]
    required: Option<String>,
    accepted: String,
    #[serde(default)]
    default: Option<DefaultDecl>,
    doc: String,
    since: String,
    #[serde(default)]
    min_int: Option<i64>,
    #[serde(default)]
    max_int: Option<i64>,
}

impl FieldDecl {
    /// The Rust field name. A key that is a Rust keyword keeps a raw identifier,
    /// because `input-sources.sources[].type` has to stay `type` in the document.
    fn field_name(&self) -> String {
        let name = self.key.replace('-', "_");
        if RUST_KEYWORDS.contains(&name.as_str()) {
            format!("r#{name}")
        } else {
            name
        }
    }

    /// The name the Lua stubs use, which is never a raw identifier.
    fn lua_name(&self) -> String {
        self.key.replace('-', "_")
    }

    fn kind(&self) -> Kind {
        Kind::parse(&self.kind).unwrap_or(Kind::Bool)
    }

    fn int_bounds(&self) -> (i64, i64) {
        (
            self.min_int.unwrap_or_else(|| self.min.unwrap_or(0.0) as i64),
            self.max_int.unwrap_or_else(|| self.max.unwrap_or(0.0) as i64),
        )
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DefaultDecl {
    #[serde(default)]
    bool: Option<bool>,
    #[serde(default, rename = "int")]
    integer: Option<i64>,
    #[serde(default)]
    float: Option<f64>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default, rename = "enum")]
    variant: Option<String>,
    #[serde(default)]
    list: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SpreadDecl {
    field: String,
    kind: String,
    #[serde(default, rename = "enum")]
    variant_enum: Option<String>,
    #[serde(default)]
    record: Option<String>,
    #[serde(default)]
    id: Option<String>,
    accepted: String,
    doc: String,
}

impl SpreadDecl {
    fn kind(&self) -> Kind {
        Kind::parse(&self.kind).unwrap_or(Kind::EnumBoolMap)
    }
}

/// The ids a record map's keys must have.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IdShape {
    Any,
    HexPair,
    Serial,
    Renderer,
    Name,
}

impl IdShape {
    fn parse(text: &str) -> Option<IdShape> {
        Some(match text {
            "any" => IdShape::Any,
            "hex-pair" => IdShape::HexPair,
            "serial" => IdShape::Serial,
            "renderer" => IdShape::Renderer,
            "name" => IdShape::Name,
            _ => return None,
        })
    }

    fn rust(self) -> &'static str {
        match self {
            IdShape::Any => "IdShape::Any",
            IdShape::HexPair => "IdShape::HexPair",
            IdShape::Serial => "IdShape::Serial",
            IdShape::Renderer => "IdShape::Renderer",
            IdShape::Name => "IdShape::Name",
        }
    }
}

/// Every hook `crates/gnoblin-config/src/hooks.rs` implements.
///
/// The generator refuses a hook that is not here, so a typo fails the build with
/// the list in the message; `hooks.rs` repeats the list and a test compares the
/// two, so the file cannot silently lose one.
pub const HOOKS: &[&str] = &[
    "accel_curve",
    "appearance",
    "button_layout",
    "compositor",
    "frame_renderers",
    "gesture",
    "gestures",
    "pad_button",
    "tablet",
    "window_management",
    "workspace",
    "workspaces",
    "xwayland",
];

/// Report every mistake in the declarations.
fn check_schema(schema: &Schema, errors: &mut Vec<String>) {
    let enums: BTreeSet<&str> = schema.r#enum.iter().map(|decl| decl.name.as_str()).collect();
    let schema_enums: BTreeMap<&str, &EnumDecl> = schema.r#enum.iter().map(|decl| (decl.name.as_str(), decl)).collect();
    let mut seen_records: BTreeSet<String> = BTreeSet::new();
    let record_names: BTreeSet<String> = schema
        .record
        .iter()
        .map(|decl| decl.name.clone().unwrap_or_default())
        .collect();
    for decl in &schema.r#enum {
        if decl.variants.is_empty() {
            errors.push(format!("enum {} has no variants", decl.name));
        }
    }
    // An alias must resolve to a variant of an enum declared for that setting.
    for alias in &schema.alias {
        let matching: Vec<&EnumDecl> = schema
            .r#enum
            .iter()
            .filter(|decl| decl.setting == alias.setting)
            .collect();
        if matching.is_empty() {
            errors.push(format!(
                "alias `{}` names setting `{}`, which no enum declares",
                alias.spelling, alias.setting
            ));
            continue;
        }
        if !matching
            .iter()
            .any(|decl| decl.variants.iter().any(|v| v.spelling == alias.canonical))
        {
            errors.push(format!(
                "alias `{}` -> `{}`: no enum for setting `{}` has that canonical spelling",
                alias.spelling, alias.canonical, alias.setting
            ));
        }
        if matching
            .iter()
            .any(|decl| decl.variants.iter().any(|v| v.spelling == alias.spelling))
        {
            errors.push(format!(
                "alias `{}` is already a current spelling of `{}`, so it could never fire",
                alias.spelling, alias.setting
            ));
        }
        if !is_release(&alias.removed_in) {
            errors.push(format!(
                "alias `{}` has removed_in `{}`, not x.y.z",
                alias.spelling, alias.removed_in
            ));
        }
    }
    for decl in &schema.record {
        let name = decl.name.clone().unwrap_or_default();
        if name.is_empty() {
            errors.push(format!("record `{}` needs a name", decl.key));
        }
        if decl.r#type.is_some() {
            errors.push(format!("record `{name}` must not declare `type`"));
        }
        if decl.spread.is_some() {
            errors.push(format!("record `{name}` cannot declare a spread"));
        }
        seen_records.insert(name);
        check_table(decl, &schema_enums, &enums, &record_names, &seen_records, errors);
    }
    for decl in &schema.section {
        if decl.r#type.is_none() && !decl.carried {
            errors.push(format!("section `{}` needs `type`", decl.key));
        }
        if decl.carried && (decl.spread.is_some() || !decl.field.is_empty() || decl.hook.is_some()) {
            errors.push(format!(
                "carried section `{}` cannot declare fields, a hook or a spread",
                decl.key
            ));
        }
        if decl.name.is_some() {
            errors.push(format!("section `{}` must not declare `name`", decl.key));
        }
        if decl.spread.is_some() && !decl.field.is_empty() {
            errors.push(format!("section `{}` has both a spread and fields", decl.key));
        }
        check_table(decl, &schema_enums, &enums, &record_names, &seen_records, errors);
    }
}

fn check_table(
    decl: &TableDecl,
    schema_enums: &BTreeMap<&str, &EnumDecl>,
    enums: &BTreeSet<&str>,
    records: &BTreeSet<String>,
    _seen: &BTreeSet<String>,
    errors: &mut Vec<String>,
) {
    let name = decl.rust_name();
    if let Some(hook) = &decl.hook
        && !HOOKS.contains(&hook.as_str())
    {
        errors.push(format!(
            "`{name}` names hook `{hook}`, which hooks.rs does not implement"
        ));
    }
    let mut keys: BTreeSet<&str> = BTreeSet::new();
    for field in &decl.field {
        if !keys.insert(field.key.as_str()) {
            errors.push(format!("`{name}` declares `{}` twice", field.key));
        }
        let kind = Kind::parse(&field.kind).unwrap_or_else(|| {
            errors.push(format!("`{name}.{}` has unknown kind `{}`", field.key, field.kind));
            Kind::Bool
        });
        match kind {
            Kind::Enum | Kind::EnumList => match &field.enum_name {
                Some(target) if !enums.contains(target.as_str()) => {
                    errors.push(format!("`{name}.{}` names unknown enum `{target}`", field.key));
                }
                None => errors.push(format!("`{name}.{}` is an enum but names no enum", field.key)),
                _ => {}
            },
            Kind::Record | Kind::RecordList | Kind::RecordMap => match &field.record {
                Some(target) if !records.contains(target) => {
                    errors.push(format!("`{name}.{}` names unknown record `{target}`", field.key));
                }
                None => errors.push(format!("`{name}.{}` is a record field but names no record", field.key)),
                _ => {}
            },
            _ => {}
        }
        if field.first_nonempty && (kind != Kind::TextList || field.inherit) {
            errors.push(format!(
                "`{name}.{}` sets first_nonempty, which only a non-inheriting text-list takes",
                field.key
            ));
        }
        if !is_release(&field.since) {
            errors.push(format!("`{name}.{}` has since `{}`, not x.y.z", field.key, field.since));
        }
        check_default(decl, field, kind, schema_enums, errors);
    }
}

/// A default must be expressible by the field's kind and inside its range.
fn check_default(
    decl: &TableDecl,
    field: &FieldDecl,
    kind: Kind,
    enums: &BTreeMap<&str, &EnumDecl>,
    errors: &mut Vec<String>,
) {
    let name = decl.rust_name();
    let Some(default) = &field.default else {
        return;
    };
    let where_ = format!("`{name}.{}`", field.key);
    match kind {
        Kind::Bool => {
            if default.bool.is_none() {
                errors.push(format!("{where_} has a non-boolean default"));
            }
        }
        Kind::Int => match default.integer {
            Some(value) => {
                let (min, max) = field.int_bounds();
                if value < min || value > max {
                    errors.push(format!("{where_} defaults to {value}, outside {min}..={max}"));
                }
            }
            None => match default.float {
                Some(value) if value.fract() == 0.0 => {
                    let (min, max) = field.int_bounds();
                    let value = value as i64;
                    if value < min || value > max {
                        errors.push(format!("{where_} defaults to {value}, outside {min}..={max}"));
                    }
                }
                _ => errors.push(format!("{where_} has a non-integer default")),
            },
        },
        Kind::Float => match default.float.or_else(|| default.integer.map(|value| value as f64)) {
            Some(value) => {
                let min = field.min.unwrap_or(f64::MIN);
                let max = field.max.unwrap_or(f64::MAX);
                let inside = value >= min && if field.exclusive_max { value < max } else { value <= max };
                if !inside {
                    errors.push(format!("{where_} defaults to {value}, outside its declared range"));
                }
            }
            None => errors.push(format!("{where_} has a non-numeric default")),
        },
        Kind::Text => {
            if default.text.is_none() {
                errors.push(format!("{where_} has a non-text default"));
            }
        }
        Kind::Enum => match (&default.variant, &field.enum_name) {
            (Some(spelling), Some(target)) => {
                let known = enums
                    .get(target.as_str())
                    .is_some_and(|decl| decl.variants.iter().any(|variant| &variant.spelling == spelling));
                if !known {
                    errors.push(format!(
                        "{where_} defaults to `{spelling}`, which `{target}` does not accept"
                    ));
                }
            }
            (None, _) => errors.push(format!("{where_} has a non-enum default")),
            (Some(_), None) => {}
        },
        Kind::EnumList | Kind::TextList => {
            if default.list.is_none() {
                errors.push(format!("{where_} has a non-list default"));
            }
        }
        _ => {
            if !matches!(
                default,
                DefaultDecl {
                    bool: None,
                    integer: None,
                    float: None,
                    text: None,
                    variant: None,
                    list: None,
                }
            ) {
                errors.push(format!("{where_} is a {} and cannot have a scalar default", field.kind));
            }
        }
    }
}

/// Whether `text` is a full `x.y.z` release.
fn is_release(text: &str) -> bool {
    let parts: Vec<&str> = text.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
}

/// The Rust type of a field, before the `Option`/`Inherit` wrapper.
fn value_type(field: &FieldDecl) -> String {
    match field.kind() {
        Kind::Bool => "bool".to_owned(),
        Kind::Int => "i64".to_owned(),
        Kind::Float => "f64".to_owned(),
        Kind::Text => "String".to_owned(),
        Kind::Enum => field.enum_name.clone().unwrap_or_default(),
        Kind::EnumList => format!("Vec<{}>", field.enum_name.clone().unwrap_or_default()),
        Kind::TextList => "Vec<String>".to_owned(),
        Kind::FloatList => "Vec<f64>".to_owned(),
        Kind::Record => field.record.clone().unwrap_or_default(),
        Kind::RecordList => format!("Vec<{}>", field.record.clone().unwrap_or_default()),
        Kind::RecordMap => format!("BTreeMap<String, {}>", field.record.clone().unwrap_or_default()),
        Kind::EnumBoolMap => format!("BTreeMap<{}, bool>", field.enum_name.clone().unwrap_or_default()),
        Kind::TextListMap => "BTreeMap<String, Vec<String>>".to_owned(),
        Kind::LayoutSelector => "LayoutSelector".to_owned(),
    }
}

/// The `Option<...>` type a struct field has.
fn field_type(field: &FieldDecl) -> String {
    let inner = value_type(field);
    if field.inherit {
        format!("Option<Inherit<{inner}>>")
    } else {
        format!("Option<{inner}>")
    }
}

/// The engine call that reads a field.
fn read_call(field: &FieldDecl) -> String {
    let key = &field.key;
    let kind = field.kind();
    let base = |name: &str| {
        if field.inherit {
            format!("inherit_{name}")
        } else {
            name.to_owned()
        }
    };
    match kind {
        Kind::Bool => format!("{}(map, \"{key}\", path, out)", base("flag")),
        Kind::Int => {
            let (min, max) = field.int_bounds();
            format!(
                "{}(map, \"{key}\", path, out, {min}..={max}, \"{}\")",
                base("integer"),
                esc(&field.accepted)
            )
        }
        Kind::Float => format!(
            "{}(map, \"{key}\", path, out, {}..={}, \"{}\")",
            base("number"),
            float_literal(field.min.unwrap_or(f64::MIN)),
            float_literal(field.max.unwrap_or(f64::MAX)),
            esc(&field.accepted)
        ),
        Kind::Text => {
            if field.nonempty {
                format!("{}(map, \"{key}\", path, out)", base("nonempty_text"))
            } else {
                format!("{}(map, \"{key}\", path, out)", base("text"))
            }
        }
        Kind::Enum => format!("{}(map, \"{key}\", \"{key}\", path, out)", base("setting")),
        Kind::EnumList => format!("{}(map, \"{key}\", \"{key}\", path, out)", base("settings")),
        Kind::TextList if field.first_nonempty => format!("nonempty_argv(map, \"{key}\", path, out)"),
        Kind::TextList => format!("{}(map, \"{key}\", path, out)", base("strings")),
        Kind::FloatList => format!(
            "{}(map, \"{key}\", path, out, {}, {}, {}, {}, \"{}\")",
            base("floats"),
            float_literal(field.min.unwrap_or(f64::MIN)),
            float_literal(field.max.unwrap_or(f64::MAX)),
            field.exclusive_max,
            field
                .length
                .map_or("None".to_owned(), |length| format!("Some({length})")),
            esc(&field.accepted)
        ),
        Kind::Record => format!(
            "{}(map, \"{key}\", path, out, \"{}\", parse_{})",
            base("record"),
            esc(&field.accepted),
            snake(&field.record.clone().unwrap_or_default())
        ),
        Kind::RecordList => format!(
            "{}(map, \"{key}\", path, out, \"{}\", parse_{})",
            base("record_list"),
            esc(&field.accepted),
            snake(&field.record.clone().unwrap_or_default())
        ),
        Kind::RecordMap => format!(
            "record_map(map, \"{key}\", path, out, \"{}\", {}, parse_{})",
            esc(&field.accepted),
            IdShape::parse(field.id.as_deref().unwrap_or("any")).map_or("IdShape::Any", IdShape::rust),
            snake(&field.record.clone().unwrap_or_default())
        ),
        Kind::EnumBoolMap => "enum_bool_map(map, path, out)".to_owned(),
        Kind::TextListMap => "text_list_map(map, path, out, IdShape::Renderer)".to_owned(),
        Kind::LayoutSelector => format!(
            "layout_selector(map, \"{key}\", path, out, \"{}\")",
            esc(&field.accepted)
        ),
    }
}

/// A float as a Rust literal that round-trips.
fn float_literal(value: f64) -> String {
    if value == f64::MAX {
        "f64::MAX".to_owned()
    } else if value == f64::MIN {
        "f64::MIN".to_owned()
    } else if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{value:.1}")
    } else {
        format!("{value:?}")
    }
}

/// `SomeRecord` -> `some_record`.
fn snake(name: &str) -> String {
    let mut out = String::new();
    for (index, character) in name.chars().enumerate() {
        if character.is_ascii_uppercase() {
            if index > 0 {
                out.push('_');
            }
            out.push(character.to_ascii_lowercase());
        } else {
            out.push(character);
        }
    }
    out
}

/// Escape a declared string for a Rust string literal.
///
/// An accepted-values text may quote a spelling (`... or "inherit"`), which is
/// part of the message a person reads, so it has to survive being emitted into
/// source rather than being rewritten in the schema.
fn esc(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

/// A spelling as a Rust variant name.
fn variant_name(spelling: &str) -> String {
    let mut name = String::new();
    for part in spelling.split(['-', '_', '.']) {
        let mut characters = part.chars();
        if let Some(first) = characters.next() {
            name.push(first.to_ascii_uppercase());
            name.extend(characters);
        }
    }
    name
}

fn render_rust(schema: &Schema) -> String {
    let mut text = String::new();
    let _ = writeln!(
        text,
        "// Generated by gnoblin-config-codegen from crates/gnoblin-config/schema/config.toml."
    );
    let _ = writeln!(
        text,
        "// Compiled through include! by build.rs. Do not edit: change the schema."
    );
    let _ = writeln!(text);
    let _ = writeln!(text, "use std::collections::BTreeMap;");
    let _ = writeln!(text);
    let _ = writeln!(text, "use crate::engine::*;");
    let _ = writeln!(
        text,
        "use crate::model::{{Alias, Default, EnumSpec, Inherit, Kind, Setting, SettingEnum}};"
    );
    let _ = writeln!(text, "use crate::path::Path;");
    let _ = writeln!(text, "use crate::value::Value;");
    let _ = writeln!(text);
    text.push_str(LAYOUT_TYPES);

    for decl in &schema.r#enum {
        let _ = writeln!(text, "/// {}", decl.doc);
        let _ = writeln!(
            text,
            "#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]"
        );
        let _ = writeln!(text, "pub enum {} {{", decl.name);
        for variant in &decl.variants {
            let _ = writeln!(text, "    /// {}", variant.doc);
            let _ = writeln!(text, "    {},", variant_name(&variant.spelling));
        }
        let _ = writeln!(text, "}}");
        let _ = writeln!(text);
        let _ = writeln!(text, "impl {} {{", decl.name);
        let _ = writeln!(text, "    /// Every accepted spelling, in reference order.");
        let _ = write!(text, "    pub const VARIANT_TEXTS: &'static [&'static str] = &[");
        for (index, variant) in decl.variants.iter().enumerate() {
            if index > 0 {
                let _ = write!(text, ", ");
            }
            let _ = write!(text, "\"{}\"", variant.spelling);
        }
        let _ = writeln!(text, "];");
        let _ = writeln!(text);
        let _ = writeln!(text, "    /// This setting as a spec.");
        let _ = writeln!(
            text,
            "    pub const SPEC: EnumSpec = EnumSpec {{ name: \"{}\", setting: \"{}\", variants: Self::VARIANT_TEXTS }};",
            decl.name, decl.setting
        );
        let _ = writeln!(text);
        let _ = writeln!(text, "    /// The stored spelling of this variant.");
        let _ = writeln!(text, "    pub const fn as_str(self) -> &'static str {{");
        let _ = writeln!(text, "        match self {{");
        for variant in &decl.variants {
            let _ = writeln!(
                text,
                "            Self::{} => \"{}\",",
                variant_name(&variant.spelling),
                variant.spelling
            );
        }
        let _ = writeln!(text, "        }}");
        let _ = writeln!(text, "    }}");
        let _ = writeln!(text);
        let _ = writeln!(text, "    /// The variant a current spelling names.");
        let _ = writeln!(text, "    pub fn parse(text: &str) -> Option<Self> {{");
        let _ = writeln!(text, "        match text {{");
        for variant in &decl.variants {
            let _ = writeln!(
                text,
                "            \"{}\" => Some(Self::{}),",
                variant.spelling,
                variant_name(&variant.spelling)
            );
        }
        let _ = writeln!(text, "            _ => None,");
        let _ = writeln!(text, "        }}");
        let _ = writeln!(text, "    }}");
        let _ = writeln!(text, "}}");
        let _ = writeln!(text);
        let _ = writeln!(text, "impl SettingEnum for {} {{", decl.name);
        let _ = writeln!(text, "    fn variants() -> &'static [&'static str] {{");
        let _ = writeln!(text, "        Self::VARIANT_TEXTS");
        let _ = writeln!(text, "    }}");
        let _ = writeln!(text, "    fn from_text(text: &str) -> Option<Self> {{");
        let _ = writeln!(text, "        Self::parse(text)");
        let _ = writeln!(text, "    }}");
        let _ = writeln!(text, "    fn as_text(self) -> &'static str {{");
        let _ = writeln!(text, "        self.as_str()");
        let _ = writeln!(text, "    }}");
        let _ = writeln!(text, "    fn spec() -> EnumSpec {{");
        let _ = writeln!(text, "        Self::SPEC");
        let _ = writeln!(text, "    }}");
        let _ = writeln!(text, "}}");
        let _ = writeln!(text);
        let _ = writeln!(text, "impl std::fmt::Display for {} {{", decl.name);
        let _ = writeln!(
            text,
            "    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{"
        );
        let _ = writeln!(text, "        formatter.write_str(self.as_str())");
        let _ = writeln!(text, "    }}");
        let _ = writeln!(text, "}}");
        let _ = writeln!(text);
    }

    let _ = writeln!(text, "/// Deprecated spellings, accepted until they are removed.");
    let _ = writeln!(text, "pub const ALIASES: &[Alias] = &[");
    for alias in &schema.alias {
        let _ = writeln!(
            text,
            "    Alias {{ setting: \"{}\", spelling: \"{}\", canonical: \"{}\", removed_in: \"{}\" }},",
            alias.setting, alias.spelling, alias.canonical, alias.removed_in
        );
    }
    let _ = writeln!(text, "];");
    let _ = writeln!(text);
    let _ = writeln!(
        text,
        "/// Every enumerated setting, for the alias test and the reference."
    );
    let _ = writeln!(text, "pub const ENUM_SPECS: &[EnumSpec] = &[");
    for decl in &schema.r#enum {
        let _ = writeln!(text, "    {}::SPEC,", decl.name);
    }
    let _ = writeln!(text, "];");
    let _ = writeln!(text);

    let mut all_hooks: BTreeSet<String> = BTreeSet::new();
    for decl in schema.record.iter().chain(schema.section.iter()) {
        if let Some(hook) = &decl.hook {
            all_hooks.insert(hook.clone());
        }
    }
    if !all_hooks.is_empty() {
        let _ = writeln!(text, "use crate::hooks;");
        let _ = writeln!(text);
        let _ = writeln!(
            text,
            "/// The hooks the schema declares, checked against hooks.rs by test."
        );
        let _ = writeln!(text, "pub const DECLARED_HOOKS: &[&str] = &[");
        for hook in &all_hooks {
            let _ = writeln!(text, "    \"{hook}\",");
        }
        let _ = writeln!(text, "];");
        let _ = writeln!(text);
    }

    // Records first, so a section can name one.
    for decl in &schema.record {
        render_table(&mut text, decl, true);
    }
    for decl in schema.section.iter().filter(|decl| !decl.carried) {
        render_table(&mut text, decl, false);
    }

    let _ = writeln!(text, "/// Every top-level key the document accepts.");
    let _ = writeln!(text, "pub const TOP_LEVEL_KEYS: &[&str] = &[");
    let mut keys: Vec<&str> = schema.section.iter().map(|decl| decl.key.as_str()).collect();
    keys.sort_unstable();
    for key in keys {
        let _ = writeln!(text, "    \"{key}\",");
    }
    let _ = writeln!(text, "];");
    let _ = writeln!(text);

    let _ = writeln!(text, "/// Sections kept in the document but not interpreted here.");
    let _ = writeln!(text, "pub const CARRIED_SECTIONS: &[&str] = &[");
    for decl in schema.section.iter().filter(|decl| decl.carried) {
        let _ = writeln!(text, "    \"{}\",", decl.key);
    }
    let _ = writeln!(text, "];");
    let _ = writeln!(text);
    let _ = writeln!(
        text,
        "/// Every declared setting, for the reference, `gnoblinctl` and tests."
    );
    let _ = writeln!(text, "pub const SETTINGS: &[Setting] = &[");
    for decl in &schema.section {
        if let Some(spread) = &decl.spread {
            let _ = writeln!(
                text,
                "    Setting {{ section: \"{}\", key: \"*\", field: \"{}\", kind: {}, default: Default::None, accepted: \"{}\", since: \"0.2.0\", overridable: {}, inherit: false, required: false }},",
                decl.key,
                spread.field,
                kind_literal_spread(spread),
                esc(&spread.accepted),
                decl.overridable
            );
            continue;
        }
        for field in &decl.field {
            let _ = writeln!(
                text,
                "    Setting {{ section: \"{}\", key: \"{}\", field: \"{}\", kind: {}, default: {}, accepted: \"{}\", since: \"{}\", overridable: {}, inherit: {}, required: {} }},",
                decl.key,
                field.key,
                field.field_name(),
                kind_literal(field),
                default_literal(field),
                esc(&field.accepted),
                field.since,
                decl.overridable,
                field.inherit,
                field.required.is_some()
            );
        }
    }
    let _ = writeln!(text, "];");
    text
}

fn kind_literal(field: &FieldDecl) -> String {
    let (min, max) = field.int_bounds();
    match field.kind() {
        Kind::Bool => "Kind::Bool".to_owned(),
        Kind::Int => format!("Kind::Int {{ min: {min}, max: {max} }}"),
        Kind::Float => format!(
            "Kind::Float {{ min: {}, max: {}, exclusive_max: {} }}",
            float_literal(field.min.unwrap_or(f64::MIN)),
            float_literal(field.max.unwrap_or(f64::MAX)),
            field.exclusive_max
        ),
        Kind::Text => format!("Kind::Text {{ nonempty: {} }}", field.nonempty),
        Kind::Enum => format!("Kind::Enum(\"{}\")", field.enum_name.clone().unwrap_or_default()),
        Kind::EnumList => format!("Kind::EnumList(\"{}\")", field.enum_name.clone().unwrap_or_default()),
        Kind::TextList => "Kind::TextList".to_owned(),
        Kind::FloatList => format!(
            "Kind::FloatList {{ min: {}, max: {}, exclusive_max: {}, length: {} }}",
            float_literal(field.min.unwrap_or(f64::MIN)),
            float_literal(field.max.unwrap_or(f64::MAX)),
            field.exclusive_max,
            field
                .length
                .map_or("None".to_owned(), |length| format!("Some({length})"))
        ),
        Kind::Record => format!("Kind::Record(\"{}\")", field.record.clone().unwrap_or_default()),
        Kind::RecordList => format!("Kind::RecordList(\"{}\")", field.record.clone().unwrap_or_default()),
        Kind::RecordMap => format!("Kind::RecordMap(\"{}\")", field.record.clone().unwrap_or_default()),
        Kind::EnumBoolMap => format!("Kind::EnumBoolMap(\"{}\")", field.enum_name.clone().unwrap_or_default()),
        Kind::TextListMap => "Kind::TextListMap".to_owned(),
        Kind::LayoutSelector => "Kind::LayoutSelector".to_owned(),
    }
}

fn kind_literal_spread(spread: &SpreadDecl) -> String {
    match spread.kind() {
        Kind::EnumBoolMap => format!(
            "Kind::EnumBoolMap(\"{}\")",
            spread.variant_enum.clone().unwrap_or_default()
        ),
        Kind::TextListMap => "Kind::TextListMap".to_owned(),
        Kind::RecordList => format!("Kind::RecordList(\"{}\")", spread.record.clone().unwrap_or_default()),
        Kind::RecordMap => format!("Kind::RecordMap(\"{}\")", spread.record.clone().unwrap_or_default()),
        _ => "Kind::Bool".to_owned(),
    }
}

fn default_literal(field: &FieldDecl) -> String {
    let Some(default) = &field.default else {
        return "Default::None".to_owned();
    };
    if let Some(value) = default.bool {
        format!("Default::Bool({value})")
    } else if let Some(value) = default.integer {
        format!("Default::Integer({value})")
    } else if let Some(value) = default.float {
        format!("Default::Float({})", float_literal(value))
    } else if let Some(value) = &default.text {
        format!("Default::Text(\"{value}\")")
    } else if let Some(value) = &default.variant {
        format!("Default::Enum(\"{value}\")")
    } else if let Some(values) = &default.list {
        let mut text = String::from("Default::List(&[");
        for (index, value) in values.iter().enumerate() {
            if index > 0 {
                text.push_str(", ");
            }
            let _ = write!(text, "\"{value}\"");
        }
        text.push_str("])");
        text
    } else {
        "Default::None".to_owned()
    }
}

/// One record or section: its struct, its key list, and its parser.
fn render_table(text: &mut String, decl: &TableDecl, is_record: bool) {
    let name = decl.rust_name();
    let _ = writeln!(text, "/// {}", decl.doc);
    let _ = writeln!(text, "#[derive(Clone, Debug, Default, PartialEq)]");
    let _ = writeln!(text, "pub struct {name} {{");
    if let Some(spread) = &decl.spread {
        let inner = match spread.kind() {
            Kind::EnumBoolMap => format!("BTreeMap<{}, bool>", spread.variant_enum.clone().unwrap_or_default()),
            Kind::TextListMap => "BTreeMap<String, Vec<String>>".to_owned(),
            Kind::RecordList => format!("Vec<{}>", spread.record.clone().unwrap_or_default()),
            Kind::RecordMap => format!("BTreeMap<String, {}>", spread.record.clone().unwrap_or_default()),
            _ => "bool".to_owned(),
        };
        let _ = writeln!(text, "    /// {}", spread.doc);
        let _ = writeln!(text, "    pub {}: {inner},", spread.field);
    }
    for field in &decl.field {
        let _ = writeln!(text, "    /// {}", field.doc);
        let _ = writeln!(text, "    pub {}: {},", field.field_name(), field_type(field));
    }
    let _ = writeln!(text, "}}");
    let _ = writeln!(text);

    if decl.spread.is_none() {
        let _ = writeln!(text, "/// The keys `{name}` accepts.");
        let _ = write!(text, "const {}: &[&str] = &[", decl.keys_const());
        let mut keys: Vec<&str> = decl.field.iter().map(|field| field.key.as_str()).collect();
        keys.sort_unstable();
        for key in keys {
            let _ = write!(text, " \"{key}\",");
        }
        let _ = writeln!(text, " ];");
        let _ = writeln!(text);
    }

    let returns = if is_record {
        format!("Option<{name}>")
    } else {
        name.to_owned()
    };
    // A section whose value is a list of records is handed its own value rather
    // than a table of settings.
    let list_section = !is_record
        && decl
            .spread
            .as_ref()
            .is_some_and(|spread| spread.kind() == Kind::RecordList);
    let first = if list_section {
        "value: &mut Value"
    } else {
        "map: &mut BTreeMap<String, Value>"
    };
    let _ = writeln!(text, "/// Parse `{}` into a [`{name}`].", decl.rust_name());
    let _ = writeln!(
        text,
        "pub(crate) fn parse_{}({first}, path: &Path, out: &mut Parser) -> {returns} {{",
        snake(name)
    );
    // Required fields are read first so a missing one is reported once and the
    // entry is dropped rather than half-built.
    for field in decl.field.iter().filter(|field| field.required.is_some()) {
        let required = field.required.clone().unwrap_or_default();
        let _ = writeln!(
            text,
            "    let {} = if require(map, \"{}\", path, out, \"{required}\") {{ {} }} else {{ None }};",
            field.field_name(),
            field.key,
            read_call(field)
        );
    }
    let binding = if decl.hook.is_some() { "mut section" } else { "section" };
    let _ = writeln!(text, "    let {binding} = {name} {{");
    if let Some(spread) = &decl.spread {
        let call = match spread.kind() {
            Kind::EnumBoolMap => "enum_bool_map(map, path, out)".to_owned(),
            Kind::TextListMap => format!(
                "text_list_map(map, path, out, {})",
                IdShape::parse("renderer").map_or("IdShape::Any", IdShape::rust)
            ),
            Kind::RecordList => format!(
                "section_records(value, path, out, \"{}\", parse_{})",
                esc(&spread.accepted),
                snake(&spread.record.clone().unwrap_or_default())
            ),
            Kind::RecordMap => format!(
                "section_record_map(map, path, out, \"{}\", {}, parse_{})",
                esc(&spread.accepted),
                IdShape::parse(spread.id.as_deref().unwrap_or("any")).map_or("IdShape::Any", IdShape::rust),
                snake(&spread.record.clone().unwrap_or_default())
            ),
            _ => "None".to_owned(),
        };
        let _ = writeln!(text, "        {}: {call},", spread.field);
    }
    for field in &decl.field {
        let value = if field.required.is_some() {
            field.field_name()
        } else {
            read_call(field)
        };
        let _ = writeln!(text, "        {}: {value},", field.field_name());
    }
    let _ = writeln!(text, "    }};");
    if let Some(hook) = &decl.hook {
        if is_record {
            let _ = writeln!(text, "    if !hooks::{hook}(&mut section, map, path, out) {{");
            let _ = writeln!(text, "        return None;");
            let _ = writeln!(text, "    }}");
        } else {
            let looked_at = if list_section { "value" } else { "map" };
            let _ = writeln!(text, "    let _ = hooks::{hook}(&mut section, {looked_at}, path, out);");
        }
    }
    if decl.spread.is_none() {
        let _ = writeln!(text, "    unknown_keys(map, path, out, {});", decl.keys_const());
    }
    if is_record {
        // Every required field must have been present; a missing one was already
        // reported by `require`.
        let required: Vec<&FieldDecl> = decl.field.iter().filter(|field| field.required.is_some()).collect();
        if required.is_empty() {
            let _ = writeln!(text, "    Some(section)");
        } else {
            let mut condition = String::new();
            for (index, field) in required.iter().enumerate() {
                if index > 0 {
                    condition.push_str(" && ");
                }
                let _ = write!(condition, "section.{}.is_some()", field.field_name());
            }
            let _ = writeln!(text, "    if {condition} {{ Some(section) }} else {{ None }}");
        }
    } else {
        let _ = writeln!(text, "    section");
    }
    let _ = writeln!(text, "}}");
    let _ = writeln!(text);
}

fn render_docs(schema: &Schema) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "Generated by `cargo xtask gen-config`; not edited by hand.");
    let _ = writeln!(text);
    for decl in &schema.section {
        let _ = writeln!(text, "## `{}`", decl.key);
        let _ = writeln!(text);
        let _ = writeln!(text, "{}", decl.doc);
        let _ = writeln!(text);
        let _ = writeln!(text, "| Key | Type | Default | Accepted | Since | Overridable |");
        let _ = writeln!(text, "| --- | --- | --- | --- | --- | --- |");
        if let Some(spread) = &decl.spread {
            let _ = writeln!(
                text,
                "| every key below | {} | none | {} | 0.2.0 | {} |",
                spread.kind, spread.accepted, decl.overridable
            );
            if spread.kind() == Kind::EnumBoolMap {
                for decl_enum in &schema.r#enum {
                    if Some(&decl_enum.name) == spread.variant_enum.as_ref() {
                        let _ = writeln!(
                            text,
                            "| {} | boolean | true | a boolean | 0.2.0 | {} |",
                            decl_enum
                                .variants
                                .iter()
                                .map(|variant| format!("`{}`", variant.spelling))
                                .collect::<Vec<String>>()
                                .join(", "),
                            decl.overridable
                        );
                    }
                }
            }
            continue;
        }
        for field in &decl.field {
            let _ = writeln!(
                text,
                "| `{}` | {} | {} | {} | {} | {} |",
                field.key,
                field.kind,
                default_text(field),
                field.accepted,
                field.since,
                if decl.overridable { "yes" } else { "no" }
            );
            let _ = field;
        }
        let _ = writeln!(text);
    }
    text
}

fn default_text(field: &FieldDecl) -> String {
    let Some(default) = &field.default else {
        return "none".to_owned();
    };
    if let Some(value) = default.bool {
        format!("`{value}`")
    } else if let Some(value) = default.integer {
        format!("`{value}`")
    } else if let Some(value) = default.float {
        format!("`{value}`")
    } else if let Some(value) = &default.text {
        format!("`\"{value}\"`")
    } else if let Some(value) = &default.variant {
        format!("`\"{value}\"`")
    } else if let Some(values) = &default.list {
        format!(
            "`{{{}}}`",
            values
                .iter()
                .map(|value| format!("\"{value}\""))
                .collect::<Vec<String>>()
                .join(", ")
        )
    } else {
        "none".to_owned()
    }
}

fn render_lua(schema: &Schema) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "--- Generated by `cargo xtask gen-config`; not edited by hand.");
    let _ = writeln!(text, "--- @class gnoblin.Configure");
    for decl in &schema.section {
        let _ = writeln!(
            text,
            "--- @field {}? gnoblin.configure.{} {}",
            decl.key, decl.key, decl.doc
        );
    }
    for decl in &schema.section {
        let _ = writeln!(text, "---");
        let _ = writeln!(text, "--- {}", decl.doc);
        let _ = writeln!(text, "--- @class gnoblin.configure.{}", decl.key);
        if let Some(spread) = &decl.spread {
            let _ = writeln!(text, "--- @field [string] {} {}", spread.kind, spread.doc);
            continue;
        }
        for field in &decl.field {
            let _ = writeln!(
                text,
                "--- @field {} {}? {}",
                field.lua_name(),
                lua_type(field),
                field.doc
            );
        }
    }
    text
}

fn lua_type(field: &FieldDecl) -> String {
    let base = match field.kind() {
        Kind::Bool => "boolean".to_owned(),
        Kind::Int | Kind::Float => "number".to_owned(),
        Kind::Text => "string".to_owned(),
        Kind::Enum => field.enum_name.clone().unwrap_or_default(),
        Kind::EnumList | Kind::TextList | Kind::FloatList => "table".to_owned(),
        Kind::Record => field.record.clone().unwrap_or_default(),
        Kind::RecordList | Kind::RecordMap => "table".to_owned(),
        Kind::EnumBoolMap => "table".to_owned(),
        Kind::TextListMap => "table".to_owned(),
        Kind::LayoutSelector => "string | table".to_owned(),
    };
    if field.inherit {
        format!("{base} | \"inherit\"")
    } else {
        base
    }
}

fn render_settings_json(schema: &Schema) -> String {
    let mut entries: Vec<serde_json::Value> = Vec::new();
    for decl in &schema.section {
        if let Some(spread) = &decl.spread {
            entries.push(serde_json::json!({
                "section": decl.key,
                "key": "*",
                "field": spread.field,
                "kind": spread.kind,
                "accepted": spread.accepted,
                "overridable": decl.overridable,
                "default": serde_json::Value::Null,
                "id": spread.id,
            }));
            continue;
        }
        for field in &decl.field {
            entries.push(serde_json::json!({
                "section": decl.key,
                "key": field.key,
                "field": field.field_name(),
                "kind": field.kind,
                "default": default_json(field),
                "accepted": field.accepted,
                "since": field.since,
                "overridable": decl.overridable,
                "inherit": field.inherit,
                "required": field.required.is_some(),
                "first_nonempty": field.first_nonempty,
            }));
        }
    }
    serde_json::to_string_pretty(&serde_json::json!({
        "format": 1,
        "generated_by": "cargo xtask gen-config",
        "settings": entries,
        "aliases": schema.alias.iter().map(|alias| serde_json::json!({
            "setting": alias.setting,
            "spelling": alias.spelling,
            "canonical": alias.canonical,
            "removed_in": alias.removed_in,
        })).collect::<Vec<serde_json::Value>>(),
    }))
    .unwrap_or_else(|error| format!("{{\"error\": \"{error}\"}}"))
}

fn default_json(field: &FieldDecl) -> serde_json::Value {
    let Some(default) = &field.default else {
        return serde_json::Value::Null;
    };
    if let Some(value) = default.bool {
        serde_json::Value::Bool(value)
    } else if let Some(value) = default.integer {
        serde_json::Value::from(value)
    } else if let Some(value) = default.float {
        serde_json::json!(value)
    } else if let Some(value) = &default.text {
        serde_json::Value::String(value.clone())
    } else if let Some(value) = &default.variant {
        serde_json::Value::String(value.clone())
    } else if let Some(values) = &default.list {
        serde_json::Value::Array(
            values
                .iter()
                .map(|value| serde_json::Value::String(value.clone()))
                .collect(),
        )
    } else {
        serde_json::Value::Null
    }
}
