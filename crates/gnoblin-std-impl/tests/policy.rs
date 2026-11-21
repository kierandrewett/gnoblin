#![allow(unsafe_code)]
//! The window policy of the native std (`policy.toml`), held to the cases of the TypeScript library's tests (`stock_focus`, `window_policy`,
//! `client_requests`, `reposition` and `input` of `crates/gnoblin-js/tests`) that this module answers.
//!
//! Each test makes the policy's services through the object protocol over the real core, the real `libgnoblin-std` and the fake compositor
//! (`ui/rig.rs`), plays the same compositor events the TypeScript tests played (a pointer edge on a window, a request, a key, a gesture) and
//! reads what the shell would have read: the seat's focus, the windows' rectangles and flags, the order of the windows. The focus timers run on
//! a clock the test moves with the shell's own (`focus_policy.now`), as the TypeScript tests moved theirs.
//!
//! **Allowances** (each is where it is made): the pure `focus_decision` cases are the TypeScript ones; `edgeTilingEnabled` and the thrown
//! `TypeError` of an unknown event have no native counterpart (an enum cannot hold an unknown word).

#[path = "ui/rig.rs"]
mod rig;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gnoblin_core_abi as abi;
use gnoblin_object::Val;
use gnoblin_testkit::compositor::ROOT;
use gnoblin_tree_client::tree_mirror::MirrorValue;
use rig::{LEFT, Rig, W, b, callback, d, en, t};

const SEAT: u64 = 21;

macro_rules! veq {
    ($a:expr, $b:expr $(, $($message:tt)+)?) => {
        assert_eq!(format!("{:?}", $a), format!("{:?}", $b) $(, $($message)+)?)
    };
}

fn text(value: &str) -> MirrorValue {
    MirrorValue::Text(value.to_owned())
}

fn record(fields: Vec<Val>) -> Val {
    Val::Record { class: 0, fields }
}

