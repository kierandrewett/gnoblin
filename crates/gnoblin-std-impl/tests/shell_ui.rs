#![allow(unsafe_code)]
//! The parts of the stock shell above the toolkit in `libgnoblin-std`, held to what the TypeScript parts did (`data/js/gnoblin-std/{alerts,
//! launcher,overview,lock}.ts`).
//!
//! Each test replays a scenario of `crates/gnoblin-js/tests/oracle/parts.rs` as a shell written over the object protocol: the same windows,
//! the same actions of the compositor in the same order, the same steps. What the fixture recorded of the TypeScript run is what the native
//! run must find at every step (`rig.rs` says what is compared): the log, the nodes the shell made whole, the snapshots of the trees and the
//! bytes of every paint list.
//!
//! **Allowances of the replay** (each is where it is made, with its reason):
//!
//! 1. `parts_alert_menu` is not replayed. It opens the window menu (`startWindowMenu`) and watches the alert close it, and the window menu is
//!    the menus lane's (NpStdMenus). What the alert does about a menu is its `close_menu` callback, which `the_alert_closes_the_menu_through_its_callback`
//!    asserts.
//! 2. `parts_alert_tracker` logs `cleanText('\ud800', 10)`, a string with a lone surrogate. A native string is UTF-8 and cannot hold one, so that
//!    sample is not made and its place in the log is not compared; the other four are.
//! 3. The fixture's alert reads `gnoblin.clients`, the TypeScript library's list; the native alert reads the windows of the compositor's tree in
//!    the order they appeared. The scenarios' windows are the same.

#[path = "ui/rig.rs"]
mod rig;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gnoblin_object::Val;
use gnoblin_testkit::oracle::{Fixtures, Value, json};
use gnoblin_tree_client::tree_mirror::MirrorValue;
use rig::{LEFT, Rig, W, b, callback, d, js, t};

type Comp = gnoblin_testkit::compositor::FakeCompositor;

/// The seat, the lock and the windows of the oracle's desktop.
const SEAT: u64 = 21;
const LOCK: u64 = 22;

/// The scenarios' world: a desktop of 1000 by 800 and `count` server-decorated windows, ids 3 on.
fn world(name: &'static str, count: u64) -> Rig {
    Rig::with_plain_windows(name, 1000.0, 800.0, count)
}

/// An object a value holds.
fn wrap(rig: &Rig, value: &Val) -> W {
    match value {
        Val::Object { class, obj } => W {
            api: rig.api,
            class: *class,
            obj: *obj,
        },
        other => panic!("{other:?} is not an object"),
    }
}

fn set(rig: &mut Rig, id: u64, key: &str, value: MirrorValue) {
    rig.external(id, key, value);
    rig.settle();
}

fn text(value: &str) -> MirrorValue {
    MirrorValue::Text(value.to_owned())
}

/// The node of the canvas that holds the keyboard, as the compositor's seat has it.
fn focus_node(comp: &Comp) -> u64 {
    match comp.value(SEAT, "focus") {
        Some(MirrorValue::Node(node)) => node,
        other => panic!("the keyboard is on {other:?}"),
    }
}

fn key(rig: &mut Rig, node: u64, key: &str, text: &str, pressed: bool) {
    rig.comp.key(node, key, text, pressed);
    rig.settle();
}

/// A key pressed and released at whatever holds the keyboard.
fn tap(rig: &mut Rig, name: &str) {
    let node = focus_node(&rig.comp);
    key(rig, node, name, "", true);
    key(rig, node, name, "", false);
}

/// Types `typed` a character at a time at whatever holds the keyboard.
fn type_secret(rig: &mut Rig, typed: &str) {
    for character in typed.chars() {
        let name = character.to_string();
        let node = focus_node(&rig.comp);
        rig.comp.key(node, &name, &name, true);
        rig.comp.key(node, &name, "", false);
    }
    rig.settle();
}

fn pointer(rig: &mut Rig, node: u64, kind: &str, x: f64, y: f64, button: u64) {
    rig.comp.pointer(node, kind, x, y, button);
    rig.settle();
}

// ---- the alert -----------------------------------------------------------------------------------------------------------------

