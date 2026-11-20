//! The stock window menu (`gnoblin-std/menus/*`): the card and its entries, where it opens, the keyboard navigator, the pointer, the
//! workspace submenu, and what each entry does.

use crate::scn::{At, Home, LEFT, RIGHT, Scn, V, World};

fn world(windows: u64) -> World {
    World::desktop(1000.0, 800.0).windows(windows)
}

/// Titlebars and the window menu the compositor's menu requests are answered by, and what a scenario reads of the open menu.
const MENU: &str = "startDecorations(); startWindowMenu();\n\
    globalThis.sel = () => windowMenu.current ? [windowMenu.current.nav.depth, windowMenu.current.nav.selected()?.label ?? null] : null;\n\
    globalThis.card = () => { const m = windowMenu.current; if (!m) return null; const p = m.panel;\n\
        return { x: p.x, y: p.y, width: p.width, height: p.height, secure: [p.secure, m.secure], rows: p.rows.map((r) => [r.label, r.enabled, r.checked]) }; };\n\
    globalThis.row = (label) => windowMenu.current.panel.rows.find((r) => r.label === label).node;\n";

const LAYER: At = At::Js("windowMenu.current.node");
const PANEL: At = At::Js("windowMenu.current.panel.node");

fn key(s: &mut Scn, name: &str) {
    s.key(LAYER, name, "", true)
        .eval("rec('sel', sel());")
        .step(&format!("key_{name}"));
}

fn ask(s: &mut Scn, node: u64, x: f64, y: f64, label: &str) {
    s.request(At::Id(node), "menu", x, y)
        .eval("rec('card', card()); rec('sel', sel());")
        .step(label);
}

#[test]
fn card_geometry_and_paint() {
    let mut s = Scn::new("menus_card", Home::Reference, &world(1), &format!("{MENU}\n"));
    s.boot();
    ask(&mut s, 3, 300.0, 200.0, "open_at_anchor");
    s.answer_measures().step("measured");
    s.tree("menu", LAYER);
    s.paint("floating", LAYER);
    ask(&mut s, 3, 990.0, 790.0, "open_near_the_corner");
    s.answer_measures().step("measured_again");
    s.paint("near_corner", LAYER);
    ask(&mut s, 3, 10.0, 10.0, "open_top_left");
    ask(&mut s, 3, -50.0, -50.0, "open_off_screen");
    s.set(3, "maximised", V::Bool(true)).step("maximised");
    ask(&mut s, 3, 50.0, 50.0, "maximised_has_restore");
    s.answer_measures().step("measured_maximised");
    s.paint("maximised", LAYER);
    s.set(3, "maximised", V::Bool(false))
        .set(3, "fullscreen", V::Bool(true))
        .step("fullscreen");
    ask(&mut s, 3, 50.0, 50.0, "fullscreen_has_restore_and_no_workspace_move");
    s.answer_measures().step("measured_fullscreen");
    s.paint("fullscreen", LAYER);
    s.set(3, "fullscreen", V::Bool(false))
        .set(3, "secure", V::Bool(true))
        .step("secure");
    ask(&mut s, 3, 50.0, 50.0, "secure_card");
    s.tree("secure", LAYER);
    s.finish();
}

