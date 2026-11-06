//! Sticky, slow and bounce keys: the three keyboard timing machines.
//!
//! Each one has the shape of a state machine with an explicit tick: the
//! compositor's monotonic milliseconds come in as [`Event::Tick`] and as the
//! `now_ms` argument of a key event, and the machine asks for the next instant
//! with [`Effect::AwaitTick`]. There is no timer, no thread and no clock here,
//! so a test drives six hours of hold and release in a loop.
//!
//! [`Event::Tick`]: crate::Event::Tick

use crate::ids::KeyCode;
use crate::{BeepReason, BounceKeysConfig, Effect, Key, Modifiers, SlowKeysConfig, StickyKeysConfig};

/// What sticky keys holds right now.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Sticky {
    /// Whether sticky keys is on. A two-key-off turn disables it at run time
    /// while the configuration still says enabled, because the user asked for
    /// it off and a settings reload that changes nothing must not undo that.
    pub enabled: bool,
    /// Modifiers that apply to the next key press only.
    pub latched: Modifiers,
    /// Modifiers that apply to every key press until the modifier is pressed
    /// again.
    pub locked: Modifiers,
}

impl Sticky {
    /// Every bit sticky keys adds to a key event.
    #[must_use]
    pub const fn effective(self) -> Modifiers {
        self.latched.union(self.locked)
    }
}

/// Sticky keys: latch, lock and clear, one modifier at a time.
///
/// A modifier press is withheld until its release decides what it meant, which
/// is why a modifier key never types into the client on its own. The decision
/// follows the reference: a modifier already locked is cleared, a modifier
/// already latched becomes locked, and a fresh press latches.
pub struct StickyKeys {
    config: StickyKeysConfig,
    enabled: bool,
    latched: Modifiers,
    locked: Modifiers,
    /// Modifier keys whose presses this machine withheld. They are not down as
    /// far as the client is concerned, so their releases are withheld too.
    held: Modifiers,
}

impl StickyKeys {
    /// A machine for this configuration.
    #[must_use]
    pub fn new(config: StickyKeysConfig) -> Self {
        Self {
            config,
            enabled: config.enabled,
            latched: Modifiers::NONE,
            locked: Modifiers::NONE,
            held: Modifiers::NONE,
        }
    }

    /// Applies a configuration change.
    ///
    /// Only a change of `enabled` clears anything: a latch taken under the old
    /// setting must not type into the new one, while retuning the beep in the
    /// middle of a chord must not cancel it.
    pub fn configure(&mut self, config: StickyKeysConfig) {
        let toggled = config.enabled != self.config.enabled;
        self.config = config;
        if toggled {
            self.enabled = config.enabled;
            self.clear();
        }
    }

    /// What sticky keys holds now.
    #[must_use]
    pub const fn state(&self) -> Sticky {
        Sticky {
            enabled: self.enabled,
            latched: self.latched,
            locked: self.locked,
        }
    }

    /// Drops every latch, lock and held modifier.
    ///
    /// A session lock, a consumer going away and a settings toggle all end here,
    /// so one place decides what "no sticky state" means and no path can
    /// remember part of a chord.
    pub fn clear(&mut self) {
        self.latched = Modifiers::NONE;
        self.locked = Modifiers::NONE;
        self.held = Modifiers::NONE;
    }

    /// Handles a key press and returns the mask the client must see, or `None`
    /// when the press is withheld.
    pub fn press(&mut self, key: &Key, effects: &mut Vec<Effect>) -> Option<Modifiers> {
        let Some(bits) = key.modifier else {
            return Some(key.modifiers.union(self.state().effective()));
        };
        if !self.enabled {
            return Some(key.modifiers);
        }
        if self.config.two_key_off && !self.held.is_empty() {
            // Two modifiers pressed together: the user is typing normally, so
            // sticky keys turns itself off and both keys behave as usual. Only
            // a settings change turns it back on.
            self.enabled = false;
            self.clear();
            return Some(key.modifiers);
        }
        self.held = self.held.union(bits);
        if self.config.beep_on_modifier {
            effects.push(Effect::Beep {
                reason: BeepReason::Modifier,
            });
        }
        None
    }

    /// Handles a key release with the same answer as [`Self::press`].
    pub fn release(&mut self, key: &Key, effects: &mut Vec<Effect>) -> Option<Modifiers> {
        let Some(bits) = key.modifier else {
            let modifiers = key.modifiers.union(self.state().effective());
            // The key that used the latch spends it. The release still carries
            // the latch, so the client sees a whole press and release with the
            // modifier held, which is what a chord looks like on the wire.
            self.latched = Modifiers::NONE;
            return Some(modifiers);
        };
        if !self.enabled {
            return Some(key.modifiers);
        }
        let bits = self.held.intersect(bits);
        self.held = self.held.remove(bits);
        if bits.is_empty() {
            // A modifier that went down before sticky keys was on: it was never
            // withheld, so it is not sticky.
            return Some(key.modifiers);
        }
        if self.locked.intersects(bits) {
            self.locked = self.locked.remove(bits);
        } else if self.latched.intersects(bits) {
            self.latched = self.latched.remove(bits);
            self.locked = self.locked.union(bits);
        } else {
            self.latched = self.latched.union(bits);
        }
        if self.config.beep_on_modifier {
            effects.push(Effect::Beep {
                reason: BeepReason::Modifier,
            });
        }
        // The press was withheld, so this release is too.
        None
    }
}

