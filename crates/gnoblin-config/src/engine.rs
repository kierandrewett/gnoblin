//! The validation engine: reading a setting out of a table, and the diagnostic
//! for one that is wrong.
//!
//! Every setting is read the same way: look the key up, check its shape, check
//! its value, and report a [`Problem`] at the setting's path when either check
//! fails. The helpers exist so that `input.touchpad.tap-button-map` and
//! `window-management.focus-mode` cannot disagree about what a wrong type or a
//! deprecated spelling does: there is one implementation of each.
//!
//! Reading rewrites a deprecated spelling to its current one in the tree it is
//! given, so the document that survives validation is already canonical and the
//! stored last-good file never holds a spelling that a later release refuses.

use std::collections::BTreeMap;
use std::ops::RangeInclusive;

use crate::generated::{LayoutOptionValue, LayoutReference, LayoutRoot, LayoutSelector};
use crate::model::{self, EnumParse, Inherit, SettingEnum};
use crate::path::Path;
use crate::problem::{Fault, Problem};
use crate::value::Value;

/// What one validation pass has found so far.
///
/// Problems and deprecations are collected, never raised: a validator that
/// stopped at the first fault would turn a three-typo config into three reloads.
pub(crate) struct Parser {
    problems: Vec<Problem>,
    deprecations: Vec<model::Deprecation>,
}

impl Parser {
    /// An empty parser.
    pub(crate) fn new() -> Parser {
        Parser {
            problems: Vec::new(),
            deprecations: Vec::new(),
        }
    }

    /// Record a problem.
    pub(crate) fn problem(&mut self, path: &Path, fault: Fault) {
        self.problems.push(Problem::new(path.clone(), fault));
    }

    /// Record that a deprecated spelling was accepted and rewritten.
    pub(crate) fn deprecation(&mut self, path: &Path, spelling: &'static str, canonical: &'static str) {
        self.deprecations.push(model::Deprecation {
            path: path.clone(),
            spelling,
            canonical,
        });
    }

    /// Whether a problem has already been reported at this exact path.
    ///
    /// A key that is refused with an explanation (a removed setting) must not
    /// also be reported as a spelling mistake: one mistake earns one problem, and
    /// the explanation is the more useful of the two.
    pub(crate) fn reported(&self, path: &Path) -> bool {
        self.problems.iter().any(|problem| &problem.path == path)
    }

    /// The problems and deprecations found so far.
    pub(crate) fn finish(self) -> (Vec<Problem>, Vec<model::Deprecation>) {
        (self.problems, self.deprecations)
    }
}

/// Whether a value is the `"inherit"` marker.
pub(crate) fn is_inherit(value: &Value) -> bool {
    matches!(value, Value::String(text) if text == "inherit")
}

/// Report every key of `map` that `accepted` does not name.
///
/// A key that is already reported is skipped, so a removed setting is reported
/// once, with its explanation, instead of twice.
pub(crate) fn unknown_keys(
    map: &BTreeMap<String, Value>,
    path: &Path,
    out: &mut Parser,
    accepted: &'static [&'static str],
) {
    for key in map.keys() {
        let key_path = path.key(key);
        if accepted.contains(&key.as_str()) || out.reported(&key_path) {
            continue;
        }
        // The path of the key itself, not of the table: a reader needs to know
        // which key is wrong, not which table.
        out.problem(&key_path, Fault::UnknownKey { accepted });
    }
}

/// Report a key that used to exist and is now refused with an explanation.
pub(crate) fn removed(map: &BTreeMap<String, Value>, key: &str, path: &Path, out: &mut Parser, detail: &str) {
    if map.contains_key(key) {
        out.problem(
            &path.key(key),
            Fault::Removed {
                detail: detail.to_owned(),
            },
        );
    }
}

/// Whether `key` is present, reporting it as required when it is not.
///
/// The readers return `None` for an absent key without a problem, because most
/// settings are optional. A required setting is asked for first, so a missing one
/// is reported once instead of silently dropping the entry that needed it.
pub(crate) fn require(
    map: &BTreeMap<String, Value>,
    key: &str,
    path: &Path,
    out: &mut Parser,
    expected: &'static str,
) -> bool {
    if map.contains_key(key) {
        return true;
    }
    out.problem(&path.key(key), Fault::Missing { expected });
    false
}

