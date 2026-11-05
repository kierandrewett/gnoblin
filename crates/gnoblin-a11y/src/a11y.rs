//! The machines together: one event in, one ordered list of effects out.
//!
//! The order below is the whole policy of a key event, and it is the reason the
//! crate is one state machine instead of five:
//!
//! 1. **Mouse keys** run first, because a keypad key must move the pointer
//!    rather than type.
//! 2. **Bounce keys** reject a repeat of the key that was released too recently.
//! 3. **Slow keys** hold the press until its delay elapses; an accepted press
//!    re-enters at step 4.
//! 4. **Sticky keys** withhold a modifier press, decide latch, lock or clear on
//!    its release, and add the latched bits to every other key.
//! 5. **The keyboard monitor** decides who sees the key: the client, a consumer
//!    or both.
//!
//! Sticky keys rewriting before the monitor is what makes a grabbed
//! `Control`-masked keystroke match a chord typed with a latched Control.

use crate::geometry::Point;
use crate::{Appearance, Config, Effect, Event, Follow, Key, KeyRouting, Published, View};

use crate::keyboard::{BounceKeys, SlowKeys, Sticky, StickyKeys};
use crate::magnifier::Magnifier;
use crate::monitor::Monitors;
use crate::mouse_keys::MouseKeys;

/// The accessibility state machines.
pub struct A11y {
    config: Config,
    sticky: StickyKeys,
    slow: SlowKeys,
    bounce: BounceKeys,
    mouse: MouseKeys,
    monitors: Monitors,
    magnifier: Magnifier,
    appearance: Appearance,
    num_lock: bool,
    locked: bool,
    now_ms: u64,
    last_published: Published,
}

