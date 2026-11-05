//! Mouse keys: the keypad as a pointer.
//!
//! The keypad moves the pointer while a movement key is held, accelerates from
//! rest, and clicks with the keys the reference uses. Everything is integer
//! arithmetic over the compositor's milliseconds, with a sub-pixel remainder
//! carried between moves so a slow setting still moves at all.

use crate::ids::Keysym;
use crate::{ButtonAction, Effect, Key, MouseKeysConfig, PointerButton};

/// The interval between pointer moves while a movement key is held, in
/// milliseconds: the same 10 Hz cadence the reference uses.
const MOTION_INTERVAL_MS: u64 = 100;

/// X11 keysyms from `keysymdef.h`, as numbers so a match arm is plain.
const KP_HOME: u32 = 0xff95;
const KP_LEFT: u32 = 0xff96;
const KP_UP: u32 = 0xff97;
const KP_RIGHT: u32 = 0xff98;
const KP_DOWN: u32 = 0xff99;
const KP_PAGE_UP: u32 = 0xff9a;
const KP_PAGE_DOWN: u32 = 0xff9b;
const KP_END: u32 = 0xff9c;
const KP_BEGIN: u32 = 0xff9d;
const KP_INSERT: u32 = 0xff9e;
const KP_DELETE: u32 = 0xff9f;
const KP_MULTIPLY: u32 = 0xffaa;
const KP_ADD: u32 = 0xffab;
const KP_SUBTRACT: u32 = 0xffad;
const KP_DECIMAL: u32 = 0xffae;
const KP_DIVIDE: u32 = 0xffaf;
const KP_0: u32 = 0xffb0;
const KP_1: u32 = 0xffb1;
const KP_2: u32 = 0xffb2;
const KP_3: u32 = 0xffb3;
const KP_4: u32 = 0xffb4;
const KP_5: u32 = 0xffb5;
const KP_6: u32 = 0xffb6;
const KP_7: u32 = 0xffb7;
const KP_8: u32 = 0xffb8;
const KP_9: u32 = 0xffb9;

/// Which way a held movement key moves the pointer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Direction {
    x: i8,
    y: i8,
}

/// The direction of a keypad movement key, or `None` for every other key.
///
/// Both the numeric keypad names and the navigation names are accepted, because
/// which keysym the key produces depends on Num Lock and on the layout, and the
/// reference accepts both.
fn direction(keysym: Keysym) -> Option<Direction> {
    let direction = match keysym.get() {
        KP_7 | KP_HOME => Direction { x: -1, y: -1 },
        KP_8 | KP_UP => Direction { x: 0, y: -1 },
        KP_9 | KP_PAGE_UP => Direction { x: 1, y: -1 },
        KP_4 | KP_LEFT => Direction { x: -1, y: 0 },
        KP_6 | KP_RIGHT => Direction { x: 1, y: 0 },
        KP_1 | KP_END => Direction { x: -1, y: 1 },
        KP_2 | KP_DOWN => Direction { x: 0, y: 1 },
        KP_3 | KP_PAGE_DOWN => Direction { x: 1, y: 1 },
        _ => return None,
    };
    Some(direction)
}

/// Whether this key does something to the pointer's buttons.
fn is_button_key(keysym: Keysym) -> bool {
    matches!(
        keysym.get(),
        KP_DIVIDE | KP_MULTIPLY | KP_SUBTRACT | KP_ADD | KP_5 | KP_BEGIN | KP_0 | KP_INSERT | KP_DECIMAL | KP_DELETE
    )
}

/// Mouse keys: keypad motion, acceleration and clicks.
pub struct MouseKeys {
    config: MouseKeysConfig,
    /// The direction of the movement key that is down, if any.
    direction: Option<Direction>,
    /// When the movement key went down, which is where acceleration starts.
    press_at_ms: u64,
    /// When the last move happened, so the next one moves only its own span.
    last_ms: u64,
    /// When the next move is due, or `None` when the loop is stopped.
    next_ms: Option<u64>,
    /// Sub-pixel motion carried between moves, so a 10 pixel per second setting
    /// still moves one pixel a second. Both axes move the same distance.
    carry: i64,
    /// The button the keypad clicks with.
    button: PointerButton,
    /// Which buttons the keypad is holding down.
    held: [bool; 3],
}

impl MouseKeys {
    /// A machine for this configuration.
    #[must_use]
    pub fn new(config: MouseKeysConfig) -> Self {
        Self {
            config,
            direction: None,
            press_at_ms: 0,
            last_ms: 0,
            next_ms: None,
            carry: 0,
            button: PointerButton::Primary,
            held: [false; 3],
        }
    }

    /// Applies a configuration change. Turning the feature off releases any
    /// button it is holding, which is the one effect this machine has to make
    /// happen when nobody is pressing a key.
    pub fn configure(&mut self, config: MouseKeysConfig, effects: &mut Vec<Effect>) {
        if self.config.enabled && !config.enabled {
            self.disable(effects);
        }
        self.config = config;
    }