/// What `startDecorations(); startNotRespondingAlert();` and the two options of the scenarios made: the decorations, the alert service and
/// the clock the alert is timed by.
struct Alert {
    _decorations: W,
    /// The stock lock screen, which the TypeScript runtime always had: a scenario that locks the session finds it up.
    _lock: W,
    alerts: W,
    clock: Rc<Cell<f64>>,
}

impl Alert {
    fn start(rig: &mut Rig) -> Alert {
        Alert::start_with(rig, true)
    }

    /// As [`Alert::start`]; with `clocked` false the alert is given no clock of its own, so it runs by the core's.
    fn start_with(rig: &mut Rig, clocked: bool) -> Alert {
        let decorations = rig.make("decorations", &[]);
        decorations.call("start", &[]);
        let alerts = rig.make("not_responding_alerts", &[]);
        alerts.call("start", &[]);
        alerts.set("arm_ms", d(1000.0));
        alerts.set("interval_ms", d(0.0));
        let clock = Rc::new(Cell::new(0.0));
        let time = Rc::clone(&clock);
        if clocked {
            alerts.set("now", callback(move |_| Ok(d(time.get()))));
        }
        rig.settle();
        Alert {
            _decorations: decorations,
            _lock: lock_screen(rig),
            alerts,
            clock,
        }
    }

    /// The alert open for window 3 (`gnoblin.clients.at(0)`), if any.
    fn card(&self, rig: &Rig) -> Option<W> {
        match self.alerts.call("card_of", &[rig.client(3)]) {
            Val::None => None,
            card => Some(wrap(rig, &card)),
        }
    }

    /// The scenarios' `cardOpen()`: `[index, armed, secure]` of the card, or none.
    fn log_card(&self, rig: &Rig) {
        let value = match self.card(rig) {
            None => Value::Null,
            Some(card) => json!([
                card.get("index").uint().expect("an index"),
                card.flag("armed"),
                card.flag("secure")
            ]),
        };
        rig.rec("card", value);
    }

    fn card_node(&self, rig: &Rig) -> u64 {
        self.card(rig).expect("an alert is open").node()
    }

    fn force_node(&self, rig: &Rig) -> u64 {
        let card = self.card(rig).expect("an alert is open");
        let buttons = card.get("choices");
        wrap(rig, &buttons.list().expect("a list")[1]).node()
    }

    fn advance(&self, rig: &mut Rig, ms: u64) {
        self.clock.set(self.clock.get() + ms as f64);
        rig.advance(ms);
    }

    /// The compositor marks a window unresponsive, or not.
    fn stuck(&self, rig: &mut Rig, node: u64, on: bool, label: &str) {
        set(rig, node, "unresponsive", MirrorValue::Bool(on));
        self.log_card(rig);
        rig.step(label);
    }

    /// A key at the card.
    fn card_key(&self, rig: &mut Rig, name: &str, label: &str) {
        let card = self.card_node(rig);
        key(rig, card, name, "", true);
        self.log_card(rig);
        rig.step(label);
    }
}

#[test]
fn not_responding_alert() {
    let mut rig = world("parts_alert", 1);
    let alert = Alert::start(&mut rig);
    rig.step("boot");
    alert.log_card(&rig);
    rig.step("responsive");
    alert.stuck(&mut rig, 3, true, "unresponsive");
    rig.answer_measures();
    rig.step("measured");
    let card = alert.card_node(&rig);
    rig.tree("alert", card);
    rig.paint("disarmed", card);
    rig.rec("focus", json!(rig.comp.flag(3, "focused").unwrap_or(false)));
    rig.step("focus_is_on_the_card");
    alert.card_key(&mut rig, "Tab", "tab_to_force_quit");
    alert.card_key(&mut rig, "Return", "force_quit_while_disarmed");
    rig.paint("tab_disarmed", card);
    alert.advance(&mut rig, 1200);
    alert.log_card(&rig);
    rig.step("armed");
    rig.answer_measures();
    rig.step("measured_armed");
    rig.paint("armed", card);
    alert.card_key(&mut rig, "Tab", "back_to_wait");
    alert.card_key(&mut rig, "Right", "right_to_force");
    alert.card_key(&mut rig, "Return", "force_quit");
    rig.step("after_force_quit");
    assert!(
        rig.comp.calls().contains(&(3, "kill".to_owned())),
        "Force quit asks the compositor to kill the window"
    );
    rig.finish();
}

