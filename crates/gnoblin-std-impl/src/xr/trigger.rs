//! The grammar of an XR trigger: the buttons of one hand held together, then `Drag` (`Grip+Drag`, `Right.Grip+Drag`,
//! `Trigger+Grip+Drag`), and the five buttons a controller node says are down.

use gnoblin_object::{Fail, Res, Val};

/// The buttons in the order a trigger is spelt and a controller node lists its inputs.
pub(crate) const BUTTONS: [&str; 5] = ["Trigger", "Grip", "Thumbstick", "Primary", "Secondary"];

/// The inputs a shell may claim, in the order the node lists them: the buttons in lower case.
pub(crate) const INPUTS: [&str; 5] = ["trigger", "grip", "thumbstick", "primary", "secondary"];

/// The reading of an XR trigger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ControllerTrigger {
    /// The canonical spelling.
    pub(crate) text: String,
    /// `Left`, `Right`, or none for either hand.
    pub(crate) hand: Option<&'static str>,
    /// The buttons, as indices into [`BUTTONS`], in canonical order.
    pub(crate) buttons: Vec<usize>,
}

impl ControllerTrigger {
    /// The `controller_trigger` record.
    pub(crate) fn val(&self) -> Val {
        Val::Record {
            class: 0,
            fields: vec![
                Val::Text(self.text.clone()),
                Val::Text("controller".to_owned()),
                Val::Text(self.hand.unwrap_or_default().to_owned()),
                Val::List(
                    self.buttons
                        .iter()
                        .map(|at| Val::Text(BUTTONS[*at].to_owned()))
                        .collect(),
                ),
            ],
        }
    }

    /// The inputs a drag of this trigger claims by default: its buttons in lower case.
    pub(crate) fn inputs(&self) -> Vec<String> {
        self.buttons.iter().map(|at| INPUTS[*at].to_owned()).collect()
    }

    /// Whether a controller of `hand` with exactly the buttons `down` (indices into [`BUTTONS`]) holds this trigger.
    pub(crate) fn matches(&self, hand: &str, down: &[usize]) -> bool {
        if let Some(wanted) = self.hand
            && !wanted.eq_ignore_ascii_case(hand)
        {
            return false;
        }
        self.held(down)
    }

    /// Whether exactly these buttons are down.
    pub(crate) fn held(&self, down: &[usize]) -> bool {
        self.buttons.len() == down.len() && self.buttons.iter().all(|button| down.contains(button))
    }
}

/// Reads `trigger`. `what` names it in a refusal: `trigger`, or `SpatialLayout trigger "rotate"`.
///
/// # Errors
///
/// A `TypeError` that names the wrong token.
pub(crate) fn parse(trigger: &str, what: &str) -> Res<ControllerTrigger> {
    if trigger.is_empty() {
        return Err(Fail::type_error(format!(
            "{what} must be an XR trigger such as \"Grip+Drag\", got an empty string"
        )));
    }
    let parts: Vec<&str> = trigger.split('+').collect();
    let last = parts.last().copied().unwrap_or_default();
    if !last.eq_ignore_ascii_case("drag") {
        return Err(Fail::type_error(format!(
            "{what}: an XR trigger ends in Drag (token \"{last}\" in \"{trigger}\")"
        )));
    }
    let mut buttons: Vec<usize> = Vec::new();
    let mut hand: Option<&'static str> = None;
    for token in &parts[..parts.len() - 1] {
        let (prefix, name) = match token.split_once('.') {
            Some((prefix, name)) => (Some(prefix.to_ascii_lowercase()), name),
            None => (None, *token),
        };
        if let Some(prefix) = prefix {
            let named = match prefix.as_str() {
                "left" => "Left",
                "right" => "Right",
                _ => {
                    return Err(Fail::type_error(format!(
                        "{what}: \"{token}\" names no hand; write Left. or Right. (in \"{trigger}\")"
                    )));
                }
            };
            if hand.is_some_and(|held| held != named) {
                return Err(Fail::type_error(format!(
                    "{what}: \"{token}\" is on the other hand; one trigger has one hand (in \"{trigger}\")"
                )));
            }
            hand = Some(named);
        }
        let Some(button) = BUTTONS.iter().position(|known| known.eq_ignore_ascii_case(name)) else {
            return Err(Fail::type_error(format!(
                "{what}: \"{token}\" is not an XR button; use Trigger, Grip, Thumbstick, Primary or Secondary (in \"{trigger}\")"
            )));
        };
        if buttons.contains(&button) {
            return Err(Fail::type_error(format!(
                "{what}: \"{token}\" repeats {} (in \"{trigger}\")",
                BUTTONS[button]
            )));
        }
        buttons.push(button);
    }
    if buttons.is_empty() {
        return Err(Fail::type_error(format!(
            "{what}: an XR trigger names at least one button before Drag (in \"{trigger}\")"
        )));
    }
    buttons.sort_unstable();
    let mut text = String::new();
    if let Some(hand) = hand {
        text.push_str(hand);
        text.push('.');
    }
    for at in &buttons {
        text.push_str(BUTTONS[*at]);
        text.push('+');
    }
    text.push_str("Drag");
    Ok(ControllerTrigger { text, hand, buttons })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trigger_reads_in_one_canonical_spelling() {
        for (written, canonical) in [
            ("Grip+Drag", "Grip+Drag"),
            ("grip+trigger+drag", "Trigger+Grip+Drag"),
            ("Right.Grip+Drag", "Right.Grip+Drag"),
            ("Secondary+Primary+Drag", "Primary+Secondary+Drag"),
            ("left.Trigger+Left.Grip+Drag", "Left.Trigger+Grip+Drag"),
        ] {
            assert_eq!(parse(written, "trigger").expect(written).text, canonical);
        }
    }

    #[test]
    fn a_bad_trigger_names_the_token() {
        for (written, token) in [
            ("Grip", "Grip"),
            ("Squeeze+Drag", "Squeeze"),
            ("Grip+Grip+Drag", "Grip"),
            ("Drag", "Drag"),
            ("Middle.Grip+Drag", "Middle.Grip"),
            ("left.Trigger+right.Trigger+Drag", "right.Trigger"),
            ("Super+MouseLeft+Drag", "Super"),
        ] {
            let fail = parse(written, "trigger").expect_err(written);
            assert!(fail.message.contains(token), "{written}: {}", fail.message);
        }
    }

    #[test]
    fn a_hand_and_the_exact_buttons_match() {
        let trigger = parse("Right.Grip+Drag", "trigger").expect("a trigger");
        assert!(trigger.matches("right", &[1]));
        assert!(!trigger.matches("left", &[1]));
        assert!(!trigger.matches("right", &[0, 1]));
        assert_eq!(trigger.inputs(), ["grip"]);
    }
}
