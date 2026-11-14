//! Tests for the hands and the claim arbitration: what a shell binding's claim keeps from the pointer, and how long a
//! press waits for one.

use xr_input::{Arbiter, BTN_LEFT, BTN_RIGHT, CLAIM_HOLD, Claims, Controller, Hand, HandState, Hands, PointerEvent};
use xr_scene::{IDENTITY, Pose};

/// No event on any frame.
const NONE: [(i64, PointerEvent); 0] = [];

/// One frame of a 60 Hz display, in nanoseconds.
const FRAME: i64 = 16_666_667;

/// The first frame at or after the hold, counting from the frame a press began on.
const HOLD_FRAMES: i64 = (CLAIM_HOLD + FRAME - 1) / FRAME;

// The shell settles a chord in 50 ms and needs a few frames to say so: the hold covers that, and a click is still a
// click. A press is forwarded no later than the hold and one frame.
const _: () = {
    assert!(CLAIM_HOLD >= 100_000_000);
    assert!(CLAIM_HOLD <= 150_000_000);
    assert!(HOLD_FRAMES * FRAME < CLAIM_HOLD + FRAME);
};

fn press(button: u32) -> PointerEvent {
    PointerEvent::Button { button, pressed: true }
}

fn release(button: u32) -> PointerEvent {
    PointerEvent::Button { button, pressed: false }
}

fn pose(x: f64) -> Pose {
    Pose {
        position: [x, 0.0, 0.0],
        orientation: IDENTITY,
    }
}

fn right(hand: HandState) -> Hands {
    Hands {
        right: Some(hand),
        ..Hands::default()
    }
}

fn grip(value: f32) -> Hands {
    right(HandState {
        grip: value,
        ..HandState::default()
    })
}

fn trigger(value: f32) -> Hands {
    right(HandState {
        trigger: value,
        ..HandState::default()
    })
}

fn stick(y: f32) -> Hands {
    right(HandState {
        thumbstick: (0.0, y),
        ..HandState::default()
    })
}

fn claiming(grip: bool, trigger: bool, thumbstick: bool) -> Claims {
    Claims {
        grip,
        trigger,
        thumbstick,
        ..Claims::default()
    }
}

/// The arbiter and the pointer's edge detection on a 60 Hz clock: the events the window would see, and on which frame.
struct Rig {
    arbiter: Arbiter,
    controller: Controller,
    frame: i64,
}

impl Rig {
    fn new() -> Self {
        Self {
            arbiter: Arbiter::default(),
            controller: Controller::default(),
            frame: 0,
        }
    }

    /// One frame: the sample the pointer gets and the events it makes of it.
    fn step(&mut self, hands: &Hands, claims: Claims) -> (xr_input::ControllerState, Vec<PointerEvent>) {
        let sample = self.arbiter.sample(hands, claims, self.frame * FRAME);
        self.frame += 1;
        let events = self.controller.update(&sample, 0.0);
        (sample, events)
    }

    /// `frames` frames of the same hands and claims, with the frame each event came on (counted from this call).
    fn run(&mut self, frames: i64, hands: &Hands, claims: Claims) -> Vec<(i64, PointerEvent)> {
        let mut seen = Vec::new();
        for at in 0..frames {
            let (_, events) = self.step(hands, claims);
            seen.extend(events.into_iter().map(|event| (at, event)));
        }
        seen
    }
}

#[test]
fn the_hands_have_names_and_are_found_by_hand() {
    assert_eq!([Hand::Left.name(), Hand::Right.name()], ["left", "right"]);
    assert_eq!(Hand::BOTH, [Hand::Left, Hand::Right]);
    let hands = Hands {
        left: Some(HandState {
            grip: 0.25,
            ..HandState::default()
        }),
        ..Hands::default()
    };
    assert_eq!(hands.get(Hand::Left).map(|hand| hand.grip), Some(0.25));
    assert!(hands.get(Hand::Right).is_none());
}

