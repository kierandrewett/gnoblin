//! Shared dimensions, colours and rules of the stock window menu: how tall a card is, where it goes, and what a key means to it.

/// The height of an entry, in logical pixels.
pub(crate) const ITEM_H: f64 = 28.0;
/// The height of a separator, in logical pixels.
pub(crate) const SEPARATOR_H: f64 = 9.0;
/// The padding inside a card, in logical pixels.
pub(crate) const PAD: f64 = 4.0;
/// The space between an entry's edge and its label, in logical pixels.
pub(crate) const EDGE: f64 = 8.0;
/// The width of a check or a submenu mark, in logical pixels.
pub(crate) const MARK: f64 = 14.0;
/// The least width of a card, in logical pixels.
pub(crate) const MIN_WIDTH: f64 = 170.0;
/// The kernel code of the left mouse button.
pub(crate) const LEFT: u64 = 272;

/// The colours of the card, as the stock look draws it.
pub(crate) const CARD: &str = "#1e1e2e";
pub(crate) const HOT: &str = "#89b4fa";
pub(crate) const TEXT: &str = "#cdd6f4";
pub(crate) const HOT_TEXT: &str = "#11111b";
pub(crate) const OFF: &str = "#6c7086";
pub(crate) const LINE: &str = "#45475a";
/// The fill of a row nobody is on.
pub(crate) const CLEAR: &str = "#00000000";

/// The height of a card whose entries are separators where `separators` says so: its padding on both sides and the height of each.
pub(crate) fn menu_height(separators: impl IntoIterator<Item = bool>) -> f64 {
    2.0 * PAD
        + separators
            .into_iter()
            .map(|separator| if separator { SEPARATOR_H } else { ITEM_H })
            .sum::<f64>()
}

/// Where a card of `size` goes when it is anchored at `anchor`: its top-left corner clamped inside `area`.
pub(crate) fn place_card(anchor: (f64, f64), size: (f64, f64), area: (f64, f64)) -> (f64, f64) {
    (
        0.0_f64.max(anchor.0.min(area.0 - size.0)),
        0.0_f64.max(anchor.1.min(area.1 - size.1)),
    )
}

/// What a key does in a menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum KeyAction {
    Escape,
    Move(i64),
    Home,
    End,
    Activate,
    Right,
    Left,
    Jump(String),
}

/// Whether JavaScript's `\s` matches the character: what a letter key may not be.
fn js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

/// What the menu does with the key `key` that typed `text`, or `None` when it does not use it. A single character that is not white
/// space is a `Jump`.
pub(crate) fn key_action(key: &str, text: &str) -> Option<KeyAction> {
    Some(match key {
        "Escape" => KeyAction::Escape,
        "Down" | "Tab" => KeyAction::Move(1),
        "Up" | "ISO_Left_Tab" => KeyAction::Move(-1),
        "Home" => KeyAction::Home,
        "End" => KeyAction::End,
        "Return" | "KP_Enter" | "space" => KeyAction::Activate,
        "Right" => KeyAction::Right,
        "Left" => KeyAction::Left,
        _ => {
            let mut chars = text.chars();
            match (chars.next(), chars.next()) {
                (Some(_), None) if !text.chars().all(js_space) => KeyAction::Jump(text.to_owned()),
                _ => return None,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_is_as_tall_as_its_entries() {
        assert_eq!(
            menu_height([false, true, false]),
            2.0 * PAD + 2.0 * ITEM_H + SEPARATOR_H
        );
        assert_eq!(menu_height([]), 2.0 * PAD);
    }

    #[test]
    fn a_card_is_clamped_inside_the_area() {
        let size = (170.0, 220.0);
        let area = (1000.0, 800.0);
        assert_eq!(place_card((300.0, 200.0), size, area), (300.0, 200.0));
        assert_eq!(place_card((990.0, 790.0), size, area), (830.0, 580.0));
        assert_eq!(place_card((-5.0, -5.0), size, area), (0.0, 0.0));
    }

    #[test]
    fn keys_map_to_actions() {
        assert_eq!(key_action("Tab", ""), Some(KeyAction::Move(1)));
        assert_eq!(key_action("ISO_Left_Tab", ""), Some(KeyAction::Move(-1)));
        assert_eq!(key_action("F5", ""), None);
        assert_eq!(key_action("c", "c"), Some(KeyAction::Jump("c".to_owned())));
        assert_eq!(key_action("x", " "), None);
        assert_eq!(key_action("ab", "ab"), None);
    }
}