#[test]
fn escape_waits_and_a_window_that_answers_may_stop_again() {
    let mut rig = world("parts_alert_wait", 2);
    let alert = Alert::start(&mut rig);
    rig.step("boot");
    alert.stuck(&mut rig, 3, true, "unresponsive");
    alert.card_key(&mut rig, "Escape", "escape_waits");
    rig.settle();
    alert.log_card(&rig);
    rig.step("no_second_alert");
    alert.stuck(&mut rig, 3, false, "answered");
    alert.stuck(&mut rig, 3, true, "again_unresponsive");
    let card = alert.card_node(&rig);
    pointer(&mut rig, card, "press", 5.0, 5.0, LEFT);
    rig.step("press_on_the_card");
    alert.advance(&mut rig, 1200);
    rig.step("armed");
    let force = alert.force_node(&rig);
    pointer(&mut rig, force, "press", 5.0, 5.0, LEFT);
    rig.step("press_force_quit");
    rig.finish();
}

#[test]
fn an_alert_closes_when_the_window_answers_goes_or_lies_under_a_lock() {
    let mut rig = world("parts_alert_closes", 2);
    let alert = Alert::start(&mut rig);
    rig.step("boot");
    alert.stuck(&mut rig, 3, true, "first_unresponsive");
    alert.stuck(&mut rig, 3, false, "first_answers");
    alert.stuck(&mut rig, 4, true, "second_unresponsive");
    rig.external_destroy(4);
    rig.settle();
    rig.rec(
        "canvases",
        json!(rig.function("ui", "canvas_count", &[]).uint().expect("a count")),
    );
    // The fixture still holds the frame of the window that went; the native run has dropped it with the window.
    rig.forget("second_gone", &["n8", "n9", "n10", "n11", "n12", "n13", "n14"]);
    rig.step("second_gone");
    set(&mut rig, LOCK, "state", text("locked"));
    rig.step("locked");
    alert.stuck(&mut rig, 3, true, "unresponsive_while_locked");
    set(&mut rig, LOCK, "state", text("not-locked"));
    alert.log_card(&rig);
    rig.step("unlocked");
    set(&mut rig, LOCK, "state", text("locked"));
    alert.log_card(&rig);
    rig.step("locked_again");
    rig.finish();
}

#[test]
fn the_alert_of_a_secure_window_is_secure_and_cleans_the_title() {
    let mut rig = world("parts_alert_secure", 1);
    let alert = Alert::start(&mut rig);
    rig.step("boot");
    rig.external(3, "secure", MirrorValue::Bool(true));
    rig.external(3, "title", text("a\nb"));
    rig.settle();
    rig.step("secure_title_with_a_newline");
    alert.stuck(&mut rig, 3, true, "unresponsive");
    rig.answer_measures();
    rig.step("measured");
    let card = alert.card_node(&rig);
    rig.tree("alert", card);
    rig.paint("secure", card);
    rig.finish();
}

#[test]
fn the_alert_closes_the_menu_through_its_callback() {
    let mut rig = Rig::scratch_with_windows(1000.0, 800.0, 1);
    let closed = Rc::new(Cell::new(0_u32));
    let alert = Alert::start(&mut rig);
    let counted = Rc::clone(&closed);
    alert.alerts.set(
        "close_menu",
        callback(move |_| {
            counted.set(counted.get() + 1);
            Ok(Val::None)
        }),
    );
    set(&mut rig, 3, "unresponsive", MirrorValue::Bool(true));
    assert_eq!(closed.get(), 1, "the menu is closed when the alert opens");
    assert!(alert.card(&rig).is_some());
}

