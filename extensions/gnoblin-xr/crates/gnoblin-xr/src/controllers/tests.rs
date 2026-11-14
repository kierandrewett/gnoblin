//! The controller nodes against a fake owner that keeps the tree the program builds.
//!
//! The runtime's side is scripted `Hands`; the owner's side is [`owner::serve`], which applies the program's `new`, `set`
//! and `destroy` ops to a tree of its own and publishes the changes. What these tests prove is the program's side of the
//! contract in `design/js-xr.md`: which nodes exist, what they hold, how often they are written, and how a claim is read
//! back. Nothing here met a real compositor, a shell or a headset.

use std::os::unix::net::UnixStream;

use gnoblin_proto::tree::op;
use xr_input::{Claims, Hand, HandState, Hands};
use xr_scene::{IDENTITY, Pose};
use xr_tree::TreeClient;

use super::owner::{self, Owner};
use super::{Controllers, MIN_INTERVAL};

/// The room's unit, as the stock shell's head camera sets it.
const METRES: f64 = 0.001;
/// One frame of a 60 Hz display, in nanoseconds: over [`MIN_INTERVAL`].
const FRAME: i64 = 16_666_667;

fn connect() -> (TreeClient, Owner) {
    let (stream, owner): (UnixStream, Owner) = owner::serve(Vec::new());
    (TreeClient::from_stream(stream).expect("attach"), owner)
}

fn near(got: Option<f64>, want: f64, what: &str) {
    let got = got.unwrap_or_else(|| panic!("{what} was never written"));
    assert!((got - want).abs() < 1e-9, "{what}: got {got}, want {want}");
}

fn at(x: f64, y: f64, z: f64) -> Pose {
    Pose {
        position: [x, y, z],
        orientation: IDENTITY,
    }
}

fn hand_at(x: f64, y: f64, z: f64) -> HandState {
    HandState {
        aim: Some(at(x, y, z)),
        ..HandState::default()
    }
}

fn both(left: HandState, right: HandState) -> Hands {
    Hands {
        left: Some(left),
        right: Some(right),
        ..Hands::default()
    }
}

fn right_only(right: HandState) -> Hands {
    Hands {
        right: Some(right),
        ..Hands::default()
    }
}

/// The `(node, key)` of every `set` of a transaction.
fn sets(ops: &[op::Properties]) -> Vec<(u64, &str)> {
    ops.iter()
        .filter(|op| op.kind == "set")
        .map(|op| (op.node, op.key.as_str()))
        .collect()
}

#[test]
fn a_tracked_hand_gets_a_node_that_holds_every_value_and_the_pose_in_the_rooms_units() {
    let (mut tree, owner) = connect();
    let mut controllers = Controllers::default();
    let hand = HandState {
        aim: Some(at(0.25, 1.1, -0.4)),
        trigger: 0.5,
        grip: 1.0,
        thumbstick: (-1.0, 0.25),
        thumbstick_click: true,
        primary: true,
        secondary: false,
    };

    controllers
        .update(&mut tree, &right_only(hand), METRES, 0)
        .expect("update");

    assert_eq!(owner.count(), 1, "the node and its values are one transaction");
    let (_, node) = owner.controller("right").expect("a controller for the right hand");
    assert_eq!(node.class, "controller");
    assert_eq!(node.flag("tracked"), Some(true));
    near(node.number("x"), 250.0, "x in node units, not metres");
    near(node.number("y"), 1100.0, "y, up as Views are");
    near(node.number("z"), -400.0, "z");
    assert_eq!(node.list("rotation"), Some([0.0, 0.0, 0.0].as_slice()));
    near(node.number("trigger"), 0.5, "trigger");
    near(node.number("grip"), 1.0, "grip");
    near(node.number("thumbstick_x"), -1.0, "thumbstick x");
    near(node.number("thumbstick_y"), 0.25, "thumbstick y");
    assert_eq!(node.flag("thumbstick_click"), Some(true));
    assert_eq!(node.flag("primary"), Some(true));
    assert_eq!(node.flag("secondary"), Some(false));
    assert!(owner.controller("left").is_none(), "the left hand is not there");
}

