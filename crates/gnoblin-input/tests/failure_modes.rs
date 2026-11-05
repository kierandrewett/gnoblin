//! One test per documented failure mode, in the order the crate docs list them.

mod common;

use common::{CODE_Q, CODE_W, E, Q, W, button_event, chord, key_event, name, pointer_chord, press, release};
use gnoblin_input::{
    Activity, BindingTable, CancelReason, Chord, ChordKind, Contact, ContactPhase, Context, Decision, Edge, EdgeBand,
    Expired, GestureAction, GestureConfig, GestureEvent, GestureKind, GesturePath, GesturePhase, GestureRoute,
    GestureRouter, GestureSpec, Insertion, Modifiers, Name, Phase, PinchDirection, PolicyQueue, Recogniser, Reply,
    RequestId, Routed, Router, Scope, ShortcutInhibit, Submitted, Target, Tick, Trigger, When,
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
        when: Some(When::Unlocked),
        action: Some(GestureAction::Command(vec!["kgx".to_owned()])),
        ..GestureSpec::default()
    }
}

fn pinch_spec(label: &str, fingers: u8, direction: PinchDirection) -> GestureSpec {
    GestureSpec {
        name: label.to_owned(),
        gesture: Some(GestureKind::Pinch),
        fingers: Some(fingers),
        direction: Some(direction),
        when: Some(When::Unlocked),
        action: Some(GestureAction::Action("workspace.next".to_owned())),
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
        router.observe(&begin, Context::Unlocked),
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
            router.observe(&update, Context::Unlocked),
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
        router.observe(&matching, Context::Unlocked),
        GestureRoute::Fired { binding: 0 }
    );
    assert_eq!(router.reserved(), None);

    // A reserved gesture whose movement does not match runs nothing and is not offered to
    // the other binding, which is the documented behaviour and the reason a second claim
    // would be worse than no claim.
    router.observe(&begin, Context::Unlocked);
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
        router.observe(&mismatched, Context::Unlocked),
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
fn a_late_decision_never_blocks_and_is_recorded() {
    let mut queue = PolicyQueue::new(4);
    let Submitted::Queued { id, deadline } = queue.submit(Tick::new(100), 16) else {
        panic!("an empty queue accepts the request");
    };
    assert_eq!(deadline, Tick::new(116));

    // A caller that must deliver now asks what has expired; nothing ever waits.
    assert!(queue.expire(Tick::new(110)).is_empty());
    let expired: Vec<Expired> = queue.expire(Tick::new(116));
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].id, id, "the event resumes as normal delivery");
    assert_eq!(queue.stats().expired, 1);

    // The decision that arrives afterwards cannot act, and the record says why.
    assert_eq!(queue.reply(id, Decision::Consume), Reply::Late { id });
    assert_eq!(queue.stats().late, 1);
    assert_eq!(queue.stats().applied, 0);
    assert_eq!(queue.pending(), 0);

    // The next event is queued normally, so one late reply does not disable the policy.
    assert!(matches!(queue.submit(Tick::new(120), 16), Submitted::Queued { .. }));
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
        Phase::Press,
    );
    assert_eq!(route(&mut router, return_press).target, Target::Binding(binding));
    assert!(router.bindings_mut().remove(binding).is_some());
    let return_release = key_event(
        gnoblin_input::KeyCode::new(28),
        gnoblin_input::Keysym::new(0xff0d),
        Modifiers::SUPER,
        Phase::Release,
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
        router.observe(&begin, Context::Unlocked),
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
        router.observe(&cancel, Context::Unlocked),
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
    router.observe(&begin, Context::Unlocked);
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
        router.observe(&stale, Context::Unlocked),
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
fn a_reload_retires_requests_before_their_replies() {
    let mut queue = PolicyQueue::new(8);
    let Submitted::Queued { id: first, .. } = queue.submit(Tick::new(10), 50) else {
        panic!("queued");
    };
    let Submitted::Queued { id: second, .. } = queue.submit(Tick::new(11), 50) else {
        panic!("queued");
    };
    // The reload retires in submission order, which is the order the caller resumes.
    assert_eq!(queue.retire_all(), vec![first, second]);
    assert_eq!(queue.stats().retired, 2);
    assert!(queue.is_empty());
    // The script answers after the reload; neither answer may act on a retired event.
    assert_eq!(queue.reply(first, Decision::Consume), Reply::Late { id: first });
    assert_eq!(queue.reply(second, Decision::Forward), Reply::Late { id: second });
    assert_eq!(queue.stats().applied, 0);
    assert_eq!(queue.stats().late, 2);
    // A reply for a request this session never issued is not a late one.
    let forged = RequestId::from_raw(9_000);
    assert!(matches!(queue.reply(forged, Decision::Forward), Reply::Unknown { .. }));
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
        route(&mut router, button_event(3, Modifiers::SUPER, Phase::Press)).target,
        Target::Binding(binding)
    );
    assert_eq!(
        route(&mut router, button_event(1, Modifiers::NONE, Phase::Press)).target,
        Target::Client,
        "an ordinary click still reaches the application"
    );
    route(&mut router, button_event(1, Modifiers::NONE, Phase::Release));
    assert_eq!(router.held(), 1, "the chord's own button is still down");
    assert_eq!(
        route(&mut router, button_event(3, Modifiers::SUPER, Phase::Release)).target,
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
