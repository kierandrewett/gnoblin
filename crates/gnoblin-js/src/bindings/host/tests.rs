//! Tests of this module.

use gnoblin_tree_client::tree_mirror::{MirrorValue, SnapshotNode, TreeMirror, TxOp};

use super::{JsScriptValue, call, ids_of_class, lower, parents_first, prepare, raise, read, stacked_windows, write};
use crate::bindings::{HostState, JsHostValue, NodeHandles};

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
    let nodes = [
        node(1, 0, "node", Vec::new()),
        node(3, 2, "window", vec![("x".into(), MirrorValue::Double(5.0))]),
        node(2, 1, "workspace", Vec::new()),
        node(4, 1, "seat", Vec::new()),
    ];
    HostState::new(TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror"))
}

#[test]
fn ids_a_script_number_cannot_hold_cross_as_stable_handles_and_come_back() {
    let handles = NodeHandles::default();
    let native = 0x8000_0000_0000_0004_u64;
    let other = 0x8000_0000_0000_0005_u64;
    let a = handles.to_script(native);
    assert!(a < 9_007_199_254_740_992.0, "a handle is an exact script number");
    assert_eq!(
        handles.to_script(native),
        a,
        "the same node is the same number every time"
    );
    assert_ne!(handles.to_script(other), a);
    assert_eq!(handles.resolve(a), Some(native));
    assert_eq!(handles.label(a).as_deref(), Some("9223372036854775812"));
    assert_eq!(handles.to_script(7), 7.0, "a small id is its own number");
    assert_eq!(handles.resolve(7.0), Some(7));
    assert_eq!(
        handles.resolve(a + 1000.0),
        None,
        "a handle never given out names nothing"
    );
    for bad in [-1.0, 1.5, f64::NAN, f64::INFINITY, 1e300] {
        assert_eq!(handles.resolve(bad), None, "{bad}");
    }
}

#[test]
fn parents_come_before_children_whatever_the_snapshot_order() {
    assert_eq!(parents_first(&state().mirror), [1, 2, 3, 4]);
}

#[test]
fn a_node_with_no_parent_is_listed_after_the_roots_tree() {
    // The compositor makes a window unparented; the shell parents it later.
    let nodes = [
        node(1, 0, "node", Vec::new()),
        node(2, 1, "seat", Vec::new()),
        node(9, 0, "window", Vec::new()),
        node(5, 0, "window", Vec::new()),
    ];
    let mirror = TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror");
    assert_eq!(parents_first(&mirror), [1, 2, 5, 9]);
}

#[test]
fn an_absent_property_reads_as_its_default_and_a_seat_exposes_none() {
    let state = state();
    assert_eq!(read(&state, 3, "x", false), Some(JsHostValue::Number(5.0)));
    assert_eq!(read(&state, 3, "title", false), Some(JsHostValue::Text(String::new())));
    assert_eq!(read(&state, 3, "focused", true), Some(JsHostValue::Flag(false)));
    assert_eq!(read(&state, 4, "x", false), None);
    assert_eq!(read(&state, 99, "x", false), None);
}

#[test]
fn a_write_is_read_back_unpublished_and_the_published_value_stays() {
    let mut state = state();
    assert_eq!(write(&mut state, 3, "x", &JsScriptValue::Number(10.0), None), None);
    assert_eq!(read(&state, 3, "x", false), Some(JsHostValue::Number(10.0)));
    assert_eq!(read(&state, 3, "x", true), Some(JsHostValue::Number(5.0)));
    assert_eq!(state.tick.ops().len(), 1);
}

#[test]
fn a_write_that_is_not_allowed_is_refused_with_a_code_and_queues_nothing() {
    let mut state = state();
    assert_eq!(
        write(&mut state, 3, "x", &JsScriptValue::Number(f64::NAN), None),
        Some("invalid-value")
    );
    assert_eq!(
        write(&mut state, 3, "x", &JsScriptValue::Text("1".into()), None),
        Some("invalid-value")
    );
    assert_eq!(
        write(&mut state, 3, "title", &JsScriptValue::Text("t".into()), None),
        Some("read-only")
    );
    assert_eq!(
        write(&mut state, 3, "nope", &JsScriptValue::Number(1.0), None),
        Some("unknown-property")
    );
    assert_eq!(
        write(&mut state, 99, "x", &JsScriptValue::Number(1.0), None),
        Some("unknown-node")
    );
    assert!(state.tick.ops().is_empty());
}

