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
fn run_hands(hands: Vec<Hands>, mut after_frame: impl FnMut(usize, &Owner)) -> Owner {
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
    owner
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

#[test]
fn an_unclaimed_grip_still_reaches_the_window_as_the_right_button() {
    let owner = run_hands(
        vec![pointing(0.0), pointing(1.0), pointing(1.0), pointing(0.0)],
        |_, _| {},
    );
    assert_eq!(
        presses_of_the_right_button(&owner),
        [1, 0],
        "pressed with the grip, released with it"
    );
}

#[test]
fn a_claimed_grip_is_withheld_from_the_window_while_the_node_still_says_it_is_squeezed() {
    let mut squeezed_while_claimed = false;
    let owner = run_hands(
        vec![pointing(0.0), pointing(1.0), pointing(1.0), pointing(0.0)],
        |frame, owner| {
            if frame == 0 {
                // The shell's Grip binding claims the input as soon as it sees the hand.
                let (right, _) = owner.controller("right").expect("the hand arrived in the first frame");
                owner.claim(right, &["grip"]);
            }
            if frame == 2 {
                let (_, node) = owner.controller("right").expect("right");
                squeezed_while_claimed = node.number("grip") == Some(1.0);
            }
        },
    );
    assert_eq!(
        presses_of_the_right_button(&owner),
        Vec::<u64>::new(),
        "no right click in the window"
    );
    assert!(squeezed_while_claimed, "the shell reads the grip it claimed");
    // The rest of the pointer is untouched: it moved over the window.
    assert!(inputs(&owner).iter().any(|(kind, ..)| kind == "motion"));
}

#[test]
fn a_grip_the_window_already_has_is_released_when_the_claim_arrives() {
    let owner = run_hands(
        vec![pointing(1.0), pointing(1.0), pointing(1.0), pointing(0.0)],
        |frame, owner| {
            if frame == 0 {
                let (right, _) = owner.controller("right").expect("right");
                owner.claim(right, &["grip"]);
            }
        },
    );
    assert_eq!(
        presses_of_the_right_button(&owner),
        [1, 0],
        "pressed before the claim, released by it, and the squeeze that goes on is the shell's"
    );
}

#[test]
fn a_claim_of_one_input_leaves_the_others_to_the_window() {
    let owner = run_hands(
        vec![
            Hands {
                right: Some(HandState {
                    trigger: 0.0,
                    ..pointing(0.0).right.expect("right")
                }),
                ..Hands::default()
            },
            Hands {
                right: Some(HandState {
                    trigger: 1.0,
                    ..pointing(0.0).right.expect("right")
                }),
                ..Hands::default()
            },
            pointing(0.0),
        ],
        |frame, owner| {
            if frame == 0 {
                let (right, _) = owner.controller("right").expect("right");
                owner.claim(right, &["grip"]);
            }
        },
    );
    let lefts: Vec<u64> = inputs(&owner)
        .into_iter()
        .filter(|(kind, button, _)| kind == "button" && *button == 0x110)
        .map(|(_, _, pressed)| pressed)
        .collect();
    assert_eq!(lefts, [1, 0], "the trigger still clicks");
}