/// The table at `key`, or a problem when it is not a table.
pub(crate) fn table<'m>(
    map: &'m mut BTreeMap<String, Value>,
    key: &str,
    path: &Path,
    out: &mut Parser,
    expected: &'static str,
) -> Option<&'m mut BTreeMap<String, Value>> {
    as_table(map.get_mut(key)?, &path.key(key), out, expected)
}

/// The value at `value` as a table, or a problem when it is not one.
pub(crate) fn as_table<'m>(
    value: &'m mut Value,
    path: &Path,
    out: &mut Parser,
    expected: &'static str,
) -> Option<&'m mut BTreeMap<String, Value>> {
    if let Value::Map(entries) = value {
        return Some(entries);
    }
    out.problem(
        path,
        Fault::WrongType {
            expected,
            found: value.kind(),
        },
    );
    None
}

/// The list at `key`, or a problem when it is not a list.
pub(crate) fn list<'m>(
    map: &'m mut BTreeMap<String, Value>,
    key: &str,
    path: &Path,
    out: &mut Parser,
    expected: &'static str,
) -> Option<&'m mut Vec<Value>> {
    match map.get_mut(key)? {
        Value::List(items) => Some(items),
        other => {
            out.problem(
                &path.key(key),
                Fault::WrongType {
                    expected,
                    found: other.kind(),
                },
            );
            None
        }
    }
}

/// The boolean at `key`.
pub(crate) fn flag(map: &mut BTreeMap<String, Value>, key: &str, path: &Path, out: &mut Parser) -> Option<bool> {
    flag_value(map.get_mut(key)?, &path.key(key), out, "a boolean")
}

/// The boolean at `value`.
pub(crate) fn flag_value(value: &Value, path: &Path, out: &mut Parser, expected: &'static str) -> Option<bool> {
    if let Value::Bool(flag) = value {
        return Some(*flag);
    }
    out.problem(
        path,
        Fault::WrongType {
            expected,
            found: value.kind(),
        },
    );
    None
}

/// The string at `key`, with an optional non-empty requirement.
pub(crate) fn text(map: &mut BTreeMap<String, Value>, key: &str, path: &Path, out: &mut Parser) -> Option<String> {
    let value = map.get(key)?;
    if let Value::String(written) = value {
        return Some(written.clone());
    }
    out.problem(
        &path.key(key),
        Fault::WrongType {
            expected: "a string",
            found: value.kind(),
        },
    );
    None
}

/// The string at `key`, which must not be empty.
pub(crate) fn nonempty_text(
    map: &mut BTreeMap<String, Value>,
    key: &str,
    path: &Path,
    out: &mut Parser,
) -> Option<String> {
    let written = text(map, key, path, out)?;
    if written.is_empty() {
        out.problem(
            &path.key(key),
            Fault::UnsupportedValue {
                expected: "a nonempty string".to_owned(),
            },
        );
        return None;
    }
    Some(written)
}

/// The integer at `key`, in `range`, converted to `T`.
pub(crate) fn integer<T: TryFrom<i64>>(
    map: &mut BTreeMap<String, Value>,
    key: &str,
    path: &Path,
    out: &mut Parser,
    range: RangeInclusive<i64>,
    expected: &str,
) -> Option<T> {
    let value = map.get(key)?;
    let Some(number) = value.as_integer() else {
        out.problem(
            &path.key(key),
            Fault::WrongType {
                expected: "an integer",
                found: value.kind(),
            },
        );
        return None;
    };
    if !range.contains(&number) {
        out.problem(
            &path.key(key),
            Fault::UnsupportedValue {
                expected: expected.to_owned(),
            },
        );
        return None;
    }
    match T::try_from(number) {
        Ok(converted) => Some(converted),
        Err(_) => {
            out.problem(
                &path.key(key),
                Fault::UnsupportedValue {
                    expected: expected.to_owned(),
                },
            );
            None
        }
    }
}