#[test]
fn claims_are_read_from_the_five_names_and_anything_else_claims_nothing() {
    let claims = Claims::from_names(&["grip", "trigger", "bogus", "Grip", ""]);
    assert_eq!(
        claims,
        Claims {
            trigger: true,
            grip: true,
            ..Claims::default()
        }
    );
    let all = Claims::from_names(&["trigger", "grip", "thumbstick", "primary", "secondary"]);
    assert!(all.trigger && all.grip && all.thumbstick && all.primary && all.secondary);
    assert_eq!(Claims::from_names::<&str>(&[]), Claims::default());
}

#[test]
fn an_unclaimed_hand_reaches_the_pointer_when_the_hold_is_over() {
    let mut rig = Rig::new();
    let hand = right(HandState {
        aim: Some(pose(0.5)),
        trigger: 0.8,
        grip: 0.9,
        thumbstick: (0.0, 1.0),
        ..HandState::default()
    });
    for _ in 0..HOLD_FRAMES {
        let (sample, _) = rig.step(&hand, Claims::default());
        assert_eq!(
            (sample.select, sample.secondary, sample.scroll),
            (0.0, 0.0, (0.0, 0.0)),
            "held back while a claim may still come"
        );
        assert_eq!(
            sample.ray.map(|ray| ray.origin),
            Some([0.5, 0.0, 0.0]),
            "the ray is not held"
        );
    }
    let (sample, _) = rig.step(&hand, Claims::default());
    assert_eq!(sample.select, 1.0, "the press");
    assert_eq!(sample.secondary, 1.0, "the grip is the secondary input");
    assert_eq!(sample.scroll, (0.0, 1.0));
    let (sample, _) = rig.step(&hand, Claims::default());
    assert_eq!((sample.select, sample.secondary), (0.8, 0.9), "then the raw values");
}

#[test]
fn the_upper_face_button_is_a_secondary_input_beside_the_grip() {
    let mut rig = Rig::new();
    let button = right(HandState {
        secondary: true,
        ..HandState::default()
    });
    assert_eq!(
        rig.run(HOLD_FRAMES + 2, &button, Claims::default()),
        [(HOLD_FRAMES, press(BTN_RIGHT))]
    );
}

#[test]
fn a_claimed_grip_never_produces_a_right_button_press() {
    // The claim is there before the press, comes while it is held, or comes on the last frame of the hold.
    for claim_at in [0, 1, 3, HOLD_FRAMES - 1] {
        let mut rig = Rig::new();
        let mut seen = rig.run(claim_at, &grip(1.0), Claims::default());
        seen.extend(rig.run(HOLD_FRAMES * 3, &grip(1.0), claiming(true, false, false)));
        assert_eq!(seen, NONE, "claimed on frame {claim_at}");
    }
}

#[test]
fn an_unclaimed_grip_is_pressed_at_the_deadline_and_not_before() {
    let mut rig = Rig::new();
    let seen = rig.run(HOLD_FRAMES + 3, &grip(1.0), Claims::default());
    assert_eq!(seen, [(HOLD_FRAMES, press(BTN_RIGHT))]);
    assert_eq!(rig.run(1, &grip(0.0), Claims::default()), [(0, release(BTN_RIGHT))]);
}

#[test]
fn a_tap_inside_the_hold_is_a_press_then_a_release_when_the_hold_is_over() {
    let mut rig = Rig::new();
    let mut seen = rig.run(2, &grip(1.0), Claims::default());
    seen.extend(rig.run(HOLD_FRAMES + 2, &grip(0.0), Claims::default()));
    assert_eq!(
        seen,
        [
            (HOLD_FRAMES - 2, press(BTN_RIGHT)),
            (HOLD_FRAMES - 1, release(BTN_RIGHT))
        ],
        "in order, on consecutive frames, after the deadline"
    );
    assert!(!rig.controller.any_down());
}

#[test]
fn a_tap_the_shell_claims_inside_the_hold_is_nothing() {
    let mut rig = Rig::new();
    let mut seen = rig.run(2, &grip(1.0), Claims::default());
    seen.extend(rig.run(3, &grip(0.0), Claims::default()));
    seen.extend(rig.run(HOLD_FRAMES * 2, &grip(0.0), claiming(true, false, false)));
    assert_eq!(seen, NONE);
}