#[test]
fn the_rotation_is_written_in_radians_as_the_tree_states_angles() {
    let (mut tree, owner) = connect();
    let mut controllers = Controllers::default();
    // A quarter turn about y.
    let half = std::f64::consts::FRAC_PI_4;
    let hand = HandState {
        aim: Some(Pose {
            position: [0.0; 3],
            orientation: [0.0, half.sin(), 0.0, half.cos()],
        }),
        ..HandState::default()
    };
    controllers
        .update(&mut tree, &right_only(hand), METRES, 0)
        .expect("update");
    let (_, node) = owner.controller("right").expect("controller");
    let rotation = node.list("rotation").expect("rotation");
    assert!(rotation[0].abs() < 1e-9 && rotation[2].abs() < 1e-9, "{rotation:?}");
    assert!((rotation[1] - std::f64::consts::FRAC_PI_2).abs() < 1e-9, "{rotation:?}");
}

#[test]
fn two_hands_are_two_nodes_each_with_its_own_hand() {
    let (mut tree, owner) = connect();
    let mut controllers = Controllers::default();
    controllers
        .update(
            &mut tree,
            &both(hand_at(-0.2, 1.0, -0.3), hand_at(0.2, 1.0, -0.3)),
            METRES,
            0,
        )
        .expect("update");
    let (left, left_node) = owner.controller("left").expect("left");
    let (right, right_node) = owner.controller("right").expect("right");
    assert_ne!(left, right);
    near(left_node.number("x"), -200.0, "left x");
    near(right_node.number("x"), 200.0, "right x");
}

#[test]
fn a_frame_writes_the_changed_values_of_both_hands_in_one_transaction_and_nothing_else() {
    let (mut tree, owner) = connect();
    let mut controllers = Controllers::default();
    let start = both(hand_at(-0.2, 1.0, -0.3), hand_at(0.2, 1.0, -0.3));
    controllers.update(&mut tree, &start, METRES, 0).expect("make");
    let (left, _) = owner.controller("left").expect("left");
    let (right, _) = owner.controller("right").expect("right");
    let made = owner.count();

    let mut next = start;
    next.left.as_mut().expect("left").grip = 1.0;
    next.right.as_mut().expect("right").trigger = 0.75;
    controllers.update(&mut tree, &next, METRES, FRAME).expect("write");

    assert_eq!(owner.count(), made + 1, "one transaction for the frame");
    let transactions = owner.transactions();
    assert_eq!(
        sets(&transactions[made]),
        [(left, "grip"), (right, "trigger")],
        "only what changed, hand by hand"
    );
    near(
        owner.controller("left").expect("left").1.number("grip"),
        1.0,
        "left grip",
    );
    near(
        owner.controller("right").expect("right").1.number("trigger"),
        0.75,
        "right trigger",
    );
}

#[test]
fn a_hand_that_has_not_changed_writes_nothing() {
    let (mut tree, owner) = connect();
    let mut controllers = Controllers::default();
    let hands = right_only(hand_at(0.2, 1.0, -0.3));
    controllers.update(&mut tree, &hands, METRES, 0).expect("make");
    let made = owner.count();
    for frame in 1..5 {
        controllers
            .update(&mut tree, &hands, METRES, frame * FRAME)
            .expect("still");
    }
    assert_eq!(owner.count(), made, "no frame wrote");
}

#[test]
fn a_moving_pose_writes_its_position_and_rotation_and_leaves_the_buttons_alone() {
    let (mut tree, owner) = connect();
    let mut controllers = Controllers::default();
    controllers
        .update(&mut tree, &right_only(hand_at(0.0, 1.0, -0.3)), METRES, 0)
        .expect("make");
    let (right, _) = owner.controller("right").expect("right");
    let made = owner.count();
    controllers
        .update(&mut tree, &right_only(hand_at(0.1, 1.0, -0.3)), METRES, FRAME)
        .expect("move");
    assert_eq!(sets(&owner.transactions()[made]), [(right, "x")], "only x moved");
}

