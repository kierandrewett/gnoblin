//! The classes of `shell.toml` as the object protocol serves them.

use std::cell::RefCell;
use std::rc::Rc;

use gnoblin_object::server::Builder;
use gnoblin_object::{Closure, Fail, Res, Val};

use super::alerts::{self, AlertTracker, clean_text, text_node};
use super::{dialog, launcher, overview};
use crate::object::client::client_node;
use crate::object::record::record;
use crate::ui::classes::{Class, box_members, canvas_members, class_with, entry_members};
use crate::ui::tree::ui_of;
use crate::ui::widget::{CanvasHandle, Ui, WidgetKind};

/// A closure a shell gave, or none: a value that is none takes the closure away.
fn closure_of(value: &Val) -> Res<Option<Closure>> {
    if value.is_none() {
        Ok(None)
    } else {
        Ok(Some(Rc::clone(value.closure()?)))
    }
}

fn closure_val(closure: Option<Closure>) -> Val {
    closure.map_or(Val::None, Val::Closure)
}

// ---- the tracker ---------------------------------------------------------------------------------------------------------------

/// The state of an `alert_tracker`.
struct TrackerHandle {
    ui: Rc<Ui>,
    state: Rc<RefCell<AlertTracker>>,
}

fn clients_val(clients: &[u64]) -> Val {
    Val::List(clients.iter().map(|client| Val::Text(client.to_string())).collect())
}

fn tracker_class(builder: &mut Builder) {
    let _ = builder
        .class::<TrackerHandle>("alert_tracker")
        .ctor(|cx, _inits| {
            Ok(TrackerHandle {
                ui: ui_of(cx)?,
                state: Rc::new(RefCell::new(AlertTracker::default())),
            })
        })
        .method("is_open", |h, cx, args| {
            let client = client_node(cx.ctx(), "is_open", &args[0])?;
            Ok(Val::Bool(h.state.borrow().is_open(client)))
        })
        .method("is_waiting", |h, cx, args| {
            let client = client_node(cx.ctx(), "is_waiting", &args[0])?;
            Ok(Val::Bool(h.state.borrow().is_waiting(client)))
        })
        .method("check", |h, _, args| {
            let clients = args[0].list()?.iter().map(text_node).collect::<Res<Vec<u64>>>()?;
            let locked = args[1].bool()?;
            let changes = h.state.borrow_mut().look(&h.ui, &clients, locked);
            Ok(record(vec![clients_val(&changes.open), clients_val(&changes.close)]))
        })
        .method("answered", |h, cx, args| {
            let client = client_node(cx.ctx(), "answered", &args[0])?;
            Ok(Val::Bool(h.state.borrow_mut().answered(client, args[1].enum_value()?)))
        })
        .function("clean_text", |_cx, args| {
            let max = usize::try_from(args[1].uint()?).unwrap_or(usize::MAX);
            Ok(Val::Text(clean_text(args[0].text()?, max)))
        });
}

// ---- the dialog ----------------------------------------------------------------------------------------------------------------

fn not_a<T>(what: &str) -> Res<T> {
    Err(Fail::type_error(format!("the canvas is not {what}")))
}

fn button_class(builder: &mut Builder) {
    fn read<R>(h: &CanvasHandle, pick: impl FnOnce(&super::ButtonPart) -> R) -> Res<R> {
        match dialog::button(&h.ui, h.cid, pick) {
            Some(value) => Ok(value),
            None => not_a("an alert button"),
        }
    }
    let class = builder
        .class::<CanvasHandle>("alert_button")
        .ctor(|_cx, _inits| Err(Fail::refused("an alert button is made by its dialog")));
    let class = box_members(canvas_members(class));
    let _ = class
        .get("label", |h, cx| {
            let label = read(h, |button| button.label)?;
            Ok(match h.ui.read(label, |widget| widget.obj) {
                Some(obj) => cx.handout(obj)?,
                None => Val::None,
            })
        })
        .get("enabled", |h, _| Ok(Val::Bool(read(h, |button| button.enabled)?)))
        .set("enabled", |h, _, v| {
            let enabled = v.bool()?;
            read(h, |_| ())?;
            dialog::change_button(&h.ui, h.cid, |button| button.enabled = enabled);
            h.ui.schedule();
            Ok(())
        })
        .get("picked", |h, _| Ok(Val::Bool(read(h, |button| button.picked)?)))
        .set("picked", |h, _, v| {
            let picked = v.bool()?;
            read(h, |_| ())?;
            dialog::change_button(&h.ui, h.cid, |button| button.picked = picked);
            h.ui.schedule();
            Ok(())
        });
}

fn dialog_class(builder: &mut Builder) {
    fn read<R>(h: &CanvasHandle, pick: impl FnOnce(&super::DialogPart) -> R) -> Res<R> {
        match dialog::read(&h.ui, h.cid, pick) {
            Some(value) => Ok(value),
            None => not_a("a not-responding dialog"),
        }
    }
    let class: Class<'_> = class_with(
        builder,
        "not_responding_dialog",
        WidgetKind::Box,
        |ui, cx, inits, cid| {
            let name = match inits.take("name") {
                Some(value) => value.text()?.to_owned(),
                None => String::new(),
            };
            let me = cx.me();
            dialog::build(ui, cx, me, cid, &name)
        },
    );
    let _ = class
        .get("name", |h, _| Ok(Val::Text(read(h, |card| card.name.clone())?)))
        .get("choices", |h, cx| dialog::choices(&h.ui, cx, h.cid))
        .get("index", |h, _| Ok(Val::Uint(read(h, |card| card.index as u64)?)))
        .get("armed", |h, _| Ok(Val::Bool(read(h, |card| card.armed)?)))
        .method("select", |h, _, args| {
            read(h, |_| ())?;
            let index = usize::try_from(args[0].uint()?).unwrap_or(usize::MAX);
            dialog::select(&h.ui, h.cid, index);
            h.ui.schedule();
            Ok(Val::None)
        })
        .method("arm", |h, _, _| {
            read(h, |_| ())?;
            dialog::arm(&h.ui, h.cid);
            Ok(Val::None)
        })
        .method("activate", |h, _, args| {
            read(h, |_| ())?;
            let index = usize::try_from(args[0].uint()?).unwrap_or(usize::MAX);
            dialog::activate(&h.ui, h.cid, index);
            Ok(Val::None)
        });
}

