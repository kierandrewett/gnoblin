//! Tests of this module.

use super::claims::{
    JsClaimOutcome, claim, claim_args, claim_device, claim_gesture, claim_pointer, grab, release_all, unclaim,
};
use super::seats::{JsSeatSettings, clear_focus, devices, focus, pointer, seat_name, seats, set_builtin_motion};
use gnoblin_tree_client::tree_mirror::{MirrorValue, SnapshotNode, TreeMirror, TxOp};

use crate::bindings::HostState;

fn node(id: u64, parent: u64, class: &str, props: Vec<(String, MirrorValue)>) -> SnapshotNode {
    SnapshotNode {
        id,
        parent,
        index: 0,
        class: class.into(),
        title: String::new(),
        app_id: String::new(),
        props,
    }
}

fn state() -> HostState {
    let seat = vec![
        ("name".to_owned(), MirrorValue::Text("seat0".into())),
        ("pointer_x".to_owned(), MirrorValue::Double(12.5)),
        ("pointer_y".to_owned(), MirrorValue::Double(-3.0)),
        ("focus".to_owned(), MirrorValue::Node(3)),
        (
            "device".to_owned(),
            MirrorValue::Texts(vec!["1:Keeb: Pro:keyboard".into(), "2:Mouse:pointer".into()]),
        ),
    ];
    let nodes = [
        node(1, 0, "node", Vec::new()),
        node(2, 1, "workspace", Vec::new()),
        node(3, 2, "window", Vec::new()),
        node(4, 1, "seat", seat),
    ];
    HostState::new(TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror"))
}

#[test]
fn a_seat_is_found_and_read_from_the_mirror() {
    let state = state();
    assert_eq!(seats(&state), [4.0]);
    assert_eq!(seat_name(&state, 4), "seat0");
    assert_eq!(pointer(&state, 4), [12.5, -3.0]);
    assert_eq!(focus(&state, 4), 3.0);
    assert_eq!(devices(&state, 4).len(), 2);
    // A node that is not a seat answers as an absent one.
    assert_eq!(pointer(&state, 3), [0.0, 0.0]);
    assert_eq!(seat_name(&state, 99), "");
}

#[test]
fn clearing_the_focus_queues_a_write_of_no_node_and_only_a_seat_takes_it() {
    let mut state = state();
    assert_eq!(
        clear_focus(&mut state, 3),
        Some("unknown-node"),
        "a window is not a seat"
    );
    assert_eq!(clear_focus(&mut state, 99), Some("unknown-node"));
    assert!(state.tick.ops().is_empty());
    assert_eq!(clear_focus(&mut state, 4), None);
    assert!(matches!(
        state.tick.ops(),
        [TxOp::Set { node: 4, key, value: MirrorValue::Node(0), .. }] if key == "focus"
    ));
}

#[test]
fn turning_the_built_in_motion_off_queues_a_write_of_the_flag_and_only_a_seat_takes_it() {
    let mut state = state();
    assert_eq!(
        set_builtin_motion(&mut state, 3, false),
        Some("unknown-node"),
        "a window is not a seat"
    );
    assert_eq!(set_builtin_motion(&mut state, 99, false), Some("unknown-node"));
    assert!(state.tick.ops().is_empty());
    assert_eq!(set_builtin_motion(&mut state, 4, false), None);
    assert!(matches!(
        state.tick.ops(),
        [TxOp::Set { node: 4, key, value: MirrorValue::Bool(false), .. }] if key == "builtin_motion"
    ));
}

#[test]
fn seat_settings_are_the_schema_defaults_until_the_compositor_publishes_them() {
    let defaults = JsSeatSettings::of(&state(), 4);
    assert_eq!(
        defaults,
        JsSeatSettings {
            edge_tiling: false,
            focus_mode: "click".into(),
            focus_new_windows: "prevent".into(),
            raise_on_click: true,
            auto_raise: false,
            auto_raise_delay: 500.0,
            focus_change_on_pointer_rest: false,
            reduced_motion: false,
            builtin_motion: true,
        }
    );
    let published = vec![
        ("edge_tiling".to_owned(), MirrorValue::Bool(true)),
        ("focus_mode".to_owned(), MirrorValue::Text("hover".into())),
        ("raise_on_click".to_owned(), MirrorValue::Bool(false)),
        ("auto_raise_delay".to_owned(), MirrorValue::Int(120)),
        ("reduced_motion".to_owned(), MirrorValue::Bool(true)),
        ("builtin_motion".to_owned(), MirrorValue::Bool(false)),
    ];
    let nodes = [node(1, 0, "node", Vec::new()), node(4, 1, "seat", published)];
    let state = HostState::new(TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror"));
    let settings = JsSeatSettings::of(&state, 4);
    assert!(settings.edge_tiling && !settings.raise_on_click);
    assert!(settings.reduced_motion, "the seat's reduced_motion is read");
    assert!(!settings.builtin_motion, "the seat's builtin_motion is read");
    assert_eq!(settings.focus_mode, "hover");
    assert_eq!(settings.auto_raise_delay, 120.0);
    assert_eq!(
        settings.focus_new_windows, "prevent",
        "an unpublished key keeps its default"
    );
}

#[test]
fn a_claim_is_a_call_with_little_endian_arguments_and_the_chord_as_its_text() {
    let mut state = state();
    assert_eq!(claim(&mut state, 4, "<Super>1", true, ""), JsClaimOutcome::Claimed(1));
    assert_eq!(
        claim(&mut state, 4, "<Super>Button1", false, "Macro Pad"),
        JsClaimOutcome::Claimed(2)
    );
    assert_eq!(claim_gesture(&mut state, 4, "swipe", 3, ""), JsClaimOutcome::Claimed(3));
    let calls: Vec<(&str, &[u8], String)> = state
        .tick
        .ops()
        .iter()
        .filter_map(|op| match op {
            TxOp::Call { method, args, text, .. } => Some((method.as_str(), args.as_slice(), text.expose().to_owned())),
            _ => None,
        })
        .collect();
    assert_eq!(calls[0], ("claim", claim_args(1, 1).as_slice(), "<Super>1".to_owned()));
    assert_eq!(
        calls[1],
        (
            "claim",
            claim_args(2, 0).as_slice(),
            "Macro Pad\n<Super>Button1".to_owned()
        )
    );
    assert_eq!(
        calls[2],
        ("claim_gesture", claim_args(3, 3).as_slice(), "swipe".to_owned())
    );
    assert_eq!(state.input.claimed(), 3);
}

#[test]
fn a_device_claim_is_a_call_with_the_id_and_the_bare_device_name_as_its_text() {
    // The retired test "Seat:claim_device is a one-line device name" and the one of `bind{ only = true }`.
    let mut state = state();
    assert_eq!(claim_device(&mut state, 4, "Macro Pad"), JsClaimOutcome::Claimed(1));
    let calls: Vec<(&str, &[u8], String)> = state
        .tick
        .ops()
        .iter()
        .filter_map(|op| match op {
            TxOp::Call { method, args, text, .. } => Some((method.as_str(), args.as_slice(), text.expose().to_owned())),
            _ => None,
        })
        .collect();
    assert_eq!(
        calls,
        [("claim_device", 1_i64.to_le_bytes().as_slice(), "Macro Pad".to_owned())]
    );
    for bad in ["", "two\nlines"] {
        assert_eq!(
            claim_device(&mut state, 4, bad),
            JsClaimOutcome::Refused("invalid-value"),
            "{bad:?}"
        );
    }
    assert_eq!(
        claim_device(&mut state, 3, "Macro Pad"),
        JsClaimOutcome::Refused("unknown-node")
    );
    assert_eq!(state.input.claimed(), 1, "a refused claim holds nothing");
}

#[test]
fn a_claim_on_anything_but_a_seat_is_refused_and_ids_are_not_reused() {
    let mut state = state();
    assert_eq!(
        claim(&mut state, 3, "<Super>1", false, ""),
        JsClaimOutcome::Refused("unknown-node")
    );
    assert_eq!(
        claim(&mut state, 99, "<Super>1", false, ""),
        JsClaimOutcome::Refused("unknown-node")
    );
    assert_eq!(
        claim(&mut state, 4, "<Super>1", false, "two\nlines"),
        JsClaimOutcome::Refused("invalid-value")
    );
    assert!(state.tick.ops().is_empty());
    assert_eq!(claim(&mut state, 4, "<Super>1", false, ""), JsClaimOutcome::Claimed(1));
    unclaim(&mut state, 1);
    assert_eq!(claim(&mut state, 4, "<Super>1", false, ""), JsClaimOutcome::Claimed(2));
}

#[test]
fn unclaim_calls_once_and_ignores_an_id_it_does_not_hold() {
    let mut state = state();
    claim(&mut state, 4, "<Super>1", false, "");
    unclaim(&mut state, 1);
    unclaim(&mut state, 1);
    unclaim(&mut state, 77);
    let methods: Vec<&str> = state
        .tick
        .ops()
        .iter()
        .filter_map(|op| match op {
            TxOp::Call { method, .. } => Some(method.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(methods, ["claim", "unclaim"]);
    assert_eq!(state.input.claimed(), 0);
}

/// The calls a state has queued, as `(method, arguments, text)`.
fn calls_of(state: &HostState) -> Vec<(String, Vec<u8>, String)> {
    state
        .tick
        .ops()
        .iter()
        .filter_map(|op| match op {
            TxOp::Call { method, args, text, .. } => Some((method.clone(), args.clone(), text.expose().to_owned())),
            _ => None,
        })
        .collect()
}

#[test]
fn a_pointer_claim_is_a_call_with_the_id_and_the_bare_device_name_as_its_text() {
    let mut state = state();
    assert_eq!(claim_pointer(&mut state, 4, "Side Pad"), JsClaimOutcome::Claimed(1));
    assert_eq!(
        calls_of(&state),
        [(
            "claim_pointer".to_owned(),
            1_i64.to_le_bytes().to_vec(),
            "Side Pad".to_owned()
        )]
    );
    // Every button of every pointer is not a claim a shell can make: the name is required, and one line.
    for bad in ["", "two\nlines"] {
        assert_eq!(
            claim_pointer(&mut state, 4, bad),
            JsClaimOutcome::Refused("invalid-value"),
            "{bad:?}"
        );
    }
    assert_eq!(
        claim_pointer(&mut state, 3, "Side Pad"),
        JsClaimOutcome::Refused("unknown-node")
    );
    assert_eq!(state.input.claimed(), 1, "a refused claim holds nothing");
}

#[test]
fn a_grab_is_a_claim_whose_second_argument_is_its_node_and_whose_text_is_its_scope() {
    let mut state = state();
    assert_eq!(grab(&mut state, 4, 1, "keyboard"), JsClaimOutcome::Claimed(1));
    assert_eq!(
        calls_of(&state),
        [("grab".to_owned(), claim_args(1, 1), "keyboard".to_owned())]
    );
    for bad in ["", "Pointer", "mouse", "keyboard\n"] {
        assert_eq!(
            grab(&mut state, 4, 1, bad),
            JsClaimOutcome::Refused("invalid-value"),
            "{bad:?}: a scope is pointer, keyboard or both"
        );
    }
    assert_eq!(grab(&mut state, 3, 1, "both"), JsClaimOutcome::Refused("unknown-node"));
    assert_eq!(state.input.claimed(), 1, "a refused grab holds nothing");
    unclaim(&mut state, 1);
    assert_eq!(
        calls_of(&state).last().map(|call| call.0.as_str()),
        Some("unclaim"),
        "a grab is given up as a claim is"
    );
}

#[test]
fn a_generation_that_ends_gives_its_grab_and_its_pointer_claim_up() {
    let mut state = state();
    assert_eq!(claim(&mut state, 4, "<Super>1", false, ""), JsClaimOutcome::Claimed(1));
    assert_eq!(grab(&mut state, 4, 1, "pointer"), JsClaimOutcome::Claimed(2));
    assert_eq!(claim_pointer(&mut state, 4, "Side Pad"), JsClaimOutcome::Claimed(3));
    release_all(&mut state);
    let methods: Vec<String> = calls_of(&state).into_iter().map(|call| call.0).collect();
    assert_eq!(
        methods,
        ["claim", "grab", "claim_pointer", "unclaim", "unclaim", "unclaim"],
        "every claim is given up, the grab with them, so the next generation grabs afresh"
    );
    assert_eq!(state.input.claimed(), 0);
}
