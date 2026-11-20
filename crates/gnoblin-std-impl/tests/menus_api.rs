#![allow(unsafe_code)]
//! What the native window menu offers beyond the recorded scenarios: a provider that adds to the stock entries, a submenu that is a list or
//! a function, the `closed` signal, the refusals, and the rows a shell makes for itself. Each test runs over a desktop with no fixture.

#[path = "ui/rig.rs"]
mod rig;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gnoblin_core_abi as abi;
use gnoblin_object::{Fail, Val};
use rig::{Rig, W, b, callback, t};

/// A desktop of 1000 by 800 with one window (node 3), the workspaces open and the window menu started.
fn started() -> (Rig, W) {
    let rig = Rig::scratch_with_windows(1000.0, 800.0, 1);
    let _workspaces = rig.make("workspaces", &[]);
    let menus = rig.make("window_menus", &[]);
    menus.call("start", &[]);
    (rig, menus)
}

fn objects(of: &Val, api: gnoblin_object::client::Api) -> Vec<W> {
    of.list()
        .expect("a list")
        .iter()
        .map(|value| match value {
            Val::Object { class, obj } => W {
                api,
                class: *class,
                obj: *obj,
            },
            other => panic!("expected an object, got {other:?}"),
        })
        .collect()
}

fn labels(rows: &[W]) -> Vec<String> {
    rows.iter().map(|row| row.text("label")).collect()
}

/// A `menu_provider` the shell implements.
fn provide(rig: &Rig, members: &[&str], run: impl FnMut(&str, &[Val]) -> Result<Val, Fail> + 'static) -> Val {
    rig.implement("menu_provider", members, run)
}

fn open(rig: &mut Rig, menus: &W) -> W {
    rig.comp.request(3, "menu", 50.0, 50.0);
    rig.settle();
    match menus.get("current") {
        Val::Object { class, obj } => W {
            api: menus.api,
            class,
            obj,
        },
        other => panic!("no menu opened: {other:?}"),
    }
}

fn press(rig: &mut Rig, menu: &W, key: &str) {
    rig.comp.key(menu.node(), key, "", true);
    rig.settle();
}

#[test]
fn the_stock_entries_are_the_ones_a_provider_adds_to() {
    let (mut rig, menus) = started();
    let stock = objects(&menus.call("default_items", &[t("3")]), menus.api);
    assert_eq!(
        labels(&stock),
        [
            "Maximise",
            "Minimise",
            "",
            "Move",
            "Resize",
            "Always on top",
            "Move to workspace",
            "",
            "Close"
        ]
    );
    assert!(stock[2].flag("separator") && stock[7].flag("separator"));
    // No keyboard move or resize was given, so the two entries are shown off.
    assert!(!stock[3].flag("enabled") && !stock[4].flag("enabled"));
    let ran = Rc::new(Cell::new(0));
    let mine = rig.make("menu_entry", &[("label", t("Mine"))]);
    let count = Rc::clone(&ran);
    mine.set(
        "run",
        callback(move |args| {
            assert_eq!(args[0].text()?, "3");
            count.set(count.get() + 1);
            Ok(Val::None)
        }),
    );
    let defaults = stock.iter().map(W::val).collect::<Vec<_>>();
    let mut all = defaults;
    all.push(mine.val());
    menus.set(
        "provider",
        provide(&rig, &["items"], move |_, _| Ok(Val::List(all.clone()))),
    );
    let menu = open(&mut rig, &menus);
    let rows = objects(&menu.obj("panel").get("rows"), menus.api);
    assert_eq!(rows.last().map(|row| row.text("label")).as_deref(), Some("Mine"));
    press(&mut rig, &menu, "End");
    press(&mut rig, &menu, "Return");
    assert_eq!(ran.get(), 1, "the entry ran once, after the menu closed");
    assert!(matches!(menus.get("current"), Val::None));
}

