//! The classes of the `menus` catalog over the object protocol: the rows, cards and layer (canvases of the toolkit), the navigator, the
//! functions of the card's rules, and the service that answers requests.

use std::cell::RefCell;
use std::rc::Rc;

use gnoblin_object::server::{Builder, Cx};
use gnoblin_object::{ErrorCode, Fail, Obj, Res, Val};

use super::card::{self, MenuPart};
use super::entry::{self, EntryObj};
use super::metrics::{self, KeyAction};
use super::navigator::Nav;
use super::windows::{self, MenuProviderRef};
use crate::object::client::client_node;
use crate::object::record::{enumeration, record};
use crate::ui::{CanvasHandle, Cid, Special, Ui, WidgetKind, box_members, canvas_members, ui_of};

/// The canvas an object of the toolkit holds.
fn canvas_of(cx: &Cx<'_>, obj: Obj) -> Res<Cid> {
    cx.with_state(obj, |handle: &mut CanvasHandle| handle.cid)
}

/// The canvases of a list of objects.
fn canvases(cx: &Cx<'_>, list: &Val) -> Res<Vec<Cid>> {
    list.list()?
        .iter()
        .map(|value| canvas_of(cx, value.object()?))
        .collect()
}

/// The object of a canvas, handed to the shell, or none when it is gone.
fn handout(ui: &Ui, cx: &Cx<'_>, cid: Cid) -> Res<Val> {
    match ui.read(cid, |widget| widget.obj) {
        Some(obj) => cx.handout(obj),
        None => Ok(Val::None),
    }
}

fn handouts(ui: &Ui, cx: &Cx<'_>, cids: &[Cid]) -> Res<Val> {
    cids.iter()
        .map(|cid| handout(ui, cx, *cid))
        .collect::<Res<Vec<_>>>()
        .map(Val::List)
}

// ---- the navigator ------------------------------------------------------------------------------------------------------------------

enum Source {
    /// The rows a shell gave.
    Own(RefCell<Nav>),
    /// The selection of the menu layer with this canvas.
    Menu(Cid),
}

/// The state of a `menu_navigator`.
pub(crate) struct NavHandle {
    ui: Rc<Ui>,
    source: Source,
}

impl NavHandle {
    /// The view of the selection a menu layer holds.
    pub(crate) fn of_menu(ui: &Rc<Ui>, menu: Cid) -> NavHandle {
        NavHandle {
            ui: Rc::clone(ui),
            source: Source::Menu(menu),
        }
    }

    fn nav(&self) -> Res<Nav> {
        match &self.source {
            Source::Own(nav) => Ok(nav.borrow().clone()),
            Source::Menu(menu) => card::nav_of(&self.ui, *menu).ok_or_else(|| Fail::stale("the menu is gone")),
        }
    }

    fn update<R>(&self, work: impl FnOnce(&mut Nav) -> R) -> Res<R> {
        match &self.source {
            Source::Own(nav) => Ok(work(&mut nav.borrow_mut())),
            Source::Menu(menu) => card::with_nav(&self.ui, *menu, work).ok_or_else(|| Fail::stale("the menu is gone")),
        }
    }
}

