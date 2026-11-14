//! Lua key chords to `keys` triggers.
//!
//! A Lua shortcut wrote its chord with the modifiers in angle brackets before the key, `<Super><Shift>q`, and a
//! `defineConfig` key writes them joined by plus, `Super+Shift+q`. Only a chord that has an exact trigger is
//! translated. A chord on a pointer button, a hardware key code or `Above_Tab` has none, and an upper-case letter is
//! a different key from the lower-case one in a Lua chord but the same trigger in `defineConfig`, so each of those is
//! reported with the reason and left for the author.

/// What a Lua chord becomes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChordTranslation {
    /// The trigger to write in `keys`, such as `Super+Shift+q`.
    Trigger(String),
    /// No exact trigger: why.
    Unmapped(String),
}

/// Keys the Lua chord grammar let an author write without their `XF86` prefix.
const XF86_KEYS: &[&str] = &[
    "AudioForward",
    "AudioLowerVolume",
    "AudioMedia",
    "AudioMicMute",
    "AudioMute",
    "AudioNext",
    "AudioPause",
    "AudioPlay",
    "AudioPrev",
    "AudioRaiseVolume",
    "AudioRandomPlay",
    "AudioRecord",
    "AudioRepeat",
    "AudioRewind",
    "AudioStop",
    "Battery",
    "Bluetooth",
    "Calculator",
    "Display",
    "Eject",
    "Explorer",
    "Favorites",
    "HomePage",
    "KbdBrightnessDown",
    "KbdBrightnessUp",
    "Mail",
    "MonBrightnessDown",
    "MonBrightnessUp",
    "PowerOff",
    "ScreenSaver",
    "Search",
    "Sleep",
    "TouchpadOff",
    "TouchpadOn",
    "TouchpadToggle",
    "WLAN",
    "WWW",
];

/// Words that `defineConfig` reads as something other than a key when they stand alone in a trigger.
const RESERVED: &[&str] = &[
    "shift",
    "ctrl",
    "control",
    "primary",
    "alt",
    "mod1",
    "meta",
    "super",
    "logo",
    "hyper",
    "mod2",
    "mod3",
    "mod4",
    "mod5",
    "drag",
    "click",
    "mouseleft",
    "mousemiddle",
    "mouseright",
    "wheel",
    "wheelup",
    "wheeldown",
    "wheelleft",
    "wheelright",
];

/// Translates a Lua chord. The modifiers keep the order they were written in.
#[must_use]
pub fn translate_chord(chord: &str) -> ChordTranslation {
    match translate(chord) {
        Ok(trigger) => ChordTranslation::Trigger(trigger),
        Err(reason) => ChordTranslation::Unmapped(reason),
    }
}

fn translate(chord: &str) -> Result<String, String> {
    if chord.is_empty() {
        return Err("the chord is empty".to_owned());
    }
    if chord.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("a chord has no spaces".to_owned());
    }
    // A lone `Super` is the tap trigger in both grammars.
    if chord == "Super" {
        return Ok("Super".to_owned());
    }
    let mut modifiers: Vec<&'static str> = Vec::new();
    let mut rest = chord;
    while let Some(stripped) = rest.strip_prefix('<') {
        let Some(close) = stripped.find('>') else {
            return Err("a modifier is never closed".to_owned());
        };
        let written = &stripped[..close];
        let modifier = match written.to_ascii_lowercase().as_str() {
            "shift" | "shft" => "Shift",
            "control" | "ctrl" | "ctl" | "primary" => "Ctrl",
            "alt" | "mod1" => "Alt",
            "meta" => "Meta",
            "super" => "Super",
            "hyper" => "Hyper",
            "mod2" | "mod3" | "mod4" | "mod5" => {
                return Err(format!("<{written}> is an X11 mask whose key depends on the keymap"));
            }
            _ => return Err(format!("<{written}> is not a modifier")),
        };
        if modifiers.contains(&modifier) {
            return Err(format!("the modifier {modifier} is written twice"));
        }
        modifiers.push(modifier);
        rest = &stripped[close + 1..];
    }
    if rest.is_empty() {
        return Err("the chord has no key".to_owned());
    }
    if rest.contains('<') || rest.contains('>') {
        return Err("a chord has one key".to_owned());
    }
    let key = key_name(rest)?;
    let mut trigger = String::new();
    for modifier in &modifiers {
        trigger.push_str(modifier);
        trigger.push('+');
    }
    trigger.push_str(&key);
    Ok(trigger)
}