    /// Stops the motion loop and lets go of every held button.
    ///
    /// A held button across a session lock or a settings toggle would click the
    /// lock screen as soon as the pointer entered a widget.
    pub fn disable(&mut self, effects: &mut Vec<Effect>) {
        self.stop();
        for button in [PointerButton::Primary, PointerButton::Middle, PointerButton::Secondary] {
            if self.held[button.slot()] {
                self.release_button(button, effects);
            }
        }
    }

    /// Handles a key press and returns whether mouse keys took it, in which case
    /// the caller must not deliver it.
    pub fn press(&mut self, key: &Key, num_lock: bool, now_ms: u64, effects: &mut Vec<Effect>) -> bool {
        if !self.config.enabled || num_lock {
            return false;
        }
        match key.keysym.get() {
            KP_DIVIDE => {
                self.button = PointerButton::Primary;
                true
            }
            KP_MULTIPLY => {
                self.button = PointerButton::Middle;
                true
            }
            KP_SUBTRACT => {
                self.button = PointerButton::Secondary;
                true
            }
            KP_5 | KP_BEGIN => {
                self.click(effects);
                true
            }
            KP_ADD => {
                self.click(effects);
                self.click(effects);
                true
            }
            KP_0 | KP_INSERT => {
                self.press_button(effects);
                true
            }
            KP_DECIMAL | KP_DELETE => {
                self.release_button(self.button, effects);
                true
            }
            keysym => {
                let Some(direction) = direction(Keysym::new(keysym)) else {
                    return false;
                };
                self.start(direction, now_ms, effects);
                true
            }
        }
    }

    /// Handles a key release and returns whether mouse keys took it.
    pub fn release(&mut self, key: &Key, num_lock: bool) -> bool {
        if !self.config.enabled || num_lock {
            return false;
        }
        if direction(key.keysym).is_some() {
            self.stop();
            return true;
        }
        is_button_key(key.keysym)
    }

    /// Moves the pointer when the loop is due.
    pub fn tick(&mut self, now_ms: u64, effects: &mut Vec<Effect>) {
        let Some(next_ms) = self.next_ms else {
            return;
        };
        let Some(direction) = self.direction else {
            self.next_ms = None;
            return;
        };
        if now_ms < next_ms {
            return;
        }
        let (dx, dy) = self.advance(direction, now_ms);
        if dx != 0 || dy != 0 {
            effects.push(Effect::PointerMotion { dx, dy });
        }
        let next_ms = now_ms.saturating_add(MOTION_INTERVAL_MS);
        self.next_ms = Some(next_ms);
        effects.push(Effect::AwaitTick { at_ms: next_ms });
    }

    /// Starts or redirects the motion loop.
    ///
    /// Pressing a second movement key changes direction without restarting the
    /// clock, so a diagonal walk keeps accelerating.
    fn start(&mut self, direction: Direction, now_ms: u64, effects: &mut Vec<Effect>) {
        self.direction = Some(direction);
        if self.next_ms.is_some() {
            return;
        }
        self.press_at_ms = now_ms;
        self.last_ms = now_ms.saturating_add(self.config.initial_delay_ms);
        self.carry = 0;
        let first_ms = self.last_ms;
        self.next_ms = Some(first_ms);
        effects.push(Effect::AwaitTick { at_ms: first_ms });
    }

    /// Stops the motion loop, keeping the pointer where it is.
    fn stop(&mut self) {
        self.direction = None;
        self.next_ms = None;
        self.carry = 0;
    }

    /// The distance this move covers: the speed the ramp has reached times the
    /// time since the last move, plus whatever was left over from last time.
    ///
    /// The ramp starts when the initial delay ends, not when the key goes down,
    /// so the pointer leaves rest slowly instead of arriving at full speed by
    /// the time the delay is over.
    fn advance(&mut self, direction: Direction, now_ms: u64) -> (i64, i64) {
        let acceleration_ms = self.config.acceleration_time_ms.max(1);
        let started_ms = self.press_at_ms.saturating_add(self.config.initial_delay_ms);
        let moving_ms = now_ms.saturating_sub(started_ms).min(acceleration_ms);
        let speed = u64::from(self.config.max_speed) * moving_ms / acceleration_ms;
        let span_ms = now_ms.saturating_sub(self.last_ms);
        self.last_ms = now_ms;
        let covered = i64::try_from(speed.saturating_mul(span_ms)).unwrap_or(i64::MAX);
        let distance = self.carry + covered;
        self.carry = distance.rem_euclid(1000);
        let whole = distance.div_euclid(1000);
        (whole * i64::from(direction.x), whole * i64::from(direction.y))
    }

    /// Press the selected button, then let it up again.
    fn click(&mut self, effects: &mut Vec<Effect>) {
        self.press_button(effects);
        self.release_button(self.button, effects);
    }

    fn press_button(&mut self, effects: &mut Vec<Effect>) {
        if self.held[self.button.slot()] {
            return;
        }
        self.held[self.button.slot()] = true;
        effects.push(Effect::PointerButton {
            button: self.button,
            action: ButtonAction::Press,
        });
    }

    fn release_button(&mut self, button: PointerButton, effects: &mut Vec<Effect>) {
        if !self.held[button.slot()] {
            return;
        }
        self.held[button.slot()] = false;
        effects.push(Effect::PointerButton {
            button,
            action: ButtonAction::Release,
        });
    }
}
