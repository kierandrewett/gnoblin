//! The trigger grammar: a trigger is the string a user writes for an input (`Super+Return`, `Super+MouseLeft+Drag`, `Swipe3Left`), and
//! [`parse`] is the one place its grammar lives. It turns the spelling into a canonical one, so two spellings of one trigger are one trigger
//! (`Ctrl+Alt+Delete` and `control+alt+delete`), and it refuses a bad spelling with the token that is wrong.
//!
//! ```text
//! trigger  := key | pointer | wheel | gesture
//! key      := (modifier "+")* keyname        `Super+Shift+1`, `XF86AudioMute`; a lone `Super` is a tap
//! pointer  := (modifier "+")* button ["+" ("Drag" | "Click")]    `Super+MouseLeft+Drag`, `MouseMiddle`
//! wheel    := (modifier "+")* "Wheel" ["Up" | "Down" | "Left" | "Right"]
//! gesture  := ("Swipe" | "Pinch") fingers(2-5) [direction]       `Swipe3Left`, `Pinch4In`
//! modifier := Shift | Ctrl (Control, Primary) | Alt (Mod1) | Meta | Super (Logo) | Hyper
//! ```
//!
//! Case does not matter except in key names, which are keysym names. A key is a keysym name, never a character that depends on the layout:
//! `Super+1` and `Super+Shift+1` are two triggers, and `Super+!` is refused (write `Super+Shift+1` or `Super+exclam`). A literal plus is
//! `plus`.

/// The modifiers a trigger may name, in the order a canonical trigger writes them.
const MODIFIER_ORDER: [&str; 6] = ["Shift", "Ctrl", "Alt", "Meta", "Super", "Hyper"];

/// The modifier a spelling names, or none.
fn modifier_of(lower: &str) -> Option<&'static str> {
    Some(match lower {
        "shift" => "Shift",
        "ctrl" | "control" | "primary" => "Ctrl",
        "alt" | "mod1" => "Alt",
        "meta" => "Meta",
        "super" | "logo" => "Super",
        "hyper" => "Hyper",
        _ => return None,
    })
}

/// X11 masks whose key depends on the keymap: the compositor refuses them, and so does the grammar.
fn refused_mask(lower: &str) -> bool {
    matches!(lower, "mod2" | "mod3" | "mod4" | "mod5")
}

/// The button a spelling names, or none.
fn button_of(lower: &str) -> Option<&'static str> {
    Some(match lower {
        "mouseleft" => "MouseLeft",
        "mousemiddle" => "MouseMiddle",
        "mouseright" => "MouseRight",
        _ => return None,
    })
}

/// The kernel-side number of a button a trigger names.
fn button_number(button: &str) -> u8 {
    match button {
        "MouseMiddle" => 2,
        "MouseRight" => 3,
        _ => 1,
    }
}

/// The wheel a spelling names: `Some(None)` for the bare wheel, `Some(Some(direction))` for one that names a direction.
fn wheel_of(lower: &str) -> Option<Option<&'static str>> {
    Some(match lower {
        "wheel" => None,
        "wheelup" => Some("up"),
        "wheeldown" => Some("down"),
        "wheelleft" => Some("left"),
        "wheelright" => Some("right"),
        _ => return None,
    })
}

/// Names the grammar writes one way, so two spellings are not two bindings that shadow each other.
fn key_name_of(lower: &str) -> Option<&'static str> {
    Some(match lower {
        "return" | "enter" => "Return",
        "escape" | "esc" => "Escape",
        "tab" => "Tab",
        "space" => "space",
        "backspace" => "BackSpace",
        "delete" | "del" => "Delete",
        "insert" | "ins" => "Insert",
        "home" => "Home",
        "end" => "End",
        "pageup" | "page_up" => "Page_Up",
        "pagedown" | "page_down" => "Page_Down",
        "left" => "Left",
        "right" => "Right",
        "up" => "Up",
        "down" => "Down",
        "print" => "Print",
        "menu" => "Menu",
        "plus" => "plus",
        "minus" => "minus",
        _ => return None,
    })
}

/// What a trigger names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::model) enum TriggerClass {
    Key,
    Pointer,
    Wheel,
    Gesture,
}

