//! The controllers through the whole bridge: hands scripted in the fake runtime, the nodes they leave in the tree, and
//! what the pointer does with an input the shell has claimed.
//!
//! The owner is `controllers::owner`, which keeps the tree the program builds and publishes it back, so the bridge reads
//! the shell's `claimed` from its own mirror as it would from a compositor. The shell's write is the owner's `claim`,
//! made between frames. What this proves is the program's half of `design/js-xr.md`; the JS bindings' half is proved in
//! `gnoblin-js`, and neither has met a headset.

use std::collections::BTreeSet;

use gnoblin_proto::tree::op;
use xr_gpu::FakePresenter;
use xr_input::{BTN_RIGHT, HandState, Hands};
use xr_scene::{IDENTITY, Options, Pose};
use xr_session::frame_loop::FrameLoop;
use xr_session::runtime::{FrameTiming, SessionState};
use xr_session::testing::{FakeRuntime, Step};
use xr_tree::TreeClient;

use super::run;
use super::tests::{ScriptedFrames, room_of, state};
use crate::controllers::owner::{self, Owner};

/// One frame of a 60 Hz display, in nanoseconds: over the interval the writes are thinned to.
const FRAME: i64 = 16_666_667;

/// A session of `frames` frames, a display frame apart.
fn session(frames: i64) -> Vec<Step> {
    let mut steps = vec![state(SessionState::Ready)];
    steps.extend((1..=frames).map(|n| {
        Step::Frame(FrameTiming {
            predicted_display_time: n * FRAME,
            should_render: true,
        })
    }));
    steps.push(state(SessionState::Exiting));
    steps
}

fn hand(x: f64, grip: f32, trigger: f32) -> Hands {
    Hands {
        right: Some(HandState {
            aim: Some(Pose {
                position: [x, 1.0, -0.3],
                orientation: IDENTITY,
            }),
            grip,
            trigger,
            ..HandState::default()
        }),
        ..Hands::default()
    }
}

/// A hand at the origin, pointing straight ahead at the View of window 3, a metre away.
fn pointing(grip: f32) -> Hands {
    Hands {
        right: Some(HandState {
            aim: Some(Pose {
                position: [0.0; 3],
                orientation: IDENTITY,
            }),
            grip,
            ..HandState::default()
        }),
        ..Hands::default()
    }
}

/// Runs the bridge over a room of one window with `hands` as the runtime's samples, one for each frame, and calls
/// `after_frame(index, owner)` after each frame (the shell's moment to write). Returns the fake owner.
fn run_hands(hands: Vec<Hands>, after_frame: impl FnMut(usize, &Owner)) -> Owner {
    run_hands_counting_frames(hands, after_frame).0
}

/// As [`run_hands`], and the number of frames the runtime ended (each with the layers it was given, if any).
fn run_hands_counting_frames(hands: Vec<Hands>, mut after_frame: impl FnMut(usize, &Owner)) -> (Owner, usize) {
    let (stream, owner) = owner::serve(room_of(&[(2, 0.0, -1000.0)]));
    let mut tree = TreeClient::from_stream(stream).expect("attach");
    let frames = i64::try_from(hands.len()).expect("a few frames");
    let mut runtime = FakeRuntime::new(session(frames));
    runtime.hands_script = hands.into();
    let mut frames = FrameLoop::new(runtime);
    let seen = owner.clone();
    let mut index = 0;
    run(
        &mut tree,
        &mut frames,
        &Options::default(),
        None,
        &mut ScriptedFrames::default(),
        &mut FakePresenter::default(),
        |_| {
            after_frame(index, &seen);
            index += 1;
        },
    )
    .expect("the bridge runs");
    let ended = frames.runtime().submitted.len();
    (owner, ended)
}

/// The first frame after which the program has sent the window `count` right-button events, or `None`.
fn frame_of_right_event(per_frame: &[usize], count: usize) -> Option<usize> {
    per_frame.iter().position(|seen| *seen >= count)
}

/// The frames a press waits for a claim, as whole frames of the 60 Hz test display: the hold, rounded up.
const HOLD_FRAMES: usize = ((xr_input::CLAIM_HOLD + FRAME - 1) / FRAME) as usize;

