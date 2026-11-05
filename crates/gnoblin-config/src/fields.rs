//! Reading a setting out of a table, and the diagnostic for one that is wrong.
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

use crate::enums::{self, EnumParse, SettingEnum};
use crate::path::Path;
use crate::problem::{Fault, Problem};
use crate::sections::Inherit;
use crate::value::Value;

/// What one validation pass has found so far.
///
/// Problems and deprecations are collected, never raised: a validator that
/// stopped at the first fault would turn a three-typo config into three reloads.
pub(crate) struct Parser {
    problems: Vec<Problem>,
    deprecations: Vec<enums::Deprecation>,
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
        self.deprecations.push(enums::Deprecation {
            path: path.clone(),
            spelling,
            canonical,
        });
    }

    /// How many problems have been recorded, so a list parser can tell whether
    /// dropping an entry was reported.
    pub(crate) fn problem_count(&self) -> usize {
        self.problems.len()
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
    pub(crate) fn finish(self) -> (Vec<Problem>, Vec<enums::Deprecation>) {
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
    match enums::parse::<T>(setting, &written) {
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
        match enums::parse::<T>(setting, &written) {
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