impl TriggerClass {
    /// The name the catalog's `trigger_kind` gives it.
    pub(in crate::model) fn name(self) -> &'static str {
        match self {
            TriggerClass::Key => "key",
            TriggerClass::Pointer => "pointer",
            TriggerClass::Wheel => "wheel",
            TriggerClass::Gesture => "gesture",
        }
    }
}

/// A trigger after parsing.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::model) struct ParsedTrigger {
    /// The canonical spelling.
    pub text: String,
    pub kind: TriggerClass,
    /// In the canonical order.
    pub modifiers: Vec<&'static str>,
    /// A key's canonical name.
    pub key: Option<String>,
    /// `MouseLeft`, `MouseMiddle` or `MouseRight`.
    pub button: Option<&'static str>,
    /// `click` or `drag`.
    pub action: Option<&'static str>,
    /// A wheel's direction, or a gesture's.
    pub direction: Option<&'static str>,
    /// `swipe` or `pinch`.
    pub gesture: Option<&'static str>,
    /// A gesture's finger count.
    pub fingers: Option<u8>,
}

impl ParsedTrigger {
    fn blank(kind: TriggerClass, text: String, modifiers: Vec<&'static str>) -> ParsedTrigger {
        ParsedTrigger {
            text,
            kind,
            modifiers,
            key: None,
            button: None,
            action: None,
            direction: None,
            gesture: None,
            fingers: None,
        }
    }
}

/// What a trigger asks the compositor to claim on a seat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::model) enum SeatClaim {
    /// A key chord or a pointer chord, in the compositor's spelling.
    Chord { text: String, grab: bool },
    /// Every touchpad gesture of a kind with a finger count.
    Gesture { gesture: &'static str, fingers: u8 },
}

impl SeatClaim {
    /// The key claims with the same chord or gesture share: a chord's text or a gesture's kind and finger count.
    pub(in crate::model) fn key(&self) -> String {
        match self {
            SeatClaim::Chord { text, .. } => format!("chord:{text}"),
            SeatClaim::Gesture { gesture, fingers } => format!("gesture:{gesture}{fingers}"),
        }
    }
}

/// The seat claim a trigger needs, in the compositor's own chord spelling (`<Shift><Super>1`), or none when the compositor has no claim for
/// it (a wheel). A drag and a click of one button share a claim: the chord's press and release both reach the shell, and the shell tells a
/// drag from a click by how far the pointer went.
pub(in crate::model) fn claim_of(trigger: &ParsedTrigger) -> Option<SeatClaim> {
    let modifiers: String = trigger
        .modifiers
        .iter()
        .map(|modifier| match *modifier {
            "Ctrl" => "<Control>".to_owned(),
            other => format!("<{other}>"),
        })
        .collect();
    match trigger.kind {
        TriggerClass::Key => Some(SeatClaim::Chord {
            text: format!("{modifiers}{}", trigger.key.as_deref().unwrap_or_default()),
            grab: false,
        }),
        TriggerClass::Pointer => Some(SeatClaim::Chord {
            text: format!(
                "{modifiers}Button{}",
                button_number(trigger.button.unwrap_or("MouseLeft"))
            ),
            grab: true,
        }),
        TriggerClass::Gesture => Some(SeatClaim::Gesture {
            gesture: trigger.gesture.unwrap_or("swipe"),
            fingers: trigger.fingers.unwrap_or(3),
        }),
        TriggerClass::Wheel => None,
    }
}

/// A spelling the grammar refuses. `message` is the text a refusal carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::model) struct TriggerRefusal {
    pub message: String,
    /// The part of the spelling that is wrong, empty when none is.
    pub token: String,
}

fn fail<T>(what: &str, trigger: &str, token: &str, reason: &str) -> Result<T, TriggerRefusal> {
    Err(TriggerRefusal {
        message: format!("{what}: {reason} (token \"{token}\" in \"{trigger}\")"),
        token: token.to_owned(),
    })
}

fn canonical_key(token: &str) -> String {
    if let Some(known) = key_name_of(&token.to_lowercase()) {
        return known.to_owned();
    }
    if is_function_key(token) {
        return format!("F{}", &token[1..]);
    }
    // A letter is the key, not a shifted letter: `Super+Q` and `Super+q` are one trigger, and the shift is written as `Shift`. Longer names
    // keep their case, because keysym names are case-sensitive.
    if token.encode_utf16().count() == 1 {
        token.to_lowercase()
    } else {
        token.to_owned()
    }
}