/// The number at `key`, in `range`, as an `f64`.
///
/// An integer is accepted wherever a number is: JSON and Lua both make the
/// distinction easy to blur, and `speed = 0` means the same as `speed = 0.0`.
pub(crate) fn number(
    map: &mut BTreeMap<String, Value>,
    key: &str,
    path: &Path,
    out: &mut Parser,
    range: RangeInclusive<f64>,
    expected: &str,
) -> Option<f64> {
    let value = map.get(key)?;
    if let Value::Float(number) = value
        && !number.is_finite()
    {
        out.problem(&path.key(key), Fault::NotFinite);
        return None;
    }
    let Some(number) = value.as_number() else {
        out.problem(
            &path.key(key),
            Fault::WrongType {
                expected: "a number",
                found: value.kind(),
            },
        );
        return None;
    };
    if !range.contains(&number) {
        out.problem(
            &path.key(key),
            Fault::UnsupportedValue {
                expected: expected.to_owned(),
            },
        );
        return None;
    }
    Some(number)
}

/// The enumerated value at `key`, resolving a deprecated spelling in place.
pub(crate) fn setting<T: SettingEnum>(
    map: &mut BTreeMap<String, Value>,
    key: &str,
    setting: &str,
    path: &Path,
    out: &mut Parser,
) -> Option<T> {
    let slot = map.get_mut(key)?;
    let written = match &*slot {
        Value::String(text) => text.clone(),
        other => {
            out.problem(
                &path.key(key),
                Fault::WrongType {
                    expected: "a name",
                    found: other.kind(),
                },
            );
            return None;
        }
    };
    match model::parse::<T>(setting, &written) {
        EnumParse::Current(value) => Some(value),
        EnumParse::Deprecated { value, spelling } => {
            *slot = Value::string(value.as_text());
            out.deprecation(&path.key(key), spelling, value.as_text());
            Some(value)
        }
        EnumParse::Unsupported => {
            out.problem(
                &path.key(key),
                Fault::UnsupportedValue {
                    expected: T::spec().expected(),
                },
            );
            None
        }
    }
}

/// The list of strings at `key`.
pub(crate) fn strings(
    map: &mut BTreeMap<String, Value>,
    key: &str,
    path: &Path,
    out: &mut Parser,
) -> Option<Vec<String>> {
    let entry_path = path.key(key);
    let items = list(map, key, path, out, "a list of strings")?;
    let mut written: Vec<String> = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        match item {
            Value::String(text) => written.push(text.clone()),
            other => out.problem(
                &entry_path.entry(index),
                Fault::WrongType {
                    expected: "a string",
                    found: other.kind(),
                },
            ),
        }
    }
    Some(written)
}

/// The argv at `key`: a list of strings whose first entry names the program.
///
/// An empty argv has nothing to run, and an empty first entry names no program,
/// so both are refused here rather than at spawn time.
pub(crate) fn nonempty_argv(
    map: &mut BTreeMap<String, Value>,
    key: &str,
    path: &Path,
    out: &mut Parser,
) -> Option<Vec<String>> {
    let words = strings(map, key, path, out)?;
    if words.first().is_none_or(String::is_empty) {
        out.problem(
            &path.key(key),
            Fault::UnsupportedValue {
                expected: "a nonempty argv whose first entry names the program".to_owned(),
            },
        );
        return None;
    }
    Some(words)
}

/// The list of enumerated values at `key`.
pub(crate) fn settings<T: SettingEnum>(
    map: &mut BTreeMap<String, Value>,
    key: &str,
    setting: &str,
    path: &Path,
    out: &mut Parser,
) -> Option<Vec<T>> {
    let entry_path = path.key(key);
    let items = list(map, key, path, out, "a list of names")?;
    let mut values: Vec<T> = Vec::with_capacity(items.len());
    for (index, item) in items.iter_mut().enumerate() {
        let written = match &*item {
            Value::String(text) => text.clone(),
            other => {
                out.problem(
                    &entry_path.entry(index),
                    Fault::WrongType {
                        expected: "a name",
                        found: other.kind(),
                    },
                );
                continue;
            }
        };
        match model::parse::<T>(setting, &written) {
            EnumParse::Current(value) => values.push(value),
            EnumParse::Deprecated { value, spelling } => {
                *item = Value::string(value.as_text());
                out.deprecation(&entry_path.entry(index), spelling, value.as_text());
                values.push(value);
            }
            EnumParse::Unsupported => out.problem(
                &entry_path.entry(index),
                Fault::UnsupportedValue {
                    expected: T::spec().expected(),
                },
            ),
        }
    }
    Some(values)
}

