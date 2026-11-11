//! One test per documented failure mode, in the order the crate docs list them.

mod common;

use common::{CODE_Q, CODE_W, E, Q, W, button_event, chord, key_event, name, pointer_chord, press, release};
use gnoblin_input::{
    Activity, BindingTable, ButtonState, CancelReason, Chord, ChordKind, Contact, ContactPhase, DeviceId, DeviceKey,
    DeviceSelector, Edge, EdgeBand, GestureConfig, GestureEvent, GestureKind, GestureLock, GesturePath, GesturePhase,
    GestureRoute, GestureRouter, GestureSpec, HeldEdge, Insertion, KeyState, Keysym, Modifiers, Name, PinchDirection,
    Recogniser, Routed, Router, Scope, ShortcutInhibit, Target, Tick, Trigger,
};

fn route(router: &mut Router, event: gnoblin_input::Event) -> Routed {
    router.route(event, ShortcutInhibit::Clear)
}

fn swipe_spec(label: &str, fingers: u8) -> GestureSpec {
    GestureSpec {
        name: label.to_owned(),
        gesture: Some(GestureKind::Swipe),
        fingers: Some(fingers),
        path: Some(vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)]),
        command: Some(vec!["kgx".to_owned()]),
        ..GestureSpec::default()
    }
}

fn pinch_spec(label: &str, fingers: u8, direction: PinchDirection) -> GestureSpec {
    GestureSpec {
        name: label.to_owned(),
        gesture: Some(GestureKind::Pinch),
        fingers: Some(fingers),
        direction: Some(direction),
        command: Some(vec!["kgx".to_owned()]),
        ..GestureSpec::default()
    }
}

fn contact(slot: u8, phase: ContactPhase, x: f64, y: f64) -> Contact {
    Contact { slot, phase, x, y }
}

/// A stationary two-finger gather.
fn gather(recogniser: &mut Recogniser, tick: u64) {
    let frame = [
        contact(0, ContactPhase::Begin, 100.0, 100.0),
        contact(1, ContactPhase::Begin, 120.0, 100.0),
    ];
    assert!(
        recogniser.feed(Tick::new(tick), &frame).is_none(),
        "a stationary pair begins nothing"
    );
}

/// A two-finger frame that has moved far enough to recognise a swipe.
fn swipe_frame(recogniser: &mut Recogniser, tick: u64) -> GestureEvent {
    let frame = [
        contact(0, ContactPhase::Update, 100.0, 170.0),
        contact(1, ContactPhase::Update, 120.0, 170.0),
    ];
    recogniser
        .feed(Tick::new(tick), &frame)
        .expect("a 70 unit movement recognises a swipe")
}

#[test]
fn a_chord_is_canonicalised_one_way() {
    let mut table = BindingTable::new();
    table
        .insert(&name("first"), chord("<Super><Shift>q"), Trigger::Press, false)
        .expect("first");

    for spelling in [
        "<shft><super>q",
        "<Shift><Super>Q",
        "<Super><Shift>q",
        "<Shift><Super>q",
    ] {
        let canonical = chord(spelling);
        assert_eq!(canonical.to_string(), "<Shift><Super>q", "{spelling}");
        assert_eq!(canonical, chord("<Super><Shift>q"), "{spelling}");
        assert!(
            table.insert(&name("second"), canonical, Trigger::Press, false).is_err(),
            "{spelling} is the same chord, so it cannot be a second binding"
        );
    }

    // The XF86 prefix rule: one key, one identity, one written form.
    assert_eq!(chord("AudioMute"), chord("XF86AudioMute"));
    assert_eq!(chord("audiomute").to_string(), "XF86AudioMute");

    // Key codes are hexadecimal, written back in lower case.
    assert_eq!(chord("0X3F"), chord("0x3f"));
    assert_eq!(chord("0X3F").to_string(), "0x3f");

    // A pointer chord has its own canonical order and its own identity.
    assert_eq!(pointer_chord("<alt><ctl>Button2").to_string(), "<Control><Alt>Button2");
    assert_ne!(chord("<Super>q"), pointer_chord("<Super>Button1"));
}

