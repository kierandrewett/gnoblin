//! The stock window menu, not-responding alert, entry, bar, clock, toast, overview and launcher
//! (`data/js/gnoblin/{menus,alerts,widgets,panel,overview,launcher}.ts`) and the titlebar's gestures and reserved room, over the
//! fake compositor of `tests/common`.
//!
//! The cases are the Lua shell's (`crates/gnoblin-lua/tests/lua/{menus,alerts,ui}_test.lua`), with the same inputs and the same
//! expected values; where the JavaScript shell differs the test says why. A key goes to the node that holds the seat's
//! focus, a press to the node it lands on, as the real path delivers them.

use gnoblin_tree_client::tree_mirror::MirrorValue;

mod common;
use common::*;

const RIGHT: u64 = 273;
const MIDDLE: u64 = 274;

const IMPORTS: &str = "import * as g from \"gnoblin\";\nconst { gnoblin: _runtime, ...exported } = g;\nObject.assign(globalThis, exported);\n";

/// A shell over a screen of 1000 by 800, one seat, one lock and `windows` windows (ids 3 on, titled `win1`...), whose
/// `init.ts` is `body`.
fn world(windows: u64, body: &str) -> Rig {
    let mut rig = rig_setup(IMPORTS, body, move |comp| {
        comp.add_node(
            20,
            ROOT,
            "screen",
            vec![
                ("width", MirrorValue::Double(1000.0)),
                ("height", MirrorValue::Double(800.0)),
                ("primary", MirrorValue::Bool(true)),
            ],
        );
        comp.add_node(21, ROOT, "seat", vec![("name", text("seat0"))]);
        comp.add_node(22, ROOT, "lock", Vec::new());
        for n in 0..windows {
            comp.add_window(3 + n, &format!("win{}", n + 1), "foot");
        }
    });
    let diagnostics = rig.settle();
    assert!(diagnostics.is_empty(), "{diagnostics:?} {:?}", rig.load_diagnostics);
    rig
}

fn number(rig: &mut Rig, expression: &str) -> f64 {
    rig.eval(&format!("String({expression})")).parse().expect("a number")
}

/// The node of the open menu's layer.
fn layer(rig: &mut Rig) -> u64 {
    number(rig, "windowMenu.current.node") as u64
}

/// The compositor asks for the menu of window `node` at `x`, `y`.
fn ask(rig: &mut Rig, node: u64, x: f64, y: f64) {
    rig.comp.request(node, "menu", x, y);
    rig.settle();
}

fn menu_open(rig: &mut Rig) -> bool {
    rig.eval("String(windowMenu.current !== null)") == "true"
}

fn press_key(rig: &mut Rig, key: &str) {
    let at = layer(rig);
    rig.comp.key(at, key, "", true);
    rig.settle();
}

fn selected(rig: &mut Rig) -> String {
    rig.eval("String(windowMenu.current.nav.selected()?.label)")
}

fn row_node(rig: &mut Rig, label: &str) -> u64 {
    number(
        rig,
        &format!("windowMenu.current.panel.rows.find((r) => r.label === '{label}').node"),
    ) as u64
}

fn labels(rig: &mut Rig) -> String {
    rig.eval("windowMenu.current.panel.rows.map((r) => r.label).join(',')")
}

fn calls_of(rig: &Rig, method: &str) -> usize {
    rig.comp.calls().iter().filter(|(_, m)| m == method).count()
}

// ---- the menu: the cases of menus_test.lua --------------------------------------------------------------------------

#[test]
fn a_menu_request_opens_the_window_menu_with_its_corner_at_the_anchor_and_the_keys_going_to_it() {
    let mut rig = world(1, "defineConfig({ decorations: true });");
    ask(&mut rig, 3, 300.0, 200.0);
    assert!(menu_open(&mut rig), "a menu request must open the menu");
    assert_eq!(
        rig.eval("String(windowMenu.current.client === gnoblin.clients.at(0))"),
        "true"
    );
    assert_eq!(
        rig.eval("`${windowMenu.current.panel.x},${windowMenu.current.panel.y}`"),
        "300,200"
    );
    let at = layer(&mut rig);
    assert_eq!(
        rig.comp.value(21, "focus"),
        Some(MirrorValue::Node(at)),
        "the keys go to the menu"
    );
    assert_eq!(
        labels(&mut rig),
        "Maximise,Minimise,Move,Resize,Always on top,Move to workspace,Close"
    );
}

#[test]
fn a_menu_near_the_screens_edge_is_moved_back_inside_it_and_is_as_wide_as_its_longest_label() {
    let mut rig = world(1, "defineConfig({ decorations: true });");
    ask(&mut rig, 3, 990.0, 790.0);
    let fits = rig.eval(
        "(() => { const p = windowMenu.current.panel; \
         return `${p.x + p.width <= 1000},${p.y + p.height <= 800},${p.x > 0 && p.y > 0}`; })()",
    );
    assert_eq!(
        fits, "true,true,true",
        "the card ends inside the screen and holds its whole card"
    );
    ask(&mut rig, 3, 10.0, 10.0);
    assert_eq!(
        rig.eval("String(windowMenu.current.panel.width >= 170)"),
        "true",
        "never narrower than the minimum"
    );
    assert_eq!(
        rig.eval(
            "String(windowMenu.current.panel.rows.every((r) => r.natural() + 8 <= windowMenu.current.panel.width))"
        ),
        "true"
    );
}