#[test]
fn with_no_clock_given_the_alert_arms_by_the_cores_clock() {
    // The shell gave `now` to no one: the arm delay is due on the clock that wakes the toolkit for it, which is the one `Rig::advance` moves.
    let mut rig = Rig::scratch_with_windows(1000.0, 800.0, 1);
    let alert = Alert::start_with(&mut rig, false);
    set(&mut rig, 3, "unresponsive", MirrorValue::Bool(true));
    let card = alert.card(&rig).expect("an alert opens");
    assert!(!card.flag("armed"), "the card is not armed at once");
    rig.advance(500);
    let card = alert.card(&rig).expect("the alert is still open");
    assert!(!card.flag("armed"), "the delay is not over");
    rig.advance(700);
    let card = alert.card(&rig).expect("the alert is still open");
    assert!(card.flag("armed"), "after the delay the card is armed");
    assert!(
        rig.reported().is_empty(),
        "the alert read the core's clock without complaint"
    );
}

#[test]
fn the_tracker_and_the_text_it_shows() {
    let mut rig = world("parts_alert_tracker", 1);
    rig.step("boot");
    let tracker = rig.make("alert_tracker", &[]);
    let out = Rc::new(RefCell::new(Vec::<String>::new()));
    let look = |rig: &Rig, locked: bool| {
        let changes = tracker.call("check", &[Val::List(vec![t("3")]), b(locked)]);
        let fields = changes.record().expect("changes");
        out.borrow_mut().push(format!(
            "{}{}",
            fields[0].list().expect("opens").len(),
            fields[1].list().expect("closes").len()
        ));
        let _ = rig;
    };
    let client = rig.client(3);
    let answered = |answer: u32| {
        out.borrow_mut().push(
            tracker
                .call("answered", &[client.clone(), rig::en(answer)])
                .bool()
                .expect("a flag")
                .to_string(),
        );
    };
    look(&rig, false);
    set(&mut rig, 3, "unresponsive", MirrorValue::Bool(true));
    look(&rig, false);
    look(&rig, false);
    answered(1);
    look(&rig, false);
    set(&mut rig, 3, "unresponsive", MirrorValue::Bool(false));
    look(&rig, false);
    set(&mut rig, 3, "unresponsive", MirrorValue::Bool(true));
    look(&rig, false);
    answered(2);
    look(&rig, true);
    set(&mut rig, 3, "unresponsive", MirrorValue::Bool(false));
    look(&rig, false);
    set(&mut rig, 3, "unresponsive", MirrorValue::Bool(true));
    look(&rig, true);
    look(&rig, false);
    rig.rec("tracker", json!(out.borrow().clone()));
    let clean = |input: &str, max: u64| -> String {
        rig.function("alert_tracker", "clean_text", &[t(input), Val::Uint(max)])
            .text()
            .expect("text")
            .to_owned()
    };
    let samples = [
        clean("a\nb", 80),
        clean(&"x".repeat(70), 60),
        clean("tab\there", 80),
        clean("caf\u{e9} \u{2713}", 80),
    ];
    // The fixture's second sample is the length of the cut text, and its third is the lone surrogate no native string can hold.
    let fixture = Fixtures::new("crates/gnoblin-std-impl/tests/fixtures/reference").read_json("parts_alert_tracker");
    let wanted = fixture["steps"][1]["log"]
        .as_array()
        .and_then(|log| log.iter().find(|entry| entry["label"] == "clean"))
        .map(|entry| entry["value"].clone())
        .expect("the fixture's `clean` entry");
    assert_eq!(json!(samples[0]), wanted[0]);
    assert_eq!(json!(samples[1].chars().count()), wanted[1]);
    assert_eq!(json!(samples[2]), wanted[3]);
    rig.skip(&["clean"]);
    assert_eq!(json!(samples[3]), wanted[4]);
    rig.step("tracker_and_text");
    rig.finish();
}

// ---- the launcher --------------------------------------------------------------------------------------------------------------