#[test]
fn two_bindings_never_shadow_each_other() {
    let mut table = BindingTable::new();
    let added = table
        .insert(&name("close_window"), chord("<Super>q"), Trigger::Press, false)
        .expect("close_window");
    assert!(matches!(added, Insertion::Added(_)));

    let conflict = table
        .insert(&name("quit"), chord("<Super>q"), Trigger::Press, false)
        .expect_err("the chord is taken");
    assert_eq!(conflict.existing_name, name("close_window"));
    assert_eq!(conflict.incoming_name, name("quit"));
    assert_eq!(conflict.chord, chord("<Super>q"));
    let message = conflict.to_string();
    assert!(
        message.contains("close_window") && message.contains("quit"),
        "{message}"
    );

    // The refused declaration changed nothing.
    assert_eq!(table.len(), 1);
    assert_eq!(
        table.lookup(&chord("<Super>q")).expect("still there").name(),
        &name("close_window")
    );

    // Both edges count: one physical press cannot serve a press trigger and a release
    // trigger at once, because a release trigger reserves its press.
    assert!(
        table
            .insert(&name("late"), chord("<Super>q"), Trigger::Release, false)
            .is_err()
    );

    // A later declaration under the same name replaces in place and keeps its identity,
    // so a press held across a reload still names a live binding.
    let replaced = table
        .insert(&name("close_window"), chord("<Super>w"), Trigger::Press, false)
        .expect("replace");
    assert_eq!(replaced, Insertion::Replaced(added.id()));
    assert_eq!(table.len(), 1);
    assert!(table.lookup(&chord("<Super>q")).is_none());
    assert_eq!(
        table.lookup(&chord("<Super>w")).expect("moved").name(),
        &name("close_window")
    );
}

#[test]
fn a_gesture_is_claimed_once_or_not_at_all() {
    let bindings = vec![
        swipe_spec("right", 3).validate().expect("swipe"),
        pinch_spec("out", 3, PinchDirection::Out).validate().expect("pinch"),
    ];
    let mut router = GestureRouter::new(bindings).expect("a swipe and a pinch may share a finger count");

    let begin = GestureEvent::Begin {
        tick: Tick::new(1),
        kind: GestureKind::Swipe,
        fingers: 3,
        edge: Some(Edge::Right),
    };
    assert_eq!(
        router.observe(&begin, GestureLock::Unlocked),
        GestureRoute::Reserved { binding: 0 }
    );
    // No update can hand the gesture to another binding.
    for step in 2..6 {
        let update = GestureEvent::Update {
            tick: Tick::new(step),
            kind: GestureKind::Swipe,
            fingers: 3,
            edge: Some(Edge::Right),
            dx: 10.0,
            dy: 0.0,
            scale: 1.0,
            angle_delta: 0.0,
        };
        assert_eq!(
            router.observe(&update, GestureLock::Unlocked),
            GestureRoute::Reserved { binding: 0 }
        );
    }
    let matching = GestureEvent::End {
        tick: Tick::new(6),
        kind: GestureKind::Swipe,
        fingers: 3,
        edge: Some(Edge::Right),
        path: GesturePath::from_points(vec![(0.0, 0.0), (60.0, 0.0), (60.0, 60.0)]),
        travel: 120.0,
        scale: 1.0,
        duration: 5,
    };
    assert_eq!(
        router.observe(&matching, GestureLock::Unlocked),
        GestureRoute::Fired { binding: 0 }
    );
    assert_eq!(router.reserved(), None);

    // A reserved gesture whose movement does not match runs nothing and is not offered to
    // the other binding, which is the documented behaviour and the reason a second claim
    // would be worse than no claim.
    router.observe(&begin, GestureLock::Unlocked);
    let mismatched = GestureEvent::End {
        tick: Tick::new(9),
        kind: GestureKind::Swipe,
        fingers: 3,
        edge: Some(Edge::Right),
        path: GesturePath::from_points(vec![(0.0, 0.0), (0.0, 60.0), (60.0, 60.0)]),
        travel: 120.0,
        scale: 1.0,
        duration: 2,
    };
    assert_eq!(
        router.observe(&mismatched, GestureLock::Unlocked),
        GestureRoute::MatchedNothing { binding: 0 }
    );
    assert_eq!(router.stats().unmatched, 1);

    // Two declarations that would compete for one movement are refused, not ordered.
    let error = GestureRouter::new(vec![
        swipe_spec("right", 3).validate().expect("first"),
        swipe_spec("left", 3).validate().expect("second"),
    ])
    .expect_err("the second would never win");
    assert!(error.to_string().contains("right") && error.to_string().contains("left"));
}