#[test]
fn two_quick_taps_are_both_delivered_in_order() {
    let mut rig = Rig::new();
    let mut seen = Vec::new();
    for hands in [grip(1.0), grip(0.0), grip(1.0), grip(0.0)] {
        seen.extend(
            rig.run(1, &hands, Claims::default())
                .into_iter()
                .map(|(_, event)| event),
        );
    }
    seen.extend(
        rig.run(HOLD_FRAMES * 2, &grip(0.0), Claims::default())
            .into_iter()
            .map(|(_, event)| event),
    );
    assert_eq!(
        seen,
        [
            press(BTN_RIGHT),
            release(BTN_RIGHT),
            press(BTN_RIGHT),
            release(BTN_RIGHT)
        ]
    );
}

#[test]
fn a_press_is_aimed_where_the_hand_was_when_it_went_down() {
    let mut rig = Rig::new();
    let pulled = |x: f64| {
        right(HandState {
            aim: Some(pose(x)),
            trigger: 1.0,
            ..HandState::default()
        })
    };
    let mut origins = Vec::new();
    // The ray drifts while the hold runs, as it does when the finger pulls.
    for n in 1..=10_i32 {
        let (sample, events) = rig.step(&pulled(0.1 * f64::from(n)), Claims::default());
        if events == [press(BTN_LEFT)] {
            origins.push(sample.ray.map(|ray| ray.origin));
        }
    }
    assert_eq!(origins, [Some([0.1, 0.0, 0.0])], "the press lands where the pull began");
}

#[test]
fn a_claim_takes_only_the_input_it_names() {
    let hand = right(HandState {
        trigger: 1.0,
        grip: 1.0,
        thumbstick: (0.0, -1.0),
        secondary: true,
        ..HandState::default()
    });
    let at_the_deadline = |claims: Claims| {
        let mut rig = Rig::new();
        for _ in 0..HOLD_FRAMES {
            rig.step(&hand, claims);
        }
        rig.step(&hand, claims).0
    };
    let sample = at_the_deadline(claiming(true, false, false));
    assert_eq!(
        (sample.select, sample.scroll),
        (1.0, (0.0, -1.0)),
        "trigger and stick go on"
    );
    assert_eq!(sample.secondary, 1.0, "the face button still sends the secondary input");

    let sample = at_the_deadline(Claims {
        grip: true,
        secondary: true,
        ..Claims::default()
    });
    assert_eq!(sample.secondary, 0.0, "both sources of the secondary input are claimed");

    let sample = at_the_deadline(claiming(false, true, true));
    assert_eq!((sample.select, sample.scroll), (0.0, (0.0, 0.0)));
    assert_eq!(sample.secondary, 1.0);
}

#[test]
fn a_claimed_trigger_never_clicks_and_an_unclaimed_one_clicks_after_the_hold() {
    let mut rig = Rig::new();
    assert_eq!(
        rig.run(HOLD_FRAMES * 3, &trigger(1.0), claiming(false, true, false)),
        NONE
    );
    let mut rig = Rig::new();
    assert_eq!(
        rig.run(HOLD_FRAMES + 2, &trigger(1.0), Claims::default()),
        [(HOLD_FRAMES, press(BTN_LEFT))]
    );
    let mut rig = Rig::new();
    let mut tap = rig.run(1, &trigger(1.0), Claims::default());
    tap.extend(rig.run(HOLD_FRAMES + 2, &trigger(0.0), Claims::default()));
    assert_eq!(
        tap.iter().map(|(_, event)| *event).collect::<Vec<_>>(),
        [press(BTN_LEFT), release(BTN_LEFT)]
    );
}

#[test]
fn a_claimed_stick_does_not_scroll_and_an_unclaimed_one_scrolls_after_the_hold() {
    let mut rig = Rig::new();
    for _ in 0..HOLD_FRAMES * 2 {
        let (sample, _) = rig.step(&stick(1.0), claiming(false, false, true));
        assert_eq!(sample.scroll, (0.0, 0.0));
    }
    let mut rig = Rig::new();
    for frame in 0..=HOLD_FRAMES {
        let (sample, _) = rig.step(&stick(1.0), Claims::default());
        let expected = if frame < HOLD_FRAMES { (0.0, 0.0) } else { (0.0, 1.0) };
        assert_eq!(sample.scroll, expected, "frame {frame}");
    }
}

