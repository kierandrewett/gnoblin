//! Fixtures shared by the integration tests.
//!
//! xkbcommon is not a dependency of this crate, so the tests supply the key-name table the
//! mechanism layer would: the same rule (`xkb_keysym_from_name` with
//! `XKB_KEYSYM_CASE_INSENSITIVE`) over the handful of names these tests use.

#![allow(dead_code)]

use gnoblin_input::chord::{Chord, ChordKind, Keysym, Keysyms, Symbol};
use gnoblin_input::name::Name;
use gnoblin_input::{ButtonState, Event, KeyCode, KeyState, Modifiers};

/// The keysyms the tests use.
pub const Q: Keysym = Keysym::new(0x71);
pub const W: Keysym = Keysym::new(0x77);
pub const E: Keysym = Keysym::new(0x65);

/// The key codes the tests use. They do not have to match the symbols: the point of the
/// routing tests is that a code and a symbol are separate things.
pub const CODE_Q: KeyCode = KeyCode::new(24);
pub const CODE_W: KeyCode = KeyCode::new(25);

/// A key-name table for the tests.
pub struct TestKeysyms;

impl Keysyms for TestKeysyms {
    fn resolve(&self, name: &str) -> Option<Symbol> {
        let lowered = name.to_ascii_lowercase();
        let (symbol, canonical) = match lowered.as_str() {
            "q" => (0x71, "q"),
            "w" => (0x77, "w"),
            "e" => (0x65, "e"),
            "return" => (0xff0d, "Return"),
            "f9" => (0xffc6, "F9"),
            "space" => (0x20, "space"),
            "super_l" => (0xffeb, "Super_L"),
            "xf86audiomute" => (0x1008_ff13, "XF86AudioMute"),
            "xf86audioraisevolume" => (0x1008_ff12, "XF86AudioRaiseVolume"),
            _ => return None,
        };
        Some(Symbol::new(Keysym::new(symbol), canonical))
    }
}

/// Parse a keyboard chord with the test table.
pub fn chord(text: &str) -> Chord {
    Chord::parse(text, ChordKind::Key, &TestKeysyms).expect(text)
}

/// Parse a pointer chord with the test table.
pub fn pointer_chord(text: &str) -> Chord {
    Chord::parse(text, ChordKind::Pointer, &TestKeysyms).expect(text)
}

/// Validate a binding name.
pub fn name(text: &str) -> Name {
    Name::new(text).expect(text)
}

/// A key press or release.
pub fn key_event(code: KeyCode, symbol: Keysym, modifiers: Modifiers, state: KeyState) -> Event {
    Event::Key {
        key_code: code,
        symbol,
        modifiers,
        state,
        repeat: false,
    }
}

/// A key press.
pub fn press(code: KeyCode, symbol: Keysym, modifiers: Modifiers) -> Event {
    key_event(code, symbol, modifiers, KeyState::Pressed)
}

/// A key release.
pub fn release(code: KeyCode, symbol: Keysym, modifiers: Modifiers) -> Event {
    key_event(code, symbol, modifiers, KeyState::Released)
}

/// A button press or release.
pub fn button_event(button: u8, modifiers: Modifiers, state: ButtonState) -> Event {
    Event::Button {
        button,
        modifiers,
        state,
    }
}