#[test]
fn a_grab_start_does_not_steal_a_held_keys_release() {
    let mut router = Router::new();
    assert_eq!(
        route(&mut router, press(CODE_Q, Q, Modifiers::NONE)).target,
        Target::Client
    );

    let grab = router.begin_grab(Scope::Both);
    // New input belongs to the grab.
    assert_eq!(
        route(&mut router, press(CODE_W, W, Modifiers::NONE)).target,
        Target::Grab(grab)
    );

    // The release of the key that was already held does not.
    let routed = route(&mut router, release(CODE_Q, Q, Modifiers::NONE));
    assert_eq!(routed.target, Target::Client);
    assert!(routed.followed_press);

    // The key the grab took releases back to the grab.
    assert_eq!(
        route(&mut router, release(CODE_W, W, Modifiers::NONE)).target,
        Target::Grab(grab)
    );
    assert_eq!(router.held(), 0);
    assert_eq!(router.stats().orphan_releases, 0);
    assert!(router.end_grab(grab));
}

#[test]
fn a_consumed_press_keeps_its_release() {
    // A grab that ends before its press is released keeps the release, so the client
    // never receives an orphan release.
    let mut router = Router::new();
    let grab = router.begin_grab(Scope::Keyboard);
    assert_eq!(
        route(&mut router, press(CODE_Q, Q, Modifiers::NONE)).target,
        Target::Grab(grab)
    );
    assert!(router.end_grab(grab));
    assert_eq!(
        route(&mut router, release(CODE_Q, Q, Modifiers::NONE)).target,
        Target::Grab(grab),
        "the grab still owns the release it took"
    );

    // A binding removed by a reload still consumes its held press's release, and does not
    // fire the callback the reload removed.
    let mut router = Router::new();
    let binding = router
        .bind(&name("terminal"), chord("<Super>Return"), Trigger::Press, false)
        .expect("terminal")
        .id();
    let return_press = key_event(
        gnoblin_input::KeyCode::new(28),
        gnoblin_input::Keysym::new(0xff0d),
        Modifiers::SUPER,
        KeyState::Pressed,
    );
    assert_eq!(route(&mut router, return_press).target, Target::Binding(binding));
    assert!(router.bindings_mut().remove(binding).is_some());
    let return_release = key_event(
        gnoblin_input::KeyCode::new(28),
        gnoblin_input::Keysym::new(0xff0d),
        Modifiers::SUPER,
        KeyState::Released,
    );
    let routed = route(&mut router, return_release);
    assert_eq!(routed.target, Target::Binding(binding));
    assert_eq!(routed.fired, None, "a removed binding does not run");
    assert_eq!(router.stats().client_deliveries, 0, "nothing leaked to the client");
}

#[test]
fn a_held_key_keeps_the_target_its_press_chose() {
    // Pressed under a grab: the release stays with the grab even after it ends.
    let mut router = Router::new();
    let grab = router.begin_grab(Scope::Keyboard);
    route(&mut router, press(CODE_Q, Q, Modifiers::NONE));
    router.end_grab(grab);
    assert_eq!(
        route(&mut router, release(CODE_Q, Q, Modifiers::NONE)).target,
        Target::Grab(grab)
    );

    // Pressed under a binding: the release stays with the binding even while a grab that
    // would take a fresh press is active.
    let mut router = Router::new();
    let binding = router
        .bind(&name("q"), chord("q"), Trigger::Press, false)
        .expect("q")
        .id();
    assert_eq!(
        route(&mut router, press(CODE_Q, Q, Modifiers::NONE)).target,
        Target::Binding(binding)
    );
    let grab = router.begin_grab(Scope::Both);
    assert_eq!(
        route(&mut router, press(CODE_W, W, Modifiers::NONE)).target,
        Target::Grab(grab)
    );
    assert_eq!(
        route(&mut router, release(CODE_Q, Q, Modifiers::NONE)).target,
        Target::Binding(binding)
    );
    assert_eq!(
        route(&mut router, release(CODE_W, W, Modifiers::NONE)).target,
        Target::Grab(grab)
    );
    assert_eq!(router.held(), 0);
}

