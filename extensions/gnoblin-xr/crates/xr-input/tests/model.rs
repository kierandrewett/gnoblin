//! Tests for the controller input model: edges, hysteresis, scroll and ordering.

use xr_input::{
    BTN_LEFT, BTN_RIGHT, Controller, ControllerState, MAX_STEP, PointerEvent, SCROLL_DEADZONE, SCROLL_RATE,
    pointing_ray,
};
use xr_scene::{IDENTITY, Pose};

/// No events: an empty array that names its element type.
const NONE: [PointerEvent; 0] = [];

fn select(value: f32) -> ControllerState {
    ControllerState {
        select: value,
        ..ControllerState::default()
    }
}

fn secondary(value: f32) -> ControllerState {
    ControllerState {
        secondary: value,
        ..ControllerState::default()
    }
}

fn stick(x: f32, y: f32) -> ControllerState {
    ControllerState {
        scroll: (x, y),
        ..ControllerState::default()
    }
}

fn press(button: u32) -> PointerEvent {
    PointerEvent::Button { button, pressed: true }
}

fn release(button: u32) -> PointerEvent {
    PointerEvent::Button { button, pressed: false }
}

#[test]
fn the_event_codes_are_the_evdev_buttons() {
    assert_eq!((BTN_LEFT, BTN_RIGHT), (0x110, 0x111));
}

#[test]
fn a_click_presses_then_releases_the_left_button_once_each() {
    let mut controller = Controller::default();
    assert_eq!(controller.update(&select(0.0), 0.0), NONE);
    assert_eq!(controller.update(&select(1.0), 0.016), [press(BTN_LEFT)]);
    assert!(controller.any_down());
    assert_eq!(
        controller.update(&select(1.0), 0.016),
        NONE,
        "a held button is not a new edge"
    );
    assert_eq!(controller.update(&select(0.0), 0.016), [release(BTN_LEFT)]);
    assert!(!controller.any_down());
    assert_eq!(controller.update(&select(0.0), 0.016), NONE);
}

#[test]
fn a_trigger_presses_at_three_quarters_and_releases_at_a_quarter() {
    let mut controller = Controller::default();
    assert_eq!(controller.update(&select(0.74), 0.0), NONE, "below the press threshold");
    assert_eq!(
        controller.update(&select(0.75), 0.0),
        [press(BTN_LEFT)],
        "the threshold itself presses"
    );
    assert_eq!(
        controller.update(&select(0.5), 0.0),
        NONE,
        "between the thresholds the button holds"
    );
    assert_eq!(
        controller.update(&select(0.26), 0.0),
        NONE,
        "still above the release threshold"
    );
    assert_eq!(
        controller.update(&select(0.25), 0.0),
        [release(BTN_LEFT)],
        "the threshold itself releases"
    );
    assert_eq!(
        controller.update(&select(0.5), 0.0),
        NONE,
        "half pull does not press again"
    );
}

#[test]
fn a_trigger_resting_between_the_thresholds_never_chatters() {
    let mut controller = Controller::default();
    for value in [0.4, 0.6, 0.45, 0.7, 0.3, 0.55] {
        assert_eq!(controller.update(&select(value), 0.016), NONE, "{value}");
    }
    controller.update(&select(0.9), 0.016);
    for value in [0.6, 0.4, 0.74, 0.3, 0.5] {
        assert_eq!(controller.update(&select(value), 0.016), NONE, "{value} while held");
    }
    assert!(controller.any_down());
}

#[test]
fn a_value_that_is_not_a_number_changes_nothing() {
    let mut controller = Controller::default();
    assert_eq!(controller.update(&select(f32::NAN), 0.0), NONE);
    controller.update(&select(1.0), 0.0);
    assert_eq!(controller.update(&select(f32::NAN), 0.0), NONE);
    assert!(controller.any_down());
}

#[test]
fn secondary_is_the_right_button_and_the_two_are_independent() {
    let mut controller = Controller::default();
    assert_eq!(controller.update(&secondary(1.0), 0.0), [press(BTN_RIGHT)]);
    let both = ControllerState {
        select: 1.0,
        secondary: 1.0,
        ..ControllerState::default()
    };
    assert_eq!(
        controller.update(&both, 0.0),
        [press(BTN_LEFT)],
        "right is already down"
    );
    let only_left = ControllerState {
        select: 1.0,
        ..ControllerState::default()
    };
    assert_eq!(controller.update(&only_left, 0.0), [release(BTN_RIGHT)]);
    assert!(controller.any_down());
    assert_eq!(controller.update(&ControllerState::default(), 0.0), [release(BTN_LEFT)]);
    assert!(!controller.any_down());
}

#[test]
fn events_come_in_order_select_secondary_scroll() {
    let mut controller = Controller::default();
    let everything = ControllerState {
        ray: None,
        select: 1.0,
        secondary: 1.0,
        scroll: (0.0, -1.0),
    };
    let events = controller.update(&everything, 0.05);
    assert_eq!(events.len(), 3, "{events:?}");
    assert_eq!(events[0], press(BTN_LEFT));
    assert_eq!(events[1], press(BTN_RIGHT));
    assert!(matches!(events[2], PointerEvent::Scroll { .. }));
}

