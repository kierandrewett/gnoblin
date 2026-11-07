//! Validation of namespace names and keys.
//!
//! A namespace name becomes a file name, so the rules here are what keep a caller from
//! naming a file outside the store's root. The charset has no `.` and no `/`, which also
//! rules out `..` and every absolute path. Keys never reach the file system; they are
//! text inside a table, and the rules here only keep them unambiguous for prefix listing.

use crate::StoreError;

/// The longest namespace name, in bytes.
pub const NAME_LIMIT: usize = 64;

/// The longest key, in bytes. A key may hold `/`, so `mime/image/png` is one key.
pub const KEY_LIMIT: usize = 256;

/// Checks a namespace name: 1 to [`NAME_LIMIT`] bytes of `a-z`, `0-9`, `-` and `_`, starting
/// with a letter or a digit. The first character rule stops a name that reads as a flag.
pub fn validate_name(name: &str) -> Result<(), StoreError> {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > NAME_LIMIT {
        return Err(StoreError::InvalidName(format!("a name is 1 to {NAME_LIMIT} bytes")));
    }
    if !(bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit()) {
        return Err(StoreError::InvalidName("a name starts with a-z or 0-9".to_owned()));
    }
    if bytes
        .iter()
        .any(|byte| !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')))
    {
        return Err(StoreError::InvalidName(
            "a name uses only a-z, 0-9, '-' and '_'".to_owned(),
        ));
    }
    Ok(())
}

/// Checks a key: 1 to [`KEY_LIMIT`] bytes, with no NUL and no leading or trailing `/`.
pub fn validate_key(key: &str) -> Result<(), StoreError> {
    if key.is_empty() || key.len() > KEY_LIMIT {
        return Err(StoreError::InvalidKey(format!("a key is 1 to {KEY_LIMIT} bytes")));
    }
    if key.contains('\0') {
        return Err(StoreError::InvalidKey("a key holds no NUL byte".to_owned()));
    }
    if key.starts_with('/') || key.ends_with('/') {
        return Err(StoreError::InvalidKey(
            "a key has no leading or trailing '/'".to_owned(),
        ));
    }
    Ok(())
}

/// Checks a prefix for listing: the empty prefix lists everything, and any other prefix is a
/// key-shaped string without a trailing `/`, so `window/1` names a segment and `window/` is
/// refused rather than silently meaning something else.
pub fn validate_prefix(prefix: &str) -> Result<(), StoreError> {
    if prefix.is_empty() {
        Ok(())
    } else {
        validate_key(prefix)
    }
}

/// Whether `key` lies under `prefix` on whole segments: the prefix itself, or a key that
/// continues it after a `/`. `window/1` covers `window/1/x` and not `window/10`.
pub fn covers(prefix: &str, key: &str) -> bool {
    prefix.is_empty() || key == prefix || (key.starts_with(prefix) && key.as_bytes().get(prefix.len()) == Some(&b'/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_lowercase_and_cannot_leave_the_root() {
        assert!(validate_name("kv-1_a").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name("../etc").is_err());
        assert!(validate_name("a/b").is_err());
        assert!(validate_name("a.b").is_err());
        assert!(validate_name("Upper").is_err());
        assert!(validate_name("-lead").is_err());
        assert!(validate_name(&"a".repeat(NAME_LIMIT + 1)).is_err());
    }

    #[test]
    fn a_key_has_no_nul_and_no_edge_slash() {
        assert!(validate_key("mime/image/png").is_ok());
        assert!(validate_key("").is_err());
        assert!(validate_key("/a").is_err());
        assert!(validate_key("a/").is_err());
        assert!(validate_key("a\0b").is_err());
        assert!(validate_key(&"k".repeat(KEY_LIMIT + 1)).is_err());
    }

    #[test]
    fn a_prefix_matches_whole_segments_only() {
        assert!(covers("window/1", "window/1"));
        assert!(covers("window/1", "window/1/x"));
        assert!(!covers("window/1", "window/10"));
        assert!(covers("", "anything"));
        assert!(validate_prefix("window/").is_err());
    }
}
