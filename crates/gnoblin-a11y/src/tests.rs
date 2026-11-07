//! One test per failure mode in the crate documentation.
//!
//! Every test drives the machine through [`A11y::apply`] and its queries only:
//! no private field, no timer and no clock. A test here fails when the
//! compositor could type a key it must not, hand a grabbed key to the client, or
//! show an area outside the output.

use crate::geometry::{Point, Rect, Scale};
use crate::ids::{KeyCode, Keysym, OutputId, OwnerId};
use crate::{
    A11y, Appearance, BounceKeysConfig, ButtonState, Config, Effect, Event, Follow, Key, KeyDecision, KeyRouting,
    KeyState, Keystroke, MagnifierArea, MagnifierConfig, Mode, Modifiers, MouseKeysConfig, Output, PointerButton,
    Published, SlowKeysConfig, StickyKeysConfig,
};

const SHIFT_L: u32 = 0xffe1;
const CONTROL_L: u32 = 0xffe3;
const LETTER_A: u32 = 0x61;
const LETTER_B: u32 = 0x62;
const KP_0: u32 = 0xffb0;
const KP_6: u32 = 0xffb6;

const OWNER: OwnerId = OwnerId::new(1);

fn sticky_config() -> Config {
    Config {
        sticky_keys: StickyKeysConfig {
            enabled: true,
            two_key_off: false,
            beep_on_modifier: false,
        },
        ..Config::default()
    }
}

fn two_key_config() -> Config {
    Config {
        sticky_keys: StickyKeysConfig {
            enabled: true,
            two_key_off: true,
            beep_on_modifier: false,
        },
        ..Config::default()
    }
}

fn slow_config() -> Config {
    Config {
        slow_keys: SlowKeysConfig {
            enabled: true,
            delay_ms: 300,
            beep_on_press: false,
            beep_on_accept: false,
            beep_on_reject: false,
        },
        ..Config::default()
    }
}

fn bounce_config() -> Config {
    Config {
        bounce_keys: BounceKeysConfig {
            enabled: true,
            delay_ms: 300,
            beep_on_reject: false,
        },
        ..Config::default()
    }
}

fn mouse_config() -> Config {
    Config {
        mouse_keys: MouseKeysConfig {
            enabled: true,
            max_speed: 1000,
            acceleration_time_ms: 100,
            initial_delay_ms: 100,
        },
        ..Config::default()
    }
}

fn ramp_config() -> Config {
    Config {
        mouse_keys: MouseKeysConfig {
            enabled: true,
            max_speed: 1000,
            acceleration_time_ms: 200,
            initial_delay_ms: 0,
        },
        ..Config::default()
    }
}

fn magnifier_config(follow: Follow) -> Config {
    Config {
        magnifier: MagnifierConfig {
            enabled: true,
            mode: Mode::FullScreen,
            zoom: Scale::integral(2),
            follow,
        },
        ..Config::default()
    }
}

fn key(keycode: u32, keysym: u32) -> Key {
    Key {
        keycode: KeyCode::new(keycode),
        keysym: Keysym::new(keysym),
        modifiers: Modifiers::NONE,
        unichar: 0,
        modifier: None,
    }
}

fn modifier(keycode: u32, keysym: u32, bits: Modifiers) -> Key {
    Key {
        keycode: KeyCode::new(keycode),
        keysym: Keysym::new(keysym),
        modifiers: Modifiers::NONE,
        unichar: 0,
        modifier: Some(bits),
    }
}

fn shift() -> Key {
    modifier(50, SHIFT_L, Modifiers::SHIFT)
}

fn control() -> Key {
    modifier(37, CONTROL_L, Modifiers::CONTROL)
}

fn press(machine: &mut A11y, key: Key) -> Vec<Effect> {
    machine.apply(Event::KeyPressed(key))
}

fn release(machine: &mut A11y, key: Key) -> Vec<Effect> {
    machine.apply(Event::KeyReleased(key))
}

fn tick(machine: &mut A11y, now_ms: u64) -> Vec<Effect> {
    machine.apply(Event::Tick { now_ms })
}