fn none() -> Val {
    Val::None
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> Val {
    record(vec![d(x), d(y), d(w), d(h)])
}

/// What the seat's keyboard is on: a node, or 0.
fn focus_of(rig: &Rig) -> u64 {
    match rig.comp.value(SEAT, "focus") {
        Some(MirrorValue::Node(node)) => node,
        _ => 0,
    }
}

fn rect_of(rig: &Rig, id: u64) -> (f64, f64, f64, f64) {
    let n = |key: &str| rig.comp.number(id, key);
    (n("x"), n("y"), n("width"), n("height"))
}

/// The windows back to front, as the root holds them.
fn order(rig: &Rig, windows: &[u64]) -> Vec<u64> {
    rig.comp
        .children(ROOT)
        .into_iter()
        .filter(|id| windows.contains(id))
        .collect()
}

fn set(rig: &mut Rig, id: u64, key: &str, value: MirrorValue) {
    rig.external(id, key, value);
    rig.settle();
}

fn seat(rig: &mut Rig, key: &str, value: MirrorValue) {
    set(rig, SEAT, key, value);
}

fn edge(rig: &mut Rig, window: u64, kind: &str) {
    rig.comp.pointer(
        window,
        kind,
        5.0,
        5.0,
        if kind == "press" || kind == "release" { LEFT } else { 0 },
    );
    rig.settle();
}

/// The shell's clock the policy's timers run by, with the compositor's moved together.
struct Clock {
    now: Rc<Cell<f64>>,
}

impl Clock {
    fn give(rig: &Rig, service: &W) -> Clock {
        let now = Rc::new(Cell::new(1000.0));
        let read = Rc::clone(&now);
        service.set("now", callback(move |_| Ok(Val::Double(read.get()))));
        let _ = rig;
        Clock { now }
    }

    fn advance(&self, rig: &mut Rig, ms: u64) {
        self.now.set(self.now.get() + ms as f64);
        rig.advance(ms);
    }
}

fn focus_policy(rig: &Rig) -> W {
    rig.make("focus_policy", &[])
}

fn started_focus(rig: &Rig) -> (W, Clock) {
    let focus = focus_policy(rig);
    let clock = Clock::give(rig, &focus);
    focus.call("start", &[]);
    (focus, clock)
}

fn settings(mode: u32, new_windows: u32, raise: bool, auto: bool, delay: f64, rest: bool) -> Val {
    record(vec![en(mode), en(new_windows), b(raise), b(auto), d(delay), b(rest)])
}

fn event(kind: u32, fields: &[(usize, Val)]) -> Val {
    let mut all = vec![en(kind), Val::None, Val::None, Val::None, Val::None];
    for (at, value) in fields {
        all[*at] = value.clone();
    }
    record(all)
}

fn action(value: &Val) -> String {
    let Val::Record { fields, .. } = value else {
        panic!("a record, got {value:?}");
    };
    let show = |value: &Val| match value {
        Val::Bool(flag) => flag.to_string(),
        Val::Double(number) => format!("{number}"),
        Val::None => "null".to_owned(),
        other => panic!("{other:?}"),
    };
    fields.iter().map(show).collect::<Vec<_>>().join("/")
}

// ---- the focus decisions, pure ------------------------------------------------------------------------------------------------------

#[test]
fn the_focus_decision_is_a_pure_function_of_the_settings_and_one_event() {
    let rig = Rig::scratch_policy(0);
    let decide =
        |settings: Val, event: Val| action(&rig.function("focus_policy", "focus_decision", &[settings, event]));
    let (click, sloppy, follow) = (1, 2, 3);
    let (allow, prevent) = (1, 2);
    let base = |mode| settings(mode, prevent, true, false, 500.0, false);
    let (added, enter, motion, leave, press) = (1, 2, 3, 4, 5);
    let from = |launcher: bool| event(added, &[(1, b(launcher))]);
    let held = |kind, held: bool| event(kind, &[(2, b(held))]);
    let leaving = |focused: bool, here: bool| event(leave, &[(3, b(here)), (4, b(focused))]);
    veq!(decide(base(click), from(false)), "true/false/false/null/null");
    veq!(decide(base(click), from(true)), "false/false/false/null/null");
    veq!(
        decide(settings(click, allow, true, false, 500.0, false), from(true)),
        "true/false/false/null/null"
    );
    veq!(decide(base(click), event(press, &[])), "true/false/true/null/null");
    veq!(
        decide(settings(click, prevent, false, false, 500.0, false), event(press, &[])),
        "true/false/false/null/null"
    );
    veq!(decide(base(click), held(enter, false)), "false/false/false/null/null");
    veq!(decide(base(sloppy), held(enter, false)), "true/false/false/null/null");
    veq!(decide(base(sloppy), held(motion, false)), "true/false/false/null/null");
    veq!(decide(base(follow), held(motion, false)), "false/false/false/null/null");
    veq!(decide(base(sloppy), held(enter, true)), "false/false/false/null/null");
    let auto = settings(sloppy, prevent, true, true, 500.0, false);
    veq!(decide(auto.clone(), held(enter, false)), "true/false/false/null/500");
    veq!(decide(auto, held(motion, false)), "true/false/false/null/null");
    let rests = settings(sloppy, prevent, true, false, 500.0, true);
    veq!(decide(rests, held(enter, false)), "false/false/false/200/null");
    veq!(decide(base(follow), leaving(true, false)), "false/true/false/null/null");
    veq!(
        decide(base(follow), leaving(false, false)),
        "false/false/false/null/null"
    );
    veq!(decide(base(follow), leaving(true, true)), "false/false/false/null/null");
    veq!(
        decide(base(sloppy), leaving(true, false)),
        "false/false/false/null/null"
    );
}

#[test]
fn the_focus_predicates_answer_for_the_settings() {
    let rig = Rig::scratch_policy(0);
    let call = |name: &str, args: &[Val]| rig.function("focus_policy", name, args);
    let hover = settings(2, 2, true, false, 500.0, false);
    veq!(call("opens_focused", &[hover.clone(), b(true)]), b(false));
    veq!(call("hover_focus", &[hover.clone(), b(true), b(false)]), b(true));
    veq!(call("hover_focus", &[hover.clone(), b(false), b(true)]), b(false));
    veq!(call("rest_focus", std::slice::from_ref(&hover)), b(false));
    veq!(call("leave_clears", &[hover]), b(false));
    let strict = settings(3, 2, true, false, 500.0, true);
    veq!(call("hover_focus", &[strict.clone(), b(true), b(false)]), b(false));
    veq!(call("rest_focus", std::slice::from_ref(&strict)), b(true));
    veq!(call("leave_clears", &[strict]), b(true));
}

#[test]
fn the_settings_are_the_seats_with_its_words_translated() {
    let mut rig = Rig::scratch_policy(0);
    let focus = focus_policy(&rig);
    let read = |focus: &W| match focus.get("settings") {
        Val::Record { fields, .. } => fields,
        other => panic!("{other:?}"),
    };
    veq!(read(&focus)[0], en(1), "a seat that says nothing is click focus");
    seat(&mut rig, "focus_mode", text("hover"));
    veq!(read(&focus)[0], en(2));
    seat(&mut rig, "focus_mode", text("hover-strict"));
    veq!(read(&focus)[0], en(3));
    seat(&mut rig, "focus_new_windows", text("allow"));
    seat(&mut rig, "auto_raise_delay", MirrorValue::Double(40.0));
    let fields = read(&focus);
    veq!((fields[1].clone(), fields[4].clone()), (en(1), d(40.0)));
    seat(&mut rig, "focus_mode", text("sticky"));
    let member = focus
        .api
        .member(focus.class, abi::MEMBER_PROPERTY, "settings")
        .expect("settings");
    assert!(
        focus.api.get(focus.obj, member).is_err(),
        "an unknown seat mode is refused"
    );
}

// ---- the focus policy, through the shell ----------------------------------------------------------------------------------------------

#[test]
fn a_new_window_takes_focus_and_a_window_from_before_the_policy_does_not() {
    let mut rig = Rig::scratch_policy(2);
    let (_focus, _clock) = started_focus(&rig);
    rig.settle();
    veq!(focus_of(&rig), 0, "the windows already there are left alone");
    rig.comp.create_window(8, "kitty", "shell");
    rig.settle();
    veq!(focus_of(&rig), 8);
}

#[test]
fn nothing_answers_before_start() {
    let mut rig = Rig::scratch_policy(2);
    let _focus = focus_policy(&rig);
    rig.comp.create_window(8, "kitty", "shell");
    rig.settle();
    edge(&mut rig, 3, "press");
    veq!(focus_of(&rig), 0, "the policy has not started");
}

#[test]
fn a_click_focuses_the_window_and_raises_it_when_raise_on_click_is_on() {
    let mut rig = Rig::scratch_policy(2);
    let (_focus, _clock) = started_focus(&rig);
    edge(&mut rig, 4, "press");
    edge(&mut rig, 4, "release");
    veq!(focus_of(&rig), 4);
    veq!(order(&rig, &[3, 4]), [3, 4], "the topmost window needs no raise");
    edge(&mut rig, 3, "press");
    edge(&mut rig, 3, "release");
    veq!(focus_of(&rig), 3);
    veq!(order(&rig, &[3, 4]), [4, 3], "window 3 is raised");
}

#[test]
fn a_click_only_focuses_when_the_seat_says_raise_on_click_is_off() {
    let mut rig = Rig::scratch_policy(2);
    seat(&mut rig, "raise_on_click", MirrorValue::Bool(false));
    let (_focus, _clock) = started_focus(&rig);
    edge(&mut rig, 3, "press");
    veq!(focus_of(&rig), 3);
    veq!(order(&rig, &[3, 4]), [3, 4]);
}

#[test]
fn click_mode_ignores_the_pointer_over_a_window() {
    let mut rig = Rig::scratch_policy(2);
    let (_focus, _clock) = started_focus(&rig);
    edge(&mut rig, 4, "enter");
    edge(&mut rig, 4, "motion");
    veq!(focus_of(&rig), 0);
}

#[test]
fn sloppy_focus_follows_the_pointer_on_entry_and_keeps_focus_on_leave() {
    let mut rig = Rig::scratch_policy(2);
    seat(&mut rig, "focus_mode", text("hover"));
    let (_focus, _clock) = started_focus(&rig);
    edge(&mut rig, 4, "enter");
    veq!(focus_of(&rig), 4);
    edge(&mut rig, 3, "leave");
    veq!(focus_of(&rig), 4, "sloppy focus keeps the focus where it was");
    edge(&mut rig, 4, "leave");
    veq!(focus_of(&rig), 4);
}

#[test]
fn follow_mouse_focuses_on_entry_only_and_clears_focus_when_the_pointer_leaves_the_focused_window() {
    let mut rig = Rig::scratch_policy(2);
    seat(&mut rig, "focus_mode", text("hover-strict"));
    let (_focus, _clock) = started_focus(&rig);
    edge(&mut rig, 4, "enter");
    edge(&mut rig, 4, "motion");
    veq!(focus_of(&rig), 4);
    edge(&mut rig, 3, "leave");
    veq!(focus_of(&rig), 4, "window 3 is not the focused one");
    edge(&mut rig, 4, "leave");
    veq!(focus_of(&rig), 0, "the pointer left the focused window");
}

#[test]
fn a_window_pressed_holds_hover_focus_until_the_button_is_released() {
    let mut rig = Rig::scratch_policy(2);
    seat(&mut rig, "focus_mode", text("hover"));
    let (_focus, _clock) = started_focus(&rig);
    edge(&mut rig, 3, "press");
    veq!(focus_of(&rig), 3);
    edge(&mut rig, 4, "enter");
    veq!(focus_of(&rig), 3, "a button is held");
    edge(&mut rig, 3, "release");
    edge(&mut rig, 4, "leave");
    edge(&mut rig, 4, "enter");
    veq!(focus_of(&rig), 4);
}

#[test]
fn auto_raise_lifts_the_hovered_focused_window_after_the_delay_unless_the_pointer_left() {
    let mut rig = Rig::scratch_policy(2);
    seat(&mut rig, "focus_mode", text("hover"));
    seat(&mut rig, "auto_raise", MirrorValue::Bool(true));
    seat(&mut rig, "auto_raise_delay", MirrorValue::Double(40.0));
    let (_focus, clock) = started_focus(&rig);
    edge(&mut rig, 3, "enter");
    veq!(focus_of(&rig), 3, "entering focuses at once; the raise waits");
    veq!(order(&rig, &[3, 4]), [3, 4]);
    clock.advance(&mut rig, 80);
    veq!(
        order(&rig, &[3, 4]),
        [4, 3],
        "after the delay the focused window is raised"
    );
    edge(&mut rig, 4, "enter");
    edge(&mut rig, 3, "enter");
    edge(&mut rig, 3, "leave");
    clock.advance(&mut rig, 80);
    veq!(order(&rig, &[3, 4]), [4, 3], "a leave cancels the pending raise");
}

#[test]
fn auto_raise_is_off_by_default_and_a_window_that_lost_focus_is_not_raised() {
    let mut rig = Rig::scratch_policy(2);
    seat(&mut rig, "focus_mode", text("hover"));
    let (_focus, clock) = started_focus(&rig);
    edge(&mut rig, 3, "enter");
    clock.advance(&mut rig, 30);
    veq!(order(&rig, &[3, 4]), [3, 4], "auto_raise is false");
    seat(&mut rig, "auto_raise", MirrorValue::Bool(true));
    seat(&mut rig, "auto_raise_delay", MirrorValue::Double(40.0));
    edge(&mut rig, 4, "leave");
    edge(&mut rig, 3, "enter");
    set(&mut rig, SEAT, "focus", MirrorValue::Node(4));
    clock.advance(&mut rig, 80);
    veq!(
        order(&rig, &[3, 4]),
        [3, 4],
        "the compositor gave the focus to window 4 first"
    );
}

#[test]
fn a_pointer_rest_focuses_after_the_rest_time_and_a_leave_cancels_it() {
    let mut rig = Rig::scratch_policy(2);
    seat(&mut rig, "focus_mode", text("hover"));
    seat(&mut rig, "focus_change_on_pointer_rest", MirrorValue::Bool(true));
    let (_focus, clock) = started_focus(&rig);
    edge(&mut rig, 4, "enter");
    veq!(focus_of(&rig), 0, "a rest is not an entry");
    clock.advance(&mut rig, 260);
    veq!(focus_of(&rig), 4);
    edge(&mut rig, 3, "enter");
    edge(&mut rig, 3, "leave");
    clock.advance(&mut rig, 260);
    veq!(focus_of(&rig), 4, "the pointer left before it rested");
}

// ---- the requests of windows ----------------------------------------------------------------------------------------------------------

fn started_state(rig: &Rig) -> W {
    let state = rig.make("window_state", &[]);
    state.call("start", &[]);
    state
}

fn request(rig: &mut Rig, window: u64, name: &str) {
    rig.comp.request(window, name, 0.0, 0.0);
    rig.settle();
}

#[test]
fn a_maximise_request_fills_the_work_area_and_unmaximise_returns_the_floating_rectangle() {
    let mut rig = Rig::scratch_policy(1);
    let _state = started_state(&rig);
    let before = rect_of(&rig, 3);
    request(&mut rig, 3, "maximize");
    veq!(rig.comp.flag(3, "maximised"), Some(true));
    veq!(rect_of(&rig, 3), (0.0, 0.0, 1000.0, 800.0));
    request(&mut rig, 3, "maximize");
    veq!(
        rect_of(&rig, 3),
        (0.0, 0.0, 1000.0, 800.0),
        "a second request changes nothing"
    );
    request(&mut rig, 3, "unmaximize");
    veq!(rig.comp.flag(3, "maximised"), Some(false));
    veq!(rect_of(&rig, 3), before);
}

#[test]
fn a_fullscreen_request_covers_the_output_and_goes_on_top() {
    let mut rig = Rig::scratch_policy(2);
    let _state = started_state(&rig);
    let before = rect_of(&rig, 3);
    request(&mut rig, 3, "fullscreen");
    veq!(rig.comp.flag(3, "fullscreen"), Some(true));
    veq!(rect_of(&rig, 3), (0.0, 0.0, 1000.0, 800.0));
    veq!(order(&rig, &[3, 4]), [4, 3]);
    request(&mut rig, 3, "unfullscreen");
    veq!(rect_of(&rig, 3), before);
}

#[test]
fn a_fullscreen_window_keeps_the_top_when_another_is_raised() {
    let mut rig = Rig::scratch_policy(2);
    let _state = started_state(&rig);
    request(&mut rig, 4, "fullscreen");
    request(&mut rig, 3, "raise");
    veq!(
        order(&rig, &[3, 4]),
        [3, 4],
        "the fullscreen window is put back over it"
    );
}

#[test]
fn minimising_the_focused_window_hands_the_keyboard_to_the_topmost_sibling_that_shows() {
    let mut rig = Rig::scratch_policy(3);
    let _state = started_state(&rig);
    set(&mut rig, SEAT, "focus", MirrorValue::Node(5));
    request(&mut rig, 5, "minimize");
    veq!(rig.comp.flag(5, "minimised"), Some(true));
    veq!(focus_of(&rig), 4);
    request(&mut rig, 3, "minimize");
    veq!(focus_of(&rig), 4, "a window that did not hold the keyboard leaves it");
    request(&mut rig, 4, "minimize");
    veq!(focus_of(&rig), 0, "nobody is left to take it");
    set(&mut rig, SEAT, "focus", MirrorValue::Node(0));
    request(&mut rig, 5, "unminimize");
    veq!(focus_of(&rig), 5, "nothing held the keyboard");
}

#[test]
fn a_raise_and_a_lower_request_move_the_window_in_the_stack() {
    let mut rig = Rig::scratch_policy(3);
    let _state = started_state(&rig);
    request(&mut rig, 3, "raise");
    veq!(order(&rig, &[3, 4, 5]), [4, 5, 3]);
    request(&mut rig, 5, "lower");
    veq!(order(&rig, &[3, 4, 5]), [5, 4, 3]);
}

#[test]
fn a_close_request_asks_the_window_to_close() {
    let mut rig = Rig::scratch_policy(1);
    let _state = started_state(&rig);
    request(&mut rig, 3, "close");
    assert!(
        rig.comp
            .calls()
            .iter()
            .any(|(node, name)| *node == 3 && name == "close")
    );
}

#[test]
fn the_first_listener_that_handles_a_request_answers_it_and_the_stock_answer_does_not_run() {
    let mut rig = Rig::scratch_policy(2);
    let state = started_state(&rig);
    let heard: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let api = rig.api;
    let listener = |label: &'static str, takes: bool, heard: Rc<RefCell<Vec<String>>>| {
        rig.implement("request_listener", &["on_request"], move |_, args| {
            let Val::Object { class, obj } = &args[0] else {
                panic!("a request");
            };
            let request = W {
                api,
                class: *class,
                obj: *obj,
            };
            veq!(request.get("client"), t("3"));
            heard.borrow_mut().push(label.to_owned());
            if takes {
                request.call("handle", &[]);
                veq!(request.get("handled"), b(true));
            }
            Ok(Val::None)
        })
    };
    let first = listener("all-a", true, Rc::clone(&heard));
    let second = listener("all-b", true, Rc::clone(&heard));
    let own = listener("window-3", false, Rc::clone(&heard));
    state.call("add_listener", &[first.clone(), none()]);
    state.call("add_listener", &[second.clone(), none()]);
    state.call("add_listener", &[own, t("3")]);
    request(&mut rig, 3, "maximize");
    veq!(
        *heard.borrow(),
        ["window-3", "all-a"],
        "a window's own listeners come first, and the first that handles it answers"
    );
    veq!(
        rig.comp.flag(3, "maximised"),
        None::<bool>,
        "the stock answer did not run"
    );
    state.call("remove_listener", &[first]);
    heard.borrow_mut().clear();
    request(&mut rig, 3, "maximize");
    veq!(*heard.borrow(), ["window-3", "all-b"]);
}