#[test]
fn keyboard_navigation() {
    let mut s = Scn::new("menus_keys", Home::Reference, &world(1), &format!("{MENU}\n"));
    s.boot();
    ask(&mut s, 3, 50.0, 50.0, "open");
    for name in [
        "Down",
        "Down",
        "Tab",
        "Up",
        "ISO_Left_Tab",
        "Home",
        "End",
        "Down",
        "Up",
        "Up",
        "F5",
        "space",
        "BackSpace",
    ] {
        key(&mut s, name);
    }
    s.key(LAYER, "c", "c", true).eval("rec('sel', sel());").step("letter_c");
    s.key(LAYER, "m", "m", true).eval("rec('sel', sel());").step("letter_m");
    s.key(LAYER, "m", "m", true)
        .eval("rec('sel', sel());")
        .step("letter_m_again");
    s.key(LAYER, "Escape", "", false)
        .eval("rec('sel', sel());")
        .step("escape_released");
    key(&mut s, "Escape");
    ask(&mut s, 3, 50.0, 50.0, "open_again");
    key(&mut s, "Down");
    key(&mut s, "Return");
    s.eval("rec('maximised', gnoblin.clients.at(0).maximised);")
        .step("maximised_by_return");
    s.eval(
        "rec('actions', ['Escape','Down','Tab','Up','ISO_Left_Tab','Home','End','Return','KP_Enter','space','Right','Left','F5'].map((k) => menuKeyAction(k, '')));\n\
         rec('text_keys', [menuKeyAction('c', 'c'), menuKeyAction('x', ' '), menuKeyAction('ab', 'ab')]);",
    )
    .step("key_actions");
    s.eval(
        "const rows = ['Maximise','Minimise','Move','Resize','Always on top','Move to workspace','Close'].map((label) => ({ label, enabled: true, submenu: false }));\n\
         const nav = new MenuNavigator(rows); const at = () => nav.selected()?.label; const out = [];\n\
         nav.move(-1); out.push(at()); nav.home(); out.push(at()); nav.end(); out.push(at()); nav.move(1); out.push(at()); nav.move(-1); out.push(at());\n\
         nav.jump('c'); out.push(at()); nav.jump('m'); out.push(at()); nav.jump('m'); out.push(at());\n\
         rec('navigator', out);\n\
         rec('placement', [placeCard({ x: 300, y: 200 }, { width: 170, height: 220 }, { width: 1000, height: 800 }),\n\
           placeCard({ x: 990, y: 790 }, { width: 170, height: 220 }, { width: 1000, height: 800 }),\n\
           placeCard({ x: -5, y: -5 }, { width: 170, height: 220 }, { width: 1000, height: 800 })]);\n\
         rec('height', [menuHeight([{ label: 'a' }, { separator: true }, { label: 'b' }]), menuHeight([])]);",
    )
    .step("navigator_and_metrics");
    s.finish();
}

#[test]
fn pointer_selection_and_press() {
    let mut s = Scn::new("menus_pointer", Home::Reference, &world(1), &format!("{MENU}\n"));
    s.boot();
    s.set(3, "maximised", V::Bool(true)).step("maximised");
    ask(&mut s, 3, 50.0, 50.0, "open");
    s.pointer(At::Js("row('Minimise')"), "enter", 4.0, 4.0, 0)
        .eval("rec('sel', sel());")
        .step("enter_minimise");
    s.pointer(At::Js("row('Move')"), "enter", 4.0, 4.0, 0)
        .eval("rec('sel', sel());")
        .step("enter_disabled_move");
    s.pointer(At::Js("row('Move')"), "press", 4.0, 4.0, LEFT)
        .eval("rec('sel', sel());")
        .step("press_disabled_move");
    s.pointer(PANEL, "press", 1.0, 1.0, LEFT)
        .eval("rec('sel', sel());")
        .step("press_padding");
    s.pointer(At::Js("row('Close')"), "press", 5.0, 5.0, RIGHT)
        .eval("rec('sel', sel());")
        .step("right_press_entry");
    s.pointer(LAYER, "press", 5.0, 5.0, LEFT)
        .eval("rec('sel', sel());")
        .step("press_outside");
    ask(&mut s, 3, 50.0, 50.0, "open_again");
    s.pointer(LAYER, "press", 5.0, 5.0, RIGHT)
        .eval("rec('sel', sel());")
        .step("right_press_outside");
    ask(&mut s, 3, 50.0, 50.0, "open_third");
    s.pointer(At::Js("row('Restore')"), "press", 5.0, 5.0, LEFT)
        .eval("rec('maximised', gnoblin.clients.at(0).maximised); rec('sel', sel());")
        .step("press_restore");
    s.finish();
}