/// Every key event that reached the session, in order.
fn keys(effects: &[Effect]) -> Vec<KeyRouting> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Key(routing) => Some(routing.clone()),
            _ => None,
        })
        .collect()
}

/// Every pointer move mouse keys produced, in order.
fn motions(effects: &[Effect]) -> Vec<(i64, i64)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::PointerMotion { dx, dy } => Some((*dx, *dy)),
            _ => None,
        })
        .collect()
}

/// The magnifier's view from this step's effects.
fn view_of(effects: &[Effect]) -> MagnifierArea {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Magnifier(view) => *view,
            _ => None,
        })
        .expect("the magnifier must publish its view")
}

/// The published state from this step's effects.
fn published_of(effects: &[Effect]) -> Published {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Publish(published) => Some(*published),
            _ => None,
        })
        .expect("the machine must publish its state")
}

fn layout() -> Vec<Output> {
    vec![
        Output {
            id: OutputId::new(1),
            bounds: Rect::new(0, 0, 1920, 1080),
        },
        Output {
            id: OutputId::new(2),
            bounds: Rect::new(1920, 0, 1920, 1080),
        },
    ]
}

// 1. A grab outlives the screen reader.
#[test]
fn a_grab_ends_when_its_owner_drops() {
    let mut machine = A11y::new();
    machine.apply(Event::WatchKeyboard { owner: OWNER });
    machine.apply(Event::GrabKeyboard { owner: OWNER });
    assert_eq!(
        keys(&press(&mut machine, key(38, LETTER_A)))[0].decision,
        KeyDecision::Consumed
    );

    machine.apply(Event::MonitorDropped { owner: OWNER });

    let routed = keys(&press(&mut machine, key(38, LETTER_A)));
    assert_eq!(routed[0].decision, KeyDecision::PassThrough);
    assert!(routed[0].monitors.is_empty());
    assert_eq!(machine.monitors().count(), 0);
}

// 2. A dropped consumer still receives keys.
#[test]
fn a_dropped_owner_receives_nothing() {
    let mut machine = A11y::new();
    machine.apply(Event::WatchKeyboard { owner: OWNER });
    assert_eq!(keys(&press(&mut machine, key(38, LETTER_A)))[0].monitors, vec![OWNER]);

    machine.apply(Event::MonitorDropped { owner: OWNER });

    let routed = keys(&press(&mut machine, key(39, LETTER_B)));
    assert!(routed[0].monitors.is_empty());
}

// 3. A grabbed key also reaches the focused client.
#[test]
fn a_grabbed_key_never_reaches_the_focused_client() {
    let mut machine = A11y::new();
    machine.apply(Event::WatchKeyboard { owner: OWNER });
    machine.apply(Event::GrabKeyboard { owner: OWNER });

    let routed = keys(&press(&mut machine, key(38, LETTER_A)));

    assert_eq!(routed[0].decision, KeyDecision::Consumed);
    assert_eq!(routed[0].monitors, vec![OWNER]);
}

// 4. A watcher without a grab suppresses typing.
#[test]
fn watching_does_not_suppress_the_key() {
    let mut machine = A11y::new();
    machine.apply(Event::WatchKeyboard { owner: OWNER });

    let routed = keys(&press(&mut machine, key(38, LETTER_A)));

    assert_eq!(routed[0].decision, KeyDecision::Both);
    assert_eq!(routed[0].monitors, vec![OWNER]);
}

// 5. Ungrabbing leaves the grab in place.
#[test]
fn ungrab_keyboard_restores_normal_handling() {
    let mut machine = A11y::new();
    machine.apply(Event::GrabKeyboard { owner: OWNER });
    assert_eq!(
        keys(&press(&mut machine, key(38, LETTER_A)))[0].decision,
        KeyDecision::Consumed
    );

    machine.apply(Event::UngrabKeyboard { owner: OWNER });

    let routed = keys(&press(&mut machine, key(38, LETTER_A)));
    assert_eq!(routed[0].decision, KeyDecision::PassThrough);
}