/// One value for each frame: `[(1, 0.0), (3, 1.0)]` is one frame at 0 and three at 1.
fn runs(pattern: &[(usize, f32)]) -> Vec<f32> {
    pattern
        .iter()
        .flat_map(|(frames, value)| vec![*value; *frames])
        .collect()
}

/// The right hand pointing at the window with the grip squeezed to each value in turn.
fn grips(pattern: &[(usize, f32)]) -> Vec<Hands> {
    runs(pattern).into_iter().map(pointing).collect()
}

/// The right hand pointing at the window with the trigger pulled to each value in turn.
fn triggers(pattern: &[(usize, f32)]) -> Vec<Hands> {
    runs(pattern)
        .into_iter()
        .map(|trigger| Hands {
            right: Some(HandState {
                trigger,
                ..pointing(0.0).right.expect("right")
            }),
            ..Hands::default()
        })
        .collect()
}

/// The right hand pointing at the window with the stick pushed up to each value in turn.
fn sticks(pattern: &[(usize, f32)]) -> Vec<Hands> {
    runs(pattern)
        .into_iter()
        .map(|push| Hands {
            right: Some(HandState {
                thumbstick: (0.0, push),
                ..pointing(0.0).right.expect("right")
            }),
            ..Hands::default()
        })
        .collect()
}

/// The ids of every controller the program made.
fn controller_ids(owner: &Owner) -> BTreeSet<u64> {
    owner
        .transactions()
        .iter()
        .flatten()
        .filter(|op| op.kind == "new" && op.class == "controller")
        .map(|op| op.id)
        .collect()
}

/// The transactions that made, wrote or destroyed a controller, oldest first.
fn controller_transactions(owner: &Owner) -> Vec<Vec<op::Properties>> {
    let ids = controller_ids(owner);
    owner
        .transactions()
        .into_iter()
        .filter(|tx| tx.iter().any(|op| op.class == "controller" || ids.contains(&op.node)))
        .collect()
}

/// The pointer's inputs to windows, as `(kind, button, pressed)`: motion and leave have no button.
fn inputs(owner: &Owner) -> Vec<(String, u64, u64)> {
    owner
        .transactions()
        .iter()
        .flatten()
        .filter(|op| op.kind == "input")
        .map(|op| (op.input_kind.clone(), op.button, op.state))
        .collect()
}

fn presses_of_the_right_button(owner: &Owner) -> Vec<u64> {
    inputs(owner)
        .into_iter()
        .filter(|(kind, button, _)| kind == "button" && *button == u64::from(BTN_RIGHT))
        .map(|(_, _, pressed)| pressed)
        .collect()
}

#[test]
fn scripted_poses_and_presses_become_the_writes_of_the_controller_node_one_at_most_a_frame() {
    let script = vec![
        hand(0.2, 0.0, 0.0),
        hand(0.25, 0.0, 0.0),
        hand(0.25, 1.0, 1.0),
        hand(0.25, 1.0, 1.0),
        Hands::default(),
    ];
    let mut seen = Vec::new();
    let mut per_frame = Vec::new();
    let owner = run_hands(script, |_, owner| {
        let before: usize = per_frame.iter().sum();
        let total = controller_transactions(owner).len();
        per_frame.push(total - before);
        // Whole units: the room's `metres` of 0.001 turns the pose's metres into units with a rounding error.
        let whole = |value: Option<f64>| value.map(f64::round);
        seen.push(owner.controller("right").map(|(_, node)| {
            (
                whole(node.number("x")),
                whole(node.number("trigger")),
                whole(node.number("grip")),
            )
        }));
    });

    let at = |x: f64, press: f64| Some((Some(x), Some(press), Some(press)));
    assert_eq!(
        seen,
        [at(200.0, 0.0), at(250.0, 0.0), at(250.0, 1.0), at(250.0, 1.0), None,],
        "the node after each frame: made, moved, pressed, left alone, then gone with the hand"
    );
    assert_eq!(
        per_frame,
        [1, 1, 1, 0, 1],
        "the node is made, written twice, not written when nothing changed and destroyed, a transaction each"
    );
    assert!(owner.nodes_of("controller").is_empty(), "no hand is left behind");
}