#[test]
fn a_flick_of_the_stick_inside_the_hold_scrolls_nothing() {
    let mut rig = Rig::new();
    for hands in [stick(1.0), stick(1.0), stick(0.0)] {
        assert_eq!(rig.step(&hands, Claims::default()).0.scroll, (0.0, 0.0));
    }
    for _ in 0..HOLD_FRAMES * 2 {
        assert_eq!(rig.step(&stick(0.0), Claims::default()).0.scroll, (0.0, 0.0));
    }
}

#[test]
fn a_press_the_window_already_has_is_released_when_the_claim_arrives() {
    let mut rig = Rig::new();
    let seen = rig.run(HOLD_FRAMES + 1, &trigger(1.0), Claims::default());
    assert_eq!(seen, [(HOLD_FRAMES, press(BTN_LEFT))]);
    assert_eq!(
        rig.run(1, &trigger(1.0), claiming(false, true, false)),
        [(0, release(BTN_LEFT))],
        "no button stays down in the window"
    );
    assert!(!rig.controller.any_down());
}

#[test]
fn a_press_that_began_under_a_claim_never_reaches_the_window_even_when_the_claim_goes() {
    let mut rig = Rig::new();
    let claimed = claiming(true, false, false);
    assert_eq!(rig.run(1, &grip(1.0), claimed), NONE);
    // The binding is removed while the hand still squeezes: not a click, however long it goes on.
    assert_eq!(rig.run(HOLD_FRAMES * 2, &grip(1.0), Claims::default()), NONE);
    // Let go, then squeeze again: that press is the window's, after its own hold.
    assert_eq!(rig.run(2, &grip(0.0), Claims::default()), NONE);
    assert_eq!(
        rig.run(HOLD_FRAMES + 1, &grip(1.0), Claims::default()),
        [(HOLD_FRAMES, press(BTN_RIGHT))]
    );
}

#[test]
fn with_no_right_controller_the_gaze_aims_and_nothing_is_pressed() {
    let mut arbiter = Arbiter::default();
    let hands = Hands {
        left: Some(HandState {
            trigger: 1.0,
            grip: 1.0,
            ..HandState::default()
        }),
        gaze: Some(pose(-1.0)),
        ..Hands::default()
    };
    for frame in 0..=HOLD_FRAMES + 1 {
        let sample = arbiter.sample(&hands, Claims::default(), frame * FRAME);
        assert_eq!(sample.ray.map(|ray| ray.origin), Some([-1.0, 0.0, 0.0]));
        assert_eq!(
            (sample.select, sample.secondary, sample.scroll),
            (0.0, 0.0, (0.0, 0.0)),
            "the pointer is the right hand's"
        );
    }
}

#[test]
fn a_right_controller_without_an_aim_pose_falls_back_to_the_gaze() {
    let mut arbiter = Arbiter::default();
    let hands = Hands {
        right: Some(HandState::default()),
        gaze: Some(pose(2.0)),
        ..Hands::default()
    };
    assert_eq!(
        arbiter.sample(&hands, Claims::default(), 0).ray.map(|ray| ray.origin),
        Some([2.0, 0.0, 0.0])
    );
    assert!(
        arbiter
            .sample(&Hands::default(), Claims::default(), FRAME)
            .ray
            .is_none(),
        "a runtime with no input source aims nowhere"
    );
}

#[test]
fn losing_the_hand_forgets_a_held_claim_and_a_press_that_waited() {
    let mut rig = Rig::new();
    assert_eq!(rig.run(1, &grip(1.0), claiming(true, false, false)), NONE);
    assert_eq!(rig.run(1, &Hands::default(), Claims::default()), NONE);
    assert_eq!(
        rig.run(HOLD_FRAMES + 1, &grip(1.0), Claims::default()),
        [(HOLD_FRAMES, press(BTN_RIGHT))],
        "a new squeeze is the window's"
    );

    // A press that waited when the hand went is not clicked for it.
    let mut rig = Rig::new();
    assert_eq!(rig.run(2, &grip(1.0), Claims::default()), NONE);
    assert_eq!(rig.run(HOLD_FRAMES * 2, &Hands::default(), Claims::default()), NONE);
    assert!(!rig.controller.any_down());
}
