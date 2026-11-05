//! Dotted paths into the configuration document.
//!
//! Every diagnostic names the setting it is about, and the recovery panel, the
//! reload result and the session log all print that name. One type for the name
//! means a test can assert on the location of a problem rather than on the
//! wording of a message, and it means the wording can change without changing
//! what locates a problem.

use std::fmt;

/// One step of a [`Path`].
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Segment {
    /// A key in a table, spelled as the document spells it (`locate-pointer`).
    Key(String),
    /// A position in a list, stored from zero.
    ///
    /// Rendering adds one: `workspaces[1]` is the first entry, because the
    /// reader of a diagnostic counts entries from one and the C configuration
    /// core already reports list positions that way.
    Index(usize),
}

/// The location of a setting inside the configuration document.
///
/// A path is built by descending, never parsed back from text: the document
/// layer is the only place that knows how a setting is spelled, so the text form
/// exists for people and for tests, not as an input format.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Path {
    segments: Vec<Segment>,
}

impl Path {
    /// The document itself, rendered as `<document>`.
    pub const fn root() -> Path {
        Path { segments: Vec::new() }
    }

    /// The path of `key` inside this one.
    #[must_use]
    pub fn key(&self, key: &str) -> Path {
        let mut segments = self.segments.clone();
        segments.push(Segment::Key(key.to_owned()));
        Path { segments }
    }

    /// The path of the list entry at `index`, counted from zero.
    #[must_use]
    pub fn entry(&self, index: usize) -> Path {
        let mut segments = self.segments.clone();
        segments.push(Segment::Index(index));
        Path { segments }
    }

    /// Whether this names the whole document.
    pub fn is_root(&self) -> bool {
        self.segments.is_empty()
    }

    /// The first key under the document, which is the section a problem belongs
    /// to. Salvage drops a whole section, so this is the key it drops.
    pub fn section(&self) -> Option<&str> {
        match self.segments.first() {
            Some(Segment::Key(key)) => Some(key),
            _ => None,
        }
    }

    /// The segments in order.
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }
}

impl fmt::Display for Path {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.segments.is_empty() {
            return formatter.write_str("<document>");
        }
        for (position, segment) in self.segments.iter().enumerate() {
            match segment {
                Segment::Key(key) => {
                    // A key after another segment is joined to it; the first key
                    // is the whole path. An index attaches without a separator.
                    if position > 0 {
                        formatter.write_str(".")?;
                    }
                    formatter.write_str(key)?;
                }
                Segment::Index(index) => write!(formatter, "[{}]", index + 1)?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_joined_by_dots_and_lists_are_counted_from_one() {
        let path = Path::root().key("workspaces").entry(0).key("name");
        assert_eq!(path.to_string(), "workspaces[1].name");
    }

    #[test]
    fn the_root_names_the_whole_document() {
        assert_eq!(Path::root().to_string(), "<document>");
        assert!(Path::root().is_root());
        assert_eq!(Path::root().section(), None);
    }

    #[test]
    fn the_section_is_the_first_key() {
        let path = Path::root().key("input").key("mouse").key("speed");
        assert_eq!(path.section(), Some("input"));
    }
}