// 6. A grabbed keystroke matches with the wrong modifiers.
#[test]
fn a_grabbed_keystroke_needs_its_modifiers() {
    let mut machine = A11y::new();
    machine.apply(Event::SetKeyGrabs {
        owner: OWNER,
        modifiers: Vec::new(),
        keystrokes: vec![Keystroke {
            keysym: Keysym::new(LETTER_A),
            modifiers: Modifiers::CONTROL,
        }],
    });

    let mut chorded = key(38, LETTER_A);
    chorded.modifiers = Modifiers::CONTROL;
    assert_eq!(keys(&press(&mut machine, chorded))[0].decision, KeyDecision::Consumed);
    assert_eq!(
        keys(&press(&mut machine, key(38, LETTER_A)))[0].decision,
        KeyDecision::PassThrough
    );
}

// 7. A chord pressed while a grabbed modifier is held is not grabbed.
#[test]
fn a_held_grabbed_modifier_grabs_every_key() {
    let mut machine = A11y::new();
    machine.apply(Event::SetKeyGrabs {
        owner: OWNER,
        modifiers: vec![Keysym::new(SHIFT_L)],
        keystrokes: Vec::new(),
    });

    assert_eq!(keys(&press(&mut machine, shift()))[0].decision, KeyDecision::Consumed);
    assert_eq!(
        keys(&press(&mut machine, key(38, LETTER_A)))[0].decision,
        KeyDecision::Consumed
    );

    release(&mut machine, shift());

    assert_eq!(
        keys(&press(&mut machine, key(38, LETTER_A)))[0].decision,
        KeyDecision::PassThrough
    );
}

// 8. Sticky keys leave a modifier latched across a session lock.
#[test]
fn a_session_lock_clears_sticky_modifiers() {
    let mut machine = A11y::with_config(sticky_config());
    press(&mut machine, shift());
    release(&mut machine, shift());
    assert_eq!(machine.sticky().latched, Modifiers::SHIFT);

    let effects = machine.apply(Event::SessionLocked);

    assert!(machine.sticky().latched.is_empty());
    assert!(machine.sticky().locked.is_empty());
    let published = published_of(&effects);
    assert!(published.locked);
    assert!(published.sticky.latched.is_empty());
    assert!(keys(&press(&mut machine, key(38, LETTER_A)))[0].modifiers.is_empty());
}

// 9. Sticky keys leave a modifier latched when a consumer drops.
#[test]
fn a_consumer_drop_clears_sticky_modifiers() {
    let mut machine = A11y::with_config(sticky_config());
    machine.apply(Event::WatchKeyboard { owner: OWNER });
    press(&mut machine, shift());
    release(&mut machine, shift());
    assert_eq!(machine.sticky().latched, Modifiers::SHIFT);

    machine.apply(Event::MonitorDropped { owner: OWNER });

    assert!(machine.sticky().latched.is_empty());
    assert!(machine.sticky().locked.is_empty());
}

// 10. A latched modifier applies to more than one key.
#[test]
fn a_latched_modifier_applies_to_one_key_only() {
    let mut machine = A11y::with_config(sticky_config());
    press(&mut machine, shift());
    release(&mut machine, shift());

    assert_eq!(
        keys(&press(&mut machine, key(38, LETTER_A)))[0].modifiers,
        Modifiers::SHIFT
    );
    assert_eq!(
        keys(&release(&mut machine, key(38, LETTER_A)))[0].modifiers,
        Modifiers::SHIFT
    );
    assert!(machine.sticky().latched.is_empty());
    assert!(keys(&press(&mut machine, key(39, LETTER_B)))[0].modifiers.is_empty());
}

// 11. A second press latches instead of locking.
#[test]
fn a_second_press_locks_the_modifier() {
    let mut machine = A11y::with_config(sticky_config());
    press(&mut machine, shift());
    release(&mut machine, shift());
    press(&mut machine, shift());
    release(&mut machine, shift());

    assert!(machine.sticky().latched.is_empty());
    assert_eq!(machine.sticky().locked, Modifiers::SHIFT);
}