#[test]
fn the_nodes_written_values_are_in_the_rooms_units_with_the_pose_y_up() {
    let owner = run_hands(vec![hand(0.5, 0.0, 0.0)], |_, _| {});
    // The room's `metres` is 0.001, so 0.5 m is 500 units and a hand 1 m up is 1000 units up.
    let writes = controller_transactions(&owner);
    let first = &writes[0];
    let value = |key: &str| first.iter().find(|op| op.key == key).map(|op| op.value_double.round());
    assert_eq!(value("x"), Some(500.0));
    assert_eq!(value("y"), Some(1000.0));
    assert_eq!(value("z"), Some(-300.0));
}

#[test]
fn the_session_ending_destroys_the_controller_and_the_seat() {
    let owner = run_hands(vec![hand(0.2, 0.0, 0.0), hand(0.2, 0.0, 0.0)], |_, _| {});
    assert!(owner.nodes_of("controller").is_empty());
    assert!(owner.nodes_of("seat").is_empty(), "the pointer's seat goes with it");
    let destroys: Vec<u64> = owner
        .transactions()
        .iter()
        .flatten()
        .filter(|op| op.kind == "destroy")
        .map(|op| op.node)
        .collect();
    let ids = controller_ids(&owner);
    let controller = ids.iter().next().expect("a controller was made");
    let seat = destroys
        .iter()
        .position(|node| node != controller)
        .expect("the seat is destroyed");
    let hand = destroys
        .iter()
        .position(|node| node == controller)
        .expect("the controller is destroyed");
    assert!(
        hand < seat,
        "the hand goes before the seat, so the shell never sees a hand without its session"
    );
}

/// The window's `BTN_LEFT` events, pressed (`1`) or released (`0`), in order.
fn presses_of_the_left_button(owner: &Owner) -> Vec<u64> {
    inputs(owner)
        .into_iter()
        .filter(|(kind, button, _)| kind == "button" && *button == 0x110)
        .map(|(_, _, pressed)| pressed)
        .collect()
}

/// How many scroll events the window got.
fn scrolls(owner: &Owner) -> usize {
    inputs(owner).iter().filter(|(kind, ..)| kind == "scroll").count()
}

/// Every button event the window got, of any button, as `(button, pressed)`.
fn buttons(owner: &Owner) -> Vec<(u64, u64)> {
    inputs(owner)
        .into_iter()
        .filter(|(kind, ..)| kind == "button")
        .map(|(_, button, pressed)| (button, pressed))
        .collect()
}

#[test]
fn an_unclaimed_grip_reaches_the_window_as_the_right_button_after_the_hold_and_not_before() {
    let mut per_frame = Vec::new();
    let owner = run_hands(grips(&[(1, 0.0), (14, 1.0), (2, 0.0)]), |_, owner| {
        per_frame.push(presses_of_the_right_button(owner).len());
    });
    assert_eq!(
        presses_of_the_right_button(&owner),
        [1, 0],
        "pressed, then released with the grip, in order"
    );
    // The squeeze began on frame 1. Nothing reaches the window while a claim may still come, and the press is there
    // by the first frame at or after the hold.
    let pressed = frame_of_right_event(&per_frame, 1).expect("the press was delivered");
    assert_eq!(pressed, 1 + HOLD_FRAMES, "the press waited for the hold and no longer");
    assert!(per_frame[..pressed].iter().all(|seen| *seen == 0));
}

#[test]
fn a_claimed_grip_is_withheld_from_the_window_while_the_node_still_says_it_is_squeezed() {
    let mut squeezed_while_claimed = false;
    let owner = run_hands(grips(&[(1, 0.0), (14, 1.0), (2, 0.0)]), |frame, owner| {
        if frame == 0 {
            // The shell's Grip binding claims the input as soon as it sees the hand.
            let (right, _) = owner.controller("right").expect("the hand arrived in the first frame");
            owner.claim(right, &["grip"]);
        }
        if frame == 12 {
            let (_, node) = owner.controller("right").expect("right");
            squeezed_while_claimed = node.number("grip") == Some(1.0);
        }
    });
    assert!(
        presses_of_the_right_button(&owner).is_empty(),
        "no right click in the window"
    );
    assert!(squeezed_while_claimed, "the shell reads the grip it claimed");
    // The rest of the pointer is untouched: it moved over the window.
    assert!(inputs(&owner).iter().any(|(kind, ..)| kind == "motion"));
}