#[test]
fn a_lost_hand_destroys_its_node_and_a_hand_that_returns_is_a_new_one() {
    let (mut tree, owner) = connect();
    let mut controllers = Controllers::default();
    controllers
        .update(
            &mut tree,
            &both(hand_at(-0.2, 1.0, -0.3), hand_at(0.2, 1.0, -0.3)),
            METRES,
            0,
        )
        .expect("make");
    let (first_left, _) = owner.controller("left").expect("left");
    let (right, _) = owner.controller("right").expect("right");

    controllers
        .update(&mut tree, &right_only(hand_at(0.2, 1.0, -0.3)), METRES, FRAME)
        .expect("lose the left hand");
    assert!(owner.controller("left").is_none(), "the lost hand's node is gone");
    assert_eq!(
        owner.controller("right").map(|(id, _)| id),
        Some(right),
        "the other is untouched"
    );
    let last = owner.transactions().pop().expect("a transaction");
    assert_eq!(
        last.iter().map(|op| (op.kind.as_str(), op.node)).collect::<Vec<_>>(),
        [("destroy", first_left)]
    );

    controllers
        .update(
            &mut tree,
            &both(hand_at(-0.4, 1.2, -0.5), hand_at(0.2, 1.0, -0.3)),
            METRES,
            2 * FRAME,
        )
        .expect("the left hand returns");
    let (second_left, node) = owner.controller("left").expect("a controller for the left hand again");
    assert_ne!(second_left, first_left, "a new node, so the shell sees a hand arrive");
    near(node.number("x"), -400.0, "the pose of the hand that came back");
}

#[test]
fn a_hand_that_is_lost_while_a_write_is_thinned_is_still_destroyed_at_once() {
    let (mut tree, owner) = connect();
    let mut controllers = Controllers::default();
    controllers
        .update(&mut tree, &right_only(hand_at(0.2, 1.0, -0.3)), METRES, 0)
        .expect("make");
    controllers
        .update(&mut tree, &Hands::default(), METRES, 1)
        .expect("lose it a nanosecond later");
    assert!(
        owner.controller("right").is_none(),
        "losing a hand is not a write that waits"
    );
}

#[test]
fn a_hand_the_runtime_cannot_locate_keeps_its_last_pose_and_reads_untracked() {
    let (mut tree, owner) = connect();
    let mut controllers = Controllers::default();
    controllers
        .update(&mut tree, &right_only(hand_at(0.2, 1.0, -0.3)), METRES, 0)
        .expect("make");
    let (right, _) = owner.controller("right").expect("right");
    let made = owner.count();

    let out_of_sight = HandState {
        aim: None,
        trigger: 1.0,
        ..HandState::default()
    };
    controllers
        .update(&mut tree, &right_only(out_of_sight), METRES, FRAME)
        .expect("out of sight");
    assert_eq!(
        sets(&owner.transactions()[made]),
        [(right, "tracked"), (right, "trigger")],
        "the pose is not rewritten"
    );
    let (_, node) = owner.controller("right").expect("the node stays");
    assert_eq!(node.flag("tracked"), Some(false));
    near(node.number("x"), 200.0, "the last pose is kept");

    let back = HandState {
        trigger: 1.0,
        ..hand_at(0.3, 1.0, -0.3)
    };
    controllers
        .update(&mut tree, &right_only(back), METRES, 2 * FRAME)
        .expect("back in sight");
    let (_, node) = owner.controller("right").expect("node");
    assert_eq!(node.flag("tracked"), Some(true));
    near(node.number("x"), 300.0, "the new pose");
}

#[test]
fn a_hand_that_is_out_of_sight_when_first_seen_starts_untracked_at_the_origin() {
    let (mut tree, owner) = connect();
    let mut controllers = Controllers::default();
    controllers
        .update(&mut tree, &right_only(HandState::default()), METRES, 0)
        .expect("make");
    let (_, node) = owner.controller("right").expect("right");
    assert_eq!(node.flag("tracked"), Some(false));
    near(node.number("x"), 0.0, "x");
    near(node.number("y"), 0.0, "y");
    near(node.number("z"), 0.0, "z");
}

#[test]
fn a_pose_that_cannot_be_written_is_taken_for_a_hand_out_of_sight() {
    let (mut tree, owner) = connect();
    let mut controllers = Controllers::default();
    controllers
        .update(&mut tree, &right_only(hand_at(f64::NAN, 1.0, -0.3)), METRES, 0)
        .expect("a pose that is not a number does not fail the frame");
    let (_, node) = owner.controller("right").expect("right");
    assert_eq!(node.flag("tracked"), Some(false));
}

#[test]
fn values_the_owner_would_refuse_are_clamped_or_taken_for_rest() {
    let (mut tree, owner) = connect();
    let mut controllers = Controllers::default();
    let hand = HandState {
        trigger: f32::NAN,
        grip: 7.0,
        thumbstick: (f32::INFINITY, -9.0),
        ..hand_at(0.0, 1.0, -0.3)
    };
    controllers
        .update(&mut tree, &right_only(hand), METRES, 0)
        .expect("update");
    let (_, node) = owner.controller("right").expect("right");
    near(node.number("trigger"), 0.0, "a trigger that is not a number");
    near(node.number("grip"), 1.0, "a grip past 1");
    near(node.number("thumbstick_x"), 0.0, "a stick that is not finite");
    near(node.number("thumbstick_y"), -1.0, "a stick past -1");
}