// 12. A third press does not clear the lock.
#[test]
fn a_third_press_clears_the_lock() {
    let mut machine = A11y::with_config(sticky_config());
    for _ in 0..3 {
        press(&mut machine, shift());
        release(&mut machine, shift());
    }

    assert!(machine.sticky().latched.is_empty());
    assert!(machine.sticky().locked.is_empty());
}

// 13. A locked modifier is spent by the next key.
#[test]
fn a_locked_modifier_applies_to_every_key() {
    let mut machine = A11y::with_config(sticky_config());
    for _ in 0..2 {
        press(&mut machine, shift());
        release(&mut machine, shift());
    }

    assert_eq!(
        keys(&press(&mut machine, key(38, LETTER_A)))[0].modifiers,
        Modifiers::SHIFT
    );
    release(&mut machine, key(38, LETTER_A));
    assert_eq!(
        keys(&press(&mut machine, key(39, LETTER_B)))[0].modifiers,
        Modifiers::SHIFT
    );
}

// 14. Two modifiers pressed together latch instead of turning sticky keys off.
#[test]
fn two_modifiers_together_turn_sticky_keys_off() {
    let mut machine = A11y::with_config(two_key_config());
    assert!(keys(&press(&mut machine, shift())).is_empty());

    let routed = keys(&press(&mut machine, control()));

    assert_eq!(routed.len(), 1);
    assert!(!machine.sticky().enabled);
    assert!(machine.sticky().latched.is_empty());
    assert!(machine.sticky().locked.is_empty());
}

// 15. A withheld modifier press reaches the client.
#[test]
fn a_withheld_modifier_press_never_reaches_the_client() {
    let mut machine = A11y::with_config(sticky_config());

    assert!(keys(&press(&mut machine, shift())).is_empty());
    assert!(keys(&release(&mut machine, shift())).is_empty());
    assert_eq!(machine.sticky().latched, Modifiers::SHIFT);
}

// 16. Slow keys accept a press before the delay.
#[test]
fn slow_keys_hold_the_press_until_the_delay() {
    let mut machine = A11y::with_config(slow_config());
    tick(&mut machine, 1000);

    let effects = press(&mut machine, key(38, LETTER_A));

    assert!(keys(&effects).is_empty());
    assert!(effects.contains(&Effect::AwaitTick { at_ms: 1300 }));
    assert!(keys(&tick(&mut machine, 1299)).is_empty());
    let routed = keys(&tick(&mut machine, 1300));
    assert_eq!(routed.len(), 1);
    assert_eq!(routed[0].state, KeyState::Pressed);
    assert_eq!(routed[0].key.keysym, Keysym::new(LETTER_A));
    assert!(routed[0].modifiers.is_empty());
}

// 17. Slow keys accept a tap that ended before the delay.
#[test]
fn a_tap_that_ends_before_the_delay_is_rejected() {
    let mut machine = A11y::with_config(slow_config());
    tick(&mut machine, 1000);
    press(&mut machine, key(38, LETTER_A));

    assert!(keys(&release(&mut machine, key(38, LETTER_A))).is_empty());
    assert!(keys(&tick(&mut machine, 1300)).is_empty());
    assert!(machine.published().slow_keys);
}

// 18. Disabling slow keys delivers the press it was holding.
#[test]
fn disabling_slow_keys_drops_pending_presses() {
    let mut machine = A11y::with_config(slow_config());
    tick(&mut machine, 1000);
    press(&mut machine, key(38, LETTER_A));

    machine.apply(Event::Configure(Config::default()));

    assert!(keys(&tick(&mut machine, 1300)).is_empty());
    assert_eq!(keys(&press(&mut machine, key(39, LETTER_B))).len(), 1);
}

// 19. Bounce keys swallow a different key.
#[test]
fn bounce_keys_admit_a_different_key() {
    let mut machine = A11y::with_config(bounce_config());
    tick(&mut machine, 1000);
    assert_eq!(keys(&press(&mut machine, key(38, LETTER_A))).len(), 1);
    release(&mut machine, key(38, LETTER_A));

    assert_eq!(keys(&press(&mut machine, key(39, LETTER_B))).len(), 1);
}