#[test]
fn a_layout_change_under_a_held_key_keeps_its_target() {
    let mut router = Router::new();
    let binding = router
        .bind(&name("q"), chord("q"), Trigger::Press, false)
        .expect("q")
        .id();
    assert_eq!(
        route(&mut router, press(CODE_Q, Q, Modifiers::NONE)).target,
        Target::Binding(binding)
    );

    // The layout changed under the held key: the same hardware code now reports a
    // different symbol. Routing the release by the symbol would miss the binding and leak
    // an orphan release to the client, so ownership is recorded by code.
    let changed_release = release(CODE_Q, E, Modifiers::NONE);
    let routed = router.route(changed_release, ShortcutInhibit::Clear);
    assert_eq!(routed.target, Target::Binding(binding));
    assert!(routed.followed_press);
    assert_eq!(router.held(), 0);

    // The same symbol and modifiers as a fresh press is a different edge: no chord claims
    // it, so it reaches the client.
    assert_eq!(
        route(&mut router, press(CODE_Q, E, Modifiers::NONE)).target,
        Target::Client
    );
    route(&mut router, release(CODE_Q, E, Modifiers::NONE));
}

#[test]
fn a_removed_device_cancels_the_gesture_once() {
    let mut recogniser = Recogniser::new(GestureConfig::default());
    let mut router = GestureRouter::new(vec![swipe_spec("right", 2).validate().expect("swipe")]).expect("one binding");

    gather(&mut recogniser, 1);
    let begin = swipe_frame(&mut recogniser, 2);
    assert_eq!(begin.phase(), GesturePhase::Begin);
    assert_eq!(
        router.observe(&begin, GestureLock::Unlocked),
        GestureRoute::Reserved { binding: 0 }
    );

    let cancel = recogniser
        .device_removed(Tick::new(3))
        .expect("the gesture is cancelled");
    assert!(matches!(
        cancel,
        GestureEvent::Cancel {
            reason: CancelReason::DeviceRemoved,
            ..
        }
    ));
    assert_eq!(
        router.observe(&cancel, GestureLock::Unlocked),
        GestureRoute::Cancelled { binding: Some(0) }
    );
    assert_eq!(router.reserved(), None);
    assert!(
        recogniser.device_removed(Tick::new(4)).is_none(),
        "a second removal invents nothing"
    );
    assert!(!recogniser.is_active(), "no end event is invented for a removed device");
    assert_eq!(recogniser.fingers(), 0);
}

#[test]
fn a_lost_terminal_event_ends_the_gesture() {
    let config = GestureConfig {
        stale_ticks: 10,
        ..GestureConfig::default()
    };
    let mut recogniser = Recogniser::new(config);
    let mut router = GestureRouter::new(vec![swipe_spec("right", 2).validate().expect("swipe")]).expect("one binding");

    gather(&mut recogniser, 1);
    let begin = swipe_frame(&mut recogniser, 2);
    router.observe(&begin, GestureLock::Unlocked);
    assert_eq!(router.reserved(), Some(0));

    // The terminal frame never arrives. The stream expires and the reservation is over,
    // so a shell animation ends instead of waiting forever.
    assert!(recogniser.expire(Tick::new(11)).is_none(), "not yet stale");
    let stale = recogniser.expire(Tick::new(12)).expect("the stream is stale");
    assert!(matches!(
        stale,
        GestureEvent::Cancel {
            reason: CancelReason::Stale,
            ..
        }
    ));
    assert_eq!(
        router.observe(&stale, GestureLock::Unlocked),
        GestureRoute::Cancelled { binding: Some(0) }
    );

    // A gather whose contacts went quiet is dropped too, so the next gesture starts
    // cleanly instead of continuing the old one.
    gather(&mut recogniser, 20);
    assert!(recogniser.expire(Tick::new(30)).is_none());
    assert_eq!(recogniser.fingers(), 0);
    gather(&mut recogniser, 40);
    assert_eq!(
        swipe_frame(&mut recogniser, 41).fingers(),
        2,
        "a new gesture begins normally"
    );
}