#[test]
fn a_move_request_drags_the_window_with_the_pointer_until_a_button_edge() {
    let mut rig = Rig::scratch_policy(1);
    let state = started_state(&rig);
    set(&mut rig, 3, "x", MirrorValue::Double(100.0));
    set(&mut rig, 3, "y", MirrorValue::Double(100.0));
    rig.comp.set_pointer(SEAT, 150.0, 150.0);
    rig.settle();
    veq!(state.call("begin_move", &[t("3")]), b(true));
    veq!(state.get("dragging"), b(true));
    rig.comp.set_pointer(SEAT, 250.0, 200.0);
    rig.advance(20);
    veq!(rect_of(&rig, 3).0, 200.0);
    veq!(rect_of(&rig, 3).1, 150.0);
    edge(&mut rig, 3, "release");
    veq!(state.get("dragging"), b(false));
    rig.comp.set_pointer(SEAT, 400.0, 400.0);
    rig.advance(20);
    veq!(rect_of(&rig, 3).0, 200.0, "the drag has ended");
}

#[test]
fn a_move_or_resize_request_leaves_a_maximised_or_fullscreen_window_alone() {
    let mut rig = Rig::scratch_policy(1);
    let state = started_state(&rig);
    rig.comp.set_pointer(SEAT, 50.0, 50.0);
    request(&mut rig, 3, "maximize");
    veq!(state.call("begin_move", &[t("3")]), b(false));
    veq!(state.call("begin_resize", &[t("3")]), b(false));
}