// 20. Bounce keys accept a repeat inside the window.
#[test]
fn bounce_keys_reject_a_repeat_inside_the_window() {
    let mut machine = A11y::with_config(bounce_config());
    tick(&mut machine, 1000);
    assert_eq!(keys(&press(&mut machine, key(38, LETTER_A))).len(), 1);
    release(&mut machine, key(38, LETTER_A));

    tick(&mut machine, 1100);
    assert!(keys(&press(&mut machine, key(38, LETTER_A))).is_empty());

    tick(&mut machine, 1400);
    assert_eq!(keys(&press(&mut machine, key(38, LETTER_A))).len(), 1);
}

// 21. A bounced press still produces a release.
#[test]
fn a_bounced_press_produces_no_release() {
    let mut machine = A11y::with_config(bounce_config());
    tick(&mut machine, 1000);
    press(&mut machine, key(38, LETTER_A));
    release(&mut machine, key(38, LETTER_A));
    tick(&mut machine, 1100);

    assert!(keys(&press(&mut machine, key(38, LETTER_A))).is_empty());
    assert!(keys(&release(&mut machine, key(38, LETTER_A))).is_empty());
}

// 22. Mouse keys move the pointer before the initial delay.
#[test]
fn mouse_keys_wait_for_the_initial_delay() {
    let mut machine = A11y::with_config(mouse_config());
    tick(&mut machine, 1000);

    let effects = press(&mut machine, key(90, KP_6));

    assert!(keys(&effects).is_empty());
    assert!(effects.contains(&Effect::AwaitTick { at_ms: 1100 }));
    assert!(motions(&tick(&mut machine, 1099)).is_empty());
    assert!(motions(&tick(&mut machine, 1100)).is_empty());
    assert_eq!(motions(&tick(&mut machine, 1200)), vec![(100, 0)]);
}

// 23. Mouse keys jump straight to full speed.
#[test]
fn mouse_keys_accelerate_to_the_maximum_speed() {
    let mut machine = A11y::with_config(ramp_config());
    press(&mut machine, key(90, KP_6));

    assert_eq!(motions(&tick(&mut machine, 100)), vec![(50, 0)]);
    assert_eq!(motions(&tick(&mut machine, 200)), vec![(100, 0)]);
    assert_eq!(motions(&tick(&mut machine, 300)), vec![(100, 0)]);
}

// 24. Mouse keys keep moving after the key is released.
#[test]
fn releasing_the_movement_key_stops_the_pointer() {
    let mut machine = A11y::with_config(ramp_config());
    press(&mut machine, key(90, KP_6));
    tick(&mut machine, 100);

    let effects = release(&mut machine, key(90, KP_6));

    assert!(effects.is_empty());
    assert!(motions(&tick(&mut machine, 200)).is_empty());
    assert!(tick(&mut machine, 300).is_empty());
}

// 25. Mouse keys leave a button pressed when they are disabled.
#[test]
fn disabling_mouse_keys_releases_pressed_buttons() {
    let mut machine = A11y::with_config(mouse_config());
    let effects = press(&mut machine, key(82, KP_0));
    assert!(effects.contains(&Effect::PointerButton {
        button: PointerButton::Primary,
        state: ButtonState::Pressed,
    }));

    let effects = machine.apply(Event::Configure(Config::default()));

    assert!(effects.contains(&Effect::PointerButton {
        button: PointerButton::Primary,
        state: ButtonState::Released,
    }));
}

// 26. Mouse keys intercept the keypad while Num Lock is on.
#[test]
fn mouse_keys_step_aside_for_num_lock() {
    let mut machine = A11y::with_config(mouse_config());
    machine.apply(Event::NumLockChanged { active: true });

    let routed = keys(&press(&mut machine, key(90, KP_6)));

    assert_eq!(routed.len(), 1);
    assert_eq!(routed[0].decision, KeyDecision::PassThrough);
}

