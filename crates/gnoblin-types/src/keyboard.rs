//! The keyboard and button values every input path passes around: a key code, a
//! keysym, a modifier set, and the state of a key or a button.
//!
//! `gnoblin-input` (the chord grammar and the delivery router) and `gnoblin-a11y`
//! (sticky, slow and bounce keys, mouse keys, the screen reader's grab table) both
//! own a copy of these values in the Mutter tree, and neither copy can be the one
//! the other uses: a chord parsed by one is compared against an event mask built
//! by the other, and an accessibility machine hands back a mask the router then
//! delivers to a client. A spelling difference becomes a bug, so the values live
//! here and both crates name these.
//!
//! # The modifier bit layout
//!
//! [`Modifiers`] is one bit set over a fixed mask, and the mask is the one the
//! compositor's key events already carry:
//!
//! | Bits | What they are |
//! | --- | --- |
//! | `1 << 0` to `1 << 7` | XKB's eight real modifiers: shift, lock, control, mod1 to mod5 |
//! | `1 << 26`, `1 << 27`, `1 << 28` | the virtual Super, Hyper and Meta modifiers |
//!
//! Both halves are values a key event can carry, not a private encoding:
//! Clutter's event state (what Mutter, and therefore the tree this replaces,
//! reports for a key) is exactly this mask, and the departing `state` field on the
//! wire carries it. So a chord parsed into [`Modifiers`] is compared with a live
//! event mask by equality, and the accessibility machines latch and match the same
//! bits the router delivers. Nothing converts between two modifier dialects,
//! because there are no two dialects.
//!
//! The eight real modifiers are the full mask, not the six Gnoblin names, because
//! an event mask legitimately contains Mod2 (Num Lock), Mod3 (Level Three) or Mod5
//! (a layout's group switch): a set that could not represent them would silently
//! drop a bit and make two different masks compare equal. Alias constants
//! ([`Modifiers::ALT`]) name one bit twice rather than costing a bit.
//!
//! # What this module does not decide
//!
//! Which keysym a written key name means (a table, not a rule), which bit a
//! keymap's modifier contributes, and what the written form of a chord is. Those
//! belong to the mechanism layer and to `gnoblin-input`; this module holds the
//! values only, so a policy crate can name a key without depending on libinput or
//! xkbcommon.

use std::fmt;

/// A hardware key code, as libinput and XKB report it.
///
/// The numbers are libinput's `evdev` codes, and the XKB `+8` offset may already
/// be applied: a key code identifies the physical key, so it does not change when
/// the layout does, which is why a held key keeps its target across a layout
/// change and bounce keys recognises a repeat as the same key.
///
/// The width is [`u32`] rather than the `u8` of the X11 core protocol because
/// libinput reports 32-bit codes, and a key code past 255 is still a key: narrowing
/// it would either truncate one into another key's number or invent a refusal the
/// kernel does not have.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct KeyCode(u32);

impl KeyCode {
    /// Wraps a hardware key code.
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw code, for the composition root's key table, a log line or a test.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Display for KeyCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "keycode {}", self.0)
    }
}

/// An XKB keysym: the symbol a key produces in the current layout.
///
/// A keysym is what a key *means*, where a [`KeyCode`] is which key was pressed, so
/// it changes with the layout. The chord grammar compares keysyms (that is what a
/// written chord names), the screen reader grabs keys by keysym
/// (`org.freedesktop.a11y.KeyboardMonitor.SetKeyGrabs`), and the accessibility
/// machines recognise the keypad by keysym. The value is the X11 keysym number,
/// which is what xkbcommon returns, and this crate never interprets it: only a
/// table can say which names exist.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Keysym(u32);

impl Keysym {
    /// Wraps an X11 keysym value.
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw keysym value, for a name table, a wire decoder or a test.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Display for Keysym {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "keysym 0x{:x}", self.0)
    }
}

/// A set of modifier bits, in the layout this module documents.
///
/// The constants name the bits Gnoblin reasons about. A layout that maps a second
/// key to Shift, or remaps Caps Lock to a real modifier, contributes the same bit,
/// so no caller needs a keysym-to-bit table and none can disagree with the layout.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Modifiers(u32);