#[test]
fn the_launcher_runs_a_command_and_closes() {
    let mut rig = world("parts_launcher", 1);
    let spawned = Rc::new(RefCell::new(Vec::<String>::new()));
    let open = |rig: &Rig| -> W {
        let run = Rc::clone(&spawned);
        let opened = rig.function(
            "launcher",
            "open",
            &[callback(move |args| {
                run.borrow_mut().push(args[0].text()?.to_owned());
                Ok(Val::None)
            })],
        );
        wrap(rig, &opened)
    };
    rig.step("boot");
    let launcher = open(&rig);
    rig.settle();
    rig.answer_measures();
    rig.rec(
        "rect",
        json!([
            js(launcher.num("x")),
            js(launcher.num("y")),
            js(launcher.num("width")),
            js(launcher.num("height"))
        ]),
    );
    rig.rec("again", json!(open(&rig).obj == launcher.obj));
    rig.step("opened");
    let node = launcher.node();
    rig.tree("launcher", node);
    rig.paint("empty", node);
    for character in ["f", "o", "o"] {
        key(&mut rig, node, character, character, true);
    }
    rig.answer_measures();
    rig.step("typed");
    rig.paint("typed", node);
    key(&mut rig, node, "Return", "", true);
    rig.rec("spawned", json!(spawned.borrow().clone()));
    rig.rec("alive", json!(launcher.flag("alive")));
    rig.step("submitted");
    // The TypeScript toolkit asked again for the text of the new entry and drew its estimate until the compositor answered.
    rig.fixed.unanswer();
    let launcher = open(&rig);
    rig.settle();
    let node = launcher.node();
    rig.step("reopened");
    key(&mut rig, node, "Escape", "", true);
    rig.rec("spawned", json!(spawned.borrow().clone()));
    rig.rec("alive", json!(launcher.flag("alive")));
    rig.step("escaped");
    rig.fixed.unanswer();
    let launcher = open(&rig);
    rig.settle();
    let node = launcher.node();
    key(&mut rig, node, " ", " ", true);
    key(&mut rig, node, "Return", "", true);
    rig.rec("spawned", json!(spawned.borrow().clone()));
    rig.rec("alive", json!(launcher.flag("alive")));
    rig.step("blank_runs_nothing");
    rig.finish();
}

// ---- the overview --------------------------------------------------------------------------------------------------------------

#[test]
fn the_overview_dims_the_windows_of_workspaces_that_are_not_shown() {
    let mut rig = world("parts_overview", 2);
    let list = rig.make("workspaces", &[]);
    let spaces: Vec<W> = ["a", "b"]
        .into_iter()
        .map(|id| {
            let space = rig.make("workspace", &[("id", t(id))]);
            list.call("add", &[space.val()]);
            space
        })
        .collect();
    rig.settle();
    let overview = rig.make("overview", &[]);
    let opacities = |comp: &Comp| -> Value {
        json!(
            [3_u64, 4]
                .iter()
                .map(|id| match comp.value(*id, "opacity") {
                    Some(MirrorValue::Double(value)) => js(value),
                    _ => js(1.0),
                })
                .collect::<Vec<_>>()
        )
    };
    rig.step("boot");
    spaces[1].obj("clients").call("add", &[rig.client(4)]);
    rig.settle();
    rig.step("second_window_moves_to_the_last_workspace");
    rig.rec("on", json!(overview.call("toggle", &[]).bool().expect("a flag")));
    rig.settle();
    rig.rec("opacity", opacities(&rig.comp));
    rig.step("on");
    rig.rec("on", json!(overview.call("toggle", &[]).bool().expect("a flag")));
    rig.settle();
    rig.rec("opacity", opacities(&rig.comp));
    rig.step("off");
    for _ in 0..2 {
        rig.rec("on", json!(overview.call("toggle", &[]).bool().expect("a flag")));
    }
    rig.settle();
    rig.rec("opacity", opacities(&rig.comp));
    rig.step("toggled_twice");
    rig.finish();
}

// ---- the lock screen -----------------------------------------------------------------------------------------------------------

/// The time the lock screen shows: 2023-11-14 22:13:20 UTC, as the scenarios fix `Date.now`.
const FIXED_TIME: f64 = 1_700_000_000_000.0;

fn lock_screen(rig: &Rig) -> W {
    rig.make("lock_screen", &[("now", callback(|_| Ok(d(FIXED_TIME))))])
}

/// The compositor answers the last password check the shell sent.
fn answer_check(rig: &mut Rig, result: &str, error: &str) {
    let token = rig.comp.checks().last().copied().expect("a password check was sent");
    rig.comp.answer(token, result, error);
    rig.settle();
}