/// `F1` to `F35`, in either case.
fn is_function_key(token: &str) -> bool {
    let mut chars = token.chars();
    if !matches!(chars.next(), Some('f' | 'F')) {
        return false;
    }
    let digits = chars.as_str();
    matches!(digits.parse::<u32>(), Ok(1..=35))
        && digits.bytes().all(|byte| byte.is_ascii_digit())
        && !digits.starts_with('0')
}

/// Whether `token` is a gesture word: `swipe` or `pinch` first, in either case.
fn starts_gesture(token: &str) -> bool {
    let lower = token.to_lowercase();
    lower.starts_with("swipe") || lower.starts_with("pinch")
}

fn parse_gesture(what: &str, trigger: &str, token: &str) -> Result<ParsedTrigger, TriggerRefusal> {
    let lower = token.to_lowercase();
    let (gesture, rest): (&'static str, &str) = if let Some(rest) = lower.strip_prefix("swipe") {
        ("swipe", rest)
    } else if let Some(rest) = lower.strip_prefix("pinch") {
        ("pinch", rest)
    } else {
        return fail(
            what,
            trigger,
            token,
            "is not a gesture; expected Swipe or Pinch with a finger count",
        );
    };
    let digits_end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    let (digits, suffix) = rest.split_at(digits_end);
    let direction: Option<&'static str> = match suffix {
        "" => None,
        "left" => Some("left"),
        "right" => Some("right"),
        "up" => Some("up"),
        "down" => Some("down"),
        "in" => Some("in"),
        "out" => Some("out"),
        _ => {
            return fail(
                what,
                trigger,
                token,
                "is not a gesture; expected Swipe or Pinch with a finger count",
            );
        }
    };
    if digits.is_empty() {
        return fail(
            what,
            trigger,
            token,
            "is not a gesture; expected Swipe or Pinch with a finger count",
        );
    }
    let trimmed = digits.trim_start_matches('0');
    let fingers = trimmed.parse::<u64>().unwrap_or(u64::MAX);
    if !(2..=5).contains(&fingers) {
        let shown = if trimmed.is_empty() { "0" } else { trimmed };
        return fail(
            what,
            trigger,
            token,
            &format!("a gesture has 2 to 5 fingers, not {shown}"),
        );
    }
    let allowed: &[&str] = if gesture == "swipe" {
        &["left", "right", "up", "down"]
    } else {
        &["in", "out"]
    };
    if let Some(direction) = direction
        && !allowed.contains(&direction)
    {
        return fail(
            what,
            trigger,
            token,
            &format!("a {gesture} goes {}, not {direction}", allowed.join(", ")),
        );
    }
    let name = if gesture == "swipe" { "Swipe" } else { "Pinch" };
    let suffix = direction.map_or_else(String::new, |direction| {
        let mut chars = direction.chars();
        chars
            .next()
            .map(|first| first.to_uppercase().chain(chars).collect())
            .unwrap_or_default()
    });
    let fingers = u8::try_from(fingers).unwrap_or(5);
    let mut parsed = ParsedTrigger::blank(TriggerClass::Gesture, format!("{name}{fingers}{suffix}"), Vec::new());
    parsed.gesture = Some(gesture);
    parsed.fingers = Some(fingers);
    parsed.direction = direction;
    Ok(parsed)
}