#[test]
fn a_resize_request_takes_the_corner_the_pointer_is_near() {
    let mut rig = Rig::scratch_policy(1);
    let state = started_state(&rig);
    // The window is at (10, 20) and 300 by 200: its bottom right corner is (310, 220).
    rig.comp.set_pointer(SEAT, 308.0, 218.0);
    rig.settle();
    veq!(state.call("begin_resize", &[t("3")]), b(true));
    rig.comp.set_pointer(SEAT, 358.0, 238.0);
    rig.advance(20);
    let (x, y, w, h) = rect_of(&rig, 3);
    veq!((x, y, w, h), (10.0, 20.0, 350.0, 220.0));
}

#[test]
fn the_rectangles_a_window_returns_to_are_the_services_to_keep_across_a_reload() {
    let mut rig = Rig::scratch_policy(1);
    let state = started_state(&rig);
    request(&mut rig, 3, "maximize");
    let kept = state.call("kept", &[t("3")]);
    let Val::Record { fields, .. } = &kept else {
        panic!("{kept:?}");
    };
    assert_ne!(
        format!("{:?}", fields[0]),
        format!("{:?}", Val::None),
        "the floating rectangle is kept"
    );
    veq!(fields[1], Val::None);
    state.call("keep", &[t("3"), record(vec![none(), rect(1.0, 2.0, 3.0, 4.0)])]);
    let again = state.call("kept", &[t("3")]);
    let Val::Record { fields, .. } = &again else {
        panic!("{again:?}");
    };
    veq!(fields[1], rect(1.0, 2.0, 3.0, 4.0));
}