#[test]
fn down_and_return_run_maximise_the_window_maximises_the_menu_closes_and_the_keyboard_goes_back() {
    let mut rig = world(1, "defineConfig({ decorations: true });");
    ask(&mut rig, 3, 50.0, 50.0);
    press_key(&mut rig, "Down");
    assert_eq!(
        selected(&mut rig),
        "Maximise",
        "Down from nothing lands on the first entry"
    );
    press_key(&mut rig, "Return");
    assert_eq!(
        rig.comp.flag(3, "maximised"),
        Some(true),
        "Return runs the selected entry"
    );
    assert!(!menu_open(&mut rig), "running an entry closes the menu");
    assert_eq!(
        rig.comp.value(21, "focus"),
        Some(MirrorValue::Node(0)),
        "the keyboard goes back to nothing"
    );
}

#[test]
fn up_home_end_tab_and_a_letter_move_the_selection_as_the_lua_menu_does() {
    // The pure model: rows are labels with an enabled flag, in the order of the window menu.
    let mut rig = world(1, "export {};");
    let got = rig.eval(
        "(() => { const rows = ['Maximise','Minimise','Move','Resize','Always on top','Move to workspace','Close'] \
         .map((label) => ({ label, enabled: true, submenu: false })); const nav = new MenuNavigator(rows); \
         const at = () => nav.selected()?.label; const out = []; \
         nav.move(-1); out.push(at()); nav.home(); out.push(at()); nav.end(); out.push(at()); \
         nav.move(1); out.push(at()); nav.move(-1); out.push(at()); \
         nav.jump('c'); out.push(at()); nav.jump('m'); out.push(at()); nav.jump('m'); out.push(at()); \
         return out.join(','); })()",
    );
    assert_eq!(
        got, "Close,Maximise,Close,Maximise,Close,Close,Maximise,Minimise",
        "Up from nothing is the last, Home and End the ends, Tab wraps, Shift+Tab goes back, a letter jumps on and wraps"
    );
    let keys = rig.eval(
        "['Escape','Down','Tab','Up','ISO_Left_Tab','Home','End','Return','KP_Enter','space','Right','Left','F5'] \
         .map((k) => JSON.stringify(menuKeyAction(k, ''))).join(' ') + ' ' + JSON.stringify(menuKeyAction('c', 'c')) \
         + ' ' + JSON.stringify(menuKeyAction('x', ' ')) + ' ' + JSON.stringify(menuKeyAction('ab', 'ab'))",
    );
    assert_eq!(
        keys,
        r#"{"do":"escape"} {"do":"move","step":1} {"do":"move","step":1} {"do":"move","step":-1} {"do":"move","step":-1} {"do":"home"} {"do":"end"} {"do":"activate"} {"do":"activate"} {"do":"activate"} {"do":"right"} {"do":"left"} null {"do":"jump","letter":"c"} null null"#
    );
}

#[test]
fn escape_closes_the_menu_a_release_does_nothing_and_a_key_it_does_not_use_does_not_leave_it() {
    let mut rig = world(1, "defineConfig({ decorations: true });");
    ask(&mut rig, 3, 50.0, 50.0);
    let at = layer(&mut rig);
    rig.comp.key(at, "Escape", "", false);
    rig.settle();
    assert!(menu_open(&mut rig), "a release of Escape is not Escape");
    press_key(&mut rig, "F5");
    assert!(menu_open(&mut rig), "an unused key leaves the menu open");
    press_key(&mut rig, "Escape");
    assert!(!menu_open(&mut rig), "Escape closes the menu");
    assert_eq!(rig.comp.flag(3, "maximised"), None, "and nothing ran");
}

#[test]
fn a_press_outside_the_card_closes_the_menu_a_press_on_its_padding_does_not_and_a_left_press_on_an_entry_runs_it() {
    let mut rig = world(1, "defineConfig({ decorations: true });");
    ask(&mut rig, 3, 50.0, 50.0);
    let (at, panel) = (
        layer(&mut rig),
        number(&mut rig, "windowMenu.current.panel.node") as u64,
    );
    rig.comp.pointer(panel, "press", 1.0, 1.0, LEFT);
    rig.settle();
    assert!(menu_open(&mut rig), "a press on the card's padding is not a click away");
    let maximise = row_node(&mut rig, "Maximise");
    rig.comp.pointer(maximise, "press", 5.0, 5.0, RIGHT);
    rig.settle();
    assert!(
        menu_open(&mut rig) && rig.comp.flag(3, "maximised").is_none(),
        "a right press on an entry runs nothing"
    );
    rig.comp.pointer(at, "press", 5.0, 5.0, LEFT);
    rig.settle();
    assert!(!menu_open(&mut rig), "a press on the layer closes the menu");
    ask(&mut rig, 3, 50.0, 50.0);
    let at = layer(&mut rig);
    rig.comp.pointer(at, "press", 5.0, 5.0, RIGHT);
    rig.settle();
    assert!(!menu_open(&mut rig), "a right press outside closes it too");
    ask(&mut rig, 3, 50.0, 50.0);
    let maximise = row_node(&mut rig, "Maximise");
    rig.comp.pointer(maximise, "press", 5.0, 5.0, LEFT);
    rig.settle();
    assert_eq!(
        rig.comp.flag(3, "maximised"),
        Some(true),
        "a left press on an entry runs it"
    );
    assert!(!menu_open(&mut rig), "and closes the menu");
}

