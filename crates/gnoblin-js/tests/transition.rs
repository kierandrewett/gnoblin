//! `gnoblin.requestTransition` over a fake compositor: the callback runs once, its writes are staged and travel as one
//! transaction of timed writes, a layout change inside it is computed once and every window tweens from where it is, and a
//! failure leaves the previous state (`design/js-research/spec-review.md`, rows AN1, AN2, AN6 to AN8).
//!
//! The fake compositor applies a write at once, so what a test reads back from it is the logical state the transition
//! committed; what the compositor draws on the way is its tween player's (`gnoblin-wayland/tests/tree_live.rs`) and is
//! asserted there. Here the proof is the transaction: which writes, timed how, in how many transactions.

mod common;

use common::Rig;
use common::policy::{SEAT, exec, settle, world};
use gnoblin_tree_client::tree_mirror::{MirrorValue, TxOp};

/// A timed or plain write the shell sent: `(node, key, value, seconds, samples)`.
#[derive(Debug, Clone, PartialEq)]
struct Write {
    node: u64,
    key: String,
    value: MirrorValue,
    seconds: Option<f64>,
    samples: usize,
}

fn writes(rig: &Rig, mark: usize) -> Vec<Write> {
    rig.comp
        .ops_since(mark)
        .into_iter()
        .filter_map(|op| match op {
            TxOp::Set {
                node,
                key,
                value,
                seconds,
                samples,
                ..
            } => Some(Write {
                node,
                key,
                value,
                seconds,
                samples: samples.map_or(0, |list| list.len()),
            }),
            _ => None,
        })
        .collect()
}

/// The `(node, key)` of every timed write after `mark`, sorted.
fn timed(rig: &Rig, mark: usize) -> Vec<(u64, String)> {
    let mut keys: Vec<_> = writes(rig, mark)
        .into_iter()
        .filter(|write| write.seconds.is_some())
        .map(|write| (write.node, write.key))
        .collect();
    keys.sort();
    keys
}

fn plain(rig: &Rig, mark: usize) -> Vec<(u64, String)> {
    let mut keys: Vec<_> = writes(rig, mark)
        .into_iter()
        .filter(|write| write.seconds.is_none())
        .map(|write| (write.node, write.key))
        .collect();
    keys.sort();
    keys
}