#[test]
fn an_activate_request_brings_a_minimised_window_back_focused_and_raised() {
    let mut rig = Rig::scratch_policy(2);
    let list = rig.make("workspaces", &[]);
    let main = rig.make("workspace", &[("id", t("main"))]);
    list.call("add", &[main.val()]);
    rig.settle();
    let _state = started_state(&rig);
    request(&mut rig, 3, "minimize");
    request(&mut rig, 3, "activate");
    veq!(rig.comp.flag(3, "minimised"), Some(false));
    veq!(focus_of(&rig), 3);
    veq!(order(&rig, &[3, 4]), [4, 3]);
}

// ---- the keyboard move and resize ---------------------------------------------------------------------------------------------------------

fn floating_world() -> (Rig, W) {
    let rig = Rig::scratch_policy(1);
    let list = rig.make("workspaces", &[]);
    let main = rig.make("workspace", &[("id", t("main"))]);
    list.call("add", &[main.val()]);
    let reposition = rig.make("repositioning", &[]);
    reposition.call("start", &[]);
    let mut rig = rig;
    rig.settle();
    (rig, reposition)
}

fn key(rig: &mut Rig, grab: &W, name: &str) {
    rig.comp.key(grab.node(), name, "", true);
    rig.settle();
}

#[test]
fn the_arrow_keys_move_the_window_by_a_step_and_return_keeps_the_place() {
    let (mut rig, mode) = floating_world();
    let start = rect_of(&rig, 3);
    veq!(mode.call("begin", &[t("3"), en(1)]), b(true));
    let grab = match mode.get("grab") {
        Val::Object { class, obj } => W {
            api: mode.api,
            class,
            obj,
        },
        other => panic!("{other:?}"),
    };
    veq!(mode.get("current"), t("3"));
    key(&mut rig, &grab, "Right");
    key(&mut rig, &grab, "Down");
    key(&mut rig, &grab, "Down");
    veq!(rect_of(&rig, 3), (start.0 + 16.0, start.1 + 32.0, start.2, start.3));
    key(&mut rig, &grab, "Return");
    veq!(mode.get("current"), Val::None);
    veq!(rect_of(&rig, 3), (start.0 + 16.0, start.1 + 32.0, start.2, start.3));
    veq!(focus_of(&rig), 3, "the window has the keyboard back");
}