#[test]
fn the_pointer_selects_an_entry_and_a_disabled_entry_cannot_be_selected_or_run_and_the_keyboard_skips_it() {
    let mut rig = world(1, "defineConfig({ decorations: true });");
    rig.comp.external_set(3, "maximised", MirrorValue::Bool(true));
    rig.settle();
    ask(&mut rig, 3, 50.0, 50.0);
    assert_eq!(
        labels(&mut rig).split(',').count(),
        7,
        "a maximised window offers Restore"
    );
    assert!(labels(&mut rig).starts_with("Restore,"));
    let minimise = row_node(&mut rig, "Minimise");
    rig.comp.pointer(minimise, "enter", 4.0, 4.0, 0);
    rig.settle();
    assert_eq!(selected(&mut rig), "Minimise", "the pointer selects what it is over");
    let moving = row_node(&mut rig, "Move");
    rig.comp.pointer(moving, "enter", 4.0, 4.0, 0);
    rig.settle();
    assert_eq!(selected(&mut rig), "Minimise", "a disabled entry is not selected");
    rig.comp.pointer(moving, "press", 4.0, 4.0, LEFT);
    rig.settle();
    assert!(
        menu_open(&mut rig),
        "pressing a disabled entry runs nothing and keeps the menu"
    );
    // Down from nothing: Restore, Minimise, then past Move and Resize.
    ask(&mut rig, 3, 50.0, 50.0);
    for _ in 0..3 {
        press_key(&mut rig, "Down");
    }
    assert_eq!(selected(&mut rig), "Always on top");
    press_key(&mut rig, "Up");
    assert_eq!(selected(&mut rig), "Minimise", "and back over them");
}

#[test]
fn restore_minimise_and_close_do_what_they_say() {
    let mut rig = world(1, "defineConfig({ decorations: true });");
    rig.comp.external_set(3, "maximised", MirrorValue::Bool(true));
    rig.settle();
    ask(&mut rig, 3, 50.0, 50.0);
    press_key(&mut rig, "Down");
    press_key(&mut rig, "Return");
    assert_eq!(rig.comp.flag(3, "maximised"), Some(false), "Restore leaves maximise");
    ask(&mut rig, 3, 50.0, 50.0);
    press_key(&mut rig, "Down");
    press_key(&mut rig, "Down");
    press_key(&mut rig, "Return");
    assert_eq!(rig.comp.flag(3, "minimised"), Some(true), "Minimise minimises");
    rig.comp.external_set(3, "minimised", MirrorValue::Bool(false));
    rig.settle();
    ask(&mut rig, 3, 50.0, 50.0);
    press_key(&mut rig, "End");
    press_key(&mut rig, "Return");
    assert_eq!(calls_of(&rig, "close"), 1, "Close calls the window's close");
    assert!(!menu_open(&mut rig), "and the menu is gone");
}

#[test]
fn always_on_top_pins_the_window_and_the_entry_shows_the_tick() {
    let mut rig = world(2, "defineConfig({ decorations: true });");
    ask(&mut rig, 3, 50.0, 50.0);
    assert_eq!(
        rig.eval("String(windowMenu.current.panel.rows[4].checked)"),
        "false",
        "it starts unchecked"
    );
    press_key(&mut rig, "End");
    press_key(&mut rig, "Up");
    press_key(&mut rig, "Up");
    assert_eq!(selected(&mut rig), "Always on top");
    press_key(&mut rig, "Return");
    assert_eq!(
        rig.eval("String(windowMenu.isAbove(gnoblin.clients.at(0)))"),
        "true",
        "the window is on top"
    );
    // A window raised later does not go over it: the pinned one is raised again.
    exec(&mut rig, "gnoblin.clients.at(gnoblin.clients.length - 1).raise()");
    rig.settle();
    ask(&mut rig, 3, 50.0, 50.0);
    assert_eq!(
        rig.eval("String(windowMenu.current.panel.rows[4].checked)"),
        "true",
        "the entry shows the tick"
    );
}

#[test]
fn move_to_workspace_opens_a_submenu_right_opens_it_return_moves_the_window_and_left_and_escape_go_back() {
    let mut rig = world(
        1,
        "defineConfig({ workspaces: [{ id: '1' }, { id: '2' }, { id: '3' }] });",
    );
    ask(&mut rig, 3, 50.0, 50.0);
    press_key(&mut rig, "End");
    press_key(&mut rig, "Up");
    assert_eq!(selected(&mut rig), "Move to workspace");
    press_key(&mut rig, "Right");
    assert_eq!(
        rig.eval("String(windowMenu.current.nav.depth)"),
        "2",
        "Right opens the submenu"
    );
    assert_eq!(
        rig.eval("String(windowMenu.current.nav.subRows.length)"),
        "3",
        "one entry for each workspace"
    );
    assert_eq!(
        rig.eval("`${windowMenu.current.nav.subRows[0].enabled},${windowMenu.current.nav.subRows[0].checked}`"),
        "false,true",
        "the workspace the window is on is ticked and cannot be chosen"
    );
    assert_eq!(
        rig.eval("String(windowMenu.current.sub.x >= windowMenu.current.panel.x + windowMenu.current.panel.width - 2)"),
        "true",
        "the submenu opens beside the card"
    );
    assert_eq!(
        selected(&mut rig),
        "2",
        "the selection is on the first entry that can be chosen"
    );
    press_key(&mut rig, "Left");
    assert_eq!(selected(&mut rig), "Move to workspace", "Left goes back to the card");
    press_key(&mut rig, "Right");
    press_key(&mut rig, "Escape");
    assert!(menu_open(&mut rig) && rig.eval("String(windowMenu.current.nav.depth)") == "1");
    press_key(&mut rig, "Right");
    press_key(&mut rig, "Return");
    assert_eq!(
        rig.eval("gnoblin.clients.at(0).workspace.id"),
        "2",
        "Return moves the window to that workspace"
    );
    assert!(!menu_open(&mut rig));
}

