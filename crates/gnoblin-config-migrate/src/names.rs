//! Names: which Lua sections have no TypeScript home yet and why, and how a string is written in TypeScript.
//!
//! The specification's object model has no home for most of the config document's sections, so the converter does not
//! carry their settings over. Each one is reported with what a home would need, the status the design records in
//! `design/js-spec-native.md` (3.11, "Dropped settings"). The statuses are one table here so a setting's reason is
//! the same wherever it was met.

/// The sections of the config document that are plain tables of settings, by their Lua names. None has a TypeScript
/// home yet; the converter reports each setting of them.
pub(crate) const SECTIONS: &[&str] = &[
    "compositor",
    "window-management",
    "cursor",
    "appearance",
    "dialogs",
    "input",
    "input-sources",
    "accessibility",
    "layer-shell",
    "keyring",
    "auth",
    "location",
    "compat",
    "capture",
    "monitors",
    "xwayland",
    "logging",
    "protocols",
];

/// The document sections that are lists or tables the converter reshapes into statements, or reports whole: the Lua
/// name of each.
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

/// What a setting of `window-management` that sets the focus policy needs: the first of the two rows 3.11 gives it.
const FOCUS: &str = "the shell has no focus policy on the seat; the runtime keeps click focus, new windows \
                     focused and raise on click";

/// What a snapping setting needs: a window dragged to an edge, and a drag kept inside the work area.
pub(crate) const SNAPPING: &str = "the shell has no snapping policy on the seat or the layout; the runtime keeps edge \
                        snapping on";

/// What `autostart` needs.
pub(crate) const AUTOSTART: &str = "the shell has no session-scoped startup list, and `gnoblin.spawn` in \
                                    init.ts runs again on every reload; start the command from your session by hand";

/// What a drawn-path touchpad gesture needs.
pub(crate) const TOUCHPAD_GESTURES: &str = "the shell has no path gesture trigger; a swipe or pinch with a fixed \
                                            direction is `input.bind`";

/// What a workspace set needs.
pub(crate) const WORKSPACE_SETS: &str = "the shell has no workspace sets; a function that shows one workspace \
                                         on each display does the same";

/// What a rule field that a `Client` cannot be written for needs.
pub(crate) const RULE_FIELD: &str = "a `Client` has no property to write it to yet";

/// What the `input` sections need: the device objects exist, the converter does not read the Lua settings into them.
const INPUT: &str = "the device settings are `Keyboard`, `Mouse` and `Touchpad` properties (`repeatRate`, `sensitivity`, \
                     `naturalScroll`), and the converter does not translate the Lua input sections: set them by hand";

/// What every other section needs.
const GENERIC: &str = "the shell has no compositor-wide settings object, and only the device settings of \
                       `input` reach the document";

/// What a setting of `section` needs before it has a TypeScript home: the sentence after "has no TypeScript home yet".
/// `setting` is the first key under the section, which decides it for the sections that mix several concerns.
pub(crate) fn follow_up(section: &str, setting: &str) -> &'static str {
    match (section, setting) {
        ("window-management", "edge-tiling" | "constrain-drag-to-work-area") => SNAPPING,
        ("window-management", key)
            if key.starts_with("focus-") || key.starts_with("auto-raise") || key == "raise-on-click" =>
        {
            FOCUS
        }
        ("dialogs", "style") => {
            "the shell has no style object on the dialog manager; the `gnoblin.dialogs` listeners are not \
             affected"
        }
        ("keyring", _) => {
            "the shell has no compositor-wide settings object read before the session starts; the keyring \
             keeps its default, enabled"
        }
        ("protocols", _) => "the shell has no compositor-wide settings object for the protocols",
        ("input" | "input-sources", _) => INPUT,
        _ => GENERIC,
    }
}

/// Whether `name` is a plain section of the config document.
pub(crate) fn is_section(name: &str) -> bool {
    SECTIONS.contains(&name)
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
    fn every_section_of_the_document_is_handled_here() {
        // A section the schema gains is either a plain table the converter reports or a list it reshapes. This fails
        // until someone decides which.
        for section in gnoblin_config::TOP_LEVEL_KEYS {
            assert!(
                is_section(section) || RESHAPED.contains(section),
                "the document section `{section}` is not handled by the converter"
            );
        }
        for section in RESHAPED.iter().chain(SECTIONS) {
            assert!(
                gnoblin_config::TOP_LEVEL_KEYS.contains(section),
                "`{section}` is no longer a document section"
            );
        }
    }

    #[test]
    fn the_reason_depends_on_the_setting_where_a_section_mixes_concerns() {
        assert_eq!(follow_up("window-management", "focus-mode"), FOCUS);
        assert_eq!(follow_up("window-management", "auto-raise-delay"), FOCUS);
        assert_eq!(follow_up("window-management", "raise-on-click"), FOCUS);
        assert_eq!(follow_up("window-management", "edge-tiling"), SNAPPING);
        assert_eq!(follow_up("window-management", "constrain-drag-to-work-area"), SNAPPING);
        assert_eq!(follow_up("window-management", "check-alive-timeout"), GENERIC);
        assert!(follow_up("dialogs", "style").contains("dialog manager"));
        assert!(follow_up("keyring", "enabled").contains("keeps its default"));
        assert_eq!(follow_up("input", "keyboard"), INPUT);
        assert_eq!(follow_up("cursor", "size"), GENERIC);
    }

    #[test]
    fn strings_are_written_as_typescript() {
        assert_eq!(string("a\"b\\c\nd"), "\"a\\\"b\\\\c\\nd\"");
        assert_eq!(string("\u{1}"), "\"\\u0001\"");
        assert_eq!(string("caf\u{e9}"), "\"caf\u{e9}\"");
    }
}