/// The boolean at `key`, or `"inherit"`.
pub(crate) fn inherit_flag(
    map: &mut BTreeMap<String, Value>,
    key: &str,
    path: &Path,
    out: &mut Parser,
) -> Option<Inherit<bool>> {
    let value = map.get(key)?;
    if is_inherit(value) {
        return Some(Inherit::Inherit);
    }
    flag_value(value, &path.key(key), out, "a boolean or \"inherit\"").map(Inherit::Value)
}

/// The number at `key`, or `"inherit"`.
pub(crate) fn inherit_number(
    map: &mut BTreeMap<String, Value>,
    key: &str,
    path: &Path,
    out: &mut Parser,
    range: RangeInclusive<f64>,
    expected: &str,
) -> Option<Inherit<f64>> {
    if is_inherit(map.get(key)?) {
        return Some(Inherit::Inherit);
    }
    number(map, key, path, out, range, expected).map(Inherit::Value)
}

/// The integer at `key`, or `"inherit"`.
pub(crate) fn inherit_integer<T: TryFrom<i64>>(
    map: &mut BTreeMap<String, Value>,
    key: &str,
    path: &Path,
    out: &mut Parser,
    range: RangeInclusive<i64>,
    expected: &str,
) -> Option<Inherit<T>> {
    if is_inherit(map.get(key)?) {
        return Some(Inherit::Inherit);
    }
    integer::<T>(map, key, path, out, range, expected).map(Inherit::Value)
}

/// The enumerated value at `key`, or `"inherit"`.
pub(crate) fn inherit_setting<T: SettingEnum>(
    map: &mut BTreeMap<String, Value>,
    key: &str,
    setting_key: &str,
    path: &Path,
    out: &mut Parser,
) -> Option<Inherit<T>> {
    if is_inherit(map.get(key)?) {
        return Some(Inherit::Inherit);
    }
    setting::<T>(map, key, setting_key, path, out).map(Inherit::Value)
}

/// The shape a record map's keys must have.
///
/// The ids are device identifiers and renderer names, which the compositor looks
/// up, so an id that cannot be looked up is reported here rather than becoming a
/// setting that silently never applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdShape {
    /// Any key: the record checks its own content.
    Any,
    /// `vvvv:pppp`, four hex digits a side.
    HexPair,
    /// A hex serial, or `default-vvvv:pppp`.
    Serial,
    /// A frame renderer name: 1 to 64 letters, digits, `_` or `-`, not `native`.
    Renderer,
    /// A name of 1 to 64 letters, digits, `_` or `-`, such as a shortcut's name.
    Name,
}

impl IdShape {
    /// Whether `id` has this shape.
    pub fn accepts(self, id: &str) -> bool {
        match self {
            IdShape::Any => true,
            IdShape::HexPair => hex_pair(id),
            IdShape::Serial => hex_pair(id.strip_prefix("default-").unwrap_or(id)) && !id.is_empty(),
            IdShape::Renderer => {
                let mut characters = id.chars();
                characters.next().is_some_and(|first| first.is_ascii_alphanumeric())
                    && id.len() <= 64
                    && id != "native"
                    && characters
                        .all(|character| character.is_ascii_alphanumeric() || character == '_' || character == '-')
            }
            IdShape::Name => id.len() <= 64 && !id.is_empty() && id.chars().all(name_character),
        }
    }