#[test]
fn escape_puts_the_window_back() {
    let (mut rig, mode) = floating_world();
    let start = rect_of(&rig, 3);
    mode.call("begin", &[t("3"), en(1)]);
    let grab = match mode.get("grab") {
        Val::Object { class, obj } => W {
            api: mode.api,
            class,
            obj,
        },
        other => panic!("{other:?}"),
    };
    key(&mut rig, &grab, "Left");
    key(&mut rig, &grab, "Escape");
    veq!(rect_of(&rig, 3), start);
}

#[test]
fn the_arrow_keys_grow_and_shrink_a_window_in_resize_mode_down_to_the_least_size() {
    let (mut rig, mode) = floating_world();
    mode.call("begin", &[t("3"), en(2)]);
    let grab = match mode.get("grab") {
        Val::Object { class, obj } => W {
            api: mode.api,
            class,
            obj,
        },
        other => panic!("{other:?}"),
    };
    key(&mut rig, &grab, "Right");
    veq!(rect_of(&rig, 3).2, 316.0);
    for _ in 0..20 {
        key(&mut rig, &grab, "Left");
        key(&mut rig, &grab, "Up");
    }
    veq!((rect_of(&rig, 3).2, rect_of(&rig, 3).3), (120.0, 80.0));
    key(&mut rig, &grab, "space");
    veq!(mode.get("current"), Val::None);
}

#[test]
fn the_mode_refuses_a_maximised_window_and_the_closing_of_its_window_puts_it_back() {
    let (mut rig, mode) = floating_world();
    set(&mut rig, 3, "maximised", MirrorValue::Bool(true));
    veq!(
        mode.call("begin", &[t("3"), en(1)]),
        b(false),
        "a maximised window is refused"
    );
    set(&mut rig, 3, "maximised", MirrorValue::Bool(false));
    veq!(mode.call("begin", &[t("3"), en(1)]), b(true));
    rig.external_destroy(3);
    rig.settle();
    rig.advance(20);
    veq!(mode.get("current"), Val::None, "the window went");
}

#[test]
fn a_window_a_mode_unsnapped_is_snapped_again_when_the_mode_is_cancelled() {
    let (mut rig, mode) = floating_world();
    seat(&mut rig, "edge_tiling", MirrorValue::Bool(true));
    let state = match rig.function("client_state", "of", &[t("3")]) {
        Val::Object { class, obj } => W {
            api: rig.api,
            class,
            obj,
        },
        other => panic!("{other:?}"),
    };
    let floating = rect_of(&rig, 3);
    veq!(
        state.call("snap_to", &[en(1), rect(0.0, 0.0, 1000.0, 800.0), b(true)]),
        b(true)
    );
    rig.settle();
    let snapped = rect_of(&rig, 3);
    veq!(snapped.2, 500.0);
    mode.call("begin", &[t("3"), en(1)]);
    rig.settle();
    veq!(rect_of(&rig, 3), floating, "the mode unsnaps first");
    let grab = match mode.get("grab") {
        Val::Object { class, obj } => W {
            api: mode.api,
            class,
            obj,
        },
        other => panic!("{other:?}"),
    };
    key(&mut rig, &grab, "Escape");
    veq!(rect_of(&rig, 3), snapped);
    assert_ne!(
        format!("{:?}", state.get("snap")),
        format!("{:?}", Val::None),
        "the snap is back"
    );
}

// ---- snap -----------------------------------------------------------------------------------------------------------------------------------