#[test]
fn workspace_submenu() {
    let mut s = Scn::new(
        "menus_submenu",
        Home::Reference,
        &world(1),
        &format!("{MENU}\nfor (const id of ['1', '2', '3']) gnoblin.workspaces.add(new Workspace({{ id }}));\n"),
    );
    s.boot();
    ask(&mut s, 3, 50.0, 50.0, "open");
    key(&mut s, "End");
    key(&mut s, "Up");
    key(&mut s, "Right");
    s.eval("const n = windowMenu.current.nav; rec('submenu', { depth: n.depth, rows: n.subRows.map((r) => [r.label, r.enabled, r.checked]), x: windowMenu.current.sub.x, panel: [windowMenu.current.panel.x, windowMenu.current.panel.width] });")
        .answer_measures()
        .step("submenu_state");
    s.tree("submenu", LAYER);
    s.paint("submenu", LAYER);
    key(&mut s, "Left");
    key(&mut s, "Right");
    key(&mut s, "Escape");
    s.eval("rec('depth', windowMenu.current.nav.depth);")
        .step("escape_in_submenu_goes_back");
    key(&mut s, "Right");
    key(&mut s, "Return");
    s.eval("rec('workspace', gnoblin.clients.at(0).workspace.id);")
        .step("moved_to_workspace_2");
    ask(&mut s, 3, 50.0, 50.0, "open_on_the_new_workspace");
    s.finish();
}

#[test]
fn entries_run() {
    let mut s = Scn::new("menus_actions", Home::Reference, &world(2), &format!("{MENU}\n"));
    s.boot();
    ask(&mut s, 3, 50.0, 50.0, "open");
    s.eval("rec('above', windowMenu.current.panel.rows[4].checked);")
        .step("not_pinned");
    key(&mut s, "End");
    key(&mut s, "Up");
    key(&mut s, "Up");
    key(&mut s, "Return");
    s.eval("rec('above', windowMenu.isAbove(gnoblin.clients.at(0)));")
        .step("pinned");
    s.eval("gnoblin.clients.at(gnoblin.clients.length - 1).raise();")
        .settle_frames()
        .step("raise_other_window");
    ask(&mut s, 3, 50.0, 50.0, "open_pinned");
    s.answer_measures().step("measured");
    s.paint("pinned_tick", LAYER);
    key(&mut s, "Escape");
    s.set(3, "maximised", V::Bool(true)).step("maximised");
    ask(&mut s, 3, 50.0, 50.0, "open_maximised");
    key(&mut s, "Down");
    key(&mut s, "Return");
    s.eval("rec('maximised', gnoblin.clients.at(0).maximised);")
        .step("restored");
    ask(&mut s, 3, 50.0, 50.0, "open_for_minimise");
    key(&mut s, "Down");
    key(&mut s, "Down");
    key(&mut s, "Return");
    s.eval("rec('minimised', gnoblin.clients.at(0).minimised);")
        .step("minimised");
    s.set(3, "minimised", V::Bool(false)).step("unminimised");
    ask(&mut s, 3, 50.0, 50.0, "open_for_close");
    key(&mut s, "End");
    key(&mut s, "Return");
    s.step("closed");
    s.finish();
}

