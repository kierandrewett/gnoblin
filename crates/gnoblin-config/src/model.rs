//! The shared shapes of the configuration document.
//!
//! These are the few types that both the generated code and the hand-written
//! engine have to name: how an enumerated setting parses, what a deprecated
//! spelling is, how a setting's value kind and default are described, and the
//! `"inherit"` marker. Everything that only exists to describe the document
//! lives here so the generated module can import it and stay a list of
//! declarations.
//!
//! The generated module is `include!`d into `crate::generated` by `lib.rs`; the
//! types here are hand-written because the generator would otherwise have to
//! emit its own vocabulary, and a schema cannot describe how a schema is read.

use std::fmt;

use crate::path::Path;

/// A setting that accepts a value or the word `"inherit"`.
///
/// `"inherit"` is not a value the compositor applies: it clears an override so
/// the system preference shows through again. Keeping it in the type stops a
/// caller from treating "not set" and "clear the override" as the same thing,
/// which is the difference a reload of layered files makes visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inherit<T> {
    /// An explicit value.
    Value(T),
    /// Clear the override and follow the system setting.
    Inherit,
}

impl<T> Inherit<T> {
    /// The explicit value, or `None` when this clears the override.
    pub fn value(&self) -> Option<&T> {
        match self {
            Inherit::Value(value) => Some(value),
            Inherit::Inherit => None,
        }
    }
}

/// A setting whose values are a fixed set of names.
///
/// The generated enums implement this, so one reader parses any of them, reports
/// the accepted values and never sees a spelling that is not in the set.
pub trait SettingEnum: Copy + fmt::Debug + PartialEq + Eq {
    /// Every accepted spelling, in the order the reference lists them.
    fn variants() -> &'static [&'static str];

    /// The variant a current spelling names.
    fn from_text(text: &str) -> Option<Self>;

    /// The stored spelling of a variant.
    fn as_text(self) -> &'static str;

    /// This setting's identity, for the alias table and the reference.
    fn spec() -> EnumSpec;
}

/// The identity and accepted spellings of one enumerated setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnumSpec {
    /// The Rust name of the enum, so a catalogue reader can match a
    /// [`Kind::Enum`] to its accepted spellings.
    pub name: &'static str,
    /// The document key the setting is normally stored under, or a descriptive
    /// label where several keys share the enum.
    pub setting: &'static str,
    /// Every accepted spelling.
    pub variants: &'static [&'static str],
}

impl EnumSpec {
    /// The accepted values, spelled for a reader, including the deprecated
    /// spellings that still parse.
    pub fn expected(&self) -> String {
        let mut expected = String::from("one of ");
        for (position, variant) in self.variants.iter().enumerate() {
            if position > 0 {
                expected.push_str(", ");
            }
            expected.push_str(variant);
        }
        let deprecated: Vec<&str> = crate::generated::ALIASES
            .iter()
            .filter(|alias| alias.setting == self.setting)
            .map(|alias| alias.spelling)
            .collect();
        if !deprecated.is_empty() {
            expected.push_str("; deprecated spellings still accepted: ");
            for (position, spelling) in deprecated.iter().enumerate() {
                if position > 0 {
                    expected.push_str(", ");
                }
                expected.push_str(spelling);
            }
        }
        expected
    }
}

/// A deprecated spelling and the current one it stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Alias {
    /// The document key the spelling is written under.
    pub setting: &'static str,
    /// The spelling that is going away.
    pub spelling: &'static str,
    /// The current spelling it means.
    pub canonical: &'static str,
    /// The release that stops accepting the spelling.
    pub removed_in: &'static str,
}

/// The outcome of parsing one enum value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnumParse<T> {
    /// A current spelling.
    Current(T),
    /// A deprecated spelling, and the variant it means.
    Deprecated {
        /// The variant the spelling means.
        value: T,
        /// The spelling that was written.
        spelling: &'static str,
    },
    /// Not a spelling of this setting at all.
    Unsupported,
}