#[test]
fn a_late_tick_does_not_move_the_idle_clock_backwards() {
    let mut activity = Activity::new();
    assert!(activity.record(Tick::new(100)));
    assert!(!activity.record(Tick::new(90)), "an out-of-order tick is not progress");
    assert_eq!(activity.last(), Some(Tick::new(100)));
    assert_eq!(activity.idle_ticks(Tick::new(110)), 10);
    assert_eq!(
        activity.idle_ticks(Tick::new(90)),
        0,
        "idle time saturates rather than wrapping"
    );
    assert!(activity.is_idle(Tick::new(110), 10));
    assert!(!activity.is_idle(Tick::new(109), 10));
    assert!(
        activity.is_idle(Tick::new(0), 0),
        "a zero timeout asks whether anything happened"
    );
}

#[test]
fn a_pointer_chord_needs_a_modifier_and_keeps_the_click() {
    let mut router = Router::new();
    assert!(
        Chord::parse("<Super>Button1", ChordKind::Key, &common::TestKeysyms).is_err(),
        "a keyboard chord cannot end in a button"
    );
    assert!(
        Chord::parse("Button1", ChordKind::Pointer, &common::TestKeysyms).is_err(),
        "a pointer chord without a modifier would swallow every click"
    );

    let binding = router
        .bind(&name("resize"), pointer_chord("<Super>Button3"), Trigger::Press, false)
        .expect("resize")
        .id();
    assert_eq!(
        route(&mut router, button_event(3, Modifiers::SUPER, ButtonState::Pressed)).target,
        Target::Binding(binding)
    );
    assert_eq!(
        route(&mut router, button_event(1, Modifiers::NONE, ButtonState::Pressed)).target,
        Target::Client,
        "an ordinary click still reaches the application"
    );
    route(&mut router, button_event(1, Modifiers::NONE, ButtonState::Released));
    assert_eq!(router.held(), 1, "the chord's own button is still down");
    assert_eq!(
        route(&mut router, button_event(3, Modifiers::SUPER, ButtonState::Released)).target,
        Target::Binding(binding),
        "its release belongs to the binding too"
    );
    assert_eq!(router.held(), 0);
}

#[test]
fn an_edge_swipe_is_reported_with_the_edge_it_started_at() {
    let band = EdgeBand {
        width: 1000.0,
        height: 500.0,
        band: 30.0,
    };
    let mut recogniser = Recogniser::new(GestureConfig {
        edge: Some(band),
        ..GestureConfig::default()
    });
    let frame = [
        contact(0, ContactPhase::Begin, 990.0, 250.0),
        contact(1, ContactPhase::Begin, 970.0, 250.0),
    ];
    recogniser.feed(Tick::new(1), &frame);
    let moved = [
        contact(0, ContactPhase::Update, 930.0, 250.0),
        contact(1, ContactPhase::Update, 910.0, 250.0),
    ];
    let begin = recogniser.feed(Tick::new(2), &moved).expect("begin");
    assert_eq!(begin.edge(), Some(Edge::Right));
    assert_eq!(begin.kind(), GestureKind::Swipe);

    // The same movement away from an edge is an ordinary swipe, and a pinch never has an
    // edge even if it starts in the band.
    let mut middle = Recogniser::new(GestureConfig {
        edge: Some(band),
        ..GestureConfig::default()
    });
    let frame = [
        contact(0, ContactPhase::Begin, 500.0, 250.0),
        contact(1, ContactPhase::Begin, 480.0, 250.0),
    ];
    middle.feed(Tick::new(1), &frame);
    let moved = [
        contact(0, ContactPhase::Update, 440.0, 250.0),
        contact(1, ContactPhase::Update, 420.0, 250.0),
    ];
    assert_eq!(middle.feed(Tick::new(2), &moved).expect("begin").edge(), None);

    let mut pinch = Recogniser::new(GestureConfig {
        edge: Some(band),
        ..GestureConfig::default()
    });
    let frame = [
        contact(0, ContactPhase::Begin, 20.0, 250.0),
        contact(1, ContactPhase::Begin, 30.0, 250.0),
    ];
    pinch.feed(Tick::new(1), &frame);
    let spread = [
        contact(0, ContactPhase::Update, -10.0, 250.0),
        contact(1, ContactPhase::Update, 60.0, 250.0),
    ];
    let begin = pinch.feed(Tick::new(2), &spread).expect("begin");
    assert_eq!(begin.kind(), GestureKind::Pinch);
    assert_eq!(begin.edge(), None);
}