#[test]
fn frames_closer_than_the_interval_are_thinned_and_the_last_values_still_arrive() {
    let (mut tree, owner) = connect();
    let mut controllers = Controllers::default();
    let trigger = |value: f32| {
        right_only(HandState {
            trigger: value,
            ..hand_at(0.2, 1.0, -0.3)
        })
    };
    controllers.update(&mut tree, &trigger(0.0), METRES, 0).expect("make");
    let made = owner.count();

    // A display much faster than the compositor takes writes: a frame every millisecond, the trigger changing in each.
    for step in 1..MIN_INTERVAL / 1_000_000 {
        controllers
            .update(&mut tree, &trigger(step as f32 / 20.0), METRES, step * 1_000_000)
            .expect("thinned");
    }
    assert_eq!(owner.count(), made, "no frame inside the interval wrote");

    controllers
        .update(&mut tree, &trigger(1.0), METRES, MIN_INTERVAL)
        .expect("the first frame past the interval");
    assert_eq!(owner.count(), made + 1, "one write for all the frames");
    near(
        owner.controller("right").expect("right").1.number("trigger"),
        1.0,
        "the latest value",
    );

    // And the next frame is thinned by the write that just happened, not by the one before it.
    controllers
        .update(&mut tree, &trigger(0.5), METRES, MIN_INTERVAL + 1_000_000)
        .expect("thinned again");
    assert_eq!(owner.count(), made + 1);
}

#[test]
fn a_clock_that_steps_back_does_not_stop_the_writes() {
    let (mut tree, owner) = connect();
    let mut controllers = Controllers::default();
    let grip = |value: f32| {
        right_only(HandState {
            grip: value,
            ..hand_at(0.2, 1.0, -0.3)
        })
    };
    controllers
        .update(&mut tree, &grip(0.0), METRES, 10 * FRAME)
        .expect("make");
    let made = owner.count();
    controllers
        .update(&mut tree, &grip(1.0), METRES, FRAME)
        .expect("an earlier display time");
    assert_eq!(owner.count(), made + 1);
}

/// Waits for what the owner has published to reach the client's mirror.
fn catch_up(tree: &mut TreeClient) {
    for _ in 0..200 {
        if tree.poll().expect("poll") {
            return;
        }
    }
    panic!("the owner's change never arrived");
}

#[test]
fn the_claims_are_what_the_shell_wrote_on_the_node_and_nothing_before_it_did() {
    let (mut tree, owner) = connect();
    let mut controllers = Controllers::default();
    assert_eq!(
        controllers.claims(&tree, Hand::Right),
        Claims::default(),
        "no node, no claim"
    );
    controllers
        .update(&mut tree, &right_only(hand_at(0.2, 1.0, -0.3)), METRES, 0)
        .expect("make");
    assert_eq!(
        controllers.claims(&tree, Hand::Right),
        Claims::default(),
        "nothing claimed yet"
    );
    let (right, _) = owner.controller("right").expect("right");

    owner.claim(right, &["grip", "thumbstick", "bogus"]);
    catch_up(&mut tree);
    assert_eq!(
        controllers.claims(&tree, Hand::Right),
        Claims {
            grip: true,
            thumbstick: true,
            ..Claims::default()
        }
    );
    assert_eq!(
        controllers.claims(&tree, Hand::Left),
        Claims::default(),
        "the left hand has no node"
    );

    owner.claim(right, &[]);
    catch_up(&mut tree);
    assert_eq!(
        controllers.claims(&tree, Hand::Right),
        Claims::default(),
        "the shell let go"
    );
}

#[test]
fn a_claim_does_not_stop_the_values_being_written() {
    let (mut tree, owner) = connect();
    let mut controllers = Controllers::default();
    controllers
        .update(&mut tree, &right_only(hand_at(0.2, 1.0, -0.3)), METRES, 0)
        .expect("make");
    let (right, _) = owner.controller("right").expect("right");
    owner.claim(right, &["grip"]);
    catch_up(&mut tree);
    let squeezed = right_only(HandState {
        grip: 1.0,
        ..hand_at(0.2, 1.0, -0.3)
    });
    controllers.update(&mut tree, &squeezed, METRES, FRAME).expect("write");
    near(
        owner.controller("right").expect("right").1.number("grip"),
        1.0,
        "the shell reads the grip it claimed",
    );
}