// ---- the service ---------------------------------------------------------------------------------------------------------------

/// The state of the `not_responding_alerts` service: the generation's toolkit.
struct AlertsHandle {
    ui: Rc<Ui>,
}

fn alerts_class(builder: &mut Builder) {
    let _ = builder
        .class::<AlertsHandle>("not_responding_alerts")
        .ctor(|cx, _inits| {
            let ui = ui_of(cx)?;
            let me = cx.me();
            let state = Rc::clone(&ui.shell.borrow().alerts.tracker);
            let tracker = cx.spawn(
                "alert_tracker",
                TrackerHandle {
                    ui: Rc::clone(&ui),
                    state,
                },
            )?;
            cx.set_parent(tracker, me)?;
            {
                let mut shell = ui.shell.borrow_mut();
                shell.alerts.obj = me;
                shell.alerts.tracker_obj = tracker;
            }
            Ok(AlertsHandle { ui })
        })
        .get("started", |h, _| Ok(Val::Bool(h.ui.shell.borrow().alerts.started)))
        .get("arm_ms", |h, _| Ok(Val::Double(h.ui.shell.borrow().alerts.arm_ms)))
        .set("arm_ms", |h, _, v| {
            h.ui.shell.borrow_mut().alerts.arm_ms = v.double()?;
            h.ui.schedule();
            Ok(())
        })
        .get("interval_ms", |h, _| {
            Ok(Val::Double(h.ui.shell.borrow().alerts.interval_ms))
        })
        .set("interval_ms", |h, _, v| {
            h.ui.shell.borrow_mut().alerts.interval_ms = v.double()?;
            h.ui.schedule();
            Ok(())
        })
        .get("now", |h, _| Ok(closure_val(h.ui.shell.borrow().alerts.now.clone())))
        .set("now", |h, _, v| {
            h.ui.shell.borrow_mut().alerts.now = closure_of(&v)?;
            Ok(())
        })
        .get("close_menu", |h, _| {
            Ok(closure_val(h.ui.shell.borrow().alerts.close_menu.clone()))
        })
        .set("close_menu", |h, _, v| {
            h.ui.shell.borrow_mut().alerts.close_menu = closure_of(&v)?;
            Ok(())
        })
        .get("tracker", |h, cx| cx.handout(h.ui.shell.borrow().alerts.tracker_obj))
        .method("start", |h, _, _| {
            alerts::start(&h.ui)?;
            Ok(Val::None)
        })
        .method("stop", |h, _, _| {
            alerts::stop(&h.ui);
            Ok(Val::None)
        })
        .method("check", |h, _, _| {
            alerts::check(&h.ui);
            Ok(Val::None)
        })
        .method("card_of", |h, cx, args| {
            alerts::card_of(&h.ui, cx, client_node(cx.ctx(), "card_of", &args[0])?)
        });
}

// ---- the launcher and the overview ---------------------------------------------------------------------------------------------

fn launcher_class(builder: &mut Builder) {
    let class = class_with(builder, "launcher", WidgetKind::Entry, |ui, cx, _inits, cid| {
        let me = cx.me();
        launcher::build(ui, cx, me, cid)
    });
    let _ = entry_members(class)
        .get("spawn", |h, _| Ok(closure_val(launcher::spawn_of(&h.ui, h.cid))))
        .set("spawn", |h, _, v| {
            let spawn = closure_of(&v)?;
            launcher::set_spawn(&h.ui, h.cid, spawn);
            Ok(())
        })
        .function("open", |cx, args| {
            let ui = ui_of(cx)?;
            let spawn = args.first().and_then(|value| value.closure().ok()).map(Rc::clone);
            launcher::open(&ui, cx, spawn)
        });
}

/// The state of the `overview` service: the generation's toolkit.
struct OverviewHandle {
    ui: Rc<Ui>,
}

fn overview_class(builder: &mut Builder) {
    let _ = builder
        .class::<OverviewHandle>("overview")
        .ctor(|cx, _inits| Ok(OverviewHandle { ui: ui_of(cx)? }))
        .get("enabled", |h, _| Ok(Val::Bool(h.ui.shell.borrow().overview.on)))
        .get("dimmed_opacity", |h, _| {
            Ok(Val::Double(h.ui.shell.borrow().overview.dimmed))
        })
        .set("dimmed_opacity", |h, _, v| {
            h.ui.shell.borrow_mut().overview.dimmed = v.double()?;
            Ok(())
        })
        .method("toggle", |h, cx, _| {
            let on = overview::toggle(&h.ui)?;
            cx.changed("enabled");
            Ok(Val::Bool(on))
        });
}

/// Implements the classes of the shell module.
pub(crate) fn register(builder: &mut Builder) {
    tracker_class(builder);
    button_class(builder);
    dialog_class(builder);
    alerts_class(builder);
    launcher_class(builder);
    overview_class(builder);
}
