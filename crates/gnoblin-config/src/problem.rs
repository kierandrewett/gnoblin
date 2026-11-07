//! Validation problems and the report that carries all of them.
//!
//! Validation never stops at the first bad setting. A user who mistyped three
//! keys should learn about all three from one reload rather than by fixing them
//! one reload at a time, and the recovery panel has one place to show them. So a
//! validator collects [`Problem`]s and the caller decides what to do with the
//! report: reject the document (a reload), or drop the sections that hold
//! problems and carry on (login, see [`crate::salvage`]).
//!
//! Every problem carries a [`Path`] and an accepted-values hint, and both are
//! part of the message a person reads.

use std::error::Error;
use std::fmt;

use crate::path::Path;

/// Why a setting was rejected.
///
/// The fault is the machine-readable half: a test asserts that a misspelled key
/// is an `UnknownKey`, not that a sentence contains a word. The message is the
/// half a person reads, and [`Fault`]'s [`fmt::Display`] builds it so the
/// accepted-values hint cannot be forgotten at a call site.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fault {
    /// The table does not accept this key at all.
    UnknownKey {
        /// Every key the table does accept, spelled as the document spells it.
        accepted: &'static [&'static str],
    },
    /// The key used to exist and is now refused with an explanation.
    Removed {
        /// What replaces it, or why it is gone.
        detail: String,
    },
    /// The value has the wrong shape: a list where a table belongs.
    WrongType {
        /// What was expected, as a noun phrase: `a table`, `a boolean`.
        expected: &'static str,
        /// What was found, from [`crate::value::Value::kind`].
        found: &'static str,
    },
    /// The setting is required and is not set.
    Missing {
        /// What is required, as a noun phrase: `a name`.
        expected: &'static str,
    },
    /// The value has the right shape but is not one of the accepted values.
    UnsupportedValue {
        /// The accepted values, spelled out for the reader.
        expected: String,
    },
    /// Two settings that cannot both be set.
    Conflict {
        /// The setting the rejected one conflicts with.
        other: Path,
    },
    /// The document nests deeper than the parser will follow.
    TooDeep {
        /// The deepest nesting the parser accepts.
        maximum: usize,
    },
    /// The value is a float that is not finite.
    NotFinite,
    /// The setting is in the schema but this build cannot apply it yet. The
    /// reason names the missing prerequisite, so a user knows what to wait for
    /// rather than assuming the setting was accepted and ignored. Refusal is a
    /// load error: a rule list is never partly applied.
    Refused {
        /// What is missing, as a sentence fragment: `shaders need the WASM host`.
        reason: &'static str,
    },
}

impl fmt::Display for Fault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Fault::UnknownKey { accepted } => {
                formatter.write_str("unknown setting; accepted: ")?;
                for (position, key) in accepted.iter().enumerate() {
                    if position > 0 {
                        formatter.write_str(", ")?;
                    }
                    formatter.write_str(key)?;
                }
                Ok(())
            }
            Fault::Removed { detail } => write!(formatter, "no longer supported; {detail}"),
            Fault::WrongType { expected, found } => write!(formatter, "expected {expected}, found {found}"),
            Fault::Missing { expected } => write!(formatter, "required: {expected}"),
            Fault::UnsupportedValue { expected } => write!(formatter, "expected {expected}"),
            Fault::Conflict { other } => write!(formatter, "cannot be combined with {other}"),
            Fault::TooDeep { maximum } => write!(formatter, "nested deeper than {maximum} levels"),
            Fault::NotFinite => formatter.write_str("expected a finite number"),
            Fault::Refused { reason } => write!(formatter, "{reason}, not yet built in this compositor"),
        }
    }
}

/// One rejected setting: where it is and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    /// The setting that was rejected.
    pub path: Path,
    /// Why it was rejected.
    pub fault: Fault,
}

impl Problem {
    /// A problem at `path` with `fault`.
    pub fn new(path: Path, fault: Fault) -> Problem {
        Problem { path, fault }
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.path, self.fault)
    }
}

/// Every problem found in one document.
///
/// A report is only ever constructed with at least one problem: a document that
/// validates produces a value, not an empty report, so a caller never has to ask
/// whether an `Err` really failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    problems: Vec<Problem>,
}

impl Report {
    /// Build a report from the problems a validator collected.
    ///
    /// # Panics
    ///
    /// Panics in debug builds when `problems` is empty, because an empty report
    /// means the validator meant to return a document and did not.
    pub(crate) fn new(problems: Vec<Problem>) -> Report {
        debug_assert!(!problems.is_empty(), "a report needs at least one problem");
        Report { problems }
    }

    /// The problems, in the order validation reached them.
    pub fn problems(&self) -> &[Problem] {
        &self.problems
    }

    /// The problems, in order.
    pub fn iter(&self) -> std::slice::Iter<'_, Problem> {
        self.problems.iter()
    }

    /// The distinct top-level sections that hold a problem, in document order.
    ///
    /// A problem that is not inside a section (the root is not a table, or the
    /// whole document is nested too deeply) is not named here; salvage treats
    /// that as "nothing can be kept".
    pub(crate) fn sections(&self) -> Vec<&str> {
        let mut sections: Vec<&str> = Vec::new();
        for problem in &self.problems {
            if let Some(section) = problem.path.section()
                && !sections.contains(&section)
            {
                sections.push(section);
            }
        }
        sections
    }
}

impl fmt::Display for Report {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (position, problem) in self.problems.iter().enumerate() {
            if position > 0 {
                formatter.write_str("\n")?;
            }
            write!(formatter, "{problem}")?;
        }
        Ok(())
    }
}

impl Error for Report {}

#[cfg(test)]
mod tests {
    use super::*;

    fn unknown(path: Path) -> Problem {
        Problem::new(
            path,
            Fault::UnknownKey {
                accepted: &["debug", "blur"],
            },
        )
    }

    #[test]
    fn a_problem_names_its_setting_and_the_accepted_keys() {
        let problem = unknown(Path::root().key("compositor").key("debug2"));
        assert_eq!(
            problem.to_string(),
            "compositor.debug2: unknown setting; accepted: debug, blur"
        );
    }

    #[test]
    fn a_report_prints_one_problem_per_line() {
        let report = Report::new(vec![
            unknown(Path::root().key("compositor").key("a")),
            unknown(Path::root().key("cursor").key("b")),
        ]);
        assert_eq!(report.to_string().lines().count(), 2);
        assert_eq!(report.problems().len(), 2);
    }

    #[test]
    fn sections_are_named_once_each() {
        let report = Report::new(vec![
            unknown(Path::root().key("input").key("a")),
            unknown(Path::root().key("cursor").key("b")),
            unknown(Path::root().key("input").key("c")),
        ]);
        assert_eq!(report.sections(), vec!["input", "cursor"]);
    }

    #[test]
    fn a_problem_outside_a_section_names_no_section() {
        let report = Report::new(vec![Problem::new(
            Path::root(),
            Fault::WrongType {
                expected: "a table",
                found: "a list",
            },
        )]);
        assert!(report.sections().is_empty());
        assert_eq!(report.to_string(), "<document>: expected a table, found a list");
    }
}