/// Parses a trigger into its parts and canonical spelling, or refuses it naming the token that is wrong. `what` names the thing that holds the
/// trigger, for the message.
pub(in crate::model) fn parse(trigger: &str, what: &str) -> Result<ParsedTrigger, TriggerRefusal> {
    if trigger.is_empty() {
        return Err(TriggerRefusal {
            message: format!("{what} must be a trigger such as \"Super+Return\", got "),
            token: String::new(),
        });
    }
    let parts: Vec<&str> = trigger.split('+').collect();
    for (at, part) in parts.iter().enumerate() {
        if part.is_empty() {
            let after = if at == 0 {
                "the start".to_owned()
            } else {
                format!("\"{}\"", parts[at - 1])
            };
            return fail(
                what,
                trigger,
                "",
                &format!("an empty part after {after}; write a literal plus as \"plus\""),
            );
        }
        if part.chars().any(char::is_whitespace) {
            return fail(what, trigger, part, "has whitespace; join the parts with plus alone");
        }
    }
    if let Some(at) = parts.iter().position(|part| starts_gesture(part)) {
        if parts.len() > 1 {
            let token = parts[if at == 0 { 1 } else { at }];
            return fail(what, trigger, token, "a gesture takes no modifiers or other parts");
        }
        return parse_gesture(what, trigger, parts[0]);
    }

    let mut modifiers: Vec<&'static str> = Vec::new();
    let mut button: Option<&'static str> = None;
    let mut wheel: Option<Option<&'static str>> = None;
    let mut action: Option<&'static str> = None;
    let mut key: Option<String> = None;
    let mut last = "";
    for token in &parts {
        let lower = token.to_lowercase();
        let modifier = modifier_of(&lower);
        if refused_mask(&lower) {
            return fail(
                what,
                trigger,
                token,
                "names a keymap-dependent mask; write Super, Alt, Meta or Hyper",
            );
        }
        if let Some(modifier) = modifier {
            if button.is_some() || wheel.is_some() || key.is_some() || action.is_some() {
                return fail(
                    what,
                    trigger,
                    token,
                    "is a modifier after the key; modifiers come first",
                );
            }
            if modifiers.contains(&modifier) {
                return fail(what, trigger, token, &format!("repeats the modifier {modifier}"));
            }
            modifiers.push(modifier);
        } else if let Some(named) = button_of(&lower) {
            if button.is_some() || wheel.is_some() || key.is_some() {
                return fail(
                    what,
                    trigger,
                    token,
                    "is a second input; a trigger has one key, button or wheel",
                );
            }
            button = Some(named);
        } else if let Some(named) = wheel_of(&lower) {
            if button.is_some() || wheel.is_some() || key.is_some() {
                return fail(
                    what,
                    trigger,
                    token,
                    "is a second input; a trigger has one key, button or wheel",
                );
            }
            wheel = Some(named);
        } else if lower == "drag" || lower == "click" {
            if button.is_none() {
                return fail(
                    what,
                    trigger,
                    token,
                    &format!("needs a mouse button before it, such as Super+MouseLeft+{token}"),
                );
            }
            if action.is_some() {
                return fail(what, trigger, token, "is a second action");
            }
            action = Some(if lower == "drag" { "drag" } else { "click" });
        } else {
            if button.is_some() || wheel.is_some() || action.is_some() {
                return fail(
                    what,
                    trigger,
                    token,
                    "is a key after a mouse button or wheel; a trigger has one input",
                );
            }
            if let Some(first) = &key {
                return fail(
                    what,
                    trigger,
                    token,
                    &format!("is a second key after \"{first}\"; a trigger has one key"),
                );
            }
            if token.encode_utf16().count() == 1 && !token.chars().all(|c| c.is_ascii_alphanumeric()) {
                return fail(
                    what,
                    trigger,
                    token,
                    "is a character, not a key name; write the keysym name (exclam, minus, plus) or Shift with the key",
                );
            }
            if !token.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return fail(
                    what,
                    trigger,
                    token,
                    "is not a key name; a key name is letters, digits and underscores",
                );
            }
            key = Some(canonical_key(token));
        }
        last = token;
    }

    let ordered: Vec<&'static str> = MODIFIER_ORDER
        .iter()
        .copied()
        .filter(|modifier| modifiers.contains(modifier))
        .collect();
    let prefix: String = ordered.iter().map(|modifier| format!("{modifier}+")).collect();
    if let Some(button) = button {
        // A bare button is a click, and so is `+Click`: one canonical spelling for the same trigger.
        let click = action != Some("drag");
        let text = format!("{prefix}{button}{}", if click { "" } else { "+Drag" });
        let mut parsed = ParsedTrigger::blank(TriggerClass::Pointer, text, ordered);
        parsed.button = Some(button);
        parsed.action = Some(if click { "click" } else { "drag" });
        return Ok(parsed);
    }
    if let Some(direction) = wheel {
        let name = direction.map_or_else(
            || "Wheel".to_owned(),
            |direction| {
                let mut chars = direction.chars();
                let first = chars.next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
                format!("Wheel{first}{}", chars.as_str())
            },
        );
        let mut parsed = ParsedTrigger::blank(TriggerClass::Wheel, format!("{prefix}{name}"), ordered);
        parsed.direction = direction;
        return Ok(parsed);
    }
    let Some(key) = key else {
        // Only modifiers were written. A lone Super is a tap; anything else has no key to wait for.
        if ordered.len() == 1 && ordered[0] == "Super" {
            let mut parsed = ParsedTrigger::blank(TriggerClass::Key, "Super".to_owned(), Vec::new());
            parsed.key = Some("Super".to_owned());
            return Ok(parsed);
        }
        return fail(
            what,
            trigger,
            last,
            if ordered.len() == 1 {
                "a lone modifier is only a trigger for Super (a tap)"
            } else {
                "ends with a modifier and has no key"
            },
        );
    };
    let mut parsed = ParsedTrigger::blank(TriggerClass::Key, format!("{prefix}{key}"), ordered);
    parsed.key = Some(key);
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(trigger: &str) -> String {
        parse(trigger, "trigger").expect("accepted").text
    }

    fn refusal(trigger: &str) -> String {
        parse(trigger, "trigger").expect_err("refused").message
    }

    #[test]
    fn two_spellings_of_a_trigger_are_one_text() {
        assert_eq!(text("super+return"), "Super+Return");
        assert_eq!(text("Super+Shift+1"), "Shift+Super+1");
        assert_eq!(text("control+alt+delete"), "Ctrl+Alt+Delete");
        assert_eq!(text("Primary+q"), "Ctrl+q");
        assert_eq!(text("Super+Q"), "Super+q");
        assert_eq!(text("Super+pagedown"), "Super+Page_Down");
        assert_eq!(text("Logo+Return"), "Super+Return");
        assert_eq!(text("Mod1+Tab"), "Alt+Tab");
        assert_eq!(text("super+f12"), "Super+F12");
        assert_eq!(text("Super+MouseLeft+Click"), "Super+MouseLeft");
        assert_eq!(text("super+mouseleft+drag"), "Super+MouseLeft+Drag");
        assert_eq!(text("Super+wheelup"), "Super+WheelUp");
        assert_eq!(text("swipe4"), "Swipe4");
        assert_eq!(text("pinch2in"), "Pinch2In");
        assert_eq!(text("Super"), "Super");
    }

    #[test]
    fn a_trigger_asks_the_seat_for_the_compositors_spelling() {
        let claim = |trigger: &str| claim_of(&parse(trigger, "trigger").expect("accepted"));
        assert_eq!(
            claim("Super+Shift+1"),
            Some(SeatClaim::Chord {
                text: "<Shift><Super>1".to_owned(),
                grab: false
            })
        );
        assert_eq!(
            claim("Ctrl+Alt+Delete"),
            Some(SeatClaim::Chord {
                text: "<Control><Alt>Delete".to_owned(),
                grab: false
            })
        );
        assert_eq!(claim("Super+MouseLeft+Drag"), claim("Super+MouseLeft"));
        assert_eq!(
            claim("Super+MouseLeft"),
            Some(SeatClaim::Chord {
                text: "<Super>Button1".to_owned(),
                grab: true
            })
        );
        assert_eq!(
            claim("Swipe3Left"),
            Some(SeatClaim::Gesture {
                gesture: "swipe",
                fingers: 3
            })
        );
        assert_eq!(claim("Super+Wheel"), None);
    }

    #[test]
    fn a_refusal_names_the_token() {
        assert_eq!(
            refusal("Super+"),
            "trigger: an empty part after \"Super\"; write a literal plus as \"plus\" (token \"\" in \"Super+\")"
        );
        assert_eq!(
            refusal("Super+!"),
            "trigger: is a character, not a key name; write the keysym name (exclam, minus, plus) or Shift with the key (token \"!\" in \"Super+!\")"
        );
        assert_eq!(
            refusal("Swipe6"),
            "trigger: a gesture has 2 to 5 fingers, not 6 (token \"Swipe6\" in \"Swipe6\")"
        );
        assert_eq!(
            refusal("Pinch3Left"),
            "trigger: a pinch goes in, out, not left (token \"Pinch3Left\" in \"Pinch3Left\")"
        );
        assert_eq!(refusal(""), "trigger must be a trigger such as \"Super+Return\", got ");
        assert_eq!(parse("Super+Super+1", "x").expect_err("refused").token, "Super");
    }
}