#[test]
fn a_pointer_near_an_edge_names_the_zone_and_a_zone_names_a_rectangle() {
    let rig = Rig::scratch_policy(0);
    let area = rect(0.0, 32.0, 1920.0, 1048.0);
    let zone = |x: f64, y: f64, margin: Option<f64>| {
        let mut args = vec![d(x), d(y), area.clone()];
        if let Some(margin) = margin {
            args.push(d(margin));
        }
        match rig.function("snap", "edge_zone", &args) {
            Val::Record { fields, .. } => format!("{:?}", fields[0]),
            Val::None => "none".to_owned(),
            other => panic!("{other:?}"),
        }
    };
    let (left, right, top) = (format!("{:?}", en(1)), format!("{:?}", en(2)), format!("{:?}", en(3)));
    veq!(zone(2.0, 500.0, None), left);
    veq!(zone(1917.0, 500.0, None), right);
    veq!(zone(900.0, 33.0, None), top);
    veq!(zone(960.0, 500.0, None), "none");
    veq!(zone(1.0, 33.0, None), left, "left wins over top at a corner");
    veq!(zone(1919.0, 33.0, None), right);
    veq!(zone(6.0, 500.0, None), "none");
    veq!(zone(6.0, 500.0, Some(6.0)), left);
    let tile = |zone: u32, area: Val| match rig.function("snap", "tile_rect", &[en(zone), area]) {
        Val::Record { fields, .. } => fields
            .iter()
            .map(|field| match field {
                Val::Double(number) => format!("{number}"),
                other => panic!("{other:?}"),
            })
            .collect::<Vec<_>>()
            .join(","),
        other => panic!("{other:?}"),
    };
    veq!(tile(1, area.clone()), "0,32,960,1048");
    veq!(tile(2, area.clone()), "960,32,960,1048");
    veq!(tile(3, area), "0,32,1920,1048");
    veq!(tile(2, rect(0.0, 0.0, 101.0, 50.0)), "50,0,51,50");
}

// ---- gestures -------------------------------------------------------------------------------------------------------------------------------

#[test]
fn a_swipe_has_a_way_by_its_longer_axis_once_it_has_gone_the_threshold() {
    let rig = Rig::scratch_policy(0);
    let way = |dx: f64, dy: f64, threshold: Option<f64>| {
        let mut args = vec![d(dx), d(dy)];
        if let Some(threshold) = threshold {
            args.push(d(threshold));
        }
        match rig.function("gestures", "direction", &args) {
            Val::Record { fields, .. } => match fields[0] {
                Val::Enum { value, .. } => value,
                ref other => panic!("{other:?}"),
            },
            Val::None => 0,
            other => panic!("{other:?}"),
        }
    };
    veq!(way(-60.0, 10.0, None), 1, "left");
    veq!(way(60.0, 10.0, None), 2, "right");
    veq!(way(5.0, -60.0, None), 3, "up");
    veq!(way(5.0, 60.0, None), 4, "down");
    veq!(way(40.0, 10.0, None), 0, "less than the threshold");
    veq!(way(40.0, 10.0, Some(30.0)), 2);
    veq!(way(50.0, -50.0, None), 2, "a tie goes to the horizontal axis");
}

#[test]
fn an_unknown_action_name_is_refused() {
    let rig = Rig::scratch_policy(0);
    let gestures = rig.make("gestures", &[]);
    assert!(gestures.try_call("run", &[t("window.fly")]).is_err());
    let swipe = rig.make("swipe_action_binding", &[]);
    assert!(swipe.try_call("set_action", &[en(1), t("window.fly")]).is_err());
    assert!(swipe.try_call("set_action", &[en(1), t("workspace.next")]).is_ok());
}

/// A swipe of `fingers` over `steps` (the touchpad units the centre of the fingers has gone at each update), then its end.
fn swipe(rig: &mut Rig, claim: i64, fingers: i64, steps: &[(f64, f64)], end: &str) {
    rig.comp.gesture(claim, "swipe", fingers, "begin", (0.0, 0.0), 1.0, 0.0);
    rig.settle();
    for (dx, dy) in steps {
        rig.comp
            .gesture(claim, "swipe", fingers, "update", (*dx, *dy), 1.0, 0.0);
        rig.settle();
    }
    let at = steps.last().copied().unwrap_or((0.0, 0.0));
    rig.comp.gesture(claim, "swipe", fingers, end, at, 1.0, 0.0);
    rig.settle();
}

fn two_workspaces(rig: &Rig) -> (W, W, W) {
    let list = rig.make("workspaces", &[]);
    let first = rig.make("workspace", &[("id", t("one"))]);
    let second = rig.make("workspace", &[("id", t("two"))]);
    list.call("add", &[first.val()]);
    list.call("add", &[second.val()]);
    (list, first, second)
}