fn navigator(builder: &mut Builder) {
    let _ = builder
        .class::<NavHandle>("menu_navigator")
        .ctor(|cx, inits| {
            let ui = ui_of(cx)?;
            let rows = match inits.take("rows") {
                Some(rows) => canvases(cx, &rows)?,
                None => Vec::new(),
            };
            Ok(NavHandle {
                ui,
                source: Source::Own(RefCell::new(Nav::new(rows))),
            })
        })
        .get("rows", |h, cx| handouts(&h.ui, cx, &h.nav()?.rows))
        .get("sub_rows", |h, cx| handouts(&h.ui, cx, &h.nav()?.sub))
        .get("depth", |h, _| Ok(Val::Uint(u64::from(h.nav()?.depth()))))
        .get("index", |h, _| Ok(Val::Int(h.nav()?.index())))
        .get("selected", |h, cx| match h.nav()?.selected() {
            Some(row) => handout(&h.ui, cx, row),
            None => Ok(Val::None),
        })
        .method("active", |h, cx, _| handouts(&h.ui, cx, h.nav()?.active()))
        .method("is_selected", |h, cx, args| {
            let row = canvas_of(cx, args[0].object()?)?;
            Ok(Val::Bool(h.nav()?.is_selected(row)))
        })
        .method("select", |h, cx, args| {
            let row = canvas_of(cx, args[0].object()?)?;
            h.update(|nav| nav.select(row))?;
            Ok(Val::None)
        })
        .method("move", |h, _, args| {
            let step = args[0].int()?;
            let info = card::row_of(&h.ui);
            h.update(|nav| nav.step(step.signum(), &info))?;
            Ok(Val::None)
        })
        .method("home", |h, _, _| {
            let info = card::row_of(&h.ui);
            h.update(|nav| nav.home(&info))?;
            Ok(Val::None)
        })
        .method("end", |h, _, _| {
            let info = card::row_of(&h.ui);
            h.update(|nav| nav.end(&info))?;
            Ok(Val::None)
        })
        .method("jump", |h, _, args| {
            let info = card::row_of(&h.ui);
            let letter = args[0].text()?.to_owned();
            h.update(|nav| nav.jump(&letter, &info))?;
            Ok(Val::None)
        })
        .method("open_submenu", |h, cx, args| {
            let rows = canvases(cx, &args[0])?;
            let info = card::row_of(&h.ui);
            h.update(|nav| nav.open_submenu(rows, &info))?;
            Ok(Val::None)
        })
        .method("close_submenu", |h, _, _| {
            h.update(Nav::close_submenu)?;
            Ok(Val::None)
        });
}

// ---- the canvases of a card -------------------------------------------------------------------------------------------------------

fn item_of(h: &CanvasHandle) -> Res<card::ItemPart> {
    h.ui.read(h.cid, |widget| match &widget.sp {
        Special::Item(item) => Some((**item).clone()),
        _ => None,
    })
    .flatten()
    .ok_or_else(|| Fail::stale("the row is gone"))
}

fn menu_of(h: &CanvasHandle) -> Res<MenuPart> {
    card::part(&h.ui, h.cid).ok_or_else(|| Fail::stale("the menu is gone"))
}

fn canvas_classes(builder: &mut Builder) {
    let item = builder.class::<CanvasHandle>("menu_item").ctor(|cx, inits| {
        let ui = ui_of(cx)?;
        let entry = match inits.take("entry") {
            Some(value) => entry::read(cx, value.object()?)?,
            None => return Err(Fail::type_error("a menu_item is made from an entry")),
        };
        let me = cx.me();
        let handle = ui.make_for(me, WidgetKind::MenuItem)?;
        card::fill_item(&ui, cx, me, handle.cid, &entry)?;
        Ok(handle)
    });
    let _ = box_members(canvas_members(item))
        .get("entry", |h, cx| {
            let item = item_of(h)?;
            let obj = cx.spawn("menu_entry", EntryObj::from(item.entry))?;
            let held = cx.handout(obj);
            cx.unpin(obj);
            held
        })
        .get("label", |h, _| Ok(Val::Text(item_of(h)?.entry.label)))
        .get("enabled", |h, _| Ok(Val::Bool(item_of(h)?.entry.enabled)))
        .get("checked", |h, _| Ok(Val::Bool(item_of(h)?.entry.checked)))
        .get("submenu", |h, _| Ok(Val::Bool(item_of(h)?.entry.opens())))
        .get("selected", |h, _| Ok(Val::Bool(item_of(h)?.selected)))
        .set("selected", |h, _, value| {
            let on = value.bool()?;
            h.ui.edit(h.cid, |widget| {
                if let Special::Item(item) = &mut widget.sp {
                    item.selected = on;
                }
            })
        })
        .get("label_text", |h, cx| handout(&h.ui, cx, item_of(h)?.label))
        .method("ink", |h, _, _| {
            let item = item_of(h)?;
            Ok(Val::Text(card::ink(&item.entry, item.selected).to_owned()))
        })
        .method("natural", |h, _, _| Ok(Val::Double(card::natural(&h.ui, h.cid))));

    let refused = |what: &'static str| {
        move |_: &mut Cx<'_>, _: &mut gnoblin_object::server::Inits| -> Res<CanvasHandle> {
            Err(Fail::refused(
                ErrorCode::Unsupported,
                format!("{what} is made by the library when a menu opens"),
            ))
        }
    };
    let panel = builder.class::<CanvasHandle>("menu_panel").ctor(refused("a menu card"));
    let _ = box_members(canvas_members(panel)).get("rows", |h, cx| {
        let rows =
            h.ui.read(h.cid, |widget| match &widget.sp {
                Special::Panel(part) => part.rows.clone(),
                _ => Vec::new(),
            })
            .unwrap_or_default();
        handouts(&h.ui, cx, &rows)
    });

    let menu = builder
        .class::<CanvasHandle>("window_menu")
        .ctor(refused("a window menu"));
    let _ = box_members(canvas_members(menu))
        .get("client", |h, _| Ok(Val::Text(menu_of(h)?.client.to_string())))
        .get("before", |h, _| Ok(Val::Uint(menu_of(h)?.before)))
        .get("nav", |h, cx| cx.handout(menu_of(h)?.nav_obj))
        .get("panel", |h, cx| handout(&h.ui, cx, menu_of(h)?.panel))
        .get("sub", |h, cx| match menu_of(h)?.sub {
            Some(sub) => handout(&h.ui, cx, sub),
            None => Ok(Val::None),
        })
        .method("activate", |h, cx, args| {
            let row = canvas_of(cx, args[0].object()?)?;
            card::activate(&h.ui, h.cid, row);
            Ok(Val::None)
        })
        .method("open_submenu", |h, cx, args| {
            let row = canvas_of(cx, args[0].object()?)?;
            card::open_submenu(&h.ui, h.cid, row);
            Ok(Val::None)
        })
        .method("close_submenu", |h, _, _| {
            card::close_submenu(&h.ui, h.cid);
            Ok(Val::None)
        });
}

