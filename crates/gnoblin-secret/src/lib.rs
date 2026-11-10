//! Text that must not leak.
//!
//! A password crosses several hops (the shell's Lua, the tree transaction, the wire, the
//! compositor's batch) and each hop may log, format or keep a copy. [`SecretText`] is the one
//! type those hops share: its `Debug` never shows the text, it has no `Display`, and the bytes
//! are overwritten when the value is dropped, so a value that has been handed on leaves no
//! readable copy behind it in that owner.

use std::fmt;

use zeroize::Zeroize;

/// Text that must never be printed, logged or kept after use.
///
/// `Debug` prints `<redacted>` whatever the text is. There is no `Display`, so the text cannot
/// reach a `format!` by accident; read it with [`SecretText::expose`] where it is consumed.
/// Dropping the value, or calling [`SecretText::wipe`], overwrites the bytes first.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SecretText(String);

impl SecretText {
    /// Wrap `text`. The caller's copy, if it has one, is the caller's to wipe.
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// The text, for the one place that consumes it.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// True when there is no text.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Overwrite the bytes with zeros and empty the value now, rather than when it is dropped.
    pub fn wipe(&mut self) {
        self.0.zeroize();
    }
}

/// Overwrite a plain `String` that held secret text with zeros and empty it. For the copies a
/// wire type keeps, which cannot hold a [`SecretText`].
pub fn wipe(text: &mut String) {
    text.zeroize();
}

impl fmt::Debug for SecretText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretText(<redacted>)")
    }
}

impl Drop for SecretText {
    fn drop(&mut self) {
        self.wipe();
    }
}

#[cfg(test)]
mod tests {
    use super::SecretText;

    #[test]
    fn debug_never_shows_the_text() {
        let secret = SecretText::new("hunter2-correct-horse");
        let plain = format!("{secret:?}");
        let pretty = format!("{secret:#?}");
        assert!(!plain.contains("hunter2"), "{plain}");
        assert!(!pretty.contains("hunter2"), "{pretty}");
        assert_eq!(plain, "SecretText(<redacted>)");
    }

    #[test]
    fn expose_returns_the_text_to_its_consumer() {
        let secret = SecretText::new("chord");
        assert_eq!(secret.expose(), "chord");
        assert!(!secret.is_empty());
        assert!(SecretText::default().is_empty());
    }

    #[test]
    fn wipe_empties_the_value_in_place() {
        let mut secret = SecretText::new("hunter2");
        secret.wipe();
        assert!(secret.is_empty());
        assert_eq!(secret.expose(), "");
    }

    #[test]
    fn wipe_text_empties_a_plain_string() {
        let mut text = String::from("hunter2");
        super::wipe(&mut text);
        assert!(text.is_empty());
    }

    #[test]
    fn equal_text_compares_equal_and_clones_independently() {
        let mut copy = SecretText::new("abc");
        let original = SecretText::new("abc");
        assert_eq!(copy, original);
        copy.wipe();
        assert_eq!(original.expose(), "abc", "wiping a clone leaves the original");
    }
}