    /// The accepted-values text for a key of this shape.
    pub fn expected(self) -> &'static str {
        match self {
            IdShape::Any => "a device id",
            IdShape::HexPair => "a device id of four hex digits, a colon, and four hex digits",
            IdShape::Serial => "a hex serial, or default- followed by four hex digits, a colon, and four hex digits",
            IdShape::Renderer => "a name of at most 64 letters, digits, underscores or dashes, other than native",
            IdShape::Name => "a name of 1 to 64 letters, digits, underscores or dashes",
        }
    }
}

/// Whether `character` may appear in a name: a letter, a digit, `_` or `-`.
fn name_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_' || character == '-'
}

/// Whether `text` is `vvvv:pppp` in hex.
fn hex_pair(text: &str) -> bool {
    match text.split_once(':') {
        Some((vendor, product)) => hex_shaped(vendor, 4) && hex_shaped(product, 4),
        None => false,
    }
}

/// Whether `text` is exactly `length` hex digits.
fn hex_shaped(text: &str, length: usize) -> bool {
    text.len() == length && text.chars().all(|character| character.is_ascii_hexdigit())
}

/// The record at `key`, or a problem when it is not a table.
pub(crate) fn record<'m, T>(
    map: &'m mut BTreeMap<String, Value>,
    key: &str,
    path: &Path,
    out: &mut Parser,
    expected: &'static str,
    parse: fn(&mut BTreeMap<String, Value>, &Path, &mut Parser) -> Option<T>,
) -> Option<T> {
    let entry_path = path.key(key);
    let fields = as_table(map.get_mut(key)?, &entry_path, out, expected)?;
    parse(fields, &entry_path, out)
}

/// The record at `key`, or `"inherit"`.
pub(crate) fn inherit_record<T>(
    map: &mut BTreeMap<String, Value>,
    key: &str,
    path: &Path,
    out: &mut Parser,
    expected: &'static str,
    parse: fn(&mut BTreeMap<String, Value>, &Path, &mut Parser) -> Option<T>,
) -> Option<Inherit<T>> {
    if is_inherit(map.get(key)?) {
        return Some(Inherit::Inherit);
    }
    record(map, key, path, out, expected, parse).map(Inherit::Value)
}

/// The list of records at `key`.
pub(crate) fn record_list<T>(
    map: &mut BTreeMap<String, Value>,
    key: &str,
    path: &Path,
    out: &mut Parser,
    expected: &'static str,
    parse: fn(&mut BTreeMap<String, Value>, &Path, &mut Parser) -> Option<T>,
) -> Option<Vec<T>> {
    let list_path = path.key(key);
    let items = list(map, key, path, out, expected)?;
    Some(parse_records(items.as_mut_slice(), &list_path, out, parse))
}

/// The list of records at `key`, or `"inherit"`.
pub(crate) fn inherit_record_list<T>(
    map: &mut BTreeMap<String, Value>,
    key: &str,
    path: &Path,
    out: &mut Parser,
    expected: &'static str,
    parse: fn(&mut BTreeMap<String, Value>, &Path, &mut Parser) -> Option<T>,
) -> Option<Inherit<Vec<T>>> {
    if is_inherit(map.get(key)?) {
        return Some(Inherit::Inherit);
    }
    record_list(map, key, path, out, expected, parse).map(Inherit::Value)
}

/// The list a section is, for a section whose value is a list of records rather
/// than a table of settings.
pub(crate) fn section_records<T>(
    value: &mut Value,
    path: &Path,
    out: &mut Parser,
    expected: &'static str,
    parse: fn(&mut BTreeMap<String, Value>, &Path, &mut Parser) -> Option<T>,
) -> Vec<T> {
    let Some(items) = list_value(value, path, out, expected) else {
        return Vec::new();
    };
    parse_records(items.as_mut_slice(), path, out, parse)
}

/// The list at `value`, or a problem when it is not a list.
pub(crate) fn list_value<'m>(
    value: &'m mut Value,
    path: &Path,
    out: &mut Parser,
    expected: &'static str,
) -> Option<&'m mut Vec<Value>> {
    if let Value::List(items) = value {
        return Some(items);
    }
    out.problem(
        path,
        Fault::WrongType {
            expected,
            found: value.kind(),
        },
    );
    None
}