fn scroll_of(events: &[PointerEvent]) -> (f64, f64) {
    match events {
        [PointerEvent::Scroll { dx, dy }] => (*dx, *dy),
        other => panic!("expected one scroll event, got {other:?}"),
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-4
}

#[test]
fn a_stick_inside_the_deadzone_does_not_scroll() {
    let mut controller = Controller::default();
    assert_eq!(controller.update(&stick(0.1, 0.1), 0.05), NONE);
    assert_eq!(
        controller.update(&stick(0.0, SCROLL_DEADZONE), 0.05),
        NONE,
        "the edge itself is inside"
    );
    assert_eq!(controller.update(&stick(-0.1, 0.0), 0.05), NONE);
}

#[test]
fn full_deflection_scrolls_at_the_rate_per_second() {
    let mut controller = Controller::default();
    // Down on the stick is down the page: a positive vertical amount.
    let (dx, dy) = scroll_of(&controller.update(&stick(0.0, -1.0), 0.05));
    assert!(
        close(dx, 0.0) && close(dy, f64::from(SCROLL_RATE) * 0.05),
        "({dx}, {dy})"
    );
    // Up is up the page; right is positive horizontal.
    let (dx, dy) = scroll_of(&controller.update(&stick(0.0, 1.0), 0.05));
    assert!(
        close(dx, 0.0) && close(dy, -f64::from(SCROLL_RATE) * 0.05),
        "({dx}, {dy})"
    );
    let (dx, dy) = scroll_of(&controller.update(&stick(1.0, 0.0), 0.05));
    assert!(
        close(dx, f64::from(SCROLL_RATE) * 0.05) && close(dy, 0.0),
        "({dx}, {dy})"
    );
    let (dx, _) = scroll_of(&controller.update(&stick(-1.0, 0.0), 0.05));
    assert!(close(dx, -f64::from(SCROLL_RATE) * 0.05));
}

#[test]
fn scroll_speed_grows_from_zero_at_the_edge_of_the_deadzone() {
    let mut controller = Controller::default();
    // Halfway between the deadzone edge and full deflection: half speed.
    let halfway = SCROLL_DEADZONE + (1.0 - SCROLL_DEADZONE) / 2.0;
    let (_, dy) = scroll_of(&controller.update(&stick(0.0, -halfway), 0.05));
    assert!(close(dy, f64::from(SCROLL_RATE) * 0.05 / 2.0), "{dy}");
    // A deflection just past the edge scrolls just above nothing.
    let (_, dy) = scroll_of(&controller.update(&stick(0.0, -(SCROLL_DEADZONE + 0.01)), 0.05));
    assert!(dy > 0.0 && dy < 0.5, "{dy}");
}

#[test]
fn a_diagonal_stick_scrolls_along_both_axes_at_the_same_total_speed() {
    let mut controller = Controller::default();
    let side = std::f32::consts::FRAC_1_SQRT_2;
    let (dx, dy) = scroll_of(&controller.update(&stick(side, -side), 0.05));
    assert!(close(dx, dy) && dx > 0.0, "({dx}, {dy})");
    assert!(close(dx.hypot(dy), f64::from(SCROLL_RATE) * 0.05), "{}", dx.hypot(dy));
}

#[test]
fn a_stick_past_full_deflection_is_capped() {
    let mut controller = Controller::default();
    let (_, dy) = scroll_of(&controller.update(&stick(0.0, -2.0), 0.05));
    assert!(close(dy, f64::from(SCROLL_RATE) * 0.05), "{dy}");
}

#[test]
fn scroll_scales_with_the_frame_time_and_a_stall_is_clamped() {
    let mut controller = Controller::default();
    let (_, short) = scroll_of(&controller.update(&stick(0.0, -1.0), 0.01));
    let (_, long) = scroll_of(&controller.update(&stick(0.0, -1.0), 0.02));
    assert!(close(long, short * 2.0), "{short} {long}");

    let (_, stalled) = scroll_of(&controller.update(&stick(0.0, -1.0), 30.0));
    assert!(close(stalled, f64::from(SCROLL_RATE * MAX_STEP)), "{stalled}");
}

#[test]
fn no_time_passing_scrolls_nothing() {
    let mut controller = Controller::default();
    assert_eq!(
        controller.update(&stick(0.0, -1.0), 0.0),
        NONE,
        "the first sample of a session"
    );
    assert_eq!(
        controller.update(&stick(0.0, -1.0), -1.0),
        NONE,
        "a clock that went backwards"
    );
    assert_eq!(controller.update(&stick(0.0, -1.0), f32::NAN), NONE);
    assert_eq!(controller.update(&stick(f32::NAN, 0.0), 0.05), NONE);
}

#[test]
fn the_pointing_ray_is_the_aim_pose_else_the_gaze_else_nothing() {
    let aim = Pose {
        position: [0.2, 1.0, -0.1],
        orientation: IDENTITY,
    };
    let head = Pose {
        position: [0.0, 1.6, 0.0],
        orientation: IDENTITY,
    };
    let from_aim = pointing_ray(Some(&aim), Some(&head)).expect("a ray");
    assert_eq!(from_aim.origin, [0.2, 1.0, -0.1], "a tracked controller beats the gaze");
    assert_eq!(from_aim.direction, [0.0, 0.0, -1.0]);

    let gaze = pointing_ray(None, Some(&head)).expect("a ray");
    assert_eq!(
        gaze.origin,
        [0.0, 1.6, 0.0],
        "with no controller the ray leaves the head"
    );
    assert_eq!(gaze.direction, [0.0, 0.0, -1.0]);

    assert_eq!(pointing_ray(None, None), None);
}