impl Modifiers {
    /// No modifier at all.
    pub const NONE: Self = Self(0);
    /// The Shift bit.
    pub const SHIFT: Self = Self(1 << 0);
    /// The Lock bit, which is Caps Lock.
    ///
    /// Present because an event mask carries it. Sticky keys never latches a lock
    /// modifier: a latch is a modifier that applies to the next key, and a lock
    /// applies until it is pressed again, so the two cannot be one gesture.
    pub const LOCK: Self = Self(1 << 1);
    /// The Control bit.
    pub const CONTROL: Self = Self(1 << 2);
    /// The Mod1 bit. On every layout Gnoblin ships, this is Alt.
    pub const MOD1: Self = Self(1 << 3);
    /// The Mod2 bit. Num Lock on every layout Gnoblin ships.
    pub const MOD2: Self = Self(1 << 4);
    /// The Mod3 bit. Level Three (AltGr) on most layouts.
    pub const MOD3: Self = Self(1 << 5);
    /// The Mod4 bit.
    pub const MOD4: Self = Self(1 << 6);
    /// The Mod5 bit.
    pub const MOD5: Self = Self(1 << 7);
    /// The virtual Super bit, usually the Windows-logo key.
    pub const SUPER: Self = Self(1 << 26);
    /// The virtual Hyper bit.
    pub const HYPER: Self = Self(1 << 27);
    /// The virtual Meta bit.
    ///
    /// Distinct from Alt and Super on purpose: a chord written `<Meta>q` must not
    /// fire on `<Alt>q`, and merging the two bits would make it impossible to say
    /// which the user meant.
    pub const META: Self = Self(1 << 28);
    /// The bit Alt is: Mod1.
    pub const ALT: Self = Self::MOD1;

    /// Wraps a raw mask, for an event the mechanism layer decoded.
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw mask, for a wire format, an event state or a log line.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Whether no modifier is set.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether every bit of `other` is set here.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether any bit of `other` is set here.
    #[must_use]
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    /// The bits of `other` that are also set here.
    #[must_use]
    pub const fn intersect(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    /// This set plus `other`.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// This set without the bits of `other`.
    #[must_use]
    pub const fn remove(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }
}

impl fmt::Display for Modifiers {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "modifiers 0x{:08x}", self.0)
    }
}

/// Which half of a key press and release pair an edge is.
///
/// The two names are one value rather than the `bool` each crate used, because a
/// boolean field forces every caller to remember which way round it is at the call
/// site and lets a routing table equate "released" with "not pressed".
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum KeyState {
    /// The key went down.
    Pressed,
    /// The key came up.
    Released,
}

impl KeyState {
    /// Whether this is the release half.
    #[must_use]
    pub const fn is_release(self) -> bool {
        matches!(self, Self::Released)
    }
}

impl fmt::Display for KeyState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Pressed => "pressed",
            Self::Released => "released",
        })
    }
}

/// Which half of a button press and release pair an edge is.
///
/// A separate type from [`KeyState`] so a pointer edge cannot be routed as a key
/// edge: the two share their spelling and nothing else.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ButtonState {
    /// The button went down.
    Pressed,
    /// The button came up.
    Released,
}

impl ButtonState {
    /// Whether this is the release half.
    #[must_use]
    pub const fn is_release(self) -> bool {
        matches!(self, Self::Released)
    }
}

impl fmt::Display for ButtonState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Pressed => "pressed",
            Self::Released => "released",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{ButtonState, KeyCode, KeyState, Keysym, Modifiers};

    #[test]
    fn the_named_modifiers_are_the_documented_bits() {
        assert_eq!(Modifiers::SHIFT.bits(), 1 << 0);
        assert_eq!(Modifiers::LOCK.bits(), 1 << 1);
        assert_eq!(Modifiers::CONTROL.bits(), 1 << 2);
        assert_eq!(Modifiers::MOD1.bits(), 1 << 3);
        assert_eq!(Modifiers::MOD5.bits(), 1 << 7);
        assert_eq!(Modifiers::SUPER.bits(), 1 << 26);
        assert_eq!(Modifiers::HYPER.bits(), 1 << 27);
        assert_eq!(Modifiers::META.bits(), 1 << 28);
        assert_eq!(Modifiers::ALT, Modifiers::MOD1);
    }

    #[test]
    fn alt_and_meta_are_not_the_same_chord() {
        // The reason Meta has a bit of its own: a chord that mentions it must not
        // match a mask that only has Alt.
        assert!(!Modifiers::ALT.contains(Modifiers::META));
        assert!(!Modifiers::META.intersects(Modifiers::SUPER.union(Modifiers::ALT)));
    }

    #[test]
    fn a_real_modifier_outside_the_six_names_survives() {
        let num_lock = Modifiers::MOD2;
        let measured = Modifiers::SHIFT.union(num_lock);
        assert!(measured.contains(num_lock));
        assert_eq!(measured.intersect(num_lock), num_lock);
        assert_eq!(measured.remove(num_lock), Modifiers::SHIFT);
        assert_ne!(measured, Modifiers::SHIFT, "a set that drops Mod2 would compare equal");
    }

    #[test]
    fn every_copies_the_same_values() {
        assert_eq!(KeyCode::new(94).get(), 94);
        assert_eq!(Keysym::new(0xff1b).get(), 0xff1b);
        assert!(KeyState::Released.is_release());
        assert!(!KeyState::Pressed.is_release());
        assert!(ButtonState::Released.is_release());
        assert!(!ButtonState::Pressed.is_release());
    }
}