#[test]
fn a_submenu_is_a_list_or_a_function() {
    let (mut rig, menus) = started();
    let pick = Rc::new(RefCell::new(Vec::<String>::new()));
    let child = |label: &str, log: &Rc<RefCell<Vec<String>>>, rig: &Rig| {
        let entry = rig.make("menu_entry", &[("label", t(label))]);
        let log = Rc::clone(log);
        let name = label.to_owned();
        entry.set(
            "run",
            callback(move |_| {
                log.borrow_mut().push(name.clone());
                Ok(Val::None)
            }),
        );
        entry
    };
    let listed = rig.make("menu_entry", &[("label", t("Listed"))]);
    listed.set(
        "submenu",
        Val::List(vec![child("one", &pick, &rig).val(), child("two", &pick, &rig).val()]),
    );
    let asked = rig.make("menu_entry", &[("label", t("Asked"))]);
    let made: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let made_for = Rc::clone(&made);
    let source = Rc::clone(&pick);
    let api = menus.api;
    let class = rig.api.class("menu_entry").expect("the class");
    asked.set(
        "submenu_entries",
        callback(move |args| {
            made_for.borrow_mut().push(args[0].text()?.to_owned());
            let entry = api
                .create(
                    class,
                    &[(
                        api.member(class, abi::MEMBER_PROPERTY, "label").expect("label"),
                        t("three"),
                    )],
                )
                .map_err(|fail| Fail::err(fail.to_string()))?;
            let log = Rc::clone(&source);
            let run = api.member(class, abi::MEMBER_PROPERTY, "run").expect("run");
            api.set(
                entry,
                run,
                &callback(move |_| {
                    log.borrow_mut().push("three".to_owned());
                    Ok(Val::None)
                }),
            )
            .map_err(|fail| Fail::err(fail.to_string()))?;
            Ok(Val::List(vec![Val::Object { class, obj: entry }]))
        }),
    );
    let items = Val::List(vec![listed.val(), asked.val()]);
    menus.set("provider", provide(&rig, &["items"], move |_, _| Ok(items.clone())));
    let menu = open(&mut rig, &menus);
    press(&mut rig, &menu, "Down");
    press(&mut rig, &menu, "Right");
    let nav = menu.obj("nav");
    assert_eq!(nav.get("depth").uint().expect("depth"), 2);
    assert_eq!(labels(&objects(&nav.get("sub_rows"), menus.api)), ["one", "two"]);
    press(&mut rig, &menu, "Left");
    press(&mut rig, &menu, "Down");
    press(&mut rig, &menu, "Right");
    assert_eq!(
        *made.borrow(),
        ["3"],
        "the function was asked for the window when the submenu opened"
    );
    assert_eq!(labels(&objects(&nav.get("sub_rows"), menus.api)), ["three"]);
    press(&mut rig, &menu, "Return");
    assert_eq!(*pick.borrow(), ["three"]);
}

#[test]
fn the_menu_says_when_it_closes() {
    let (mut rig, menus) = started();
    let menu = open(&mut rig, &menus);
    let told = Rc::new(Cell::new(0));
    let count = Rc::clone(&told);
    let signal = menu
        .api
        .member(menu.class, abi::MEMBER_SIGNAL, "closed")
        .expect("a signal");
    menu.api
        .listen(menu.obj, signal, move |_, _| {
            count.set(count.get() + 1);
            Ok(Val::None)
        })
        .expect("a listener");
    assert!(menus.call("close", &[]).bool().expect("a flag"));
    rig.settle();
    assert_eq!(told.get(), 1);
    assert!(!menus.call("close", &[]).bool().expect("a flag"), "nothing was open");
    assert!(!menu.flag("alive"), "the card is gone");
}