/// Parse each entry of a list, dropping the ones that were refused.
fn parse_records<T>(
    items: &mut [Value],
    list_path: &Path,
    out: &mut Parser,
    parse: fn(&mut BTreeMap<String, Value>, &Path, &mut Parser) -> Option<T>,
) -> Vec<T> {
    let mut records: Vec<T> = Vec::with_capacity(items.len());
    for (index, item) in items.iter_mut().enumerate() {
        let entry_path = list_path.entry(index);
        let Some(fields) = as_table(item, &entry_path, out, "a table") else {
            continue;
        };
        if let Some(record) = parse(fields, &entry_path, out) {
            records.push(record);
        }
    }
    records
}

/// The layout a `layout-selector` setting names: a text name, or a table naming a
/// module by root, path, content hash and plain options. Each problem is recorded at
/// the leaf it concerns, so the warning points at the field that is wrong. An absent
/// key is not a problem, because the setting is optional.
pub(crate) fn layout_selector(
    map: &mut BTreeMap<String, Value>,
    key: &str,
    path: &Path,
    out: &mut Parser,
    expected: &'static str,
) -> Option<LayoutSelector> {
    let leaf = path.key(key);
    let value = map.get_mut(key)?;
    let found = value.kind();
    match value {
        Value::String(name) if !name.is_empty() => Some(LayoutSelector::Named(name.clone())),
        Value::String(_) => {
            out.problem(
                &leaf,
                Fault::UnsupportedValue {
                    expected: expected.to_owned(),
                },
            );
            None
        }
        Value::Map(fields) => layout_reference(fields, &leaf, out).map(LayoutSelector::Reference),
        _ => {
            out.problem(&leaf, Fault::WrongType { expected, found });
            None
        }
    }
}

/// The fields of a module reference: `root`, `module` and `hash` are required and
/// `options` is optional. An unknown field is refused rather than ignored, so a typo
/// cannot silently change which layout runs.
fn layout_reference(fields: &mut BTreeMap<String, Value>, path: &Path, out: &mut Parser) -> Option<LayoutReference> {
    const KEYS: &[&str] = &["root", "module", "hash", "options"];
    let mut complete = true;
    for key in fields.keys() {
        if !KEYS.contains(&key.as_str()) {
            out.problem(&path.key(key), Fault::UnknownKey { accepted: KEYS });
            complete = false;
        }
    }
    let required = [
        ("root", "\"shipped\" or \"user\""),
        ("module", "a dotted module path"),
        ("hash", "a hex content hash"),
    ];
    for (key, expected) in required {
        if !require(fields, key, path, out, expected) {
            complete = false;
        }
    }
    if !complete {
        return None;
    }

    let root = match text(fields, "root", path, out)?.as_str() {
        "shipped" => LayoutRoot::Shipped,
        "user" => LayoutRoot::User,
        _ => {
            out.problem(
                &path.key("root"),
                Fault::UnsupportedValue {
                    expected: "\"shipped\" or \"user\"".to_owned(),
                },
            );
            return None;
        }
    };
    let module = text(fields, "module", path, out)?;
    if !is_module_path(&module) {
        out.problem(
            &path.key("module"),
            Fault::UnsupportedValue {
                expected: "a dotted module path of identifiers".to_owned(),
            },
        );
        return None;
    }
    let hash = text(fields, "hash", path, out)?;
    if hash.is_empty() || !hash.chars().all(|character| character.is_ascii_hexdigit()) {
        out.problem(
            &path.key("hash"),
            Fault::UnsupportedValue {
                expected: "a hex content hash".to_owned(),
            },
        );
        return None;
    }
    let options = match fields.get_mut("options") {
        None => BTreeMap::new(),
        Some(value) => layout_options(value, &path.key("options"), out)?,
    };
    Some(LayoutReference {
        root,
        module,
        hash,
        options,
    })
}

