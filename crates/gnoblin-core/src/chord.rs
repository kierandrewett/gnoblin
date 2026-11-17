//! Chord spelling.
//!
//! The compositor's grammar writes each modifier in angle brackets and the key after them: `<Super>Return`,
//! `<Control><Alt>Delete`. Its parser refuses the plus form (`Super+Return`), and the spec and most shell authors write the plus form.
//! The SDK accepts both: [`normalize_chord`] turns the plus form into the compositor's and leaves the compositor's own spelling, and
//! the bare word `Super`, alone. The key name is not checked here (the keymap is the compositor's); an unknown key is refused by the
//! claim with the compositor's text.

use crate::error::{CoreError, ErrorKind, Result};

/// The compositor's name for a modifier as written in a plus-form chord, or `None` when it is not one. Case is ignored.
fn modifier(name: &str) -> Option<&'static str> {
    match name.to_ascii_lowercase().as_str() {
        "shift" | "shft" => Some("Shift"),
        "control" | "ctrl" | "ctl" | "primary" => Some("Control"),
        "alt" | "mod1" => Some("Alt"),
        "meta" => Some("Meta"),
        "super" => Some("Super"),
        "hyper" => Some("Hyper"),
        _ => None,
    }
}

/// The chord as the compositor reads it.
///
/// - `<Super>Return` and any other spelling that has an angle bracket is returned as it is: the compositor's parser is the judge.
/// - `Super+Return`, `Ctrl+Alt+Delete` and `Super+Shift+Button1` are rewritten with each modifier in angle brackets, in the order
///   written: `<Super>Return`, `<Ctrl><Alt>Delete` becomes `<Control><Alt>Delete`, so a modifier's legacy name is spelled as the
///   compositor's canonical one.
/// - A key that is the plus sign is `plus` (the XKB name), and `Super++` is read as `Super` and the plus key.
/// - A single word (`Super`, `XF86AudioMute`, `Return`) is returned as it is.
///
/// # Errors
///
/// The text is empty, has whitespace, names a modifier more than once, or names a word before the last `+` that is not a modifier.
pub fn normalize_chord(text: &str) -> Result<String> {
    let refuse =
        |reason: String| CoreError::new(ErrorKind::Failed, format!("the chord `{text}` is not valid: {reason}"));
    if text.is_empty() {
        return Err(refuse(
            "it is empty; write a key, as `Return`, or modifiers and a key, as `Super+Return`".to_owned(),
        ));
    }
    if text.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(refuse(
            "it has a space; write `Super+Return`, with no spaces".to_owned(),
        ));
    }
    if text.contains('<') || !text.contains('+') {
        return Ok(text.to_owned());
    }
    // `Super++` and `+`: the last key is the plus sign, whose XKB name is `plus`.
    let (modifiers, key) = match text.strip_suffix('+') {
        Some("") => ("", "plus"),
        Some(head) => match head.strip_suffix('+') {
            Some(modifiers) => (modifiers, "plus"),
            None => {
                return Err(refuse(
                    "it ends with `+` and no key; the plus key is written `plus`".to_owned(),
                ));
            }
        },
        None => match text.rsplit_once('+') {
            Some((modifiers, key)) => (modifiers, key),
            None => ("", text),
        },
    };
    let mut out = String::with_capacity(text.len() + 8);
    let mut seen: Vec<&'static str> = Vec::new();
    if !modifiers.is_empty() {
        for word in modifiers.split('+') {
            let Some(name) = modifier(word) else {
                return Err(refuse(format!(
                    "`{word}` is not a modifier; the modifiers are Shift, Ctrl, Alt, Meta, Super and Hyper"
                )));
            };
            if seen.contains(&name) {
                return Err(refuse(format!("the modifier {name} is written twice")));
            }
            seen.push(name);
            out.push('<');
            out.push_str(name);
            out.push('>');
        }
    }
    if key.is_empty() {
        return Err(refuse("a `+` has no key after it".to_owned()));
    }
    out.push_str(key);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::normalize_chord;

    fn ok(text: &str) -> String {
        normalize_chord(text).unwrap_or_else(|error| panic!("`{text}` was refused: {error}"))
    }

    #[test]
    fn the_plus_form_becomes_the_compositors_form() {
        assert_eq!(ok("Super+Return"), "<Super>Return");
        assert_eq!(ok("Super+Shift+r"), "<Super><Shift>r");
        assert_eq!(ok("super+shift+R"), "<Super><Shift>R");
        assert_eq!(ok("Ctrl+Alt+Delete"), "<Control><Alt>Delete");
        assert_eq!(ok("Primary+q"), "<Control>q");
        assert_eq!(ok("Super+Button1"), "<Super>Button1");
        assert_eq!(ok("Super+0x3f"), "<Super>0x3f");
    }

    #[test]
    fn the_compositors_own_spelling_and_single_words_are_left_alone() {
        assert_eq!(ok("<Super>Return"), "<Super>Return");
        assert_eq!(ok("<Ctrl><Alt>Delete"), "<Ctrl><Alt>Delete");
        assert_eq!(ok("Super"), "Super");
        assert_eq!(ok("XF86AudioMute"), "XF86AudioMute");
        assert_eq!(ok("Return"), "Return");
    }

    #[test]
    fn the_plus_key_is_named_plus() {
        assert_eq!(ok("Super++"), "<Super>plus");
        assert_eq!(ok("Super+plus"), "<Super>plus");
        assert_eq!(ok("+"), "plus");
    }

    #[test]
    fn a_chord_that_cannot_be_either_form_is_refused_with_what_to_write() {
        for (text, needle) in [
            ("", "is empty"),
            ("Super + Return", "no spaces"),
            ("Windows+Return", "is not a modifier"),
            ("Super+Super+Return", "twice"),
            ("Return+Super", "is not a modifier"),
            ("Super+Alt+", "ends with"),
            ("Super+ +r", "no spaces"),
        ] {
            let error = normalize_chord(text).expect_err(text);
            assert!(error.to_string().contains(needle), "`{text}`: {error}");
        }
    }
}
