//! A set of modifier bits, distinct from keysyms.
//!
//! Sticky keys works on modifier bits, not on key names: a layout may map a
//! second key to Shift, or remap the Caps Lock key to a modifier, and both
//! contribute the same bit. The compositor's XKB table says which bit a key
//! contributes ([`crate::Key::modifier`]), so this crate needs no keysym-to-bit
//! table of its own and cannot disagree with the input layer about one.

/// A set of XKB modifier bits.
///
/// The constants name the real modifiers in XKB order, which is what
/// `xkb_state` reports and what an event's deprecated `state` field carries on
/// the wire. Lock modifiers (Caps Lock, Num Lock) are deliberately absent:
/// sticky keys cannot latch a lock, and a lock key that a layout remaps to a
/// real modifier still contributes that modifier's bit and so still responds to
/// sticky keys.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Modifiers(u32);

impl Modifiers {
    /// No modifiers at all.
    pub const NONE: Self = Self(0);
    /// The Shift bit.
    pub const SHIFT: Self = Self(1 << 0);
    /// The Control bit.
    pub const CONTROL: Self = Self(1 << 2);
    /// The Mod1 bit, which XKB maps to Alt.
    pub const ALT: Self = Self(1 << 3);
    /// The Mod4 bit, which XKB maps to Super.
    pub const SUPER: Self = Self(1 << 6);
    /// The Mod3 bit, which XKB maps to Hyper on most layouts.
    pub const HYPER: Self = Self(1 << 5);

    /// Wraps a raw XKB mask, for the wire types in `gnoblin-proto`.
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw XKB mask.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Whether no modifier is set.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Every bit of `other` is set here.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// The bits of `other` that are also set here.
    #[must_use]
    pub const fn intersect(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    /// Whether any bit of `other` is set here.
    #[must_use]
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
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