/// The options table of a module reference. Every value must be a plain boolean,
/// finite number or text: a table or list cannot be passed to a layout as an option.
fn layout_options(value: &mut Value, path: &Path, out: &mut Parser) -> Option<BTreeMap<String, LayoutOptionValue>> {
    let found = value.kind();
    let Value::Map(entries) = value else {
        out.problem(
            path,
            Fault::WrongType {
                expected: "a table of plain values",
                found,
            },
        );
        return None;
    };
    let mut options = BTreeMap::new();
    let mut complete = true;
    for (name, item) in entries.iter() {
        let option = match item {
            Value::Bool(flag) => LayoutOptionValue::Bool(*flag),
            Value::Integer(number) => LayoutOptionValue::Integer(*number),
            Value::Float(number) if number.is_finite() => LayoutOptionValue::Float(*number),
            Value::String(written) => LayoutOptionValue::Text(written.clone()),
            _ => {
                out.problem(
                    &path.key(name),
                    Fault::UnsupportedValue {
                        expected: "a plain value: a boolean, finite number or text".to_owned(),
                    },
                );
                complete = false;
                continue;
            }
        };
        options.insert(name.clone(), option);
    }
    complete.then_some(options)
}

/// Whether `path` is a dotted module path whose segments are identifiers.
fn is_module_path(path: &str) -> bool {
    !path.is_empty()
        && path.split('.').all(|segment| {
            !segment.is_empty()
                && segment
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_')
        })
}

/// The table at `key`, whose keys are ids and whose values are records.
pub(crate) fn record_map<T>(
    map: &mut BTreeMap<String, Value>,
    key: &str,
    path: &Path,
    out: &mut Parser,
    expected: &'static str,
    id_shape: IdShape,
    parse: fn(&mut BTreeMap<String, Value>, &Path, &mut Parser) -> Option<T>,
) -> Option<BTreeMap<String, T>> {
    let entries = table(map, key, path, out, expected)?;
    let group_path = path.key(key);
    Some(record_map_entries(entries, &group_path, out, expected, id_shape, parse))
}

/// A section whose own keys are ids and whose values are records. The section
/// is the table, so there is no key to look up first.
pub(crate) fn section_record_map<T>(
    map: &mut BTreeMap<String, Value>,
    path: &Path,
    out: &mut Parser,
    expected: &'static str,
    id_shape: IdShape,
    parse: fn(&mut BTreeMap<String, Value>, &Path, &mut Parser) -> Option<T>,
) -> BTreeMap<String, T> {
    record_map_entries(map, path, out, expected, id_shape, parse)
}

/// Parses each entry of `entries` as a record, reporting and dropping any entry
/// whose id has the wrong shape or whose value is not a table.
fn record_map_entries<T>(
    entries: &mut BTreeMap<String, Value>,
    group_path: &Path,
    out: &mut Parser,
    expected: &'static str,
    id_shape: IdShape,
    parse: fn(&mut BTreeMap<String, Value>, &Path, &mut Parser) -> Option<T>,
) -> BTreeMap<String, T> {
    let mut records: BTreeMap<String, T> = BTreeMap::new();
    let ids: Vec<String> = entries.keys().cloned().collect();
    for id in ids {
        let device_path = group_path.key(&id);
        if !id_shape.accepts(&id) {
            out.problem(
                &device_path,
                Fault::UnsupportedValue {
                    expected: id_shape.expected().to_owned(),
                },
            );
            continue;
        }
        let Some(fields) = table(&mut *entries, &id, group_path, out, expected) else {
            continue;
        };
        if let Some(record) = parse(fields, &device_path, out) {
            records.insert(id, record);
        }
    }
    records
}

/// The table whose keys are names from an enumerated setting and whose values are
/// booleans, for a section that is itself that table.
pub(crate) fn enum_bool_map<T: SettingEnum + Ord>(
    map: &mut BTreeMap<String, Value>,
    path: &Path,
    out: &mut Parser,
) -> BTreeMap<T, bool> {
    let mut toggles: BTreeMap<T, bool> = BTreeMap::new();
    let keys: Vec<String> = map.keys().cloned().collect();
    for key in keys {
        let entry_path = path.key(&key);
        let Some(setting) = T::from_text(&key) else {
            out.problem(
                &entry_path,
                Fault::UnknownKey {
                    accepted: T::variants(),
                },
            );
            continue;
        };
        if let Some(value) = flag(map, &key, path, out) {
            toggles.insert(setting, value);
        }
    }
    toggles
}