/// A press slow keys is holding until its delay elapses.
struct Pending {
    key: Key,
    deadline_ms: u64,
}

/// Slow keys: a press is accepted only after the key has been held for the
/// configured delay, and a tap that ends sooner is rejected.
pub struct SlowKeys {
    config: SlowKeysConfig,
    pending: Vec<Pending>,
}

impl SlowKeys {
    /// A machine for this configuration.
    #[must_use]
    pub fn new(config: SlowKeysConfig) -> Self {
        Self {
            config,
            pending: Vec::new(),
        }
    }

    /// Applies a configuration change.
    pub fn configure(&mut self, config: SlowKeysConfig) {
        self.config = config;
    }

    /// Drops every held press. A session lock and a settings toggle both call
    /// this, so a press held across either never turns into a keystroke.
    pub fn clear(&mut self) {
        self.pending.clear();
    }

    /// Takes a press into the delay and returns whether it was taken, in which
    /// case the caller must not deliver it.
    ///
    /// A repeat of a press that is already waiting is neither accepted twice nor
    /// reported twice, which is what a held key looks like when the delay is
    /// longer than the repeat interval.
    pub fn defer(&mut self, key: &Key, now_ms: u64, effects: &mut Vec<Effect>) -> bool {
        if !self.config.enabled || self.config.delay_ms == 0 {
            return false;
        }
        if self.pending.iter().any(|pending| pending.key.keycode == key.keycode) {
            return true;
        }
        let deadline_ms = now_ms.saturating_add(self.config.delay_ms);
        self.pending.push(Pending { key: *key, deadline_ms });
        if self.config.beep_on_press {
            effects.push(Effect::Beep {
                reason: BeepReason::Press,
            });
        }
        effects.push(Effect::AwaitTick { at_ms: deadline_ms });
        true
    }

    /// The presses whose delay has elapsed, in the order they were pressed.
    pub fn accept(&mut self, now_ms: u64, effects: &mut Vec<Effect>) -> Vec<Key> {
        let mut accepted = Vec::new();
        let mut index = 0;
        while index < self.pending.len() {
            if self.pending[index].deadline_ms <= now_ms {
                let pending = self.pending.remove(index);
                if self.config.beep_on_accept {
                    effects.push(Effect::Beep {
                        reason: BeepReason::Accept,
                    });
                }
                accepted.push(pending.key);
            } else {
                index += 1;
            }
        }
        accepted
    }

    /// When the earliest held press is due, so the compositor can arm one timer
    /// for all of them.
    #[must_use]
    pub fn deadline(&self) -> Option<u64> {
        self.pending.iter().map(|pending| pending.deadline_ms).min()
    }

    /// Rejects the press this release belongs to and returns whether it did, in
    /// which case the caller must not deliver the release either.
    pub fn reject(&mut self, key: &Key, effects: &mut Vec<Effect>) -> bool {
        let Some(index) = self
            .pending
            .iter()
            .position(|pending| pending.key.keycode == key.keycode)
        else {
            return false;
        };
        self.pending.remove(index);
        if self.config.beep_on_reject {
            effects.push(Effect::Beep {
                reason: BeepReason::Reject,
            });
        }
        true
    }
}

/// Bounce keys: a repeat of the same key inside the window is ignored, and the
/// release that would have ended it is ignored too so the client never sees a
/// release without a press.
pub struct BounceKeys {
    config: BounceKeysConfig,
    /// The key whose release opened the current window, and when it closes.
    window: Option<(KeyCode, u64)>,
    /// The key whose press this machine rejected, while it stays down.
    rejected: Option<KeyCode>,
}

impl BounceKeys {
    /// A machine for this configuration.
    #[must_use]
    pub fn new(config: BounceKeysConfig) -> Self {
        Self {
            config,
            window: None,
            rejected: None,
        }
    }

    /// Applies a configuration change, dropping any window the old setting
    /// opened.
    pub fn configure(&mut self, config: BounceKeysConfig) {
        if config.enabled != self.config.enabled || config.delay_ms != self.config.delay_ms {
            self.clear();
        }
        self.config = config;
    }

    /// Drops the open window and any press this machine rejected. A session lock
    /// calls this, so a window cannot span the cover.
    pub fn clear(&mut self) {
        self.window = None;
        self.rejected = None;
    }

    /// Rejects a repeat and returns whether it did, in which case the caller must
    /// not deliver the press.
    pub fn press(&mut self, key: &Key, now_ms: u64, effects: &mut Vec<Effect>) -> bool {
        if !self.config.enabled || self.config.delay_ms == 0 {
            return false;
        }
        let Some((keycode, until_ms)) = self.window else {
            return false;
        };
        if keycode != key.keycode || now_ms >= until_ms {
            return false;
        }
        if self.config.beep_on_reject {
            effects.push(Effect::Beep {
                reason: BeepReason::Reject,
            });
        }
        self.rejected = Some(key.keycode);
        true
    }

    /// Handles a release: opens the window for this key, or swallows the release
    /// of a press this machine rejected.
    pub fn release(&mut self, key: &Key, now_ms: u64) -> bool {
        if self.rejected == Some(key.keycode) {
            self.rejected = None;
            return true;
        }
        if !self.config.enabled || self.config.delay_ms == 0 {
            self.window = None;
            return false;
        }
        self.window = Some((key.keycode, now_ms.saturating_add(self.config.delay_ms)));
        false
    }
}