#[test]
fn move_to_workspace_is_off_for_a_fullscreen_window() {
    let mut rig = world(1, "defineConfig({ decorations: true });");
    rig.comp.external_set(3, "fullscreen", MirrorValue::Bool(true));
    rig.settle();
    ask(&mut rig, 3, 50.0, 50.0);
    assert_eq!(
        rig.eval("String(windowMenu.current.panel.rows.find((r) => r.label === 'Move to workspace').enabled)"),
        "false"
    );
}

#[test]
fn two_menus_never_stack_and_the_menu_closes_with_its_window() {
    let mut rig = world(2, "defineConfig({ decorations: true });");
    ask(&mut rig, 3, 50.0, 50.0);
    ask(&mut rig, 4, 400.0, 300.0);
    assert_eq!(
        rig.eval("String(windowMenu.current.client === gnoblin.clients.at(gnoblin.clients.length - 1))"),
        "true",
        "the newer menu is open"
    );
    let layers = rig.eval("String(uiCanvasCount())");
    ask(&mut rig, 4, 10.0, 10.0);
    assert_eq!(
        rig.eval("String(uiCanvasCount())"),
        layers,
        "asking again for the same window does not add one"
    );
    assert_eq!(
        rig.eval("String(windowMenu.current.panel.x)"),
        "10",
        "it opens where it was asked"
    );
    rig.comp.destroy(3);
    rig.settle();
    assert!(menu_open(&mut rig), "another window going leaves the menu");
    rig.comp.destroy(4);
    rig.settle();
    assert!(!menu_open(&mut rig), "its own window going closes it");
}

#[test]
fn a_menu_for_a_secure_window_is_secure_its_layer_is_not_and_none_opens_for_a_minimised_or_locked_one() {
    let mut rig = world(2, "defineConfig({ decorations: true });");
    rig.comp.external_set(3, "secure", MirrorValue::Bool(true));
    rig.settle();
    ask(&mut rig, 3, 50.0, 50.0);
    assert_eq!(
        rig.eval("`${windowMenu.current.panel.secure},${windowMenu.current.secure}`"),
        "true,false",
        "the card is secure; the transparent layer is not, or a capture would lose the screen"
    );
    ask(&mut rig, 4, 50.0, 50.0);
    assert_eq!(
        rig.eval("String(windowMenu.current.panel.secure)"),
        "false",
        "a card for an ordinary window is not"
    );
    rig.comp.external_set(22, "state", text("pending"));
    rig.settle();
    assert!(!menu_open(&mut rig), "the session locking closes the menu");
    ask(&mut rig, 4, 50.0, 50.0);
    assert!(!menu_open(&mut rig), "nothing opens over the lock screen");
    rig.comp.external_set(22, "state", text("not-locked"));
    rig.comp.external_set(4, "minimised", MirrorValue::Bool(true));
    rig.settle();
    ask(&mut rig, 4, 50.0, 50.0);
    assert!(!menu_open(&mut rig), "a minimised window has no menu");
}

#[test]
fn the_titlebars_right_click_opens_the_menu_at_the_pointer_and_its_middle_click_does_not() {
    let mut rig = world(1, "defineConfig({ decorations: true });");
    rig.comp.external_set(21, "pointer_x", MirrorValue::Double(120.0));
    rig.comp.external_set(21, "pointer_y", MirrorValue::Double(90.0));
    rig.settle();
    let bar = rig.comp.children(3)[0];
    rig.comp.pointer(bar, "press", 5.0, 5.0, MIDDLE);
    rig.settle();
    assert!(!menu_open(&mut rig), "a middle click lowers, it does not open a menu");
    rig.comp.pointer(bar, "press", 5.0, 5.0, RIGHT);
    rig.settle();
    assert!(menu_open(&mut rig), "the right click opens the menu");
    assert_eq!(
        rig.eval("`${windowMenu.current.panel.x},${windowMenu.current.panel.y}`"),
        "120,90",
        "at the pointer"
    );
    press_key(&mut rig, "Escape");
    rig.comp.external_set(21, "right_click", text("none"));
    rig.settle();
    rig.comp.pointer(bar, "press", 5.0, 5.0, RIGHT);
    rig.settle();
    assert!(!menu_open(&mut rig), "right_click = none opens no menu");
}

#[test]
fn a_script_that_replaces_items_changes_the_menu_and_one_that_replaces_show_draws_its_own() {
    let mut rig = world(
        1,
        "defineConfig({ decorations: true }); globalThis.ran = []; \
         windowMenu.items = () => [{ label: 'Only', run: (w) => globalThis.ran.push(w.title) }, { separator: true }, { label: 'Off', enabled: false }];",
    );
    ask(&mut rig, 3, 50.0, 50.0);
    assert_eq!(labels(&mut rig), "Only,Off", "the script's own entries");
    press_key(&mut rig, "Down");
    press_key(&mut rig, "Return");
    assert_eq!(rig.eval("ran.join(',')"), "win1", "and they run on the window");
    exec(
        &mut rig,
        "globalThis.seen = null; windowMenu.show = (c, x, y) => { globalThis.seen = [c.title, x, y]; return null; }",
    );
    ask(&mut rig, 3, 70.0, 80.0);
    assert_eq!(rig.eval("seen.join(',')"), "win1,70,80", "a menu request ends in show");
    assert!(!menu_open(&mut rig), "and the stock menu is not drawn");
}

