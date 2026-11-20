//! The parts of the stock shell above the toolkit: the not-responding alert, the launcher, the overview, the lock screen.

use crate::scn::{At, Home, LEFT, Scn, V, World};

fn world(windows: u64) -> World {
    World::desktop(1000.0, 800.0).windows(windows)
}

const FAST_ALERTS: &str = "startDecorations(); startWindowMenu(); startNotRespondingAlert();\n\
    notRespondingAlerts.armMs = 1000; notRespondingAlerts.intervalMs = 0;\n\
    globalThis.cardOpen = () => { const c = notRespondingAlerts.cardOf(gnoblin.clients.at(0)); return c ? [c.index, c.armed, c.secure] : null; };\n";

const CARD: At = At::Js("notRespondingAlerts.cardOf(gnoblin.clients.at(0)).node");
const FORCE: At = At::Js("notRespondingAlerts.cardOf(gnoblin.clients.at(0)).choices[1].node");

/// The lock screen shows the time of day from `Date.now()` in the machine's zone: a scenario fixes both, so what it paints is the same on every
/// machine.
const FIXED_TIME: &str = "globalThis.fakeNow = 1700000000000; Date.now = () => globalThis.fakeNow;\n\
    Date.prototype.getHours = Date.prototype.getUTCHours; Date.prototype.getMinutes = Date.prototype.getUTCMinutes;\n";

fn stuck(s: &mut Scn, node: u64, on: bool, label: &str) {
    s.set(node, "unresponsive", V::Bool(on))
        .eval("rec('card', cardOpen());")
        .step(label);
}

fn dialog_key(s: &mut Scn, key: &str, label: &str) {
    s.key(CARD, key, "", true).eval("rec('card', cardOpen());").step(label);
}

#[test]
fn not_responding_alert() {
    let mut s = Scn::new("parts_alert", Home::Reference, &world(1), FAST_ALERTS);
    s.boot();
    s.eval("rec('card', cardOpen());").step("responsive");
    stuck(&mut s, 3, true, "unresponsive");
    s.answer_measures().step("measured");
    s.tree("alert", CARD);
    s.paint("disarmed", CARD);
    s.eval("rec('focus', gnoblin.clients.at(0).focused);")
        .step("focus_is_on_the_card");
    dialog_key(&mut s, "Tab", "tab_to_force_quit");
    dialog_key(&mut s, "Return", "force_quit_while_disarmed");
    s.paint("tab_disarmed", CARD);
    s.advance(1200).eval("rec('card', cardOpen());").step("armed");
    s.answer_measures().step("measured_armed");
    s.paint("armed", CARD);
    dialog_key(&mut s, "Tab", "back_to_wait");
    dialog_key(&mut s, "Right", "right_to_force");
    dialog_key(&mut s, "Return", "force_quit");
    s.step("after_force_quit");
    s.finish();

    let mut s = Scn::new("parts_alert_wait", Home::Reference, &world(2), FAST_ALERTS);
    s.boot();
    stuck(&mut s, 3, true, "unresponsive");
    dialog_key(&mut s, "Escape", "escape_waits");
    s.settle_frames()
        .eval("rec('card', cardOpen());")
        .step("no_second_alert");
    stuck(&mut s, 3, false, "answered");
    stuck(&mut s, 3, true, "again_unresponsive");
    s.pointer(CARD, "press", 5.0, 5.0, LEFT).step("press_on_the_card");
    s.advance(1200).step("armed");
    s.pointer(FORCE, "press", 5.0, 5.0, LEFT).step("press_force_quit");
    s.finish();

    let mut s = Scn::new("parts_alert_closes", Home::Reference, &world(2), FAST_ALERTS);
    s.boot();
    stuck(&mut s, 3, true, "first_unresponsive");
    stuck(&mut s, 3, false, "first_answers");
    stuck(&mut s, 4, true, "second_unresponsive");
    s.destroy(4)
        .eval("rec('canvases', uiCanvasCount());")
        .step("second_gone");
    s.set(22, "state", V::Text("locked")).step("locked");
    stuck(&mut s, 3, true, "unresponsive_while_locked");
    s.set(22, "state", V::Text("not-locked"))
        .eval("rec('card', cardOpen());")
        .step("unlocked");
    s.set(22, "state", V::Text("locked"))
        .eval("rec('card', cardOpen());")
        .step("locked_again");
    s.finish();

    let mut s = Scn::new("parts_alert_secure", Home::Reference, &world(1), FAST_ALERTS);
    s.boot();
    s.set(3, "secure", V::Bool(true))
        .set(3, "title", V::Text("a\nb"))
        .step("secure_title_with_a_newline");
    stuck(&mut s, 3, true, "unresponsive");
    s.answer_measures().step("measured");
    s.tree("alert", CARD);
    s.paint("secure", CARD);
    s.finish();

    let mut s = Scn::new("parts_alert_menu", Home::Reference, &world(1), FAST_ALERTS);
    s.boot();
    s.request(At::Id(3), "menu", 10.0, 10.0)
        .eval("rec('menu', windowMenu.current !== null);")
        .step("menu_open");
    stuck(&mut s, 3, true, "unresponsive");
    s.eval("rec('menu', windowMenu.current !== null);")
        .step("the_alert_closed_the_menu");
    s.finish();

    let mut s = Scn::new("parts_alert_tracker", Home::Reference, &world(1), "export {};\n");
    s.boot();
    s.eval(
        "const w = { alive: true, unresponsive: false, title: 't', kill() {} }; const t = new AlertTracker(); const out = [];\n\
         const look = (locked = false) => { const c = t.check([w], locked); out.push(`${c.open.length}${c.close.length}`); };\n\
         look(); w.unresponsive = true; look(); look();\n\
         out.push(String(t.answered(w, 'wait'))); look(); w.unresponsive = false; look(); w.unresponsive = true; look();\n\
         out.push(String(t.answered(w, 'force'))); look(true); w.unresponsive = false; look(); w.unresponsive = true; look(true); look();\n\
         rec('tracker', out);\n\
         rec('clean', [cleanText('a\\nb', 80), cleanText('x'.repeat(70), 60).length, cleanText('\\ud800', 10), cleanText('tab\\there', 80), cleanText('caf\\u00e9 \\u2713', 80)]);",
    )
    .step("tracker_and_text");
    s.finish();
}