// ---- the rules ----------------------------------------------------------------------------------------------------------------------

fn metrics_class(builder: &mut Builder) {
    let _ = builder
        .class::<()>("menu_metrics")
        .function("colours", |_, _| {
            Ok(record(vec![
                Val::Text(metrics::CARD.to_owned()),
                Val::Text(metrics::HOT.to_owned()),
                Val::Text(metrics::TEXT.to_owned()),
                Val::Text(metrics::HOT_TEXT.to_owned()),
                Val::Text(metrics::OFF.to_owned()),
                Val::Text(metrics::LINE.to_owned()),
            ]))
        })
        .function("menu_height", |cx, args| {
            let entries = entry::read_all(cx, &args[0])?;
            Ok(Val::Double(metrics::menu_height(
                entries.iter().map(|entry| entry.separator),
            )))
        })
        .function("place_card", |_, args| {
            let pair = |value: &Val| -> Res<(f64, f64)> {
                match value {
                    Val::Record { fields, .. } if fields.len() >= 2 => Ok((fields[0].double()?, fields[1].double()?)),
                    other => Err(Fail::type_error(format!("expected a pair of numbers, got {other:?}"))),
                }
            };
            let (x, y) = metrics::place_card(pair(&args[0])?, pair(&args[1])?, pair(&args[2])?);
            Ok(record(vec![Val::Double(x), Val::Double(y)]))
        })
        .function("key_action", |_, args| {
            let none = Val::None;
            let made = |place: u32, step: Option<i64>, letter: Option<String>| {
                record(vec![
                    enumeration(place),
                    step.map_or_else(|| none.clone(), Val::Int),
                    letter.map_or_else(|| none.clone(), Val::Text),
                ])
            };
            Ok(match metrics::key_action(args[0].text()?, args[1].text()?) {
                None => Val::None,
                Some(KeyAction::Escape) => made(1, None, None),
                Some(KeyAction::Move(step)) => made(2, Some(step), None),
                Some(KeyAction::Home) => made(3, None, None),
                Some(KeyAction::End) => made(4, None, None),
                Some(KeyAction::Activate) => made(5, None, None),
                Some(KeyAction::Right) => made(6, None, None),
                Some(KeyAction::Left) => made(7, None, None),
                Some(KeyAction::Jump(letter)) => made(8, None, Some(letter)),
            })
        });
}

// ---- the service --------------------------------------------------------------------------------------------------------------------