impl A11y {
    /// A machine with every feature off, which is what the schema defaults say.
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(Config::default())
    }

    /// A machine for this configuration.
    #[must_use]
    pub fn with_config(config: Config) -> Self {
        Self {
            config,
            sticky: StickyKeys::new(config.sticky_keys),
            slow: SlowKeys::new(config.slow_keys),
            bounce: BounceKeys::new(config.bounce_keys),
            mouse: MouseKeys::new(config.mouse_keys),
            monitors: Monitors::new(),
            magnifier: Magnifier::new(config.magnifier),
            appearance: Appearance::default(),
            num_lock: false,
            locked: false,
            now_ms: 0,
            last_published: Published::default(),
        }
    }

    /// Applies one event and returns the effects, in the order they must happen.
    pub fn apply(&mut self, event: Event) -> Vec<Effect> {
        let mut effects = Vec::new();
        match event {
            Event::Configure(config) => self.configure(config, &mut effects),
            Event::AppearanceChanged {
                reduced_motion,
                high_contrast,
            } => {
                self.appearance = Appearance {
                    reduced_motion,
                    high_contrast,
                };
            }
            Event::KeyPressed(key) => self.press(key, &mut effects),
            Event::KeyReleased(key) => self.release(key, &mut effects),
            Event::Tick { now_ms } => self.tick(now_ms, &mut effects),
            Event::SessionLocked => self.set_locked(true, &mut effects),
            Event::SessionUnlocked => self.set_locked(false, &mut effects),
            Event::NumLockChanged { active } => self.num_lock = active,
            Event::WatchKeyboard { owner } => self.monitors.watch(owner, true),
            Event::UnwatchKeyboard { owner } => self.monitors.watch(owner, false),
            Event::GrabKeyboard { owner } => self.monitors.grab(owner, true),
            Event::UngrabKeyboard { owner } => self.monitors.grab(owner, false),
            Event::SetKeyGrabs {
                owner,
                modifiers,
                keystrokes,
            } => self.monitors.set_grabs(owner, modifiers, keystrokes),
            Event::MonitorDropped { owner } => {
                self.monitors.drop_owner(owner);
                // A grab that ends mid-chord must not leave a latched modifier
                // behind: the modifier press it withheld never reached the
                // client, so nothing else will ever clear it.
                self.sticky.clear();
            }
            Event::OutputsChanged { outputs } => {
                if self.magnifier.set_layout(&outputs) {
                    effects.push(Effect::Magnifier(self.magnifier.view()));
                }
            }
            Event::FocusMoved { at } => self.follow(Follow::Focus, at, &mut effects),
            Event::CaretMoved { at } => self.follow(Follow::Caret, at, &mut effects),
            Event::PointerMoved { at } => self.follow(Follow::Pointer, at, &mut effects),
        }
        self.publish(&mut effects);
        effects
    }

    /// What sticky keys holds now.
    #[must_use]
    pub const fn sticky(&self) -> Sticky {
        self.sticky.state()
    }

    /// The magnifier's view, or `None` when it is off or the layout is unknown.
    #[must_use]
    pub const fn view(&self) -> Option<View> {
        self.magnifier.view()
    }

    /// Whether the session is covered.
    #[must_use]
    pub const fn locked(&self) -> bool {
        self.locked
    }

    /// The live keyboard-monitor owners, in owner order.
    pub fn monitors(&self) -> impl Iterator<Item = crate::ids::OwnerId> + '_ {
        self.monitors.owners()
    }

    /// The state observers see.
    #[must_use]
    pub fn published(&self) -> Published {
        Published {
            appearance: self.appearance,
            sticky: self.sticky.state(),
            slow_keys: self.config.slow_keys.enabled,
            bounce_keys: self.config.bounce_keys.enabled,
            mouse_keys: self.config.mouse_keys.enabled,
            magnifier: self.magnifier.view(),
            locked: self.locked,
        }
    }

    fn configure(&mut self, config: Config, effects: &mut Vec<Effect>) {
        self.sticky.configure(config.sticky_keys);
        if config.slow_keys.enabled != self.config.slow_keys.enabled {
            self.slow.clear();
        }
        self.slow.configure(config.slow_keys);
        self.bounce.configure(config.bounce_keys);
        self.mouse.configure(config.mouse_keys, effects);
        if self.magnifier.configure(config.magnifier) {
            effects.push(Effect::Magnifier(self.magnifier.view()));
        }
        self.config = config;
    }

    fn press(&mut self, key: Key, effects: &mut Vec<Effect>) {
        if self.mouse.press(&key, self.num_lock, self.now_ms, effects) {
            return;
        }
        if self.bounce.press(&key, self.now_ms, effects) {
            return;
        }
        if self.slow.defer(&key, self.now_ms, effects) {
            return;
        }
        self.deliver(key, false, effects);
    }

    fn release(&mut self, key: Key, effects: &mut Vec<Effect>) {
        if self.mouse.release(&key, self.num_lock) {
            return;
        }
        if self.bounce.release(&key, self.now_ms) {
            return;
        }
        if self.slow.reject(&key, effects) {
            return;
        }
        self.deliver(key, true, effects);
    }

    fn tick(&mut self, now_ms: u64, effects: &mut Vec<Effect>) {
        if now_ms > self.now_ms {
            self.now_ms = now_ms;
        }
        for key in self.slow.accept(self.now_ms, effects) {
            self.deliver(key, false, effects);
        }
        if let Some(at_ms) = self.slow.deadline() {
            effects.push(Effect::AwaitTick { at_ms });
        }
        self.mouse.tick(self.now_ms, effects);
    }

    /// Runs the last three stages for a key that survived the timing machines.
    ///
    /// A `None` from sticky keys means the key was withheld: no effect, so the
    /// client cannot see a modifier press it never got a release for.
    fn deliver(&mut self, key: Key, released: bool, effects: &mut Vec<Effect>) {
        self.monitors.note(&key, released);
        let modifiers = if released {
            self.sticky.release(&key, effects)
        } else {
            self.sticky.press(&key, effects)
        };
        let Some(modifiers) = modifiers else {
            return;
        };
        let (decision, monitors) = self.monitors.decide(&key, modifiers);
        effects.push(Effect::Key(KeyRouting {
            released,
            key,
            modifiers,
            decision,
            monitors,
        }));
    }

    fn set_locked(&mut self, locked: bool, effects: &mut Vec<Effect>) {
        self.locked = locked;
        if locked {
            // Nothing latched or waiting may survive the cover: the lock screen
            // would receive it as a chord or a keystroke the user never typed.
            self.sticky.clear();
            self.slow.clear();
            self.bounce.clear();
            self.monitors.clear_held();
            self.mouse.disable(effects);
        }
        if self.magnifier.set_locked(locked) {
            effects.push(Effect::Magnifier(self.magnifier.view()));
        }
    }

    fn follow(&mut self, source: Follow, at: Point, effects: &mut Vec<Effect>) {
        if self.magnifier.follow(source, at) {
            effects.push(Effect::Magnifier(self.magnifier.view()));
        }
    }

    fn publish(&mut self, effects: &mut Vec<Effect>) {
        let published = self.published();
        if published != self.last_published {
            self.last_published = published;
            effects.push(Effect::Publish(published));
        }
    }
}

impl Default for A11y {
    fn default() -> Self {
        Self::new()
    }
}