#[test]
fn a_grip_that_becomes_a_claimed_drag_inside_the_hold_gives_the_window_no_button_event_at_all() {
    // The claim lands on every frame the hold covers, the last included: a drag the shell starts after its settle.
    for after in 2..HOLD_FRAMES {
        let owner = run_hands(grips(&[(1, 0.0), (16, 1.0), (2, 0.0)]), |frame, owner| {
            if frame == after {
                let (right, _) = owner.controller("right").expect("right");
                owner.claim(right, &["grip"]);
            }
            if frame == 17 {
                // The drag ends with the squeeze.
                let (right, _) = owner.controller("right").expect("right");
                owner.claim(right, &[]);
            }
        });
        assert!(
            buttons(&owner).is_empty(),
            "claimed after frame {after}: the window never heard of the grip, got {:?}",
            buttons(&owner)
        );
    }
}

#[test]
fn a_tap_the_shell_never_claims_is_a_press_then_a_release_when_the_hold_is_over() {
    let mut per_frame = Vec::new();
    let owner = run_hands(grips(&[(1, 0.0), (2, 1.0), (12, 0.0)]), |_, owner| {
        per_frame.push(presses_of_the_right_button(owner).len());
    });
    assert_eq!(presses_of_the_right_button(&owner), [1, 0], "a click, in order");
    assert_eq!(
        frame_of_right_event(&per_frame, 1),
        Some(1 + HOLD_FRAMES),
        "delivered at the deadline, though the finger was up after two frames"
    );
    assert_eq!(frame_of_right_event(&per_frame, 2), Some(2 + HOLD_FRAMES));
}

#[test]
fn a_tap_the_shell_claims_inside_the_hold_is_not_a_click() {
    let owner = run_hands(grips(&[(1, 0.0), (2, 1.0), (12, 0.0)]), |frame, owner| {
        if frame == 4 {
            let (right, _) = owner.controller("right").expect("right");
            owner.claim(right, &["grip"]);
        }
    });
    assert!(buttons(&owner).is_empty(), "got {:?}", buttons(&owner));
}

#[test]
fn a_claim_the_shell_gives_back_lets_the_next_squeeze_through_to_the_window() {
    let owner = run_hands(
        grips(&[(1, 0.0), (10, 1.0), (3, 0.0), (12, 1.0), (2, 0.0)]),
        |frame, owner| {
            let (right, _) = owner.controller("right").expect("the hand arrived in the first frame");
            if frame == 0 {
                // The shell's drag starts: it claims the grip, as `controller.claim` writes the list.
                owner.claim(right, &["grip"]);
            }
            if frame == 12 {
                // The drag ended: the shell writes the empty list, as the real compositor publishes it.
                owner.claim(right, &[]);
            }
        },
    );
    assert_eq!(
        presses_of_the_right_button(&owner),
        [1, 0],
        "the squeeze under the claim never reached the window, and the one after it did"
    );
}

#[test]
fn a_grip_the_window_already_has_is_released_when_a_late_claim_arrives() {
    let mut per_frame = Vec::new();
    let owner = run_hands(grips(&[(1, 0.0), (20, 1.0), (2, 0.0)]), |frame, owner| {
        per_frame.push(presses_of_the_right_button(owner).len());
        if frame == 12 {
            // A shell that is slower than the hold: the press went out at frame 9.
            let (right, _) = owner.controller("right").expect("right");
            owner.claim(right, &["grip"]);
        }
    });
    assert_eq!(
        presses_of_the_right_button(&owner),
        [1, 0],
        "pressed at the deadline, released by the claim, and the squeeze that goes on is the shell's"
    );
    assert_eq!(frame_of_right_event(&per_frame, 1), Some(1 + HOLD_FRAMES));
    assert_eq!(
        frame_of_right_event(&per_frame, 2),
        Some(13),
        "released the frame after the claim, long before the hand let go"
    );
}