/// The compositor says the latest timed write of `node`'s `key` sent after `from` has ended.
fn end_timed(rig: &Rig, from: usize, node: u64, key: &str, completed: bool) {
    let tween = rig
        .comp
        .ops_since(from)
        .into_iter()
        .rev()
        .find_map(|op| match op {
            TxOp::Set {
                node: target,
                key: written,
                seconds: Some(_),
                tween,
                ..
            } if target == node && written == key => Some(tween),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no timed write of {node}.{key} after {from}"));
    rig.comp.track_ended(node, key, completed, tween);
}

fn double(value: &MirrorValue) -> f64 {
    match value {
        MirrorValue::Double(number) => *number,
        other => panic!("not a number: {other:?}"),
    }
}

fn thrown(rig: &mut Rig, body: &str) -> String {
    rig.eval(&format!(
        "(() => {{ try {{ {body}; return 'no'; }} catch (e) {{ return e.name + ': ' + e.message; }} }})()"
    ))
}

fn rect_of(rig: &Rig, node: u64) -> [f64; 4] {
    ["x", "y", "width", "height"].map(|key| rig.comp.number(node, key))
}

/// A world of two windows (3 and 4) tiled by `layout`, with the clients and the first workspace in `g`.
fn two(layout: &str) -> Rig {
    let mut rig = world(2, false, layout, "");
    exec(
        &mut rig,
        "g.a = gnoblin.clients.get('3'); g.b = gnoblin.clients.get('4'); g.ws = gnoblin.workspaces.get('1'); \
         g.computes = 0; g.calls = 0; \
         g.Half = class extends Layout { compute(c) { g.computes += 1; const h = c.area.height / 2; \
             return c.children.map((n, i) => ({ node: n, tiled: true, \
                 bounds: { x: c.area.x, y: c.area.y + i * h, width: c.area.width, height: h } })); } };",
    );
    settle(&mut rig);
    rig
}

#[test]
fn the_callback_runs_once_and_its_writes_are_one_transaction_of_timed_writes_with_the_easings_samples() {
    // AN1: "callback invoked exactly once; N writes yield exactly one tree transaction".
    let mut rig = two("new Layout()");
    let mark = rig.comp.sent_count();
    exec(
        &mut rig,
        "g.t = gnoblin.requestTransition(() => { g.calls += 1; g.a.opacity = 0.5; g.b.opacity = 0.25; g.a.x = 70; }, \
         { duration: 200, easing: 'out-cubic' });",
    );
    assert_eq!(rig.eval("String(g.calls)"), "1", "the callback ran in the call");
    assert_eq!(rig.comp.sent_count(), mark, "nothing is sent before the frame ends");
    settle(&mut rig);
    rig.advance(1000);
    settle(&mut rig);
    assert_eq!(
        rig.eval("String(g.calls)"),
        "1",
        "and never again, whatever frames pass (AN4's JS half)"
    );
    assert_eq!(rig.comp.sent_count() - mark, 1, "three writes, one transaction");
    let sent = writes(&rig, mark);
    assert_eq!(
        timed(&rig, mark),
        [
            (3, "opacity".to_owned()),
            (3, "x".to_owned()),
            (4, "opacity".to_owned())
        ]
    );
    for write in &sent {
        assert_eq!(
            write.seconds,
            Some(0.2),
            "the duration is milliseconds, sent as seconds: {write:?}"
        );
        assert_eq!(write.samples, 33, "the easing's 33 samples");
    }
    let targets: Vec<f64> = sent.iter().map(|write| double(&write.value)).collect();
    assert_eq!(targets, [0.5, 0.25, 70.0]);
    assert_eq!(rig.comp.number(3, "opacity"), 0.5, "the logical state is the target");
}

#[test]
fn an_easing_name_is_the_clients_or_a_css_alias_and_anything_else_is_refused_before_the_callback_runs() {
    let mut rig = two("new Layout()");
    let mark = rig.comp.sent_count();
    for easing in [
        "linear",
        "out",
        "out-cubic",
        "in-cubic",
        "in-out",
        "ease-out",
        "ease-in-out",
        "ease-in",
    ] {
        exec(
            &mut rig,
            &format!(
                "gnoblin.requestTransition(() => {{ g.a.opacity = 0.5; }}, {{ duration: 10, easing: '{easing}' }});"
            ),
        );
    }
    settle(&mut rig);
    let sent = writes(&rig, mark);
    assert!(sent.iter().all(|write| write.seconds == Some(0.01)), "{sent:?}");
    assert_eq!(
        thrown(
            &mut rig,
            "gnoblin.requestTransition(() => { g.calls += 1; }, { easing: 'bouncy' })"
        ),
        "TypeError: gnoblin.requestTransition: easing must be one of linear, out, out-cubic, in-cubic, in-out, ease-out, ease-in-out, ease-in"
    );
    assert_eq!(
        thrown(
            &mut rig,
            "gnoblin.requestTransition(() => { g.calls += 1; }, { duration: -1 })"
        ),
        "TypeError: gnoblin.requestTransition: duration must be a finite number of milliseconds of at least 0"
    );
    assert_eq!(
        thrown(
            &mut rig,
            "gnoblin.requestTransition(() => { g.calls += 1; }, { autoplay: 'no' })"
        ),
        "TypeError: gnoblin.requestTransition: autoplay must be true or false"
    );
    assert_eq!(
        thrown(
            &mut rig,
            "gnoblin.requestTransition(() => { g.calls += 1; }, { delay: -10 })"
        ),
        "TypeError: gnoblin.requestTransition: delay must be a finite number of milliseconds of at least 0"
    );
    assert_eq!(
        thrown(
            &mut rig,
            "gnoblin.requestTransition(() => { g.calls += 1; }, { speed: 2 })"
        ),
        "TypeError: gnoblin.requestTransition: unknown option \"speed\" (known: duration, easing, delay, autoplay)"
    );
    assert_eq!(
        rig.eval("String(g.calls)"),
        "0",
        "a refused option never runs the callback"
    );
}

#[test]
fn reads_inside_the_callback_see_the_staged_value_and_a_write_outside_is_immediate() {
    let mut rig = two("new Layout()");
    let inside = rig.eval(
        "(() => { let seen = ''; gnoblin.requestTransition(() => { g.a.opacity = 0.3; g.a.rotation = [0, 1, 0]; \
         seen = g.a.opacity + '|' + g.a.rotation.join(','); }, { duration: 100 }); return seen; })()",
    );
    assert_eq!(inside, "0.3|0,1,0");
    settle(&mut rig);
    let mark = rig.comp.sent_count();
    exec(&mut rig, "g.a.opacity = 0.9;");
    settle(&mut rig);
    assert_eq!(rig.comp.sent_count() - mark, 1);
    assert_eq!(
        timed(&rig, mark),
        Vec::<(u64, String)>::new(),
        "plain assignment is not animated"
    );
    assert_eq!(plain(&rig, mark), [(3, "opacity".to_owned())]);
}

#[test]
fn a_layout_change_in_the_callback_computes_once_and_tweens_every_windows_bounds() {
    // AN2: "flat -> arc interpolates by client id", here the staged compute of a new layout.
    let mut rig = two("new DwindleLayout()");
    let before = (rect_of(&rig, 3), rect_of(&rig, 4));
    let mark = rig.comp.sent_count();
    exec(
        &mut rig,
        "g.t = gnoblin.requestTransition(() => { g.ws.layout = new g.Half(); }, { duration: 300, easing: 'out-cubic' });",
    );
    assert_eq!(
        rig.eval("String(g.computes)"),
        "1",
        "computed in the call, against the staged layout"
    );
    settle(&mut rig);
    assert_eq!(
        rig.eval("String(g.computes)"),
        "1",
        "and not again by the frames after it"
    );
    assert_eq!(rig.comp.sent_count() - mark, 1, "one transaction");
    let after = (rect_of(&rig, 3), rect_of(&rig, 4));
    assert_ne!(before, after, "the layout moved the windows");
    let mut expected = Vec::new();
    for (node, from, to) in [(3, before.0, after.0), (4, before.1, after.1)] {
        for (at, key) in ["x", "y", "width", "height"].into_iter().enumerate() {
            if from[at] != to[at] {
                expected.push((node, key.to_owned()));
            }
        }
    }
    expected.sort();
    assert_eq!(
        timed(&rig, mark),
        expected,
        "each changed bound of each window is a timed write, by node id"
    );
    assert_eq!(plain(&rig, mark), Vec::<(u64, String)>::new());
    for write in writes(&rig, mark) {
        assert_eq!(write.seconds, Some(0.3));
        let at = ["x", "y", "width", "height"]
            .iter()
            .position(|key| *key == write.key)
            .expect("a bound");
        let target = if write.node == 3 { after.0[at] } else { after.1[at] };
        assert_eq!(double(&write.value), target, "the write is the layout's staged result");
    }
}

#[test]
fn a_layout_field_changed_in_the_callback_is_computed_once_against_the_staged_value() {
    let mut rig = two("new MasterStackLayout({ ratio: 0.5 })");
    let before = (rect_of(&rig, 3), rect_of(&rig, 4));
    let mark = rig.comp.sent_count();
    exec(
        &mut rig,
        "g.t = gnoblin.requestTransition(() => { g.ws.layout.ratio = 0.7; }, { duration: 150 });",
    );
    settle(&mut rig);
    assert_eq!(rig.comp.sent_count() - mark, 1);
    let after = (rect_of(&rig, 3), rect_of(&rig, 4));
    assert_ne!(before, after, "the master column grew");
    assert!(after.0[2] > before.0[2], "{before:?} -> {after:?}");
    let keys = timed(&rig, mark);
    assert!(
        keys.contains(&(3, "width".to_owned())) && keys.contains(&(4, "x".to_owned())),
        "{keys:?}"
    );
    assert_eq!(plain(&rig, mark), Vec::<(u64, String)>::new());
}

#[test]
fn a_window_with_no_place_just_appears_while_the_others_tween() {
    let mut rig = two("new DwindleLayout()");
    rig.comp.external_set(4, "visible", MirrorValue::Bool(false));
    settle(&mut rig);
    let mark = rig.comp.sent_count();
    exec(
        &mut rig,
        "g.t = gnoblin.requestTransition(() => { g.ws.layout = new g.Half(); }, { duration: 300 });",
    );
    settle(&mut rig);
    let moved = writes(&rig, mark);
    assert!(
        moved
            .iter()
            .filter(|write| write.node == 4)
            .all(|write| write.seconds.is_none()),
        "the hidden window's place is written at once: {moved:?}"
    );
    assert!(
        moved.iter().any(|write| write.node == 4),
        "and it is written: {moved:?}"
    );
    assert!(
        moved
            .iter()
            .filter(|write| write.node == 3)
            .all(|write| write.seconds == Some(0.3)),
        "the drawn window tweens: {moved:?}"
    );
}

#[test]
fn opacity_rotation_scale_and_style_opacity_tween_and_a_style_effect_lands_at_once() {
    let mut rig = two("new Layout()");
    let mark = rig.comp.sent_count();
    exec(
        &mut rig,
        "g.t = gnoblin.requestTransition(() => { g.a.rotation = [0, 0.5, 0]; g.a.scale = [2, 2, 1]; \
         gnoblin.clients.style.opacity = 0.6; g.b.z = 40; }, \
         { duration: 250, easing: 'ease-out' });",
    );
    settle(&mut rig);
    let keys = timed(&rig, mark);
    for expected in [(3, "opacity"), (3, "rotation"), (3, "scale"), (4, "opacity"), (4, "z")] {
        assert!(
            keys.contains(&(expected.0, expected.1.to_owned())),
            "{expected:?} in {keys:?}"
        );
    }
    assert_eq!(rig.comp.number(3, "opacity"), 0.6, "the style's opacity is the node's");
    assert_eq!(rig.comp.sent_count() - mark, 1);
}

#[test]
fn nothing_animates_under_reduced_motion_but_the_end_state_and_the_finish_are_the_same() {
    let mut rig = two("new DwindleLayout()");
    rig.comp.external_set(SEAT, "reduced_motion", MirrorValue::Bool(true));
    settle(&mut rig);
    let mark = rig.comp.sent_count();
    exec(
        &mut rig,
        "g.done = []; g.t = gnoblin.requestTransition(() => { g.a.opacity = 0.5; g.ws.layout = new g.Half(); }, { duration: 300 }); \
         g.t.onFinished((r) => g.done.push(r.cancelled));",
    );
    settle(&mut rig);
    assert_eq!(
        timed(&rig, mark),
        Vec::<(u64, String)>::new(),
        "no timed write: nothing animates"
    );
    assert_eq!(rig.comp.sent_count() - mark, 1, "still one transaction");
    assert_eq!(rig.comp.number(3, "opacity"), 0.5);
    assert_eq!(rig.eval("String(g.computes)"), "1");
    assert_eq!(
        rig.eval("g.done.join()"),
        "false",
        "the lifecycle notification still comes, and not cancelled"
    );
    assert_eq!(rig.eval("String(g.t.running) + ',' + g.t.duration"), "false,0");
}

#[test]
fn a_failing_callback_leaves_the_previous_state_and_names_the_property() {
    // AN7: "invalid or stale target aborts the whole transaction; previous state intact; error names path".
    let mut rig = two("new DwindleLayout()");
    let before = (rect_of(&rig, 3), rect_of(&rig, 4));
    let style = rig.eval("String(gnoblin.clients.style.opacity)");
    let mark = rig.comp.sent_count();
    let message = thrown(
        &mut rig,
        "gnoblin.requestTransition(() => { g.b.opacity = 0.1; g.ws.layout = new g.Half(); \
         gnoblin.clients.style.opacity = 0.2; g.a.opacity = 'dim'; }, { duration: 100 })",
    );
    assert_eq!(message, "TypeError: Client#3.opacity: must be a finite number");
    settle(&mut rig);
    assert_eq!(rig.comp.sent_count(), mark, "nothing was sent");
    assert_eq!((rect_of(&rig, 3), rect_of(&rig, 4)), before);
    assert_eq!(
        rig.eval("g.ws.layout.constructor.name"),
        "DwindleLayout",
        "the layout is put back"
    );
    assert_eq!(rig.eval("String(g.b.opacity)"), "1", "the staged write is gone");
    assert_eq!(
        rig.eval("String(gnoblin.clients.style.opacity)"),
        style,
        "and so is the style write"
    );
    assert_eq!(
        rig.eval("String(g.computes)"),
        "0",
        "the callback failed before the layout pass"
    );
    // A layout that throws fails the transition with its own error, and the workspace keeps its layout.
    let message = thrown(
        &mut rig,
        "gnoblin.requestTransition(() => { g.ws.layout = new (class extends Layout { compute() { throw new Error('boom'); } })(); }, { duration: 100 })",
    );
    assert_eq!(message, "Error: boom");
    settle(&mut rig);
    assert_eq!(rig.comp.sent_count(), mark);
    assert_eq!(rig.eval("g.ws.layout.constructor.name"), "DwindleLayout");
    // A destroyed client is a stale target.
    rig.comp.destroy(4);
    settle(&mut rig);
    let mark = rig.comp.sent_count();
    let message = thrown(
        &mut rig,
        "gnoblin.requestTransition(() => { g.a.opacity = 0.5; g.b.opacity = 0.5; }, { duration: 100 })",
    );
    assert_eq!(message, "GnoblinError: Client#4.opacity: the client is gone");
    settle(&mut rig);
    assert_eq!(
        rig.comp.sent_count(),
        mark,
        "the write staged before the stale one was dropped with it"
    );
    assert_eq!(rig.eval("String(g.a.opacity)"), "1");
}

#[test]
fn a_nested_transition_and_an_async_callback_are_errors_with_a_clear_message_and_change_nothing() {
    // AN1 (a returned Promise is rejected) and AN8 (the nested call follows the decided rule: an error).
    let mut rig = two("new Layout()");
    let mark = rig.comp.sent_count();
    assert_eq!(
        thrown(
            &mut rig,
            "gnoblin.requestTransition(() => { g.a.opacity = 0.5; gnoblin.requestTransition(() => {}, { duration: 1 }); }, { duration: 100 })"
        ),
        "GnoblinError: gnoblin.requestTransition: was called inside a requestTransition callback; stage every change in the outer callback instead"
    );
    assert_eq!(
        thrown(
            &mut rig,
            "gnoblin.requestTransition(async () => { g.a.opacity = 0.5; }, { duration: 100 })"
        ),
        "TypeError: gnoblin.requestTransition: the callback returned a Promise; it runs once and synchronously, so it cannot be async or await"
    );
    assert_eq!(
        thrown(&mut rig, "gnoblin.requestTransition('nope')"),
        "TypeError: gnoblin.requestTransition: the first argument must be a function that stages the changes"
    );
    settle(&mut rig);
    assert_eq!(rig.comp.sent_count(), mark);
    assert_eq!(rig.eval("String(g.a.opacity)"), "1");
    // The failures left staging off: a plain write is immediate again.
    exec(&mut rig, "g.a.opacity = 0.7;");
    settle(&mut rig);
    assert_eq!(rig.comp.number(3, "opacity"), 0.7);
}

#[test]
fn a_call_with_a_side_effect_is_refused_inside_the_callback() {
    let mut rig = two("new Layout()");
    let mark = rig.comp.sent_count();
    assert_eq!(
        thrown(
            &mut rig,
            "gnoblin.requestTransition(() => { g.a.raise(); }, { duration: 1 })"
        ),
        "GnoblinError: Client#3.raise(): is not valid inside a requestTransition callback: it has a side effect, and a callback only stages property writes"
    );
    assert_eq!(
        thrown(
            &mut rig,
            "gnoblin.requestTransition(() => { gnoblin.spawn(['true']); }, { duration: 1 })"
        ),
        "GnoblinError: gnoblin.spawn(): is not valid inside a requestTransition callback: it has a side effect, and a callback only stages property writes"
    );
    settle(&mut rig);
    assert_eq!(rig.comp.sent_count(), mark);
    exec(&mut rig, "g.a.raise();");
    settle(&mut rig);
    assert!(rig.comp.sent_count() > mark, "raise works again outside a callback");
}

#[test]
fn the_newest_transition_owns_a_property_and_the_older_one_goes_on_with_the_rest() {
    // AN8: "overlapping transitions: newest wins per component, others continue".
    let mut rig = two("new Layout()");
    exec(
        &mut rig,
        "g.log = []; \
         g.first = gnoblin.requestTransition(() => { g.a.opacity = 0.5; g.a.x = 90; }, { duration: 400 }); \
         g.first.onFinished((r) => g.log.push('first:' + r.cancelled));",
    );
    settle(&mut rig);
    let mark = rig.comp.sent_count();
    exec(
        &mut rig,
        "g.second = gnoblin.requestTransition(() => { g.a.opacity = 0.2; }, { duration: 100 }); \
         g.second.onFinished((r) => g.log.push('second:' + r.cancelled));",
    );
    settle(&mut rig);
    assert_eq!(
        timed(&rig, mark),
        [(3, "opacity".to_owned())],
        "the newer one writes only its own property"
    );
    assert_eq!(
        rig.eval("String(g.first.running)"),
        "true",
        "the older one still owns x and goes on"
    );
    end_timed(&rig, mark, 3, "opacity", true);
    settle(&mut rig);
    assert_eq!(rig.eval("g.log.join()"), "second:false");
    end_timed(&rig, 0, 3, "x", true);
    settle(&mut rig);
    assert_eq!(rig.eval("g.log.join()"), "second:false,first:false");
    // A transition that loses every property is over at once, cancelled.
    exec(
        &mut rig,
        "g.log = []; \
         g.third = gnoblin.requestTransition(() => { g.b.opacity = 0.5; }, { duration: 400 }); \
         g.third.onFinished((r) => g.log.push('third:' + r.cancelled)); \
         g.fourth = gnoblin.requestTransition(() => { g.b.opacity = 0.9; }, { duration: 400 });",
    );
    settle(&mut rig);
    assert_eq!(rig.eval("g.log.join()"), "third:true");
    // A plain write ends the owner's track too.
    exec(&mut rig, "g.b.opacity = 0.3;");
    settle(&mut rig);
    assert_eq!(rig.eval("String(g.fourth.cancelled)"), "true");
}

#[test]
fn finished_resolves_once_with_cancelled_and_cancel_lands_the_end_values_at_once() {
    // AN6 (the part that does not need scrubbing): "cancel from several states; promise result {cancelled}".
    let mut rig = two("new Layout()");
    exec(
        &mut rig,
        "g.log = []; \
         g.t = gnoblin.requestTransition(() => { g.a.opacity = 0.5; g.a.x = 90; }, { duration: 500 }); \
         g.t.finished.then((r) => g.log.push('promise:' + r.cancelled)); \
         g.t.onFinished((r) => g.log.push('callback:' + r.cancelled)); \
         g.u = gnoblin.requestTransition(() => { g.b.opacity = 0.5; }, { duration: 200 }); \
         g.u.finished.then((r) => g.log.push('u:' + r.cancelled));",
    );
    settle(&mut rig);
    assert_eq!(rig.eval("String(g.t.running)"), "true");
    assert_eq!(rig.eval("g.log.join()"), "", "nothing before the time");
    end_timed(&rig, 0, 4, "opacity", true);
    settle(&mut rig);
    assert_eq!(rig.eval("g.log.join()"), "u:false", "the shorter one ends on its own");
    let mark = rig.comp.sent_count();
    exec(&mut rig, "g.t.cancel(); g.t.cancel();");
    settle(&mut rig);
    assert_eq!(
        plain(&rig, mark),
        [(3, "opacity".to_owned()), (3, "x".to_owned())],
        "cancel writes the end values without a duration, which ends the compositor's tracks"
    );
    assert_eq!(timed(&rig, mark), Vec::<(u64, String)>::new());
    let log = "(g.log.slice(1).sort().join() + '|' + g.log[0])";
    assert_eq!(
        rig.eval(log),
        "callback:true,promise:true|u:false",
        "settled once, cancelled"
    );
    // The compositor's late word on the tracks the cancel already took changes nothing.
    end_timed(&rig, 0, 3, "opacity", false);
    end_timed(&rig, 0, 3, "x", false);
    settle(&mut rig);
    assert_eq!(
        rig.eval(log),
        "callback:true,promise:true|u:false",
        "the timer does not settle it again"
    );
    // A callback added after the end runs once, after the current call.
    exec(&mut rig, "g.t.onFinished((r) => g.log.push('late:' + r.cancelled));");
    settle(&mut rig);
    assert_eq!(rig.eval("g.log.length + ':' + g.log.at(-1)"), "4:late:true");
}

#[test]
fn set_is_one_transition_of_one_object_and_refuses_what_is_not_a_property() {
    let mut rig = two("new DwindleLayout()");
    let mark = rig.comp.sent_count();
    exec(
        &mut rig,
        "g.t = g.a.set({ opacity: 0.92, rotation: [0, 0.12, 0] }, { duration: 250, easing: 'ease-out' });",
    );
    settle(&mut rig);
    assert_eq!(
        timed(&rig, mark),
        [(3, "opacity".to_owned()), (3, "rotation".to_owned())]
    );
    assert_eq!(rig.comp.sent_count() - mark, 1);
    assert_eq!(
        thrown(
            &mut rig,
            "g.a.set({ transform: { rotation: { x: 0, y: 0.12, z: 0 } } }, { duration: 250 })"
        ),
        "TypeError: Client#3.set: transform is not a writable property"
    );
    assert_eq!(
        thrown(&mut rig, "g.a.set({ title: 'x' })"),
        "TypeError: Client#3.set: title is not a writable property"
    );
    // Without options the change is atomic and immediate.
    let mark = rig.comp.sent_count();
    exec(&mut rig, "g.a.set({ opacity: 0.4, z: 3 });");
    settle(&mut rig);
    assert_eq!(timed(&rig, mark), Vec::<(u64, String)>::new());
    assert_eq!(plain(&rig, mark), [(3, "opacity".to_owned()), (3, "z".to_owned())]);
    // A workspace's layout is a property too, and the windows tween to the new layout.
    let mark = rig.comp.sent_count();
    exec(&mut rig, "g.ws.set({ layout: new g.Half() }, { duration: 300 });");
    settle(&mut rig);
    assert!(!timed(&rig, mark).is_empty());
    assert_eq!(plain(&rig, mark), Vec::<(u64, String)>::new());
    assert_eq!(rig.eval("g.ws.layout instanceof g.Half ? 'half' : 'other'"), "half");
}

/// The `tween` node the shell made after `mark`, and the plain writes it made to it: `(key, value)`.
fn tween_writes(rig: &Rig, mark: usize) -> (u64, Vec<(String, MirrorValue)>) {
    let ops = rig.comp.ops_since(mark);
    let tween = ops
        .iter()
        .find_map(|op| match op {
            TxOp::New { id, class } if class == "tween" => Some(*id),
            _ => None,
        })
        .expect("a transition with tracks makes a tween node");
    let writes = ops
        .iter()
        .filter_map(|op| match op {
            TxOp::Set {
                node,
                key,
                value,
                seconds: None,
                ..
            } if *node == tween => Some((key.clone(), value.clone())),
            _ => None,
        })
        .collect();
    (tween, writes)
}

#[test]
fn a_transition_with_tracks_joins_a_tween_node_and_ends_when_the_compositor_says_so() {
    let mut rig = two("new Layout()");
    let mark = rig.comp.sent_count();
    exec(
        &mut rig,
        "g.log = []; g.t = gnoblin.requestTransition(() => { g.a.opacity = 0.5; g.a.x = 70; }, { duration: 200, delay: 100 }); \
         g.t.finished.then((r) => g.log.push('done:' + r.cancelled));",
    );
    settle(&mut rig);
    let (tween, writes) = tween_writes(&rig, mark);
    assert!(
        writes.is_empty(),
        "autoplay leaves the tween at its defaults: {writes:?}"
    );
    let delays: Vec<_> = rig
        .comp
        .ops_since(mark)
        .into_iter()
        .filter_map(|op| match op {
            TxOp::Set {
                seconds: Some(_),
                delay,
                tween: joined,
                ..
            } => Some((delay, joined)),
            _ => None,
        })
        .collect();
    assert_eq!(
        delays,
        [(Some(0.1), Some(tween)), (Some(0.1), Some(tween))],
        "delay and group"
    );
    // The shell's own clock does not end it, however long passes.
    rig.advance(5000);
    settle(&mut rig);
    assert_eq!(rig.eval("g.log.join()"), "");
    end_timed(&rig, mark, 3, "opacity", true);
    settle(&mut rig);
    assert_eq!(rig.eval("g.log.join()"), "", "one track is still running");
    end_timed(&rig, mark, 3, "x", true);
    settle(&mut rig);
    assert_eq!(rig.eval("g.log.join()"), "done:false");
    let destroyed = rig
        .comp
        .ops_since(mark)
        .into_iter()
        .any(|op| matches!(op, TxOp::Destroy { node } if node == tween));
    assert!(destroyed, "the tween node goes with the transition");
}

#[test]
fn autoplay_false_holds_the_tracks_and_progress_play_and_reverse_write_the_tween() {
    let mut rig = two("new Layout()");
    let x0 = rig.comp.number(3, "x");
    let mark = rig.comp.sent_count();
    exec(
        &mut rig,
        "g.log = []; g.calls = 0; \
         g.t = gnoblin.requestTransition(() => { g.a.x = x0 + 50; }, { duration: 800, easing: 'in-out', autoplay: false }); \
         g.t.finished.then((r) => g.log.push('done:' + r.cancelled));"
            .replace("x0", &x0.to_string())
            .as_str(),
    );
    settle(&mut rig);
    let (_, held) = tween_writes(&rig, mark);
    assert_eq!(held, [("paused".to_owned(), MirrorValue::Bool(true))]);
    assert_eq!(rig.eval("String(g.t.paused)"), "true");
    let dispatches = rig.shell.dispatches();
    for (value, expected) in [(0.4, 0.4), (0.1, 0.1), (1.0, 1.0)] {
        let mark = rig.comp.sent_count();
        exec(&mut rig, &format!("g.t.progress = {value};"));
        settle(&mut rig);
        let (_, writes) = (
            0,
            rig.comp
                .ops_since(mark)
                .into_iter()
                .filter_map(|op| match op {
                    TxOp::Set { key, value, .. } if key == "progress" => Some(value),
                    _ => None,
                })
                .collect::<Vec<_>>(),
        );
        assert_eq!(writes, [MirrorValue::Double(expected)], "one write per scrub");
    }
    assert_eq!(
        rig.shell.dispatches(),
        dispatches,
        "a scrub runs no handler of the script's"
    );
    assert_eq!(
        thrown(&mut rig, "g.t.progress = 1.5"),
        "RangeError: Transition.progress: expects a number from 0 to 1, not 1.5"
    );
    let mark = rig.comp.sent_count();
    exec(&mut rig, "g.t.reverse();");
    settle(&mut rig);
    let writes: Vec<_> = rig
        .comp
        .ops_since(mark)
        .into_iter()
        .filter_map(|op| match op {
            TxOp::Set { key, value, .. } => Some((key, value)),
            _ => None,
        })
        .collect();
    assert_eq!(
        writes,
        [
            ("reverse".to_owned(), MirrorValue::Bool(true)),
            ("paused".to_owned(), MirrorValue::Bool(false))
        ]
    );
    // The compositor runs it back and says it got to the start: cancelled, and the start value is back.
    end_timed(&rig, 0, 3, "x", false);
    let mark = rig.comp.sent_count();
    settle(&mut rig);
    assert_eq!(rig.eval("g.log.join()"), "done:true");
    assert_eq!(rig.comp.number(3, "x"), x0, "the start value is put back");
    assert!(plain(&rig, mark).contains(&(3, "x".to_owned())));
}