/// The key of a chord as `defineConfig` spells it.
fn key_name(name: &str) -> Result<String, String> {
    if let Some(digits) = name.strip_prefix("Button")
        && !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit())
    {
        return Err("a pointer-button chord has no exact trigger".to_owned());
    }
    if name.starts_with("0x") || name.starts_with("0X") {
        return Err("a hardware key code has no exact trigger".to_owned());
    }
    if name.eq_ignore_ascii_case("above_tab") {
        return Err("Above_Tab has no exact trigger".to_owned());
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!(
            "\"{name}\" is not a key name; a key name is letters, digits and underscores"
        ));
    }
    let mut chars = name.chars();
    if let (Some(only), None) = (chars.next(), chars.next())
        && only.is_ascii_uppercase()
    {
        return Err(format!(
            "the upper-case key {only} is a different key from {} in a chord, and one trigger in defineConfig; write Shift+ and the lower-case letter",
            only.to_ascii_lowercase()
        ));
    }
    let lower = name.to_ascii_lowercase();
    if RESERVED.contains(&lower.as_str()) || lower.starts_with("swipe") || lower.starts_with("pinch") {
        return Err(format!(
            "\"{name}\" reads as a modifier, a button or a gesture in a trigger"
        ));
    }
    if XF86_KEYS.contains(&name) {
        return Ok(format!("XF86{name}"));
    }
    Ok(name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trigger(chord: &str) -> String {
        match translate_chord(chord) {
            ChordTranslation::Trigger(trigger) => trigger,
            ChordTranslation::Unmapped(reason) => panic!("{chord} was unmapped: {reason}"),
        }
    }

    fn reason(chord: &str) -> String {
        match translate_chord(chord) {
            ChordTranslation::Unmapped(reason) => reason,
            ChordTranslation::Trigger(trigger) => panic!("{chord} became {trigger}"),
        }
    }

    #[test]
    fn modifiers_become_plus_joined_names_in_the_order_written() {
        assert_eq!(trigger("<Super>Return"), "Super+Return");
        assert_eq!(trigger("<Super><Shift>x"), "Super+Shift+x");
        assert_eq!(trigger("<Shift><Super>x"), "Shift+Super+x");
        assert_eq!(trigger("<Alt>F8"), "Alt+F8");
        assert_eq!(trigger("<Control><Alt>Delete"), "Ctrl+Alt+Delete");
        assert_eq!(trigger("<Hyper><Meta>space"), "Hyper+Meta+space");
    }

    #[test]
    fn the_old_modifier_names_read_as_the_new_ones_in_any_case() {
        assert_eq!(trigger("<Primary>Tab"), "Ctrl+Tab");
        assert_eq!(trigger("<Ctrl>Tab"), "Ctrl+Tab");
        assert_eq!(trigger("<Ctl>Tab"), "Ctrl+Tab");
        assert_eq!(trigger("<Shft>Tab"), "Shift+Tab");
        assert_eq!(trigger("<Mod1>Tab"), "Alt+Tab");
        assert_eq!(trigger("<SUPER>q"), "Super+q");
    }

    #[test]
    fn a_key_alone_and_the_bare_super_tap_are_triggers() {
        assert_eq!(trigger("XF86AudioMute"), "XF86AudioMute");
        assert_eq!(trigger("F9"), "F9");
        assert_eq!(trigger("Super"), "Super");
        assert_eq!(trigger("<Super>1"), "Super+1");
    }

    #[test]
    fn a_media_key_written_without_its_prefix_gets_it_back() {
        assert_eq!(trigger("AudioMute"), "XF86AudioMute");
        assert_eq!(trigger("<Alt>MonBrightnessUp"), "Alt+XF86MonBrightnessUp");
    }

    #[test]
    fn what_has_no_exact_trigger_is_reported_with_the_reason() {
        assert!(reason("<Super>Button1").contains("pointer"));
        assert!(reason("Button8").contains("pointer"));
        assert!(reason("<Super>0x3f").contains("hardware key code"));
        assert!(reason("<Super>Above_Tab").contains("Above_Tab"));
        assert!(reason("<Mod4>x").contains("keymap"));
        assert!(reason("<Mod2>x").contains("keymap"));
        assert!(reason("<Bogus>x").contains("not a modifier"));
        assert!(reason("<Super><Super>x").contains("twice"));
        assert!(reason("<Control><Primary>x").contains("twice"));
        assert!(reason("<Super>").contains("no key"));
        assert!(reason("<Super").contains("never closed"));
        assert!(reason("<Super>a<Alt>").contains("one key"));
        assert!(reason("").contains("empty"));
        assert!(reason("Super+Return").contains("not a key name"));
        assert!(reason("<Super>,").contains("not a key name"));
        assert!(reason("<Super>Q").contains("upper-case"));
        assert!(reason("<Alt>Shift").contains("modifier"));
        assert!(reason("<Super>Swipe3").contains("gesture"));
        assert!(reason("<Super> q").contains("no spaces"));
    }

    #[test]
    fn a_trigger_the_runtime_reads_is_what_comes_out() {
        // The shapes the runtime's own grammar accepts: modifiers first, one key of letters, digits and underscores.
        for chord in [
            "<Super><Shift>q",
            "<Primary><Alt>Delete",
            "XF86AudioRaiseVolume",
            "<Super>KP_1",
            "<Alt>ISO_Left_Tab",
        ] {
            let text = trigger(chord);
            let mut parts: Vec<&str> = text.split('+').collect();
            let key = parts.pop().expect("a key");
            assert!(key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'), "{text}");
            let modifiers = ["Shift", "Ctrl", "Alt", "Meta", "Super", "Hyper"];
            assert!(parts.iter().all(|p| modifiers.contains(p)), "{text}");
        }
    }
}