#[test]
fn launcher() {
    let mut s = Scn::new(
        "parts_launcher",
        Home::Reference,
        &world(1),
        "globalThis.spawned = [];\n\
         globalThis.open = () => openLauncher({ spawn: (c) => globalThis.spawned.push(c) });\n",
    );
    let launcher = At::Js("l.node");
    s.boot();
    s.eval("globalThis.l = open();")
        .answer_measures()
        .eval("rec('rect', [l.x, l.y, l.width, l.height]); rec('again', open() === l);")
        .step("opened");
    s.tree("launcher", launcher);
    s.paint("empty", launcher);
    for ch in ["f", "o", "o"] {
        s.key(launcher, ch, ch, true);
    }
    s.answer_measures().step("typed");
    s.paint("typed", launcher);
    s.key(launcher, "Return", "", true)
        .eval("rec('spawned', globalThis.spawned); rec('alive', l.alive);")
        .step("submitted");
    s.eval("globalThis.l = open();").step("reopened");
    s.key(launcher, "Escape", "", true)
        .eval("rec('spawned', globalThis.spawned); rec('alive', l.alive);")
        .step("escaped");
    s.eval("globalThis.l = open();")
        .key(launcher, " ", " ", true)
        .key(launcher, "Return", "", true)
        .eval("rec('spawned', globalThis.spawned); rec('alive', l.alive);")
        .step("blank_runs_nothing");
    s.finish();
}

#[test]
fn overview() {
    let mut s = Scn::new(
        "parts_overview",
        Home::Reference,
        &world(2),
        "for (const id of ['a', 'b']) gnoblin.workspaces.add(new Workspace({ id }));\n\
         globalThis.opacities = () => gnoblin.clients.map((c) => c.opacity);\n",
    );
    s.boot();
    s.eval("gnoblin.workspaces.at(gnoblin.workspaces.length - 1).clients.add(gnoblin.clients.at(gnoblin.clients.length - 1));")
        .step("second_window_moves_to_the_last_workspace");
    s.eval("rec('on', overview.toggle()); rec('opacity', opacities());")
        .step("on");
    s.eval("rec('on', overview.toggle()); rec('opacity', opacities());")
        .step("off");
    s.eval("rec('on', overview.toggle()); rec('on', overview.toggle()); rec('opacity', opacities());")
        .step("toggled_twice");
    s.finish();
}

#[test]
fn lock_screen() {
    let mut s = Scn::new("parts_lock", Home::Reference, &world(1), FIXED_TIME);
    s.boot();
    s.set(3, "focused", V::Bool(true)).step("window_focused");
    s.set(22, "state", V::Text("pending")).step("lock_requested");
    s.answer_measures().step("measured");
    s.tree("lock", At::Id(22));
    s.paint("hint", At::Id(22));
    s.type_secret("hunter2").answer_measures().step("typed");
    s.paint("typed", At::Id(22));
    s.tap(At::Focus, "Return", "")
        .answer_check("", "denied")
        .answer_measures()
        .step("denied");
    s.paint("denied", At::Id(22));
    s.type_secret("again")
        .tap(At::Focus, "BackSpace", "")
        .tap(At::Focus, "Return", "")
        .answer_check("1500", "rate-limited")
        .answer_measures()
        .step("rate_limited");
    s.paint("rate_limited", At::Id(22));
    s.type_secret("b")
        .tap(At::Focus, "Return", "")
        .answer_check("", "busy")
        .answer_measures()
        .step("busy");
    s.paint("busy", At::Id(22));
    s.type_secret("c")
        .tap(At::Focus, "Return", "")
        .answer_check("", "failed: no PAM")
        .answer_measures()
        .step("failed");
    s.paint("failed", At::Id(22));
    s.tap(At::Focus, "Return", "").step("empty_return_is_not_an_attempt");
    s.type_secret("right").tap(At::Focus, "Return", "").step("checking");
    s.paint("checking", At::Id(22));
    s.tap(At::Focus, "Return", "").step("return_while_checking");
    s.answer_check("ok", "").step("accepted");
    s.tree("lock", At::Id(22));
    s.set(22, "state", V::Text("not-locked")).step("unlocked");
    s.tree("lock", At::Id(22));
    s.finish();

    let mut s = Scn::new("parts_lock_relock", Home::Reference, &world(1), FIXED_TIME);
    s.boot();
    s.set(22, "state", V::Text("locked")).step("locked");
    s.set(22, "state", V::Text("not-locked")).step("unlocked_elsewhere");
    s.tree("lock", At::Id(22));
    s.set(22, "state", V::Text("locked")).step("locked_again");
    s.tree("lock", At::Id(22));
    s.finish();
}
