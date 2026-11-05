//! Settings for the compositor itself: bells, blur, the cursor, the cursor
//! locator, protocol-independent behaviour and the session services Gnoblin can
//! take over.
//!
//! These sections are small and flat, so they are one module. Each is validated
//! against the allowed-key list the C configuration core checks, which is the
//! list the reference documents.

use std::collections::BTreeMap;

use crate::enums::{Blur, LocationAccuracy, VisualBellType, XwaylandExtension};
use crate::fields::{
    Parser, as_table, flag, inherit_flag, inherit_integer, inherit_setting, integer, nonempty_text, number, setting,
    settings, strings, text, unknown_keys,
};
use crate::path::Path;
use crate::sections::Inherit;
use crate::value::Value;

/// `compositor`: what the compositor draws itself and how.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Compositor {
    /// Show the frame-time and resource overlay.
    pub debug: Option<bool>,
    /// Send a locate request when the locate key is pressed.
    pub locate_pointer: Option<bool>,
    /// Flash the screen when an application rings the visual bell.
    pub visual_bell: Option<bool>,
    /// Play a sound when an application rings the audible bell.
    pub audible_bell: Option<bool>,
    /// Which part of the screen the visual bell flashes.
    pub visual_bell_type: Option<VisualBellType>,
    /// Background blur policy.
    pub blur: Option<Blur>,
    /// The key that triggers the pointer locator, `"disabled"`, or empty.
    ///
    /// Only the shape is checked here. Whether the name is a keysym this build
    /// knows is a question for `gnoblin-input`, which owns libxkbcommon; asking
    /// it here would make the document layer depend on a keyboard.
    pub locate_pointer_key: Option<String>,
}

/// The keys `compositor` accepts.
const COMPOSITOR_KEYS: &[&str] = &[
    "audible-bell",
    "blur",
    "debug",
    "locate-pointer",
    "locate-pointer-key",
    "visual-bell",
    "visual-bell-type",
];

/// Parse the `compositor` table.
pub(crate) fn parse_compositor(map: &mut BTreeMap<String, Value>, path: &Path, out: &mut Parser) -> Compositor {
    unknown_keys(map, path, out, COMPOSITOR_KEYS);
    let locate_pointer_key = text(map, "locate-pointer-key", path, out).and_then(|key| {
        if locate_pointer_key_shaped(&key) {
            Some(key)
        } else {
            out.problem(
                &path.key("locate-pointer-key"),
                crate::problem::Fault::UnsupportedValue {
                    expected: "one XKB keysym name, \"disabled\", or an empty string".to_owned(),
                },
            );
            None
        }
    });
    Compositor {
        debug: flag(map, "debug", path, out),
        locate_pointer: flag(map, "locate-pointer", path, out),
        visual_bell: flag(map, "visual-bell", path, out),
        audible_bell: flag(map, "audible-bell", path, out),
        visual_bell_type: setting(map, "visual-bell-type", "visual-bell-type", path, out),
        blur: setting(map, "blur", "blur", path, out),
        locate_pointer_key,
    }
}

/// Whether `key` has the shape of a keysym name rather than a chord.
///
/// `<Control>` is a chord, not one key, and a name with whitespace is two words.
/// Both are refused here so the mistake is reported at the config file instead
/// of at the first key press.
fn locate_pointer_key_shaped(key: &str) -> bool {
    key.is_empty()
        || key == "disabled"
        || !(key.contains('<') || key.contains('>') || key.chars().any(char::is_whitespace))
}

/// `cursor`: the Xcursor or Hyprcursor theme and its logical size.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    /// The theme name, which must be nonempty.
    pub theme: Option<String>,
    /// The size in logical pixels, 1 to 256.
    pub size: Option<u16>,
}

/// The keys `cursor` accepts.
const CURSOR_KEYS: &[&str] = &["size", "theme"];

/// Parse the `cursor` table.
pub(crate) fn parse_cursor(map: &mut BTreeMap<String, Value>, path: &Path, out: &mut Parser) -> Cursor {
    unknown_keys(map, path, out, CURSOR_KEYS);
    Cursor {
        theme: nonempty_text(map, "theme", path, out),
        size: integer(map, "size", path, out, 1..=256, "an integer from 1 to 256"),
    }
}

/// `layer-shell`: how an opening layer surface affects the focused window.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayerShell {
    /// Keep the active application window focused when a layer surface opens.
    pub preserve_active_window: Option<bool>,
}

/// The keys `layer-shell` accepts.
const LAYER_SHELL_KEYS: &[&str] = &["preserve-active-window"];

/// Parse the `layer-shell` table.
pub(crate) fn parse_layer_shell(map: &mut BTreeMap<String, Value>, path: &Path, out: &mut Parser) -> LayerShell {
    unknown_keys(map, path, out, LAYER_SHELL_KEYS);
    LayerShell {
        preserve_active_window: flag(map, "preserve-active-window", path, out),
    }
}

/// `prompts`: whether Gnoblin serves keyring and GPG passphrase prompts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Prompts {
    /// Take the prompter role on the session bus.
    pub enabled: Option<bool>,
}

/// The keys `prompts` accepts.
const PROMPTS_KEYS: &[&str] = &["enabled"];

/// Parse the `prompts` table.
pub(crate) fn parse_prompts(map: &mut BTreeMap<String, Value>, path: &Path, out: &mut Parser) -> Prompts {
    unknown_keys(map, path, out, PROMPTS_KEYS);
    Prompts {
        enabled: flag(map, "enabled", path, out),
    }
}