#[test]
fn a_name_is_one_rule_everywhere() {
    for valid in ["a", "open_terminal", "open-terminal", "A1"] {
        assert!(Name::new(valid).is_ok(), "{valid}");
    }
    for invalid in ["", "two words", "semi;colon", "a/b", &"n".repeat(65)] {
        assert!(Name::new(invalid).is_err(), "{invalid}");
    }
    // The gesture and keybinding declarations share the rule, so a name accepted in one
    // is accepted in the other.
    let spec = GestureSpec {
        name: "open_terminal".to_owned(),
        ..swipe_spec("x", 2)
    };
    assert!(spec.validate().is_ok());
    let spec = GestureSpec {
        name: "open terminal".to_owned(),
        ..swipe_spec("x", 2)
    };
    assert!(spec.validate().is_err());
}

// Failure modes 13 to 15: device identity (multi-seat phase 1, `design/multi-seat.md`, lanes 1.2 and 1.4).

const DESK: DeviceId = DeviceId::new(1);
const PAD: DeviceId = DeviceId::new(2);

/// A router that knows the two keyboards of these tests by name.
fn two_keyboards() -> Router {
    let mut router = Router::new();
    router.device_added(DESK, "Desk Keyboard");
    router.device_added(PAD, "Macro Pad");
    router
}

fn selector(device: &str) -> DeviceSelector {
    DeviceSelector::new(device).expect(device)
}

/// One key edge on `device`, with the seat's modifiers and, when given, the device's own view.
fn key_on(
    device: DeviceId,
    symbol: Keysym,
    modifiers: Modifiers,
    own: Option<DeviceKey>,
    state: KeyState,
) -> gnoblin_input::Event {
    gnoblin_input::Event::Key {
        device,
        key_code: if symbol == W { CODE_W } else { CODE_Q },
        symbol,
        modifiers,
        own,
        state,
        repeat: false,
    }
}

#[test]
fn two_keyboards_holding_one_key_do_not_collide() {
    let mut router = two_keyboards();
    // Both keyboards hold the same hardware key: the router keeps the two presses apart.
    let desk = route(&mut router, key_on(DESK, Q, Modifiers::NONE, None, KeyState::Pressed));
    let pad = route(&mut router, key_on(PAD, Q, Modifiers::NONE, None, KeyState::Pressed));
    assert_eq!((desk.target, pad.target), (Target::Client, Target::Client));
    assert_eq!(router.held(), 2, "two keyboards, two presses of one key code");
    assert_eq!((router.held_by(DESK), router.held_by(PAD)), (1, 1));
    // The releases arrive in the other order: each finds its own press.
    let pad = route(&mut router, key_on(PAD, Q, Modifiers::NONE, None, KeyState::Released));
    assert!(pad.followed_press);
    assert_eq!(
        router.held_by(DESK),
        1,
        "the pad's release did not take the desk's press"
    );
    let desk = route(&mut router, key_on(DESK, Q, Modifiers::NONE, None, KeyState::Released));
    assert!(desk.followed_press);
    let stats = router.stats();
    assert_eq!(stats.duplicate_presses, 0, "no press was counted twice");
    assert_eq!(stats.orphan_releases, 0, "no release lost its press");
    assert_eq!((stats.presses, stats.releases, stats.followed_press), (2, 2, 2));
    assert_eq!(router.held(), 0);
}