#[test]
fn a_menu_entry_that_throws_is_reported_and_the_menu_is_already_closed() {
    let mut rig = world(
        1,
        "defineConfig({ decorations: true }); windowMenu.items = () => [{ label: 'Boom', run: () => { throw new Error('boom'); } }];",
    );
    ask(&mut rig, 3, 50.0, 50.0);
    press_key(&mut rig, "Down");
    let at = layer(&mut rig);
    rig.comp.key(at, "Return", "", true);
    let diagnostics = rig.settle();
    assert!(!menu_open(&mut rig), "the menu is closed before the entry runs");
    assert!(
        diagnostics.iter().any(|d| d.contains("boom")),
        "the error is reported: {diagnostics:?}"
    );
}

#[test]
fn menus_false_leaves_the_request_unanswered_and_a_request_with_no_anchor_opens_at_the_pointer() {
    let mut rig = world(1, "defineConfig({ menus: false });");
    ask(&mut rig, 3, 50.0, 50.0);
    assert_eq!(
        rig.eval("String(windowMenu.started)"),
        "false",
        "the stock menu never started"
    );
    assert!(!menu_open(&mut rig));
    let mut rig = world(1, "defineConfig({ decorations: true });");
    rig.comp.external_set(21, "pointer_x", MirrorValue::Double(220.0));
    rig.comp.external_set(21, "pointer_y", MirrorValue::Double(140.0));
    rig.settle();
    exec(&mut rig, "windowMenu.show(gnoblin.clients.at(0))");
    rig.settle();
    assert_eq!(
        rig.eval("`${windowMenu.current.panel.x},${windowMenu.current.panel.y}`"),
        "220,140"
    );
}

// ---- the alert: the cases of alerts_test.lua ------------------------------------------------------------------------

const FAST_ALERTS: &str =
    "defineConfig({ decorations: true }); notRespondingAlerts.armSeconds = 1; notRespondingAlerts.intervalMs = 0;";

fn stuck(rig: &mut Rig, node: u64, on: bool) {
    rig.comp.external_set(node, "unresponsive", MirrorValue::Bool(on));
    rig.settle();
}

fn card(rig: &mut Rig) -> Option<u64> {
    let found = rig.eval(
        "(() => { const c = notRespondingAlerts.cardOf(gnoblin.clients.at(0)); return c ? String(c.node) : ''; })()",
    );
    found.parse().ok()
}

fn dialog_key(rig: &mut Rig, key: &str) {
    let at = card(rig).expect("an alert is open");
    rig.comp.key(at, key, "", true);
    rig.settle();
}

#[test]
fn a_responsive_window_gets_no_alert_and_an_unresponsive_one_gets_an_alert_that_names_it_and_defaults_to_wait() {
    let mut rig = world(1, FAST_ALERTS);
    rig.settle();
    assert!(card(&mut rig).is_none(), "no alert");
    stuck(&mut rig, 3, true);
    let at = card(&mut rig).expect("an alert opens");
    assert!(
        !rig.comp.canvases_showing("win1 is not responding").is_empty(),
        "it names the window"
    );
    assert_eq!(
        rig.comp.value(21, "focus"),
        Some(MirrorValue::Node(at)),
        "the keys go to it"
    );
    assert_eq!(
        rig.eval("String(notRespondingAlerts.cardOf(gnoblin.clients.at(0)).secure)"),
        "false"
    );
    assert_eq!(
        rig.eval("String(notRespondingAlerts.cardOf(gnoblin.clients.at(0)).index)"),
        "0",
        "Wait is the default"
    );
    dialog_key(&mut rig, "Escape");
    assert!(
        card(&mut rig).is_none() && calls_of(&rig, "kill") == 0,
        "Escape waits and kills nothing"
    );
    assert_eq!(
        rig.comp.value(21, "focus"),
        Some(MirrorValue::Node(0)),
        "the keyboard goes back"
    );
}

#[test]
fn the_alert_for_a_secure_window_is_secure_and_a_title_with_control_characters_is_shown_as_text() {
    let mut rig = world(1, FAST_ALERTS);
    rig.comp.external_set(3, "secure", MirrorValue::Bool(true));
    rig.comp.external_set(3, "title", text("a\nb"));
    rig.settle();
    stuck(&mut rig, 3, true);
    assert_eq!(
        rig.eval("String(notRespondingAlerts.cardOf(gnoblin.clients.at(0)).secure)"),
        "true"
    );
    assert!(
        !rig.comp.canvases_showing("a b is not responding").is_empty(),
        "control characters are replaced"
    );
}

#[test]
fn wait_keeps_the_alert_away_until_the_window_answers_and_stops_answering_again() {
    let mut rig = world(1, FAST_ALERTS);
    stuck(&mut rig, 3, true);
    dialog_key(&mut rig, "Escape");
    rig.settle();
    assert!(card(&mut rig).is_none(), "no second alert while it stays unresponsive");
    stuck(&mut rig, 3, false);
    stuck(&mut rig, 3, true);
    assert!(card(&mut rig).is_some(), "an alert again once it went and came back");
}

#[test]
fn force_quit_does_nothing_until_the_alert_is_armed_then_kills_the_windows_process_and_closes() {
    let mut rig = world(1, FAST_ALERTS);
    stuck(&mut rig, 3, true);
    dialog_key(&mut rig, "Tab");
    dialog_key(&mut rig, "Return");
    assert!(
        card(&mut rig).is_some() && calls_of(&rig, "kill") == 0,
        "Force quit ignores an early press"
    );
    rig.advance(1200);
    rig.settle();
    dialog_key(&mut rig, "Return");
    assert_eq!(calls_of(&rig, "kill"), 1, "an armed Force quit calls kill once");
    assert!(card(&mut rig).is_none(), "and the alert is gone");
    rig.settle();
    assert!(card(&mut rig).is_none(), "and does not come back");
}