#[test]
fn a_swipe_action_runs_its_function_when_the_swipe_ends_the_way_it_names() {
    let mut rig = Rig::scratch_policy(1);
    let swipe_binding = rig.make("swipe_action_binding", &[("trigger", t("Swipe4"))]);
    let heard: Rc<RefCell<Vec<(f64, f64)>>> = Rc::new(RefCell::new(Vec::new()));
    let note = Rc::clone(&heard);
    swipe_binding.call(
        "set_function",
        &[
            en(1),
            callback(move |args| {
                let Val::Record { fields, .. } = &args[0] else {
                    panic!("an event");
                };
                let (Val::Double(dx), Val::Double(dy)) = (&fields[2], &fields[3]) else {
                    panic!("numbers");
                };
                note.borrow_mut().push((*dx, *dy));
                Ok(Val::None)
            }),
        ],
    );
    swipe_binding.call("install", &[]);
    veq!(swipe_binding.get("installed"), b(true));
    rig.settle();
    let claim = rig.comp.gesture_claim("swipe", 4);
    swipe(&mut rig, claim, 4, &[(-30.0, 2.0), (-80.0, 3.0)], "end");
    veq!(
        *heard.borrow(),
        [(-80.0, 3.0)],
        "the function ran once, at the end, for left"
    );
    swipe(&mut rig, claim, 4, &[(-80.0, 3.0)], "cancel");
    veq!(heard.borrow().len(), 1, "a cancelled swipe runs nothing");
    swipe(&mut rig, claim, 4, &[(10.0, 0.0)], "end");
    veq!(heard.borrow().len(), 1, "a short swipe runs nothing");
    swipe_binding.call("uninstall", &[]);
    veq!(swipe_binding.get("installed"), b(false));
}

#[test]
fn a_swipe_action_named_workspace_next_shows_the_next_workspace() {
    let mut rig = Rig::scratch_policy(0);
    let (_list, first, second) = two_workspaces(&rig);
    rig.settle();
    veq!(first.get("active"), b(true));
    let binding = rig.make("swipe_action_binding", &[("trigger", t("Swipe4"))]);
    binding.call("set_action", &[en(1), t("workspace.next")]);
    binding.call("install", &[]);
    rig.settle();
    let claim = rig.comp.gesture_claim("swipe", 4);
    swipe(&mut rig, claim, 4, &[(-100.0, 0.0)], "end");
    veq!(second.get("active"), b(true));
}

#[test]
fn the_workspace_slide_follows_the_fingers_and_a_long_swipe_shows_the_next_workspace() {
    let mut rig = Rig::scratch_policy(0);
    let (_list, first, second) = two_workspaces(&rig);
    rig.comp.create_window(6, "a", "app");
    rig.settle();
    let slide = rig.make("workspace_slide_binding", &[]);
    slide.call("install", &[]);
    rig.settle();
    let claim = rig.comp.gesture_claim("swipe", 3);
    veq!(first.get("active"), b(true));
    // Fingers moving left bring the next workspace in from the right: half a workspace of travel.
    rig.comp.gesture(claim, "swipe", 3, "begin", (0.0, 0.0), 1.0, 0.0);
    rig.settle();
    rig.comp.gesture(claim, "swipe", 3, "update", (-200.0, 4.0), 1.0, 0.0);
    rig.settle();
    rig.comp.gesture(claim, "swipe", 3, "end", (-200.0, 4.0), 1.0, 0.0);
    rig.settle();
    rig.advance(300);
    veq!(
        second.get("active"),
        b(true),
        "past a quarter of the travel finishes the slide"
    );
}

#[test]
fn a_short_slide_settles_back_and_leaves_the_workspace_shown() {
    let mut rig = Rig::scratch_policy(1);
    let (_list, first, second) = two_workspaces(&rig);
    rig.settle();
    let at = rect_of(&rig, 3).0;
    let slide = rig.make("workspace_slide_binding", &[]);
    slide.call("install", &[]);
    rig.settle();
    let claim = rig.comp.gesture_claim("swipe", 3);
    swipe(&mut rig, claim, 3, &[(-40.0, 0.0)], "end");
    rig.advance(400);
    veq!(first.get("active"), b(true), "a tenth of the travel is not enough");
    veq!(second.get("active"), b(false));
    veq!(rect_of(&rig, 3).0, at, "the window rests where it was");
}

#[test]
fn a_vertical_swipe_is_not_a_slide() {
    let mut rig = Rig::scratch_policy(0);
    let (_list, first, _second) = two_workspaces(&rig);
    rig.settle();
    let slide = rig.make("workspace_slide_binding", &[]);
    slide.call("install", &[]);
    rig.settle();
    let claim = rig.comp.gesture_claim("swipe", 3);
    swipe(&mut rig, claim, 3, &[(-10.0, 300.0)], "end");
    rig.advance(300);
    veq!(first.get("active"), b(true));
}

#[test]
fn a_binding_refuses_a_second_install_and_a_change_while_installed() {
    let mut rig = Rig::scratch_policy(1);
    let binding = rig.make("move_binding", &[]);
    binding.call("install", &[]);
    rig.settle();
    assert!(binding.try_call("install", &[]).is_err());
    assert!(binding.try_set("snap", b(false)).is_err());
    assert!(binding.try_set("trigger", t("Alt+MouseLeft+Drag")).is_err());
    binding.call("uninstall", &[]);
    assert!(binding.try_set("snap", b(false)).is_ok());
    let twin = rig.make("move_binding", &[]);
    binding.call("install", &[]);
    assert!(
        twin.try_call("install", &[]).is_err(),
        "two bindings of one trigger and one priority are refused"
    );
    let _ = &mut rig;
}