#[test]
fn lifecycle_and_scripts() {
    let mut s = Scn::new("menus_lifecycle", Home::Reference, &world(2), &format!("{MENU}\n"));
    s.boot();
    ask(&mut s, 3, 50.0, 50.0, "open_first");
    ask(&mut s, 4, 400.0, 300.0, "open_second");
    s.eval("rec('canvases', uiCanvasCount());")
        .step("two_menus_do_not_stack");
    ask(&mut s, 4, 10.0, 10.0, "ask_again");
    s.eval("rec('canvases', uiCanvasCount());").step("same_count");
    s.destroy(3)
        .eval("rec('open', windowMenu.current !== null);")
        .step("other_window_closed");
    s.destroy(4)
        .eval("rec('open', windowMenu.current !== null);")
        .step("own_window_closed");
    s.finish();

    let mut s = Scn::new("menus_secure_lock", Home::Reference, &world(2), &format!("{MENU}\n"));
    s.boot();
    s.set(3, "secure", V::Bool(true)).step("secure");
    ask(&mut s, 3, 50.0, 50.0, "open_secure");
    ask(&mut s, 4, 50.0, 50.0, "open_plain");
    s.set(22, "state", V::Text("pending"))
        .eval("rec('open', windowMenu.current !== null);")
        .step("locking_closes");
    ask(&mut s, 4, 50.0, 50.0, "open_while_locked");
    s.set(22, "state", V::Text("not-locked"))
        .set(4, "minimised", V::Bool(true))
        .step("unlocked_and_minimised");
    ask(&mut s, 4, 50.0, 50.0, "open_minimised");
    s.finish();

    let mut s = Scn::new(
        "menus_scripted",
        Home::Reference,
        &world(1),
        &format!(
            "{MENU}\nglobalThis.ran = [];\n\
             windowMenu.items = () => [{{ label: 'Only', run: (w) => globalThis.ran.push(w.title) }}, {{ separator: true }}, {{ label: 'Off', enabled: false }}];\n"
        ),
    );
    s.boot();
    ask(&mut s, 3, 50.0, 50.0, "open");
    s.answer_measures().step("measured");
    s.paint("scripted", LAYER);
    key(&mut s, "Down");
    key(&mut s, "Return");
    s.eval("rec('ran', globalThis.ran);").step("ran");
    s.eval(
        "globalThis.seen = null; windowMenu.show = (c, x, y) => { globalThis.seen = [c.title, x, y]; return null; };",
    )
    .step("show_replaced");
    ask(&mut s, 3, 70.0, 80.0, "ask_show");
    s.eval("rec('seen', globalThis.seen);").step("show_seen");
    s.finish();

    let mut s = Scn::new(
        "menus_throwing_entry",
        Home::Reference,
        &world(1),
        &format!(
            "{MENU}\nwindowMenu.items = () => [{{ label: 'Boom', run: () => {{ throw new Error('boom'); }} }}];\n"
        ),
    );
    s.boot();
    ask(&mut s, 3, 50.0, 50.0, "open");
    key(&mut s, "Down");
    key(&mut s, "Return");
    s.eval("rec('open', windowMenu.current !== null);")
        .step("closed_before_the_entry_ran");
    s.finish();

    let mut s = Scn::new("menus_at_the_pointer", Home::Reference, &world(1), &format!("{MENU}\n"));
    s.boot();
    s.seat("pointer_x", V::Num(220.0))
        .seat("pointer_y", V::Num(140.0))
        .step("pointer_moved");
    s.eval("windowMenu.show(gnoblin.clients.at(0));")
        .eval("rec('card', card());")
        .step("shown_at_the_pointer");
    key(&mut s, "Escape");
    let bar = At::Under(3, &[0, 0]);
    s.pointer(bar, "press", 5.0, 5.0, crate::scn::MIDDLE)
        .eval("rec('open', windowMenu.current !== null);")
        .step("middle_click");
    s.pointer(bar, "press", 5.0, 5.0, RIGHT)
        .eval("rec('card', card());")
        .step("right_click_opens");
    key(&mut s, "Escape");
    s.seat("right_click", V::Text("none"))
        .pointer(bar, "press", 5.0, 5.0, RIGHT)
        .eval("rec('open', windowMenu.current !== null);")
        .step("right_click_none");
    s.finish();

    let mut s = Scn::new("menus_not_started", Home::Reference, &world(1), "startDecorations();\n");
    s.boot();
    s.request(At::Id(3), "menu", 50.0, 50.0)
        .eval("rec('started', windowMenu.started);")
        .step("asked_without_a_menu");
    s.finish();
}