#[test]
fn the_lock_screen_takes_passwords() {
    let mut rig = world("parts_lock", 1);
    let _screen = lock_screen(&rig);
    rig.settle();
    rig.step("boot");
    set(&mut rig, 3, "focused", MirrorValue::Bool(true));
    rig.step("window_focused");
    set(&mut rig, LOCK, "state", text("pending"));
    rig.step("lock_requested");
    rig.answer_measures();
    rig.step("measured");
    rig.tree("lock", LOCK);
    rig.paint("hint", LOCK);
    type_secret(&mut rig, "hunter2");
    rig.answer_measures();
    rig.step("typed");
    rig.paint("typed", LOCK);
    tap(&mut rig, "Return");
    answer_check(&mut rig, "", "denied");
    rig.answer_measures();
    rig.step("denied");
    rig.paint("denied", LOCK);
    type_secret(&mut rig, "again");
    tap(&mut rig, "BackSpace");
    tap(&mut rig, "Return");
    answer_check(&mut rig, "1500", "rate-limited");
    rig.answer_measures();
    rig.step("rate_limited");
    rig.paint("rate_limited", LOCK);
    type_secret(&mut rig, "b");
    tap(&mut rig, "Return");
    answer_check(&mut rig, "", "busy");
    rig.answer_measures();
    rig.step("busy");
    rig.paint("busy", LOCK);
    type_secret(&mut rig, "c");
    tap(&mut rig, "Return");
    answer_check(&mut rig, "", "failed: no PAM");
    rig.answer_measures();
    rig.step("failed");
    rig.paint("failed", LOCK);
    tap(&mut rig, "Return");
    rig.step("empty_return_is_not_an_attempt");
    type_secret(&mut rig, "right");
    tap(&mut rig, "Return");
    // The TypeScript toolkit drew the message at the estimate it had made of the text before the compositor answered; the native toolkit
    // measures at once, and had measured this text at an earlier attempt.
    rig.allow("checking", "n5", "height", 19.0, 16.0);
    rig.step("checking");
    rig.paint("checking", LOCK);
    tap(&mut rig, "Return");
    rig.step("return_while_checking");
    answer_check(&mut rig, "ok", "");
    rig.step("accepted");
    rig.tree("lock", LOCK);
    set(&mut rig, LOCK, "state", text("not-locked"));
    rig.step("unlocked");
    rig.tree("lock", LOCK);
    rig.finish();
}

#[test]
fn a_session_that_is_unlocked_elsewhere_takes_the_screen_down_and_locks_again() {
    let mut rig = world("parts_lock_relock", 1);
    let _screen = lock_screen(&rig);
    rig.settle();
    rig.step("boot");
    set(&mut rig, LOCK, "state", text("locked"));
    rig.step("locked");
    set(&mut rig, LOCK, "state", text("not-locked"));
    rig.step("unlocked_elsewhere");
    rig.tree("lock", LOCK);
    set(&mut rig, LOCK, "state", text("locked"));
    rig.step("locked_again");
    rig.tree("lock", LOCK);
    rig.finish();
}

// ---- what the fixtures do not hold ---------------------------------------------------------------------------------------------

#[test]
fn a_shell_that_makes_no_lock_screen_leaves_the_lock_to_the_compositors_card() {
    let mut rig = Rig::scratch_with_windows(1000.0, 800.0, 1);
    set(&mut rig, LOCK, "state", text("locked"));
    rig.settle();
    assert_eq!(
        rig.comp.children(LOCK),
        Vec::<u64>::new(),
        "no canvas is drawn under the lock"
    );
    assert_ne!(rig.comp.value(LOCK, "engaged"), Some(MirrorValue::Bool(true)));
}

#[test]
fn the_lock_screen_comes_down_when_the_shell_ends_it() {
    let mut rig = Rig::scratch_with_windows(1000.0, 800.0, 1);
    let screen = lock_screen(&rig);
    set(&mut rig, LOCK, "state", text("locked"));
    assert!(screen.flag("shown"));
    assert_eq!(rig.comp.value(LOCK, "engaged"), Some(MirrorValue::Bool(true)));
    screen.destroy();
    rig.settle();
    assert_eq!(rig.comp.value(LOCK, "engaged"), Some(MirrorValue::Bool(false)));
    assert!(rig.comp.children(LOCK).is_empty(), "the canvases went with the screen");
}

