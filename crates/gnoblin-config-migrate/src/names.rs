//! Names: which Lua section is which `defineConfig` key, how a kebab key becomes a camel one, and how a name or a
//! string is written in TypeScript.
//!
//! The mapping is the one `data/js/gnoblin/config.ts` applies in the other direction (`DOCUMENT_SECTIONS` for the
//! top-level keys, `documentTable` for the keys inside a section). A key is only rewritten when `defineConfig` turns
//! the new spelling back into exactly the old one, so a setting never changes its name on the way through.

/// A section of the config document that is a `defineConfig` key of its own: the document's kebab-case name and the
/// key to write. The same table as `DOCUMENT_SECTIONS` in `data/js/gnoblin/config.ts`.
pub(crate) const SECTIONS: &[(&str, &str)] = &[
    ("compositor", "compositor"),
    ("window-management", "windowManagement"),
    ("cursor", "cursor"),
    ("appearance", "appearance"),
    ("dialogs", "dialogs"),
    ("input", "input"),
    ("input-sources", "inputSources"),
    ("accessibility", "accessibility"),
    ("layer-shell", "layerShell"),
    ("keyring", "keyring"),
    ("auth", "auth"),
    ("location", "location"),
    ("compat", "compat"),
    ("capture", "capture"),
    ("monitors", "monitors"),
    ("xwayland", "xwayland"),
    ("logging", "logging"),
    ("protocols", "protocols"),
];

/// The document sections that are lists or tables the converter reshapes, not `defineConfig` sections: the Lua
/// name and what becomes of it.
#[cfg(test)]
pub(crate) const RESHAPED: &[&str] = &[
    "workspaces",
    "autostart",
    "shortcuts",
    "keybindings",
    "touchpad-gestures",
    "animations",
    "portals",
];

/// The `defineConfig` key of a document section, when the section is one of the plain ones.
pub(crate) fn section_key(lua_name: &str) -> Option<&'static str> {
    SECTIONS.iter().find(|(lua, _)| *lua == lua_name).map(|(_, key)| *key)
}

/// What `defineConfig` makes of a written setting name: `buttonLayout` is `button-layout`, and every key is
/// lower-cased. The same rule as `documentTable` in `config.ts`.
pub(crate) fn document_spelling(written: &str) -> String {
    let mut out = String::with_capacity(written.len() + 4);
    let mut previous: Option<char> = None;
    for c in written.chars() {
        if c.is_ascii_uppercase() && previous.is_some_and(|p| p.is_ascii_lowercase() || p.is_ascii_digit()) {
            out.push('-');
        }
        out.push(c.to_ascii_lowercase());
        previous = Some(c);
    }
    out
}

/// The camelCase spelling of a kebab-case key, or `None` when `defineConfig` would not read it back as the same
/// key. A key with a capital, an underscore, a segment that starts with a digit or an empty segment keeps its own
/// spelling, which `defineConfig` accepts as it stands.
pub(crate) fn camel_case(kebab: &str) -> Option<String> {
    if !kebab.contains('-') {
        // Nothing to change: a plain lowercase word is its own camelCase.
        return None;
    }
    let mut out = String::with_capacity(kebab.len());
    for (index, segment) in kebab.split('-').enumerate() {
        let mut chars = segment.chars();
        let first = chars.next()?;
        if !first.is_ascii_lowercase() {
            return None;
        }
        if index == 0 {
            out.push(first);
        } else {
            out.push(first.to_ascii_uppercase());
        }
        out.push_str(chars.as_str());
    }
    (document_spelling(&out) == kebab).then_some(out)
}

/// The spelling to write for a key found inside a section: camelCase when that reads back exactly, the key as it
/// stands otherwise.
pub(crate) fn setting_name(key: &str) -> String {
    camel_case(key).unwrap_or_else(|| key.to_owned())
}

/// Whether `text` can be written as a bare property name in TypeScript.
pub(crate) fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_' || first == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// A property name as TypeScript writes it: bare when it can be, a string otherwise.
pub(crate) fn property(name: &str) -> String {
    if is_identifier(name) {
        name.to_owned()
    } else {
        string(name)
    }
}

/// A TypeScript string literal. Quotes, backslashes, control characters and the two line separators are escaped;
/// everything else is written as it is.
pub(crate) fn string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{7f}' || c == '\u{2028}' || c == '\u{2029}' => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kebab_keys_become_camel_keys_that_read_back_unchanged() {
        for (kebab, camel) in [
            ("window-management", "windowManagement"),
            ("focus-new-windows", "focusNewWindows"),
            ("pointing-stick", "pointingStick"),
            ("ext-background-effect-v1", "extBackgroundEffectV1"),
            ("preserve-active-window", "preserveActiveWindow"),
        ] {
            assert_eq!(camel_case(kebab).as_deref(), Some(camel), "{kebab}");
            assert_eq!(document_spelling(camel), kebab, "{camel}");
        }
    }

    #[test]
    fn a_key_that_would_not_read_back_keeps_its_own_spelling() {
        for key in [
            "DP-1",
            "HDMI-A-1",
            "a-1b",
            "a--b",
            "-a",
            "a-",
            "edge_tiling",
            "Focus-Mode",
            "plain",
        ] {
            assert_eq!(camel_case(key), None, "{key}");
            assert_eq!(setting_name(key), key);
        }
    }

    #[test]
    fn document_spelling_matches_the_runtimes_rule() {
        assert_eq!(document_spelling("buttonLayout"), "button-layout");
        assert_eq!(document_spelling("wlrLayerShell"), "wlr-layer-shell");
        assert_eq!(document_spelling("already-kebab"), "already-kebab");
        assert_eq!(document_spelling("ABC"), "abc");
        assert_eq!(document_spelling("aB"), "a-b");
        assert_eq!(document_spelling("a1B"), "a1-b");
    }

    #[test]
    fn every_plain_section_has_a_key_and_the_key_reads_back_as_the_section() {
        for (lua, key) in SECTIONS {
            assert_eq!(document_spelling(key), *lua);
            assert_eq!(section_key(lua), Some(*key));
        }
        assert_eq!(section_key("shortcuts"), None);
    }

    #[test]
    fn every_section_of_the_document_is_handled_here() {
        // A section the schema gains is either a `defineConfig` key or a list the converter reshapes. This fails
        // until someone decides which.
        for section in gnoblin_config::TOP_LEVEL_KEYS {
            assert!(
                section_key(section).is_some() || RESHAPED.contains(section),
                "the document section `{section}` is not handled by the converter"
            );
        }
        for section in RESHAPED {
            assert!(
                gnoblin_config::TOP_LEVEL_KEYS.contains(section),
                "`{section}` is no longer a document section"
            );
        }
    }

    #[test]
    fn properties_and_strings_are_written_as_typescript() {
        assert_eq!(property("XF86AudioMute"), "XF86AudioMute");
        assert_eq!(property("Super+Return"), "\"Super+Return\"");
        assert_eq!(property("1"), "\"1\"");
        assert_eq!(string("a\"b\\c\nd"), "\"a\\\"b\\\\c\\nd\"");
        assert_eq!(string("\u{1}"), "\"\\u0001\"");
        assert_eq!(string("caf\u{e9}"), "\"caf\u{e9}\"");
    }
}