#[test]
fn a_binding_for_one_keyboard_ignores_the_other() {
    let mut router = two_keyboards();
    let bound = router
        .bind_for(
            &name("pad-q"),
            chord("q"),
            Trigger::Press,
            false,
            Some(selector("Macro Pad")),
        )
        .expect("the chord for the pad")
        .id();
    // The same chord on the desk keyboard is a client key.
    let desk = route(&mut router, key_on(DESK, Q, Modifiers::NONE, None, KeyState::Pressed));
    assert_eq!((desk.target, desk.fired), (Target::Client, None));
    let pad = route(&mut router, key_on(PAD, Q, Modifiers::NONE, None, KeyState::Pressed));
    assert_eq!((pad.target, pad.fired), (Target::Binding(bound), Some(bound)));
    // Each release follows its own press, so the pad's is consumed and the desk's reaches the client.
    let pad = route(&mut router, key_on(PAD, Q, Modifiers::NONE, None, KeyState::Released));
    assert_eq!((pad.target, pad.followed_press), (Target::Binding(bound), true));
    let desk = route(&mut router, key_on(DESK, Q, Modifiers::NONE, None, KeyState::Released));
    assert_eq!((desk.target, desk.followed_press), (Target::Client, true));
    let stats = router.stats();
    assert_eq!((stats.duplicate_presses, stats.orphan_releases), (0, 0));
    assert_eq!(stats.fired, 1, "the binding ran once, for the pad");
    // A key from a device nobody announced is no pad either.
    let unknown = route(
        &mut router,
        key_on(DeviceId::VIRTUAL, Q, Modifiers::NONE, None, KeyState::Pressed),
    );
    assert_eq!(unknown.target, Target::Client);
    route(
        &mut router,
        key_on(DeviceId::VIRTUAL, Q, Modifiers::NONE, None, KeyState::Released),
    );
}

#[test]
fn a_binding_for_a_device_beats_one_for_every_device_on_that_device_only() {
    let mut router = two_keyboards();
    let any = router
        .bind(&name("any-q"), chord("q"), Trigger::Press, false)
        .expect("the chord for every device")
        .id();
    let pad_only = router
        .bind_for(
            &name("pad-q"),
            chord("q"),
            Trigger::Press,
            false,
            Some(selector("Macro Pad")),
        )
        .expect("the same chord for the pad is a different binding")
        .id();
    let pad = route(&mut router, key_on(PAD, Q, Modifiers::NONE, None, KeyState::Pressed));
    assert_eq!(pad.target, Target::Binding(pad_only));
    let desk = route(&mut router, key_on(DESK, Q, Modifiers::NONE, None, KeyState::Pressed));
    assert_eq!(desk.target, Target::Binding(any));
    // The same chord for the same device twice is still one meaning too many.
    let conflict = router
        .bind_for(
            &name("again"),
            chord("q"),
            Trigger::Press,
            false,
            Some(selector("Macro Pad")),
        )
        .expect_err("pad Q is taken");
    assert_eq!(conflict.existing_name, name("pad-q"));
}