#[test]
fn a_press_on_a_button_works_like_the_key_and_a_press_on_the_card_keeps_the_keyboard_on_it() {
    let mut rig = world(1, FAST_ALERTS);
    stuck(&mut rig, 3, true);
    let at = card(&mut rig).expect("an alert");
    rig.comp.pointer(at, "press", 5.0, 5.0, LEFT);
    rig.settle();
    assert_eq!(
        rig.comp.value(21, "focus"),
        Some(MirrorValue::Node(at)),
        "a press on the card focuses it"
    );
    rig.advance(1200);
    rig.settle();
    let force = number(
        &mut rig,
        "notRespondingAlerts.cardOf(gnoblin.clients.at(0)).choices[1].node",
    ) as u64;
    rig.comp.pointer(force, "press", 5.0, 5.0, LEFT);
    rig.settle();
    assert!(
        calls_of(&rig, "kill") == 1 && card(&mut rig).is_none(),
        "a click on Force quit kills"
    );
}

#[test]
fn the_alert_closes_when_the_window_answers_again_or_goes_and_under_a_lock() {
    let mut rig = world(2, FAST_ALERTS);
    stuck(&mut rig, 3, true);
    assert!(card(&mut rig).is_some(), "open");
    stuck(&mut rig, 3, false);
    assert!(card(&mut rig).is_none(), "closed when it answers");
    stuck(&mut rig, 4, true);
    assert_eq!(
        rig.eval("String(notRespondingAlerts.cardOf(gnoblin.clients.at(gnoblin.clients.length - 1)) !== undefined)"),
        "true"
    );
    rig.comp.destroy(4);
    rig.settle();
    assert_eq!(
        rig.eval("String(uiCanvasCount() === 6)"),
        "true",
        "closed when the window goes: only the other window's titlebar is left"
    );
    rig.comp.external_set(22, "state", text("locked"));
    rig.settle();
    stuck(&mut rig, 3, true);
    assert!(card(&mut rig).is_none(), "nothing over the lock screen");
    rig.comp.external_set(22, "state", text("not-locked"));
    rig.settle();
    assert!(card(&mut rig).is_some(), "the alert comes once it is unlocked");
    rig.comp.external_set(22, "state", text("locked"));
    rig.settle();
    assert!(card(&mut rig).is_none(), "and goes when the session locks");
}

#[test]
fn alerts_false_turns_the_alert_off_and_an_alert_closes_an_open_window_menu() {
    let mut rig = world(1, "defineConfig({ alerts: false });");
    stuck(&mut rig, 3, true);
    assert_eq!(rig.eval("String(notRespondingAlerts.started)"), "false");
    let mut rig = world(1, FAST_ALERTS);
    ask(&mut rig, 3, 10.0, 10.0);
    assert!(menu_open(&mut rig));
    stuck(&mut rig, 3, true);
    assert!(!menu_open(&mut rig), "the alert closed the menu");
}

#[test]
fn the_alert_tracker_decides_who_gets_an_alert_and_who_is_waited_on() {
    let mut rig = world(1, "export {};");
    let got = rig.eval(
        "(() => { const w = { alive: true, unresponsive: false, title: 't', kill() {} }; const t = new AlertTracker(); \
         const out = []; const look = (locked = false) => { const c = t.check([w], locked); out.push(`${c.open.length}${c.close.length}`); }; \
         look(); w.unresponsive = true; look(); look(); \
         out.push(String(t.answered(w, 'wait'))); look(); w.unresponsive = false; look(); w.unresponsive = true; look(); \
         out.push(String(t.answered(w, 'force'))); look(true); w.unresponsive = false; look(); w.unresponsive = true; look(true); look(); \
         out.push(cleanText('a\\nb', 80), cleanText('x'.repeat(70), 60).length.toString()); return out.join(','); })()",
    );
    assert_eq!(
        got, "00,10,00,false,00,00,10,true,00,00,00,10,a b,61",
        "open once, wait keeps it away until it answers, Force quit kills, a lock holds the alert back"
    );
}

// ---- Entry, Clock, Bar, toast, overview, launcher ---------------------------------------------------------------------

#[test]
fn the_entry_model_types_deletes_moves_and_selects() {
    let mut rig = world(1, "export {};");
    let got = rig.eval(
        "(() => { const m = new EntryModel(); const out = []; const at = () => `${m.text}|${m.caret}|${m.selectedText}`; \
         m.insert('héllo'); out.push(at()); m.backspace(); out.push(at()); m.left(); m.left(); m.insert('X\\n'); out.push(at()); \
         m.home(true); out.push(at()); m.insert('Z'); out.push(at()); m.selectAll(); out.push(at()); m.delete(); out.push(at()); \
         m.insert('ab'); m.left(); m.delete(); out.push(at()); m.end(); m.right(); m.left(true); out.push(at()); \
         m.text = 'new'; out.push(at()); return out.join(' '); })()",
    );
    assert_eq!(
        got,
        "héllo|5| héll|4| héXll|3| héXll|0|héX Zll|1| Zll|3|Zll |0| a|1| a|0|a new|3|"
    );
}