fn service(builder: &mut Builder) {
    let _ = builder
        .class::<()>("window_menus")
        .ctor(|_, _| Ok(()))
        .get("started", |_, cx| Ok(Val::Bool(ui_of(cx)?.menus.borrow().started)))
        .get("current", |_, cx| {
            let ui = ui_of(cx)?;
            let current = ui.menus.borrow().current;
            match current {
                Some(menu) => handout(&ui, cx, menu),
                None => Ok(Val::None),
            }
        })
        .get("provider", |_, cx| {
            let provider = ui_of(cx)?.menus.borrow().provider.clone();
            Ok(match provider {
                Some(held) if cx.class_of(held.obj).is_ok() => cx.handout(held.obj)?,
                _ => Val::None,
            })
        })
        .set("provider", |_, cx, value| {
            let ui = ui_of(cx)?;
            let next = if value.is_none() {
                None
            } else {
                let obj = value.object()?;
                let shell = cx.shell(obj)?;
                cx.hold(obj)?;
                Some(MenuProviderRef { obj, shell })
            };
            let before = std::mem::replace(&mut ui.menus.borrow_mut().provider, next);
            if let Some(old) = before {
                cx.unhold(old.obj);
            }
            Ok(())
        })
        .get("move_with_keys", |_, cx| {
            Ok(ui_of(cx)?
                .menus
                .borrow()
                .move_with_keys
                .clone()
                .map_or(Val::None, Val::Closure))
        })
        .set("move_with_keys", |_, cx, value| {
            let ui = ui_of(cx)?;
            ui.menus.borrow_mut().move_with_keys = if value.is_none() {
                None
            } else {
                Some(Rc::clone(value.closure()?))
            };
            ui.schedule();
            Ok(())
        })
        .get("resize_with_keys", |_, cx| {
            Ok(ui_of(cx)?
                .menus
                .borrow()
                .resize_with_keys
                .clone()
                .map_or(Val::None, Val::Closure))
        })
        .set("resize_with_keys", |_, cx, value| {
            let ui = ui_of(cx)?;
            ui.menus.borrow_mut().resize_with_keys = if value.is_none() {
                None
            } else {
                Some(Rc::clone(value.closure()?))
            };
            ui.schedule();
            Ok(())
        })
        .method("start", |_, cx, _| {
            let ui = ui_of(cx)?;
            windows::start(&ui, cx);
            Ok(Val::None)
        })
        .method("default_items", |_, cx, args| {
            let ui = ui_of(cx)?;
            let client = client_node(cx.ctx(), "default_items", &args[0])?;
            entry::hand_out(cx, windows::default_entries(&ui, client))
        })
        .method("items", |_, cx, args| {
            let ui = ui_of(cx)?;
            let client = client_node(cx.ctx(), "items", &args[0])?;
            let entries = windows::items_in(&ui, cx, client)?;
            entry::hand_out(cx, entries)
        })
        .method("show", |_, cx, args| {
            let ui = ui_of(cx)?;
            let number = |at: usize| {
                args.get(at)
                    .filter(|value| !value.is_none())
                    .map(Val::double)
                    .transpose()
            };
            let client = client_node(cx.ctx(), "show", &args[0])?;
            let shown = windows::show_in(&ui, cx, client, number(1)?, number(2)?)?;
            shown.map_or(Ok(Val::None), |obj| cx.handout(obj))
        })
        .method("open", |_, cx, args| {
            let ui = ui_of(cx)?;
            let client = client_node(cx.ctx(), "open", &args[0])?;
            let opened = windows::open_in(&ui, cx, client, args[1].double()?, args[2].double()?)?;
            opened.map_or(Ok(Val::None), |obj| cx.handout(obj))
        })
        .method("close", |_, cx, _| {
            let ui = ui_of(cx)?;
            Ok(Val::Bool(windows::close(&ui)))
        })
        .method("is_above", |_, cx, args| {
            let ui = ui_of(cx)?;
            let client = client_node(cx.ctx(), "is_above", &args[0])?;
            Ok(Val::Bool(windows::is_above(&ui, client)))
        })
        .method("set_above", |_, cx, args| {
            let ui = ui_of(cx)?;
            let client = client_node(cx.ctx(), "set_above", &args[0])?;
            windows::set_above(&ui, client, args[1].bool()?);
            Ok(Val::None)
        });
}

pub(super) fn register(builder: &mut Builder) {
    entry::register(builder);
    navigator(builder);
    canvas_classes(builder);
    metrics_class(builder);
    service(builder);
}