/// `auth`: whether Gnoblin is the session's polkit authentication agent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Auth {
    /// Register as the polkit authentication agent.
    pub polkit_agent: Option<bool>,
}

/// The keys `auth` accepts.
const AUTH_KEYS: &[&str] = &["polkit-agent"];

/// Parse the `auth` table.
pub(crate) fn parse_auth(map: &mut BTreeMap<String, Value>, path: &Path, out: &mut Parser) -> Auth {
    unknown_keys(map, path, out, AUTH_KEYS);
    Auth {
        polkit_agent: flag(map, "polkit-agent", path, out),
    }
}

/// `location`: the policy Gnoblin's GeoClue agent applies.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Location {
    /// Whether location requests may be approved at all.
    pub enabled: Option<Inherit<bool>>,
    /// The greatest precision an approved request may return.
    pub max_accuracy: Option<Inherit<LocationAccuracy>>,
}

/// The keys `location` accepts.
const LOCATION_KEYS: &[&str] = &["enabled", "max-accuracy"];

/// Parse the `location` table.
pub(crate) fn parse_location(map: &mut BTreeMap<String, Value>, path: &Path, out: &mut Parser) -> Location {
    unknown_keys(map, path, out, LOCATION_KEYS);
    Location {
        enabled: inherit_flag(map, "enabled", path, out),
        max_accuracy: inherit_setting(map, "max-accuracy", "max-accuracy", path, out),
    }
}

/// `monitors`: preferences that apply to whichever monitors are connected.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Monitors {
    /// Request the privacy-screen state on monitors that have one.
    pub privacy_screen: Option<Inherit<bool>>,
    /// Scale every output by this integer step, or follow the system setting.
    pub global_scale: Option<Inherit<u8>>,
}

/// The keys `monitors` accepts.
const MONITORS_KEYS: &[&str] = &["global-scale", "privacy-screen"];

/// Parse the `monitors` table.
pub(crate) fn parse_monitors(map: &mut BTreeMap<String, Value>, path: &Path, out: &mut Parser) -> Monitors {
    unknown_keys(map, path, out, MONITORS_KEYS);
    Monitors {
        privacy_screen: inherit_flag(map, "privacy-screen", path, out),
        global_scale: inherit_integer(map, "global-scale", path, out, 0..=4, "an integer from 0 through 4"),
    }
}

/// `xwayland`: how the X11 server is started and what it may do.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Xwayland {
    /// Allow X11 clients to take keyboard grabs on override-redirect windows.
    pub allow_grabs: Option<bool>,
    /// Allow clients whose byte order differs from the server's.
    pub allow_byte_swapped_clients: Option<bool>,
    /// Resource names or classes permitted to take a grab, `!` to deny.
    pub grab_access_rules: Option<Vec<String>>,
    /// Xwayland extensions to disable.
    pub disable_extensions: Option<Vec<XwaylandExtension>>,
    /// `0` for automatic scaling, otherwise the scale Mutter is told.
    pub scaling_factor: Option<f64>,
}

/// The keys `xwayland` accepts.
const XWAYLAND_KEYS: &[&str] = &[
    "allow-byte-swapped-clients",
    "allow-grabs",
    "disable-extensions",
    "grab-access-rules",
    "scaling-factor",
];

/// The largest Xwayland scale that survives Mutter's conversion to `int`.
pub const MAX_XWAYLAND_SCALE: f64 = 2_147_483_520.0;

/// Parse the `xwayland` table.
pub(crate) fn parse_xwayland(map: &mut BTreeMap<String, Value>, path: &Path, out: &mut Parser) -> Xwayland {
    unknown_keys(map, path, out, XWAYLAND_KEYS);
    let expected = "0, or a finite number from 0.5 through 2147483520";
    let scaling_factor =
        number(map, "scaling-factor", path, out, 0.0..=MAX_XWAYLAND_SCALE, expected).and_then(|scale| {
            if scale == 0.0 || scale >= 0.5 {
                return Some(scale);
            }
            out.problem(
                &path.key("scaling-factor"),
                crate::problem::Fault::UnsupportedValue {
                    expected: expected.to_owned(),
                },
            );
            None
        });
    Xwayland {
        allow_grabs: flag(map, "allow-grabs", path, out),
        allow_byte_swapped_clients: flag(map, "allow-byte-swapped-clients", path, out),
        grab_access_rules: strings(map, "grab-access-rules", path, out),
        disable_extensions: settings(map, "disable-extensions", "disable-extensions", path, out),
        scaling_factor,
    }
}

/// Read a section table out of the document.
///
/// A section that is present but not a table is a problem at the section's own
/// path; a section that is absent is `None` and the defaults apply.
pub(crate) fn section_table<'m>(
    document: &'m mut BTreeMap<String, Value>,
    section: &str,
    path: &Path,
    out: &mut Parser,
) -> Option<&'m mut BTreeMap<String, Value>> {
    as_table(document.get_mut(section)?, &path.key(section), out, "a table")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_locate_pointer_key_must_be_one_key_name() {
        assert!(locate_pointer_key_shaped("F12"));
        assert!(locate_pointer_key_shaped("Control_L"));
        assert!(locate_pointer_key_shaped(""));
        assert!(locate_pointer_key_shaped("disabled"));
        assert!(!locate_pointer_key_shaped("<Control>F12"));
        assert!(!locate_pointer_key_shaped("Control L"));
    }
}