#[test]
fn an_entry_reads_keys_like_the_lua_entry_and_submits_with_return() {
    let mut rig = world(
        1,
        "const e = new Entry({ width: 300, height: 40 }); e.mountTop(); e.focus(); globalThis.e = e; \
         globalThis.sent = []; e.onSubmit = (t) => globalThis.sent.push(t);",
    );
    let at = number(&mut rig, "e.node") as u64;
    assert_eq!(
        rig.comp.value(21, "focus"),
        Some(MirrorValue::Node(at)),
        "the entry holds the keyboard"
    );
    for (key, text) in [("f", "f"), ("o", "o"), ("o", "o"), ("BackSpace", ""), ("x", "x")] {
        rig.comp.key(at, key, text, true);
    }
    rig.comp.key(at, "Escape", "", true);
    rig.comp.key(at, "a", "a", false);
    rig.settle();
    assert_eq!(
        rig.eval("e.text"),
        "fox",
        "typed text appends, BackSpace removes, a release types nothing"
    );
    rig.comp.key(at, "Shift_L", "", true);
    rig.comp.key(at, "Left", "", true);
    rig.comp.key(at, "Left", "", true);
    rig.comp.key(at, "Shift_L", "", false);
    rig.settle();
    assert_eq!(rig.eval("e.model.selectedText"), "ox", "Shift with the arrows selects");
    rig.comp.key(at, "Control_L", "", true);
    rig.comp.key(at, "a", "a", true);
    rig.comp.key(at, "Control_L", "", false);
    rig.comp.key(at, "Return", "", true);
    rig.settle();
    assert_eq!(rig.eval("sent.join(',')"), "fox", "Return submits the text");
    assert_eq!(rig.eval("e.model.selectedText"), "fox", "Control+A selected all");
    exec(&mut rig, "e.secret = true");
    assert_eq!(
        rig.eval("e.shown(e.text)"),
        "***",
        "a secret shows a * for each character"
    );
    rig.settle();
}

#[test]
fn the_clock_formats_the_time_and_follows_it() {
    let mut rig = world(1, "export {};");
    let got = rig.eval(
        "(() => { const d = new Date(2024, 0, 5, 14, 7, 9); return [formatTime('%H:%M', d), formatTime('%I:%M %p', d), \
         formatTime('%a %d %b %Y', d), formatTime('%T %%', d), formatTime('%Q', d), formatTime('%F %j', d)].join('|'); })()",
    );
    assert_eq!(got, "14:07|02:07 PM|Fri 05 Jan 2024|14:07:09 %|%Q|2024-01-05 005");
    exec(
        &mut rig,
        "globalThis.t = new Date(2024, 0, 5, 9, 30); globalThis.clock = new Clock({ now: () => globalThis.t })",
    );
    rig.settle();
    assert_eq!(rig.eval("clock.text"), "09:30");
    exec(&mut rig, "globalThis.t = new Date(2024, 0, 5, 9, 31)");
    rig.settle();
    assert_eq!(
        rig.eval("clock.text"),
        "09:31",
        "it follows the clock a frame at a time"
    );
}

