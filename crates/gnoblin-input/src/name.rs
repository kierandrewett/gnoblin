//! The name a binding is declared under.
//!
//! Keyboard, pointer and gesture bindings are all named by the configuration so a
//! later file can replace or disable one, and the compositor's shortcut socket
//! names dynamic bindings the same way. One validator here keeps the three
//! declarations and the socket method from drifting apart, which the plan calls
//! out as a duplicated-table defect in the C tree.

use std::fmt;

/// The longest accepted name, in bytes.
pub const MAX_NAME_LENGTH: usize = 64;

/// A binding name: 1 to 64 ASCII letters, digits, `_` or `-`.
///
/// The same rule as `gnoblin_control_dynamic_shortcut_id_valid` in the C tree, so a
/// name that `gnoblin.shortcuts.bind` accepts can also be a configured binding.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Name(String);

impl Name {
    /// Validate a declared name.
    ///
    /// Only ASCII is accepted because the name appears in diagnostics, socket
    /// replies and log lines where a confusable name would be a security problem:
    /// two names that render alike but compare differently must not exist.
    pub fn new(text: &str) -> Result<Self, NameError> {
        if text.is_empty() {
            return Err(NameError {
                text: text.to_owned(),
                problem: Problem::Empty,
            });
        }
        if text.len() > MAX_NAME_LENGTH {
            return Err(NameError {
                text: text.to_owned(),
                problem: Problem::TooLong { length: text.len() },
            });
        }
        if let Some(character) = text
            .chars()
            .find(|character| !character.is_ascii_alphanumeric() && *character != '_' && *character != '-')
        {
            return Err(NameError {
                text: text.to_owned(),
                problem: Problem::Character { character },
            });
        }
        Ok(Self(text.to_owned()))
    }

    /// The name as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Name {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Why a name was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Problem {
    Empty,
    TooLong { length: usize },
    Character { character: char },
}

/// A refused name, with the text that was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameError {
    text: String,
    problem: Problem,
}

impl NameError {
    /// The refused text.
    pub fn text(&self) -> &str {
        &self.text
    }
}

impl fmt::Display for NameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.problem {
            Problem::Empty => write!(formatter, "a binding name cannot be empty"),
            Problem::TooLong { length } => write!(
                formatter,
                "binding name \"{}\" is {length} bytes; the limit is {MAX_NAME_LENGTH}",
                self.text
            ),
            Problem::Character { character } => write!(
                formatter,
                "binding name \"{}\" contains {character:?}; use letters, digits, underscores or hyphens",
                self.text
            ),
        }
    }
}

impl std::error::Error for NameError {}

#[cfg(test)]
mod tests {
    use super::Name;

    #[test]
    fn a_name_keeps_its_text_and_refuses_the_rest() {
        assert_eq!(Name::new("open_terminal").expect("valid").as_str(), "open_terminal");
        assert_eq!(Name::new("a-b_9").expect("valid").as_str(), "a-b_9");
        assert!(Name::new("").is_err());
        assert!(Name::new("two words").is_err());
        assert!(Name::new("semi;colon").is_err());
        assert!(Name::new("na\u{ef}ve").is_err());
        assert!(Name::new(&"n".repeat(65)).is_err());
        assert!(Name::new(&"n".repeat(64)).is_ok());
    }
}