/// A shell with one XR hand, as the program makes it, next to the window of `state`.
fn hand_state() -> HostState {
    let nodes = [
        node(1, 0, "node", Vec::new()),
        node(
            6,
            0,
            "controller",
            vec![("hand".into(), MirrorValue::Text("right".into()))],
        ),
    ];
    HostState::new(TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror"))
}

fn names(names: &[&str]) -> JsScriptValue {
    JsScriptValue::Texts(names.iter().map(|name| (*name).to_owned()).collect())
}

#[test]
fn a_list_of_names_is_written_to_a_controllers_claims_and_read_back_in_order() {
    let mut state = hand_state();
    assert_eq!(
        read(&state, 6, "claimed", false),
        Some(JsHostValue::Texts(Vec::new())),
        "nothing is claimed by default"
    );
    assert_eq!(
        write(&mut state, 6, "claimed", &names(&["trigger", "grip"]), None),
        None
    );
    assert_eq!(
        read(&state, 6, "claimed", false),
        Some(JsHostValue::Texts(vec!["trigger".into(), "grip".into()]))
    );
    assert!(
        matches!(
            state.tick.ops(),
            [TxOp::Set { node: 6, key, value: MirrorValue::Texts(list), .. }]
                if key == "claimed" && list == &["trigger", "grip"]
        ),
        "{:?}",
        state.tick.ops()
    );
    // The empty array reaches the host as an empty `Numbers` and still releases every claim.
    assert_eq!(
        write(&mut state, 6, "claimed", &JsScriptValue::Numbers(Vec::new()), None),
        None
    );
    assert_eq!(read(&state, 6, "claimed", false), Some(JsHostValue::Texts(Vec::new())));
}

#[test]
fn a_claim_that_is_not_a_list_of_the_five_lowercase_names_is_refused_and_queues_nothing() {
    let mut state = hand_state();
    for bad in [
        JsScriptValue::Text("grip".into()),
        JsScriptValue::Number(1.0),
        JsScriptValue::Flag(true),
        JsScriptValue::Numbers(vec![1.0]),
        names(&["Grip"]),
        names(&["GRIP"]),
        names(&["grip", "bumper"]),
        names(&[""]),
        JsScriptValue::Other,
    ] {
        assert_eq!(
            write(&mut state, 6, "claimed", &bad, None),
            Some("invalid-value"),
            "{bad:?}"
        );
    }
    assert!(state.tick.ops().is_empty());
    // A controller's other values are the program's, whatever a script writes.
    assert_eq!(
        write(&mut state, 6, "grip", &JsScriptValue::Number(1.0), None),
        Some("read-only")
    );
}

#[test]
fn a_screen_exposes_its_size_and_primary_flag_read_only() {
    let nodes = [
        node(1, 0, "node", Vec::new()),
        node(
            5,
            1,
            "screen",
            vec![
                ("width".into(), MirrorValue::Double(1920.0)),
                ("primary".into(), MirrorValue::Bool(true)),
            ],
        ),
    ];
    let mut state = HostState::new(TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror"));
    assert_eq!(ids_of_class(&state, "Screen"), [5.0]);
    assert_eq!(read(&state, 5, "width", false), Some(JsHostValue::Number(1920.0)));
    assert_eq!(read(&state, 5, "height", false), Some(JsHostValue::Number(0.0)));
    assert_eq!(read(&state, 5, "primary", false), Some(JsHostValue::Flag(true)));
    assert_eq!(
        write(&mut state, 5, "width", &JsScriptValue::Number(1.0), None),
        Some("read-only")
    );
}

#[test]
fn a_screen_reads_its_identity_and_a_window_its_dialog_relation_with_schema_defaults() {
    let nodes = [
        node(1, 0, "node", Vec::new()),
        node(
            5,
            1,
            "screen",
            vec![
                ("make".into(), MirrorValue::Text("Gnoblin".into())),
                ("mode".into(), MirrorValue::Text("800x600@60".into())),
            ],
        ),
        node(6, 1, "window", vec![("parent_window".into(), MirrorValue::Node(7))]),
        node(7, 1, "window", Vec::new()),
    ];
    let state = HostState::new(TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror"));
    assert_eq!(
        read(&state, 5, "make", false),
        Some(JsHostValue::Text("Gnoblin".into()))
    );
    assert_eq!(
        read(&state, 5, "mode", false),
        Some(JsHostValue::Text("800x600@60".into()))
    );
    assert_eq!(read(&state, 5, "model", false), Some(JsHostValue::Text(String::new())));
    assert_eq!(
        read(&state, 5, "transform", false),
        Some(JsHostValue::Text("normal".into()))
    );
    assert_eq!(read(&state, 5, "output_scale", false), Some(JsHostValue::Number(1.0)));
    assert_eq!(read(&state, 5, "enabled", false), Some(JsHostValue::Flag(true)));
    assert_eq!(read(&state, 5, "primary", false), Some(JsHostValue::Flag(false)));
    assert_eq!(read(&state, 6, "parent_window", false), Some(JsHostValue::Number(7.0)));
    assert_eq!(read(&state, 7, "parent_window", false), Some(JsHostValue::Number(0.0)));
    assert_eq!(read(&state, 7, "from_launcher", false), Some(JsHostValue::Flag(false)));
}

#[test]
fn visible_opacity_and_the_transform_read_as_their_schema_defaults_when_never_written() {
    let state = state();
    assert_eq!(read(&state, 3, "visible", false), Some(JsHostValue::Flag(true)));
    assert_eq!(read(&state, 3, "tiled", false), Some(JsHostValue::Flag(false)));
    assert_eq!(read(&state, 3, "opacity", false), Some(JsHostValue::Number(1.0)));
    assert_eq!(read(&state, 3, "z", false), Some(JsHostValue::Number(0.0)));
    assert_eq!(
        read(&state, 3, "rotation", false),
        Some(JsHostValue::Numbers(vec![0.0, 0.0, 0.0]))
    );
    assert_eq!(
        read(&state, 3, "scale", false),
        Some(JsHostValue::Numbers(vec![1.0, 1.0, 1.0]))
    );
}

#[test]
fn a_list_property_takes_exactly_three_finite_numbers() {
    let mut state = state();
    let list = |numbers: &[f64]| JsScriptValue::Numbers(numbers.to_vec());
    assert_eq!(write(&mut state, 3, "rotation", &list(&[0.0, 1.5, 0.0]), None), None);
    assert_eq!(
        read(&state, 3, "rotation", false),
        Some(JsHostValue::Numbers(vec![0.0, 1.5, 0.0]))
    );
    assert_eq!(
        write(&mut state, 3, "rotation", &list(&[0.0, 1.5]), None),
        Some("invalid-value")
    );
    assert_eq!(
        write(&mut state, 3, "scale", &list(&[1.0, f64::INFINITY, 1.0]), None),
        Some("invalid-value")
    );
    assert_eq!(
        write(&mut state, 3, "scale", &JsScriptValue::Number(2.0), None),
        Some("invalid-value")
    );
    assert_eq!(state.tick.ops().len(), 1);
}

#[test]
fn an_animated_write_queues_its_curve_and_a_bad_curve_queues_nothing() {
    let mut state = state();
    let curve = (0.25, &[0.0, 0.5, 1.0][..]);
    assert_eq!(
        write(&mut state, 3, "x", &JsScriptValue::Number(7.0), Some(curve)),
        None
    );
    assert_eq!(
        state.tick.ops().last(),
        Some(&TxOp::Set {
            node: 3,
            key: "x".to_owned(),
            value: MirrorValue::Double(7.0),
            seconds: Some(0.25),
            delay: None,
            tween: None,
            samples: Some(vec![0.0, 0.5, 1.0]),
        })
    );
    let short = (0.25, &[1.0][..]);
    let outside = (0.25, &[0.0, 2.0][..]);
    let negative = (-1.0, &[0.0, 1.0][..]);
    for bad in [short, outside, negative] {
        assert_eq!(
            write(&mut state, 3, "x", &JsScriptValue::Number(8.0), Some(bad)),
            Some("invalid-value")
        );
    }
    assert_eq!(
        write(&mut state, 3, "visible", &JsScriptValue::Flag(false), Some(curve)),
        Some("not-animated")
    );
    assert_eq!(state.tick.ops().len(), 1);
}

#[test]
fn prepare_answers_what_a_write_would_and_queues_nothing() {
    let state = state();
    let curve = (0.25, &[0.0, 1.0][..]);
    assert_eq!(
        prepare(&state, 3, "x", &JsScriptValue::Number(7.0), None),
        Ok(MirrorValue::Double(7.0))
    );
    assert_eq!(
        prepare(&state, 3, "x", &JsScriptValue::Number(7.0), Some(curve)),
        Ok(MirrorValue::Double(7.0))
    );
    assert_eq!(
        prepare(&state, 3, "visible", &JsScriptValue::Flag(false), Some(curve)),
        Err("not-animated")
    );
    assert_eq!(
        prepare(&state, 3, "x", &JsScriptValue::Text("far".to_owned()), None),
        Err("invalid-value")
    );
    assert_eq!(
        prepare(&state, 3, "nothing", &JsScriptValue::Number(1.0), None),
        Err("unknown-property")
    );
    assert_eq!(
        prepare(&state, 99, "x", &JsScriptValue::Number(1.0), None),
        Err("unknown-node")
    );
    assert!(state.tick.ops().is_empty(), "checking queues no write");
}

#[test]
fn a_focus_call_is_a_write_of_the_seats_focus_and_needs_a_seat() {
    use gnoblin_tree_client::tree_mirror::TxOp;

    let mut state = state();
    assert_eq!(call(&mut state, 3, "focus"), None);
    assert!(
        matches!(
            state.tick.ops(),
            [TxOp::Set { node: 4, key, value: MirrorValue::Node(3), .. }] if key == "focus"
        ),
        "{:?}",
        state.tick.ops()
    );
    assert_eq!(state.tokens, 0, "a write is not a call");
    let nodes = [node(1, 0, "node", Vec::new()), node(3, 1, "window", Vec::new())];
    let mut bare = HostState::new(TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror"));
    assert_eq!(call(&mut bare, 3, "focus"), Some("no-seat"));
}

#[test]
fn a_raised_window_goes_last_after_the_unparented_windows_are_given_the_root() {
    let nodes = [
        node(1, 0, "node", Vec::new()),
        node(3, 0, "window", Vec::new()),
        node(4, 0, "window", Vec::new()),
        node(5, 0, "window", Vec::new()),
    ];
    let mut state = HostState::new(TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror"));
    assert_eq!(
        stacked_windows(&state),
        [3, 4, 5],
        "unparented windows keep their id order"
    );
    assert_eq!(raise(&mut state, 5), None);
    assert!(state.tick.ops().is_empty(), "the top window queues nothing");
    assert_eq!(raise(&mut state, 3), None);
    assert_eq!(stacked_windows(&state), [4, 5, 3]);
    assert_eq!(raise(&mut state, 4), None);
    assert_eq!(
        stacked_windows(&state),
        [5, 3, 4],
        "a second raise in the same frame sees the first"
    );
    assert_eq!(raise(&mut state, 99), Some("unknown-node"));
    assert_eq!(raise(&mut state, 1), Some("unknown-method"));
}

#[test]
fn a_lowered_window_goes_first_after_the_unparented_windows_are_given_the_root() {
    let nodes = [
        node(1, 0, "node", Vec::new()),
        node(3, 0, "window", Vec::new()),
        node(4, 0, "window", Vec::new()),
        node(5, 0, "window", Vec::new()),
    ];
    let mut state = HostState::new(TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror"));
    assert_eq!(lower(&mut state, 3), None);
    assert!(state.tick.ops().is_empty(), "the bottom window queues nothing");
    assert_eq!(lower(&mut state, 5), None);
    assert_eq!(stacked_windows(&state), [5, 3, 4]);
    assert_eq!(lower(&mut state, 4), None);
    assert_eq!(
        stacked_windows(&state),
        [4, 5, 3],
        "a second move in the same frame sees the first"
    );
    assert_eq!(lower(&mut state, 99), Some("unknown-node"));
    assert_eq!(lower(&mut state, 1), Some("unknown-method"));
}

#[test]
fn only_close_focus_and_kill_can_be_called_on_a_window() {
    let mut state = state();
    assert_eq!(call(&mut state, 3, "close"), None);
    assert_eq!(call(&mut state, 3, "kill"), None);
    assert_eq!(call(&mut state, 3, "destroy"), Some("unknown-method"));
    assert_eq!(call(&mut state, 99, "close"), Some("unknown-node"));
    assert_eq!(state.tick.ops().len(), 2);
    assert_eq!(state.tokens, 2);
}