#[test]
fn a_bar_takes_its_room_from_the_windows_and_gives_it_back_when_it_goes() {
    let mut rig = world(1, "defineConfig({ decorations: true });");
    assert_eq!(
        rig.eval("JSON.stringify(gnoblin.workspaces.area)"),
        r#"{"x":0,"y":0,"width":1000,"height":800}"#
    );
    exec(
        &mut rig,
        "globalThis.top = new Bar({ height: 32 }); globalThis.low = new Bar({ edge: 'bottom', height: 20 })",
    );
    rig.settle();
    assert_eq!(
        rig.eval("JSON.stringify(gnoblin.workspaces.area)"),
        r#"{"x":0,"y":32,"width":1000,"height":748}"#,
        "the windows are laid out between the bars"
    );
    assert_eq!(rig.eval("`${top.y},${top.width},${low.y}`"), "0,1000,780");
    exec(&mut rig, "top.hideWhen = () => true");
    assert_eq!(
        rig.eval("JSON.stringify(barInsets())"),
        r#"{"top":0,"bottom":20}"#,
        "a hidden bar takes no room"
    );
    exec(&mut rig, "top.destroy(); low.destroy()");
    assert_eq!(rig.eval("JSON.stringify(barInsets())"), r#"{"top":0,"bottom":0}"#);
}

#[test]
fn a_toast_adds_one_card_to_the_screen_fades_in_and_removes_itself() {
    let mut rig = world(1, "export {};");
    let before = number(&mut rig, "uiCanvasCount()");
    exec(
        &mut rig,
        "globalThis.t = toast('saved', { fadeIn: 0.01, hold: 0.05, fadeOut: 0.01 })",
    );
    rig.settle();
    assert_eq!(
        number(&mut rig, "uiCanvasCount()"),
        before + 2.0,
        "one card and its message"
    );
    assert!(!rig.comp.canvases_showing("saved").is_empty());
    assert_eq!(rig.eval("`${t.x},${t.y}`"), "660,44", "the top right corner");
    rig.advance(150);
    rig.settle();
    assert_eq!(number(&mut rig, "uiCanvasCount()"), before, "and it is gone");
}

#[test]
fn the_overview_keeps_every_workspace_at_full_strength_while_it_is_on_and_dims_the_others_when_it_is_off() {
    let mut rig = world(2, "defineConfig({ workspaces: [{ id: 'a' }, { id: 'b' }] });");
    exec(
        &mut rig,
        "gnoblin.workspaces.at(gnoblin.workspaces.length - 1).clients.add(gnoblin.clients.at(gnoblin.clients.length - 1))",
    );
    rig.settle();
    assert_eq!(rig.eval("String(overview.toggle())"), "true");
    rig.settle();
    assert_eq!(rig.eval("gnoblin.clients.map((c) => c.opacity).join(',')"), "1,1");
    assert_eq!(rig.eval("String(overview.toggle())"), "false");
    rig.settle();
    assert_eq!(
        rig.eval("gnoblin.clients.map((c) => c.opacity).join(',')"),
        "1,0.4",
        "the active workspace stays at full strength"
    );
}

#[test]
fn the_launcher_opens_on_the_screen_returns_spawns_the_command_and_escape_closes_it() {
    let mut rig = world(
        1,
        "globalThis.spawned = []; globalThis.open = () => openLauncher({ spawn: (c) => globalThis.spawned.push(c) });",
    );
    exec(&mut rig, "globalThis.l = open()");
    rig.settle();
    assert_eq!(
        rig.eval("`${l.x},${l.y},${l.width},${l.height}`"),
        "200,200,600,48",
        "Super+Space opens one launcher"
    );
    assert_eq!(rig.eval("String(open() === l)"), "true", "a second request reuses it");
    let at = number(&mut rig, "l.node") as u64;
    assert_eq!(rig.comp.value(21, "focus"), Some(MirrorValue::Node(at)));
    rig.comp.key(at, "Escape", "", true);
    rig.settle();
    assert_eq!(rig.eval("String(l.alive)"), "false", "Escape closes the launcher");
    exec(&mut rig, "globalThis.l = open()");
    rig.settle();
    let at = number(&mut rig, "l.node") as u64;
    for ch in ["f", "o", "o"] {
        rig.comp.key(at, ch, ch, true);
    }
    rig.comp.key(at, "Return", "", true);
    rig.settle();
    assert_eq!(rig.eval("spawned.join(',')"), "foo", "Return spawns the typed command");
    assert_eq!(rig.eval("String(l.alive)"), "false", "and closes the launcher");
}

#[test]
fn super_space_opens_the_stock_launcher_unless_a_launcher_command_or_a_key_takes_it() {
    let mut rig = world(1, "defineConfig({ decorations: true });");
    assert_eq!(
        rig.eval("[...gnoblin.input.primary.bindings].some((b) => b.trigger === 'Super+space') ? 'bound' : 'free'"),
        "bound"
    );
    let mut rig = world(1, "defineConfig({ launcher: 'wofi' });");
    assert_eq!(
        rig.eval("String([...gnoblin.input.primary.bindings].filter((b) => b.trigger === 'Super+space').length)"),
        "1",
        "a command takes the key and the entry is not bound as well"
    );
}

// ---- the titlebar: gestures, the reserved room and the fade -----------------------------------------------------------

#[test]
fn a_decorated_window_is_placed_with_its_content_below_its_bar() {
    let mut rig = world(0, "defineConfig({ decorations: true });");
    rig.comp.create_window(3, "term", "foot");
    rig.settle();
    assert_eq!(rig.eval("String(gnoblin.clients.at(0).frameInset)"), "28");
    // Smart placement puts the first window's frame at the corner of the screen: the bar there, the content a bar below.
    assert_eq!(rig.comp.number(3, "y"), 28.0, "the content starts below the bar");
    assert_eq!(
        rig.comp.number(3, "height"),
        300.0,
        "the content keeps the height the client has"
    );
    let bar = rig.comp.children(3)[0];
    assert_eq!(
        rig.comp.rect(bar),
        "0,-28,400,28",
        "the bar is above the content: no overlap"
    );
    // A second window is placed as a frame too, and does not overlap the first window's bar or content.
    rig.comp.create_window(4, "other", "foot");
    rig.settle();
    let (x, y) = (rig.comp.number(4, "x"), rig.comp.number(4, "y") - 28.0);
    assert!(
        x >= 400.0 || y >= 28.0 + 300.0,
        "the second frame lies clear of the first: {x},{y}"
    );
    rig.comp.external_set(3, "fullscreen", MirrorValue::Bool(true));
    rig.settle();
    assert_eq!(
        rig.eval("String(gnoblin.clients.at(0).frameInset)"),
        "0",
        "a fullscreen window has no bar and no inset"
    );
}

#[test]
fn the_titlebar_does_what_the_seat_says_a_double_a_middle_and_a_right_click_do() {
    let mut rig = world(1, "defineConfig({ decorations: true });");
    let bar = rig.comp.children(3)[0];
    rig.comp.pointer(bar, "press", 5.0, 5.0, LEFT);
    rig.comp.pointer(bar, "press", 5.0, 5.0, LEFT);
    rig.settle();
    assert_eq!(rig.comp.flag(3, "maximised"), Some(true), "a double click maximises");
    rig.comp.external_set(21, "double_click", text("none"));
    rig.comp.external_set(3, "maximised", MirrorValue::Bool(false));
    rig.settle();
    rig.comp.pointer(bar, "press", 5.0, 5.0, LEFT);
    rig.comp.pointer(bar, "press", 5.0, 5.0, LEFT);
    rig.settle();
    assert_eq!(
        rig.comp.flag(3, "maximised"),
        Some(false),
        "double_click = none leaves the window alone"
    );
    rig.comp.external_set(21, "middle_click", text("explode"));
    rig.comp.pointer(bar, "press", 5.0, 5.0, MIDDLE);
    let diagnostics = rig.settle();
    assert!(
        diagnostics.iter().any(|d| d.contains("must name a titlebar action")),
        "an unknown titlebar action is reported: {diagnostics:?}"
    );
}

/// Runs `code` for its effect.
fn exec(rig: &mut Rig, code: &str) {
    rig.eval(&format!("{code}; 'ok'"));
}