#[test]
fn a_write_built_on_a_mirror_that_lagged_is_written_again_at_the_next_frame() {
    let (mut tree, owner) = connect();
    let mut controllers = Controllers::default();
    controllers
        .update(&mut tree, &right_only(hand_at(0.2, 1.0, -0.3)), METRES, 0)
        .expect("make");
    let grip = right_only(HandState {
        grip: 1.0,
        ..hand_at(0.2, 1.0, -0.3)
    });
    owner.refuse_next("stale-base");
    controllers
        .update(&mut tree, &grip, METRES, FRAME)
        .expect("a stale base is not an error");
    assert_eq!(
        owner.controller("right").expect("right").1.number("grip"),
        Some(0.0),
        "the refused write changed nothing"
    );
    controllers.update(&mut tree, &grip, METRES, 2 * FRAME).expect("again");
    near(
        owner.controller("right").expect("right").1.number("grip"),
        1.0,
        "it landed",
    );
}

#[test]
fn any_other_refusal_is_not_fatal_and_the_values_are_written_when_the_owner_takes_them() {
    let (mut tree, owner) = connect();
    let mut controllers = Controllers::default();
    owner.refuse_next("rate-limited");
    controllers
        .update(&mut tree, &right_only(hand_at(0.2, 1.0, -0.3)), METRES, 0)
        .expect("a refused node is not an error");
    assert!(owner.controller("right").is_none(), "nothing was made");
    // The next frame is inside the interval of the one that was refused: the refusal counts as a write.
    controllers
        .update(&mut tree, &right_only(hand_at(0.2, 1.0, -0.3)), METRES, MIN_INTERVAL)
        .expect("again");
    assert!(owner.controller("right").is_some(), "made when the owner takes it");

    owner.refuse_next("not-finite");
    let squeezed = right_only(HandState {
        grip: 1.0,
        ..hand_at(0.2, 1.0, -0.3)
    });
    controllers
        .update(&mut tree, &squeezed, METRES, 2 * MIN_INTERVAL)
        .expect("a refused write is not an error");
    controllers
        .update(&mut tree, &squeezed, METRES, 3 * MIN_INTERVAL)
        .expect("again");
    near(
        owner.controller("right").expect("right").1.number("grip"),
        1.0,
        "written at last",
    );
}

#[test]
fn a_socket_that_fails_ends_the_update_with_an_error() {
    let (mut tree, owner) = connect();
    let mut controllers = Controllers::default();
    controllers
        .update(&mut tree, &right_only(hand_at(0.2, 1.0, -0.3)), METRES, 0)
        .expect("make");
    owner.hang_up();
    let squeezed = right_only(HandState {
        grip: 1.0,
        ..hand_at(0.2, 1.0, -0.3)
    });
    assert!(controllers.update(&mut tree, &squeezed, METRES, FRAME).is_err());
}

#[test]
fn ending_destroys_every_node_and_a_new_session_makes_them_again() {
    let (mut tree, owner) = connect();
    let mut controllers = Controllers::default();
    let hands = both(hand_at(-0.2, 1.0, -0.3), hand_at(0.2, 1.0, -0.3));
    controllers.update(&mut tree, &hands, METRES, 0).expect("make");
    assert_eq!(owner.nodes_of("controller").len(), 2);

    controllers.end(&mut tree).expect("end");
    assert!(
        owner.nodes_of("controller").is_empty(),
        "the program leaves no hand behind"
    );
    assert_eq!(controllers.claims(&tree, Hand::Right), Claims::default());

    controllers
        .update(&mut tree, &hands, METRES, FRAME)
        .expect("a new session");
    assert_eq!(owner.nodes_of("controller").len(), 2);
    controllers.end(&mut tree).expect("end again");
}

#[test]
fn ending_with_no_hands_sends_nothing() {
    let (mut tree, owner) = connect();
    let mut controllers = Controllers::default();
    controllers
        .update(&mut tree, &Hands::default(), METRES, 0)
        .expect("nothing to make");
    controllers.end(&mut tree).expect("nothing to destroy");
    assert_eq!(owner.count(), 0);
}