// 27. Mouse keys hand their keypad key to the client.
#[test]
fn mouse_keys_swallow_the_keypad() {
    let mut machine = A11y::with_config(mouse_config());

    let effects = press(&mut machine, key(90, KP_6));

    assert!(keys(&effects).is_empty());
}

// 28. The magnifier shows lock-screen content on another output.
#[test]
fn a_locked_magnifier_stays_on_its_output() {
    let mut machine = A11y::with_config(magnifier_config(Follow::Pointer));
    machine.apply(Event::OutputsChanged { outputs: layout() });
    machine.apply(Event::PointerMoved {
        at: Point::new(100, 100),
    });
    assert_eq!(machine.view().expect("a view").output, OutputId::new(1));

    machine.apply(Event::SessionLocked);
    let effects = machine.apply(Event::PointerMoved {
        at: Point::new(2000, 400),
    });

    assert!(!effects.iter().any(|effect| matches!(effect, Effect::Magnifier(_))));
    let view = machine.view().expect("a view");
    assert_eq!(view.output, OutputId::new(1));
    assert!(view.source.is_inside(Rect::new(0, 0, 1920, 1080)));
}

// 29. The magnifier's view escapes its output at an edge.
#[test]
fn the_view_is_clamped_inside_the_output() {
    let mut machine = A11y::with_config(magnifier_config(Follow::Pointer));
    machine.apply(Event::OutputsChanged { outputs: layout() });

    let effects = machine.apply(Event::PointerMoved { at: Point::new(5, 5) });
    assert_eq!(view_of(&effects).source, Rect::new(0, 0, 960, 540));

    let effects = machine.apply(Event::PointerMoved {
        at: Point::new(1919, 1079),
    });
    let view = view_of(&effects);
    assert_eq!(view.source, Rect::new(960, 540, 960, 540));
    assert!(view.source.is_inside(Rect::new(0, 0, 1920, 1080)));
}

// 30. The magnifier keeps a view of an output that is gone.
#[test]
fn a_removed_output_moves_the_view() {
    let mut machine = A11y::with_config(magnifier_config(Follow::Pointer));
    machine.apply(Event::OutputsChanged { outputs: layout() });
    let effects = machine.apply(Event::PointerMoved {
        at: Point::new(2100, 300),
    });
    assert_eq!(view_of(&effects).output, OutputId::new(2));

    let remaining = vec![layout()[0]];
    let effects = machine.apply(Event::OutputsChanged { outputs: remaining });

    let view = view_of(&effects);
    assert_eq!(view.output, OutputId::new(1));
    assert!(view.source.is_inside(Rect::new(0, 0, 1920, 1080)));
}

// 31. A follow source that is not configured moves the view.
#[test]
fn only_the_configured_follow_source_moves_the_view() {
    let mut machine = A11y::with_config(magnifier_config(Follow::Focus));
    machine.apply(Event::OutputsChanged { outputs: layout() });

    assert!(
        !machine
            .apply(Event::PointerMoved {
                at: Point::new(100, 100)
            })
            .iter()
            .any(|effect| matches!(effect, Effect::Magnifier(_)))
    );
    assert!(
        !machine
            .apply(Event::CaretMoved {
                at: Point::new(100, 100)
            })
            .iter()
            .any(|effect| matches!(effect, Effect::Magnifier(_)))
    );
    assert!(
        machine
            .apply(Event::FocusMoved {
                at: Point::new(100, 100)
            })
            .iter()
            .any(|effect| matches!(effect, Effect::Magnifier(_)))
    );
}

// 32. Reduced motion and high contrast are not published to apps.
#[test]
fn appearance_is_published_as_it_changes() {
    let mut machine = A11y::new();

    let effects = machine.apply(Event::AppearanceChanged {
        reduced_motion: true,
        high_contrast: false,
    });
    assert_eq!(
        published_of(&effects).appearance,
        Appearance {
            reduced_motion: true,
            high_contrast: false,
        }
    );

    let effects = machine.apply(Event::AppearanceChanged {
        reduced_motion: true,
        high_contrast: false,
    });
    assert!(!effects.iter().any(|effect| matches!(effect, Effect::Publish(_))));
}