#[test]
fn hide_keeps_the_screen_down_until_show() {
    let mut rig = Rig::scratch_with_windows(1000.0, 800.0, 1);
    let screen = lock_screen(&rig);
    set(&mut rig, LOCK, "state", text("locked"));
    assert!(screen.flag("shown"));
    screen.call("hide", &[]);
    rig.settle();
    assert!(!screen.flag("shown"));
    assert_eq!(rig.comp.value(LOCK, "engaged"), Some(MirrorValue::Bool(false)));
    screen.call("show", &[]);
    rig.settle();
    assert!(screen.flag("shown"));
}

#[test]
fn a_second_lock_screen_is_refused() {
    let rig = Rig::scratch_with_windows(1000.0, 800.0, 1);
    let _first = lock_screen(&rig);
    let class = rig.api.class("lock_screen").expect("the class");
    assert!(rig.api.create(class, &[]).is_err());
}

#[test]
fn the_screen_takes_the_keyboard_back_and_gives_it_to_the_window_that_took_it() {
    let mut rig = Rig::scratch_with_windows(1000.0, 800.0, 2);
    let _screen = lock_screen(&rig);
    set(&mut rig, LOCK, "state", text("locked"));
    let entry = focus_node(&rig.comp);
    // A window takes the keyboard from the screen.
    rig.external(SEAT, "focus", MirrorValue::Node(4));
    rig.settle();
    assert_eq!(focus_node(&rig.comp), entry, "the screen takes the keyboard back");
    rig.external(SEAT, "focus", MirrorValue::Node(4));
    rig.settle();
    set(&mut rig, LOCK, "state", text("not-locked"));
    assert_eq!(
        focus_node(&rig.comp),
        4,
        "the window that took it has it when the screen is down"
    );
}

#[test]
fn the_password_entry_keeps_what_it_is_given_until_it_hands_it_on() {
    let mut rig = Rig::scratch_with_windows(1000.0, 800.0, 1);
    let style = rig.function("lock_screen", "default_lock_style", &[]);
    let entry = rig.make("password_entry", &[("style", style), ("width", d(200.0))]);
    entry.mount_top();
    entry.focus();
    let seen = Rc::new(RefCell::new(Vec::<String>::new()));
    let heard = Rc::clone(&seen);
    entry.set(
        "on_submit",
        callback(move |args| {
            heard.borrow_mut().push(args[0].text()?.to_owned());
            Ok(Val::None)
        }),
    );
    rig.settle();
    let node = entry.node();
    type_secret(&mut rig, "ab\u{e9}c");
    assert_eq!(entry.get("length").uint().expect("a count"), 4);
    key(&mut rig, node, "BackSpace", "", true);
    assert_eq!(entry.get("length").uint().expect("a count"), 3);
    entry.set("enabled", b(false));
    type_secret(&mut rig, "zz");
    assert_eq!(
        entry.get("length").uint().expect("a count"),
        3,
        "a disabled entry drops keys"
    );
    entry.set("enabled", b(true));
    key(&mut rig, node, "Return", "", true);
    assert_eq!(seen.borrow().clone(), vec!["ab\u{e9}".to_owned()]);
    assert_eq!(
        entry.get("length").uint().expect("a count"),
        0,
        "the entry forgets the password"
    );
    key(&mut rig, node, "Return", "", true);
    assert_eq!(seen.borrow().len(), 1, "an empty entry submits nothing");
    entry.set("limit", Val::Uint(2));
    type_secret(&mut rig, "xyz");
    assert_eq!(
        entry.get("length").uint().expect("a count"),
        2,
        "the entry takes no more than its limit"
    );
    key(&mut rig, node, "a", "\u{7}", true);
    assert_eq!(
        entry.get("length").uint().expect("a count"),
        2,
        "a control character is not typed"
    );
}