#[test]
fn a_claim_of_one_input_leaves_the_others_to_the_window() {
    let owner = run_hands(triggers(&[(1, 0.0), (2, 1.0), (12, 0.0)]), |frame, owner| {
        if frame == 0 {
            let (right, _) = owner.controller("right").expect("right");
            owner.claim(right, &["grip"]);
        }
    });
    assert_eq!(presses_of_the_left_button(&owner), [1, 0], "the trigger still clicks");
}

/// The shell's moment: claim `name` after frame `at`.
fn claim_after(at: usize, name: &'static str) -> impl FnMut(usize, &Owner) {
    move |frame, owner| {
        if frame == at {
            let (right, _) = owner.controller("right").expect("right");
            owner.claim(right, &[name]);
        }
    }
}

#[test]
fn the_trigger_and_the_stick_are_held_and_claimed_as_the_grip_is() {
    let trigger_hands = || triggers(&[(1, 0.0), (16, 1.0), (2, 0.0)]);
    let stick_hands = || sticks(&[(1, 0.0), (16, 1.0), (2, 0.0)]);

    // Claimed inside the hold: nothing reaches the window.
    let owner = run_hands(trigger_hands(), claim_after(4, "trigger"));
    assert!(presses_of_the_left_button(&owner).is_empty());
    let owner = run_hands(stick_hands(), claim_after(4, "thumbstick"));
    assert_eq!(scrolls(&owner), 0);

    // Not claimed: forwarded after the hold, and a tap is a press and a release.
    let owner = run_hands(trigger_hands(), |_, _| {});
    assert_eq!(presses_of_the_left_button(&owner), [1, 0]);
    let owner = run_hands(triggers(&[(1, 0.0), (2, 1.0), (12, 0.0)]), |_, _| {});
    assert_eq!(presses_of_the_left_button(&owner), [1, 0], "a tap is a click");
    let owner = run_hands(stick_hands(), |_, _| {});
    assert!(scrolls(&owner) > 0, "a held stick scrolls once the hold is over");
    let owner = run_hands(sticks(&[(1, 0.0), (2, 1.0), (14, 0.0)]), |_, _| {});
    assert_eq!(scrolls(&owner), 0, "a flick shorter than the hold scrolls nothing");
}

#[test]
fn a_shell_that_never_answers_delays_only_the_press_and_the_pose_and_the_frames_go_on() {
    // No claim is ever written. The hand moves on every frame (along its own ray, so it keeps aiming at the window),
    // so the controller node is written on every frame.
    let frames = 20_i32;
    let moving: Vec<Hands> = (0..frames)
        .map(|n| Hands {
            right: Some(HandState {
                aim: Some(Pose {
                    position: [0.0, 0.0, -0.01 * f64::from(n)],
                    orientation: IDENTITY,
                }),
                grip: if (1..18).contains(&n) { 1.0 } else { 0.0 },
                ..HandState::default()
            }),
            ..Hands::default()
        })
        .collect();
    let mut per_frame = Vec::new();
    let mut writes = Vec::new();
    let (owner, ended) = run_hands_counting_frames(moving, |_, owner| {
        per_frame.push(presses_of_the_right_button(owner).len());
        let before: usize = writes.iter().sum();
        writes.push(controller_transactions(owner).len() - before);
    });
    assert_eq!(
        presses_of_the_right_button(&owner),
        [1, 0],
        "forwarded after the deadline with nobody to claim it, and released with the squeeze"
    );
    assert_eq!(frame_of_right_event(&per_frame, 1), Some(1 + HOLD_FRAMES));
    assert!(
        writes[..=HOLD_FRAMES + 1].iter().all(|count| *count >= 1),
        "the controller was written on every frame of the hold: {writes:?}"
    );
    assert_eq!(ended, 20, "every frame was ended, the hold or not");
}