/// Parse `text` as `T`, accepting the deprecated spellings listed for `setting`.
pub fn parse<T: SettingEnum>(setting: &str, text: &str) -> EnumParse<T> {
    if let Some(value) = T::from_text(text) {
        return EnumParse::Current(value);
    }
    for alias in crate::generated::ALIASES {
        if alias.setting == setting
            && alias.spelling == text
            && let Some(value) = T::from_text(alias.canonical)
        {
            return EnumParse::Deprecated {
                value,
                spelling: alias.spelling,
            };
        }
    }
    EnumParse::Unsupported
}

/// A value written with a deprecated spelling, resolved to its current one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Deprecation {
    /// Where the deprecated spelling was written.
    pub path: Path,
    /// The spelling that was written.
    pub spelling: &'static str,
    /// The spelling that replaces it.
    pub canonical: &'static str,
}

impl fmt::Display for Deprecation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}: deprecated spelling \"{}\"; use \"{}\"",
            self.path, self.spelling, self.canonical
        )
    }
}

/// The declared kind of a setting's value.
///
/// The catalogue carries this so a consumer that is not the validator (the
/// reference generator, `gnoblinctl`, a test) can describe a setting without
/// re-reading the schema, and so the defaults can be checked against the ranges
/// they are declared with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// A boolean.
    Bool,
    /// An integer in a range.
    Int {
        /// The smallest accepted value.
        min: i64,
        /// The largest accepted value.
        max: i64,
    },
    /// A number in a range.
    Float {
        /// The smallest accepted value.
        min: f64,
        /// The largest accepted value.
        max: f64,
        /// Whether `max` itself is refused.
        exclusive_max: bool,
    },
    /// A string.
    Text {
        /// Whether an empty string is refused.
        nonempty: bool,
    },
    /// A name from an enumerated setting, by the enum's Rust name.
    Enum(&'static str),
    /// A list of names from an enumerated setting.
    EnumList(&'static str),
    /// A list of strings.
    TextList,
    /// A list of numbers, optionally of a fixed length.
    FloatList {
        /// The smallest accepted value.
        min: f64,
        /// The largest accepted value.
        max: f64,
        /// Whether `max` itself is refused.
        exclusive_max: bool,
        /// The required length, when it is fixed.
        length: Option<usize>,
    },
    /// A table, by the record's Rust name.
    Record(&'static str),
    /// A list of tables.
    RecordList(&'static str),
    /// A table keyed by device id.
    RecordMap(&'static str),
    /// A table whose keys are names from an enumerated setting.
    EnumBoolMap(&'static str),
    /// A table keyed by a name, with a list of strings as each value.
    TextListMap,
}

/// The declared default of a setting.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Default {
    /// No default: the setting is unset unless the document sets it.
    None,
    /// A boolean default.
    Bool(bool),
    /// An integer default.
    Integer(i64),
    /// A number default.
    Float(f64),
    /// A string default.
    Text(&'static str),
    /// An enumerated default, by its spelling.
    Enum(&'static str),
    /// A list default.
    List(&'static [&'static str]),
}

/// One declared setting.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Setting {
    /// The top-level section the setting is written under.
    pub section: &'static str,
    /// The key inside the section, or `*` for a section whose keys are values.
    pub key: &'static str,
    /// The Rust field the value lands in.
    pub field: &'static str,
    /// The declared kind.
    pub kind: Kind,
    /// The declared default.
    pub default: Default,
    /// The accepted values, spelled for a reader.
    pub accepted: &'static str,
    /// The release that added the setting.
    pub since: &'static str,
    /// Whether the running session may override the setting.
    pub overridable: bool,
    /// Whether the setting also accepts `"inherit"`.
    pub inherit: bool,
    /// Whether the setting must be present for its entry to be accepted.
    pub required: bool,
}
