//! Tests of this module.

use gnoblin_tree_client::tree_mirror::{SnapshotNode, TreeMirror, TxOp};

use super::*;

fn snapshot(id: NodeId, parent: NodeId, class: &str) -> SnapshotNode {
    SnapshotNode {
        id,
        parent,
        index: 0,
        class: class.to_owned(),
        title: String::new(),
        app_id: String::new(),
        props: Vec::new(),
    }
}

/// A root (1) with one window (2) under it, attached under epoch 1.
fn state() -> HostState {
    let nodes = [snapshot(1, 0, "node"), snapshot(2, 1, "window")];
    HostState::new(TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror"))
}

#[test]
fn an_id_is_the_epoch_and_a_count() {
    let mut state = state();
    assert_eq!(create(&mut state), Some((1 << 32) | 1));
    assert_eq!(create(&mut state), Some((1 << 32) | 2));
}

#[test]
fn only_canvases_the_toolkit_made_are_writable() {
    let mut state = state();
    let canvas = create(&mut state).expect("a canvas");
    assert_eq!(set_number(&mut state, canvas, "width", 10.0), None);
    assert_eq!(set_flag(&mut state, canvas, "visible", false), None);
    assert_eq!(paint(&mut state, canvas, vec![1, 2, 3]), None);
    assert_eq!(set_number(&mut state, 2, "width", 10.0), Some("not-owned"));
    assert_eq!(destroy(&mut state, 2), Some("not-owned"));
    assert_eq!(set_number(&mut state, canvas, "content", 1.0), Some("unknown-property"));
    assert_eq!(set_number(&mut state, canvas, "x", f64::NAN), Some("invalid-value"));
    assert_eq!(set_flag(&mut state, canvas, "width", true), Some("unknown-property"));
}

#[test]
fn a_canvas_goes_under_any_node_and_destroying_it_twice_is_not_an_error() {
    let mut state = state();
    let canvas = create(&mut state).expect("a canvas");
    assert_eq!(insert(&mut state, canvas, 2, -1), None);
    assert_eq!(destroy(&mut state, canvas), None);
    assert_eq!(destroy(&mut state, canvas), None, "a node already gone is fine");
    assert_eq!(set_number(&mut state, canvas, "x", 1.0), Some("unknown-node"));
}

#[test]
fn a_measurement_carries_the_size_and_the_font_and_its_answer_is_kept_once() {
    let mut state = state();
    let canvas = create(&mut state).expect("a canvas");
    let token = measure(&mut state, canvas, "sans", 10.0, "Hello").expect("a token");
    let calls: Vec<(String, Vec<u8>, String)> = state
        .tick
        .ops()
        .iter()
        .filter_map(|op| match op {
            TxOp::Call { method, args, text, .. } => Some((method.clone(), args.clone(), text.expose().to_owned())),
            _ => None,
        })
        .collect();
    assert_eq!(
        calls,
        [(
            "measure".to_owned(),
            10.0_f64.to_le_bytes().to_vec(),
            "sans\nHello".to_owned()
        )]
    );
    assert!(
        !state.ui.accept_answer(token + 1, b"1 1 1", ""),
        "another call's answer"
    );
    assert!(state.ui.accept_answer(token, b"31 15 12", ""));
    assert!(
        !state.ui.accept_answer(token, b"31 15 12", ""),
        "an answer is kept once"
    );
    assert_eq!(state.ui.answers.len(), 1);
}

fn node_with(id: NodeId, parent: NodeId, class: &str, props: Vec<(String, MirrorValue)>) -> SnapshotNode {
    SnapshotNode {
        props,
        ..snapshot(id, parent, class)
    }
}

/// A root with a seat, a lock, a primary screen of 800 by 600, and two windows, one of them without a parent as the
/// compositor makes them.
fn session() -> HostState {
    let nodes = [
        snapshot(1, 0, "node"),
        node_with(
            2,
            1,
            "seat",
            vec![
                ("pointer_x".into(), MirrorValue::Double(120.0)),
                ("right_click".into(), MirrorValue::Text("none".into())),
            ],
        ),
        node_with(3, 1, "lock", Vec::new()),
        node_with(
            4,
            1,
            "screen",
            vec![
                ("width".into(), MirrorValue::Double(800.0)),
                ("height".into(), MirrorValue::Double(600.0)),
                ("primary".into(), MirrorValue::Bool(true)),
            ],
        ),
        snapshot(5, 0, "window"),
        snapshot(6, 0, "window"),
    ];
    HostState::new(TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror"))
}

#[test]
fn a_key_and_a_request_are_the_toolkits_and_a_key_for_a_node_that_is_not_ours_is_not() {
    let mut state = session();
    let canvas = create(&mut state).expect("a canvas");
    let event = |node: NodeId, kind: &str, key: &str| TreeEvent {
        frame: 0,
        seq: 0,
        node,
        kind: kind.to_owned(),
        fields: gnoblin_tree_client::tree_mirror::EventFields {
            key: key.to_owned(),
            text: key.to_owned(),
            pressed: true,
            request: "menu".to_owned(),
            ..Default::default()
        },
    };
    let handles = NodeHandles::default();
    assert!(state.ui.accept_event(&event(canvas, "key", "a"), &handles));
    assert!(
        !state.ui.accept_event(&event(5, "key", "a"), &handles),
        "a window's key is not ours"
    );
    assert!(
        state.ui.accept_event(&event(5, "request", ""), &handles),
        "a request is, whatever the node"
    );
    assert_eq!(state.ui.events.len(), 2);
    assert_eq!(state.ui.events[0].key, "a");
    assert_eq!(state.ui.events[1].request, "menu");
    assert!(
        !format!("{:?}", state.ui.events[0]).contains("\"a\""),
        "a key typed is never printed"
    );
}

#[test]
fn a_canvas_or_a_window_may_hold_the_keyboard_and_nothing_else_may() {
    let mut state = session();
    let canvas = create(&mut state).expect("a canvas");
    assert_eq!(focus(&mut state, canvas), None);
    assert_eq!(focused(&state), Some(canvas));
    assert_eq!(focus(&mut state, 5), None, "a window gets the keyboard back");
    assert_eq!(focused(&state), Some(5));
    assert_eq!(focus(&mut state, NIL), None);
    assert_eq!(focused(&state), None, "nil takes it away");
    assert_eq!(
        focus(&mut state, 4),
        Some("not-owned"),
        "a screen is not for the keyboard"
    );
    assert_eq!(focus(&mut state, 99), Some("unknown-node"));
    let mut bare = self::state();
    assert_eq!(focus(&mut bare, 2), Some("no-seat"));
}

#[test]
fn the_seat_gives_the_pointer_and_what_a_titlebar_click_does_and_nothing_else() {
    let state = session();
    assert_eq!(seat_number(&state, "pointer_x"), Some(120.0));
    assert_eq!(seat_number(&state, "pointer_y"), Some(0.0));
    assert_eq!(seat_number(&state, "focus_mode"), None);
    assert_eq!(seat_text(&state, "right_click").as_deref(), Some("none"));
    assert_eq!(
        seat_text(&state, "middle_click").as_deref(),
        Some("lower"),
        "the schema's default"
    );
    assert_eq!(seat_text(&state, "double_click").as_deref(), Some("toggle-maximize"));
    assert_eq!(seat_text(&state, "name"), None);
}

#[test]
fn the_pointer_screen_is_the_screen_the_seat_names_and_none_without_one() {
    assert_eq!(pointer_screen(&session()), None, "the seat names no screen");
    let nodes = [
        snapshot(1, 0, "node"),
        node_with(2, 1, "seat", vec![("screen".into(), MirrorValue::Node(4))]),
        node_with(4, 1, "screen", Vec::new()),
    ];
    let state = HostState::new(TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror"));
    assert_eq!(pointer_screen(&state), Some(4.0));
    let nodes = [
        snapshot(1, 0, "node"),
        node_with(2, 1, "seat", vec![("screen".into(), MirrorValue::Node(0))]),
    ];
    let state = HostState::new(TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror"));
    assert_eq!(pointer_screen(&state), None, "a cleared reference is no screen");
}

#[test]
fn the_session_is_locked_while_the_lock_is_engaged_or_not_not_locked() {
    let mut state = session();
    assert!(!locked(&state));
    state
        .tick
        .set(&state.mirror, 3, "engaged", MirrorValue::Bool(true), None, None)
        .expect("a write");
    assert!(locked(&state));
    state
        .tick
        .set(&state.mirror, 3, "engaged", MirrorValue::Bool(false), None, None)
        .expect("a write");
    state
        .tick
        .set(
            &state.mirror,
            3,
            "state",
            MirrorValue::Text("pending".into()),
            None,
            None,
        )
        .expect("a write");
    assert!(locked(&state));
}

#[test]
fn the_screen_is_the_primary_ones_size() {
    assert_eq!(screen(&session()), Some((800.0, 600.0)));
    assert_eq!(screen(&state()), None);
}

#[test]
fn a_canvas_mounted_on_top_goes_after_windows_the_compositor_left_without_a_parent() {
    let mut state = session();
    let canvas = create(&mut state).expect("a canvas");
    assert_eq!(mount_top(&mut state, canvas), None);
    let parents: Vec<(NodeId, NodeId)> = state
        .tick
        .ops()
        .iter()
        .filter_map(|op| match op {
            TxOp::Insert { node, parent, .. } => Some((*node, *parent)),
            _ => None,
        })
        .collect();
    assert_eq!(
        parents,
        [(5, 1), (6, 1), (canvas, 1)],
        "the windows first, in their order, then the canvas"
    );
    assert_eq!(mount_top(&mut state, 5), Some("not-owned"));
}
