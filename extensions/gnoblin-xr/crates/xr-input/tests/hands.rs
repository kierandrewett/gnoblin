//! Tests for the hands and the claim arbitration: what a shell binding's claim keeps from the pointer.

use xr_input::{Arbiter, BTN_LEFT, BTN_RIGHT, Claims, Controller, Hand, HandState, Hands, PointerEvent};
use xr_scene::{IDENTITY, Pose};

const NONE: [PointerEvent; 0] = [];

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

/// One frame through the arbiter and the pointer's edge detection: the events the window would see.
fn frame(arbiter: &mut Arbiter, controller: &mut Controller, hands: &Hands, claims: Claims) -> Vec<PointerEvent> {
    controller.update(&arbiter.sample(hands, claims), 0.0)
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
fn an_unclaimed_hand_reaches_the_pointer_as_it_always_did() {
    let mut arbiter = Arbiter::default();
    let hand = HandState {
        aim: Some(pose(0.5)),
        trigger: 0.8,
        grip: 0.9,
        thumbstick: (0.0, 1.0),
        ..HandState::default()
    };
    let sample = arbiter.sample(&right(hand), Claims::default());
    assert_eq!(sample.select, 0.8);
    assert_eq!(sample.secondary, 0.9, "the grip is the secondary input");
    assert_eq!(sample.scroll, (0.0, 1.0));
    assert_eq!(sample.ray.map(|ray| ray.origin), Some([0.5, 0.0, 0.0]));
}

#[test]
fn the_upper_face_button_is_a_secondary_input_beside_the_grip() {
    let mut arbiter = Arbiter::default();
    let button = HandState {
        secondary: true,
        ..HandState::default()
    };
    assert_eq!(arbiter.sample(&right(button), Claims::default()).secondary, 1.0);
}

#[test]
fn a_claimed_grip_is_not_a_right_click_and_an_unclaimed_one_still_is() {
    let mut arbiter = Arbiter::default();
    let mut controller = Controller::default();
    let squeezed = right(HandState {
        grip: 1.0,
        ..HandState::default()
    });
    let claimed = Claims {
        grip: true,
        ..Claims::default()
    };
    assert_eq!(frame(&mut arbiter, &mut controller, &squeezed, claimed), NONE);
    assert_eq!(frame(&mut arbiter, &mut controller, &squeezed, claimed), NONE);

    let mut arbiter = Arbiter::default();
    let mut controller = Controller::default();
    assert_eq!(
        frame(&mut arbiter, &mut controller, &squeezed, Claims::default()),
        [press(BTN_RIGHT)]
    );
}

#[test]
fn a_claim_takes_only_the_input_it_names() {
    // Each claim set meets a hand that is already squeezing, so each has an arbiter of its own: an input held while it
    // was claimed stays withheld until it is let go (`a_press_that_began_under_a_claim...`).
    let mut arbiter = Arbiter::default();
    let hand = right(HandState {
        trigger: 1.0,
        grip: 1.0,
        thumbstick: (0.0, -1.0),
        secondary: true,
        ..HandState::default()
    });
    let sample = arbiter.sample(
        &hand,
        Claims {
            grip: true,
            ..Claims::default()
        },
    );
    assert_eq!(
        (sample.select, sample.scroll),
        (1.0, (0.0, -1.0)),
        "trigger and stick go on"
    );
    assert_eq!(sample.secondary, 1.0, "the face button still sends the secondary input");

    let mut arbiter = Arbiter::default();
    let sample = arbiter.sample(
        &hand,
        Claims {
            grip: true,
            secondary: true,
            ..Claims::default()
        },
    );
    assert_eq!(sample.secondary, 0.0, "both sources of the secondary input are claimed");

    let mut arbiter = Arbiter::default();
    let sample = arbiter.sample(
        &hand,
        Claims {
            trigger: true,
            thumbstick: true,
            ..Claims::default()
        },
    );
    assert_eq!((sample.select, sample.scroll), (0.0, (0.0, 0.0)));
    assert_eq!(sample.secondary, 1.0);
}

#[test]
fn a_press_the_window_already_has_is_released_when_the_claim_arrives() {
    let mut arbiter = Arbiter::default();
    let mut controller = Controller::default();
    let held = right(HandState {
        trigger: 1.0,
        ..HandState::default()
    });
    assert_eq!(
        frame(&mut arbiter, &mut controller, &held, Claims::default()),
        [press(BTN_LEFT)]
    );
    let claimed = Claims {
        trigger: true,
        ..Claims::default()
    };
    assert_eq!(
        frame(&mut arbiter, &mut controller, &held, claimed),
        [release(BTN_LEFT)],
        "no button stays down in the window"
    );
    assert!(!controller.any_down());
}

#[test]
fn a_press_that_began_under_a_claim_never_reaches_the_window_even_when_the_claim_goes() {
    let mut arbiter = Arbiter::default();
    let mut controller = Controller::default();
    let squeezed = right(HandState {
        grip: 1.0,
        ..HandState::default()
    });
    let claimed = Claims {
        grip: true,
        ..Claims::default()
    };
    assert_eq!(frame(&mut arbiter, &mut controller, &squeezed, claimed), NONE);
    // The binding is removed while the hand still squeezes: not a click.
    assert_eq!(frame(&mut arbiter, &mut controller, &squeezed, Claims::default()), NONE);
    assert_eq!(frame(&mut arbiter, &mut controller, &squeezed, Claims::default()), NONE);
    // Let go, then squeeze again: that press is the window's.
    let open = right(HandState::default());
    assert_eq!(frame(&mut arbiter, &mut controller, &open, Claims::default()), NONE);
    assert_eq!(
        frame(&mut arbiter, &mut controller, &squeezed, Claims::default()),
        [press(BTN_RIGHT)]
    );
}

#[test]
fn a_claimed_stick_does_not_scroll_and_a_stick_at_rest_is_not_a_held_one() {
    let mut arbiter = Arbiter::default();
    let claimed = Claims {
        thumbstick: true,
        ..Claims::default()
    };
    let pushed = right(HandState {
        thumbstick: (0.0, 1.0),
        ..HandState::default()
    });
    assert_eq!(arbiter.sample(&pushed, claimed).scroll, (0.0, 0.0));
    // Claimed while at rest, then the claim goes and the stick is pushed: that scrolls.
    let rest = right(HandState::default());
    assert_eq!(arbiter.sample(&rest, claimed).scroll, (0.0, 0.0));
    assert_eq!(arbiter.sample(&pushed, Claims::default()).scroll, (0.0, 1.0));
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
    let sample = arbiter.sample(&hands, Claims::default());
    assert_eq!(sample.ray.map(|ray| ray.origin), Some([-1.0, 0.0, 0.0]));
    assert_eq!(
        (sample.select, sample.secondary, sample.scroll),
        (0.0, 0.0, (0.0, 0.0)),
        "the pointer is the right hand's"
    );
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
        arbiter.sample(&hands, Claims::default()).ray.map(|ray| ray.origin),
        Some([2.0, 0.0, 0.0])
    );
    assert!(
        arbiter.sample(&Hands::default(), Claims::default()).ray.is_none(),
        "a runtime with no input source aims nowhere"
    );
}

#[test]
fn losing_the_hand_forgets_a_held_claim() {
    let mut arbiter = Arbiter::default();
    let squeezed = right(HandState {
        grip: 1.0,
        ..HandState::default()
    });
    let claimed = Claims {
        grip: true,
        ..Claims::default()
    };
    arbiter.sample(&squeezed, claimed);
    arbiter.sample(&Hands::default(), Claims::default());
    assert_eq!(
        arbiter.sample(&squeezed, Claims::default()).secondary,
        1.0,
        "a new squeeze is the window's"
    );
}