#[test]
fn a_scoped_binding_reads_its_own_keyboards_modifiers() {
    let mut router = two_keyboards();
    let scoped = router
        .bind_for(
            &name("pad-shift-q"),
            chord("<Shift>q"),
            Trigger::Press,
            false,
            Some(selector("Macro Pad")),
        )
        .expect("Shift+Q for the pad")
        .id();
    let unscoped = router
        .bind(&name("any-shift-w"), chord("<Shift>w"), Trigger::Press, false)
        .expect("Shift+W for every device")
        .id();
    // Shift is held on the desk keyboard, so the seat reports it on the pad's keys too. The pad's own state
    // holds no Shift: its chord does not match, and the key is a client's.
    let pad_own = DeviceKey {
        symbol: Q,
        modifiers: Modifiers::NONE,
    };
    let routed = route(
        &mut router,
        key_on(PAD, Q, Modifiers::SHIFT, Some(pad_own), KeyState::Pressed),
    );
    assert_eq!(
        routed.target,
        Target::Client,
        "a Shift on another keyboard is not this pad's Shift"
    );
    route(
        &mut router,
        key_on(PAD, Q, Modifiers::SHIFT, Some(pad_own), KeyState::Released),
    );
    // With the Shift on the pad itself, the chord matches.
    let pad_shifted = DeviceKey {
        symbol: Q,
        modifiers: Modifiers::SHIFT,
    };
    let routed = route(
        &mut router,
        key_on(PAD, Q, Modifiers::SHIFT, Some(pad_shifted), KeyState::Pressed),
    );
    assert_eq!((routed.target, routed.fired), (Target::Binding(scoped), Some(scoped)));
    route(
        &mut router,
        key_on(PAD, Q, Modifiers::SHIFT, Some(pad_shifted), KeyState::Released),
    );
    // A binding for every device keeps matching what the seat holds, whichever keyboard held the Shift.
    let routed = route(
        &mut router,
        key_on(
            PAD,
            W,
            Modifiers::SHIFT,
            Some(DeviceKey {
                symbol: W,
                modifiers: Modifiers::NONE,
            }),
            KeyState::Pressed,
        ),
    );
    assert_eq!(routed.target, Target::Binding(unscoped));
    let stats = router.stats();
    assert_eq!((stats.duplicate_presses, stats.orphan_releases), (0, 0));
}

#[test]
fn a_removed_keyboard_forgets_only_its_own_presses() {
    let mut router = two_keyboards();
    router
        .bind_for(
            &name("pad-q"),
            chord("q"),
            Trigger::Press,
            false,
            Some(selector("Macro Pad")),
        )
        .expect("pad");
    let desk = route(&mut router, key_on(DESK, Q, Modifiers::NONE, None, KeyState::Pressed));
    let pad = route(&mut router, key_on(PAD, Q, Modifiers::NONE, None, KeyState::Pressed));
    assert_eq!(desk.target, Target::Client);
    let dropped = router.device_removed(PAD);
    assert_eq!(
        dropped,
        vec![(HeldEdge::Key(PAD, CODE_Q), pad.target)],
        "the pad's press and nothing else"
    );
    assert_eq!((router.held(), router.held_by(DESK)), (1, 1));
    assert_eq!(router.stats().dropped_with_device, 1);
    assert_eq!(router.device_name(PAD), None);
    // The desk keyboard's release still finds its press.
    let desk = route(&mut router, key_on(DESK, Q, Modifiers::NONE, None, KeyState::Released));
    assert!(desk.followed_press);
    assert_eq!(router.stats().orphan_releases, 0);
    // A device that is gone matches no scoped binding, even under its old number.
    let after = route(&mut router, key_on(PAD, Q, Modifiers::NONE, None, KeyState::Pressed));
    assert_eq!(after.target, Target::Client);
    route(&mut router, key_on(PAD, Q, Modifiers::NONE, None, KeyState::Released));
}

#[test]
fn a_pointer_chord_scoped_to_one_mouse_ignores_the_other() {
    let mut router = Router::new();
    router.device_added(DESK, "Desk Mouse");
    router.device_added(PAD, "Travel Mouse");
    let bound = router
        .bind_for(
            &name("resize"),
            pointer_chord("<Super>Button3"),
            Trigger::Press,
            false,
            Some(selector("Travel Mouse")),
        )
        .expect("a pointer chord for one mouse")
        .id();
    let click = |device: DeviceId, state: ButtonState| gnoblin_input::Event::Button {
        device,
        button: 3,
        modifiers: Modifiers::SUPER,
        state,
    };
    assert_eq!(
        route(&mut router, click(DESK, ButtonState::Pressed)).target,
        Target::Client
    );
    assert_eq!(
        route(&mut router, click(PAD, ButtonState::Pressed)).target,
        Target::Binding(bound)
    );
    assert!(route(&mut router, click(PAD, ButtonState::Released)).followed_press);
    assert!(route(&mut router, click(DESK, ButtonState::Released)).followed_press);
    let stats = router.stats();
    assert_eq!((stats.duplicate_presses, stats.orphan_releases), (0, 0));
}