/// The table whose keys are names and whose values are lists of strings.
pub(crate) fn text_list_map(
    map: &mut BTreeMap<String, Value>,
    path: &Path,
    out: &mut Parser,
    id_shape: IdShape,
) -> BTreeMap<String, Vec<String>> {
    let mut entries: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let keys: Vec<String> = map.keys().cloned().collect();
    for key in keys {
        if !id_shape.accepts(&key) {
            out.problem(
                &path.key(&key),
                Fault::UnsupportedValue {
                    expected: id_shape.expected().to_owned(),
                },
            );
            continue;
        }
        if let Some(argv) = strings(map, &key, path, out) {
            entries.insert(key, argv);
        }
    }
    entries
}

/// The list of numbers at `key`, each inside `min..=max` and, when `length` is
/// set, of exactly that length.
///
/// Nine arguments because every one of them is declared data the generated code
/// passes literally: a range, the accepted-values text and a length are separate
/// facts about a setting, and grouping them would add a type the schema would
/// then have to describe.
#[allow(clippy::too_many_arguments)]
pub(crate) fn floats(
    map: &mut BTreeMap<String, Value>,
    key: &str,
    path: &Path,
    out: &mut Parser,
    min: f64,
    max: f64,
    exclusive_max: bool,
    length: Option<usize>,
    expected: &str,
) -> Option<Vec<f64>> {
    let list_path = path.key(key);
    let items = list(map, key, path, out, "a list of numbers")?;
    if let Some(length) = length
        && items.len() != length
    {
        out.problem(
            &list_path,
            Fault::UnsupportedValue {
                expected: expected.to_owned(),
            },
        );
        return None;
    }
    let mut numbers: Vec<f64> = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let Some(number) = item.as_number() else {
            out.problem(
                &list_path.entry(index),
                Fault::WrongType {
                    expected: "a number",
                    found: item.kind(),
                },
            );
            return None;
        };
        let inside = number >= min && if exclusive_max { number < max } else { number <= max };
        if !inside {
            out.problem(
                &list_path.entry(index),
                Fault::UnsupportedValue {
                    expected: expected.to_owned(),
                },
            );
            return None;
        }
        numbers.push(number);
    }
    Some(numbers)
}

/// The list of numbers at `key`, or `"inherit"`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn inherit_floats(
    map: &mut BTreeMap<String, Value>,
    key: &str,
    path: &Path,
    out: &mut Parser,
    min: f64,
    max: f64,
    exclusive_max: bool,
    length: Option<usize>,
    expected: &str,
) -> Option<Inherit<Vec<f64>>> {
    if is_inherit(map.get(key)?) {
        return Some(Inherit::Inherit);
    }
    floats(map, key, path, out, min, max, exclusive_max, length, expected).map(Inherit::Value)
}

/// The string at `key`, or `"inherit"`.
pub(crate) fn inherit_text(
    map: &mut BTreeMap<String, Value>,
    key: &str,
    path: &Path,
    out: &mut Parser,
) -> Option<Inherit<String>> {
    if is_inherit(map.get(key)?) {
        return Some(Inherit::Inherit);
    }
    text(map, key, path, out).map(Inherit::Value)
}

/// The list of strings at `key`, or `"inherit"`.
pub(crate) fn inherit_strings(
    map: &mut BTreeMap<String, Value>,
    key: &str,
    path: &Path,
    out: &mut Parser,
) -> Option<Inherit<Vec<String>>> {
    if is_inherit(map.get(key)?) {
        return Some(Inherit::Inherit);
    }
    strings(map, key, path, out).map(Inherit::Value)
}

/// Read a section table out of the document.
///
/// A section that is present but not a table is a problem at the section's own
/// path; a section that is absent is `None` and the defaults apply.
pub(crate) fn section_table<'m>(
    document: &'m mut BTreeMap<String, Value>,
    section: &str,
    path: &Path,
    out: &mut Parser,
) -> Option<&'m mut BTreeMap<String, Value>> {
    as_table(document.get_mut(section)?, &path.key(section), out, "a table")
}