#[test]
fn a_provider_that_answers_badly_opens_no_menu() {
    let (mut rig, menus) = started();
    menus.set("provider", provide(&rig, &["items"], |_, _| Ok(Val::Bool(true))));
    rig.comp.request(3, "menu", 50.0, 50.0);
    rig.settle();
    assert!(matches!(menus.get("current"), Val::None));
    assert!(
        rig.reported()
            .iter()
            .any(|line| line.contains("the window menu's items")),
        "the failure is reported"
    );
    menus.set("provider", provide(&rig, &["items"], |_, _| Ok(Val::List(Vec::new()))));
    rig.comp.request(3, "menu", 50.0, 50.0);
    rig.settle();
    assert!(
        matches!(menus.get("current"), Val::None),
        "a menu with no entries is not shown"
    );
}

#[test]
fn a_menu_is_not_opened_for_a_window_that_is_not_one() {
    let (mut rig, menus) = started();
    rig.comp.request(21, "menu", 50.0, 50.0);
    rig.comp.request(999, "menu", 50.0, 50.0);
    rig.settle();
    assert!(matches!(menus.get("current"), Val::None));
    let refused = menus.try_call("open", &[t("not a node"), Val::Double(0.0), Val::Double(0.0)]);
    assert!(refused.is_err(), "a client is its node as text");
}

#[test]
fn the_rows_a_shell_makes_follow_the_navigator() {
    let (mut rig, _menus) = started();
    let entry = |label: &str, enabled: bool| {
        let entry = rig.make("menu_entry", &[("label", t(label))]);
        entry.set("enabled", b(enabled));
        entry
    };
    let rows: Vec<W> = [("alpha", true), ("beta", false), ("gamma", true)]
        .iter()
        .map(|(label, enabled)| rig.make("menu_item", &[("entry", entry(label, *enabled).val())]))
        .collect();
    let nav = rig.make(
        "menu_navigator",
        &[("rows", Val::List(rows.iter().map(W::val).collect()))],
    );
    let at = |nav: &W| nav.get("index").int().expect("an index");
    assert_eq!(at(&nav), -1);
    nav.call("move", &[Val::Int(1)]);
    assert_eq!(at(&nav), 0);
    nav.call("move", &[Val::Int(1)]);
    assert_eq!(at(&nav), 2, "the disabled row is skipped");
    assert!(nav.call("is_selected", &[rows[2].val()]).bool().expect("a flag"));
    let ink = |row: &W| row.call("ink", &[]).text().expect("a colour").to_owned();
    assert_eq!(
        (ink(&rows[0]), ink(&rows[1])),
        ("#cdd6f4".to_owned(), "#6c7086".to_owned())
    );
    rows[0].set("selected", b(true));
    rig.settle();
    assert_eq!(ink(&rows[0]), "#11111b");
    assert!(!rows[1].flag("enabled") && !rows[1].flag("submenu") && !rows[1].flag("checked"));
    assert_eq!(rows[2].obj("label_text").text("text"), "gamma");
    assert_eq!(rows[0].obj("entry").text("label"), "alpha");
}

#[test]
fn the_pointer_on_a_row_outside_the_submenu_closes_its_card() {
    let (mut rig, menus) = started();
    let menu = open(&mut rig, &menus);
    press(&mut rig, &menu, "End");
    press(&mut rig, &menu, "Up");
    press(&mut rig, &menu, "Right");
    let sub = menu.obj("sub");
    let nav = menu.obj("nav");
    assert_eq!(nav.get("depth").uint().expect("depth"), 2);
    let close = objects(&menu.obj("panel").get("rows"), menus.api)
        .into_iter()
        .find(|row| row.text("label") == "Close")
        .expect("a Close row");
    rig.comp.pointer(close.node(), "enter", 4.0, 4.0, 0);
    rig.settle();
    assert_eq!(nav.get("depth").uint().expect("depth"), 1);
    assert!(matches!(menu.get("sub"), Val::None), "the submenu's card is gone");
    assert!(!sub.flag("alive"), "and its canvas with it");
}
