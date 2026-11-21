//! The alert's card: what is wrong, and Wait and Force quit.
//!
//! The card is a column of three lines of text and a row of two buttons. Escape waits. Tab, Left and Right move between the buttons and
//! Return or Space presses the one the keyboard is on. Force quit ignores a press until the card is armed, so input already in flight
//! cannot press it. A press on the card or on a button does not reach what lies under it.

use std::rc::Rc;

use gnoblin_object::server::Cx;
use gnoblin_object::{Obj, Res, Val};

use super::alerts::{self, clean_text};
use super::{ButtonPart, DialogPart, ShellPart, answer};
use crate::ui::CanvasEvent;
use crate::ui::classes::enumeration;
use crate::ui::widget::{Cid, Special, Ui, WidgetKind, en};

/// The card's width in pixels.
const WIDTH: f64 = 480.0;
/// The space inside the card, in pixels.
const PAD: f64 = 20.0;
/// The space between the lines and the buttons, in pixels.
const GAP: f64 = 10.0;
/// The buttons' height in pixels.
const BUTTON_H: f64 = 44.0;

/// The canvases of the card's lines of text: what each says, its size, its colour and its height.
fn lines(name: &str) -> [(String, f64, &'static str, f64); 3] {
    [
        (
            clean_text(&format!("{name} is not responding"), 80),
            17.0,
            "#cdd6f4",
            26.0,
        ),
        (
            "You can wait for it, or force it to quit.".to_owned(),
            14.0,
            "#cdd6f4",
            20.0,
        ),
        (
            "Anything it has not saved will be lost.".to_owned(),
            14.0,
            "#a6adc8",
            20.0,
        ),
    ]
}

/// Makes the three lines of text, owned by `owner`.
fn rows(ui: &Rc<Ui>, cx: &mut Cx<'_>, owner: Obj, name: &str) -> Res<Vec<(Obj, Cid, f64)>> {
    let mut made = Vec::new();
    for (text, size, color, height) in lines(name) {
        let (obj, cid) = ui.spawn(cx, WidgetKind::Text, owner)?;
        ui.tweak(cid, |widget| {
            widget.t.text = text;
            widget.t.size = size;
            widget.t.color = color.to_owned();
            widget.height = Some(height);
        });
        made.push((obj, cid, height));
    }
    Ok(made)
}

/// Puts the lines and the buttons in the card `dialog` (its object and its canvas), and sizes and places it on `screen`.
fn assemble(
    ui: &Rc<Ui>,
    cx: &mut Cx<'_>,
    dialog: (Obj, Cid),
    rows: &[(Obj, Cid, f64)],
    secure: Option<bool>,
    screen: Option<(f64, f64)>,
    part: (&str, Option<u64>),
) -> Res<()> {
    let height: f64 = rows.iter().map(|row| row.2 + GAP).sum();
    let total = 2.0 * PAD + height + BUTTON_H;
    let (x, y) = screen.map_or((0.0, 0.0), |(width, tall)| {
        (((width - WIDTH) / 2.0).floor(), ((tall - total) / 4.0).floor().max(0.0))
    });
    ui.tweak(dialog.1, |widget| {
        widget.x = x;
        widget.y = y;
        widget.width = Some(WIDTH);
        widget.height = Some(total);
        widget.b.radius = 16.0;
        widget.b.fill = "#1e1e2e".to_owned();
        widget.b.padding = PAD;
        widget.b.gap = GAP;
        widget.b.direction = en::DIRECTION_COLUMN;
        if let Some(secure) = secure {
            widget.secure = secure;
        }
    });
    for row in rows {
        ui.link(dialog.1, row.1)?;
        cx.set_parent(row.0, dialog.0)?;
    }
    let (row_obj, row_cid) = ui.spawn(cx, WidgetKind::Box, dialog.0)?;
    ui.tweak(row_cid, |widget| {
        widget.b.direction = en::DIRECTION_ROW;
        widget.b.gap = 12.0;
        widget.height = Some(BUTTON_H);
    });
    let mut buttons = Vec::new();
    for (index, label) in ["Wait", "Force quit"].into_iter().enumerate() {
        let (button_obj, button) = ui.spawn_named(cx, "alert_button", WidgetKind::Box, dialog.0)?;
        let (_, text) = ui.spawn(cx, WidgetKind::Text, button_obj)?;
        ui.tweak(text, |widget| widget.t.text = label.to_owned());
        ui.link(button, text)?;
        ui.tweak(button, |widget| {
            widget.b.radius = 10.0;
            widget.height = Some(BUTTON_H);
            widget.b.grow = 1.0;
            widget.sp = Special::Shell(ShellPart::Button(ButtonPart {
                dialog: dialog.1,
                index,
                label: text,
                // Force quit is off until the alert is armed, and Wait is where the keyboard starts.
                enabled: index == 0,
                picked: index == 0,
            }));
        });
        buttons.push((button_obj, button));
    }
    for (button_obj, button) in &buttons {
        ui.link(row_cid, *button)?;
        cx.set_parent(*button_obj, row_obj)?;
    }
    ui.link(dialog.1, row_cid)?;
    let (name, client) = part;
    ui.tweak(dialog.1, |widget| {
        widget.sp = Special::Shell(ShellPart::Dialog(DialogPart {
            name: name.to_owned(),
            index: 0,
            armed: false,
            buttons: buttons.iter().map(|(_, cid)| *cid).collect(),
            client,
        }));
    });
    ui.schedule();
    Ok(())
}

/// Makes the card for the window the service opens it for: the lines first, then the card, as the TypeScript card was made. The card is owned
/// by `owner`.
///
/// # Errors
///
/// A canvas cannot be made or put in another.
pub(crate) fn make(
    ui: &Rc<Ui>,
    cx: &mut Cx<'_>,
    owner: Obj,
    name: &str,
    secure: bool,
    screen: Option<(f64, f64)>,
    client: Option<u64>,
) -> Res<(Obj, Cid)> {
    let lines = rows(ui, cx, owner, name)?;
    let dialog = ui.spawn_named(cx, "not_responding_dialog", WidgetKind::Box, owner)?;
    assemble(ui, cx, dialog, &lines, Some(secure), screen, (name, client))?;
    Ok(dialog)
}

/// Fills a card a shell made with its constructor: `me` is the object and `cid` the canvas the constructor made.
///
/// # Errors
///
/// A canvas cannot be made or put in another.
pub(crate) fn build(ui: &Rc<Ui>, cx: &mut Cx<'_>, me: Obj, cid: Cid, name: &str) -> Res<()> {
    let lines = rows(ui, cx, me, name)?;
    assemble(ui, cx, (me, cid), &lines, None, ui.primary_screen(), (name, None))
}

fn part(ui: &Ui, cid: Cid) -> Option<DialogPart> {
    match super::part_of(ui, cid) {
        Some(ShellPart::Dialog(part)) => Some(part),
        _ => None,
    }
}

/// AlertChanges what a card keeps.
fn change(ui: &Ui, cid: Cid, change: impl FnOnce(&mut DialogPart)) {
    ui.tweak(cid, |widget| {
        if let Special::Shell(ShellPart::Dialog(part)) = &mut widget.sp {
            change(part);
        }
    });
}

/// What a card keeps, for the class's members.
pub(crate) fn read<R>(ui: &Ui, cid: Cid, read: impl FnOnce(&DialogPart) -> R) -> Option<R> {
    part(ui, cid).map(|held| read(&held))
}

/// Whether Force quit takes a press.
pub(crate) fn is_armed(ui: &Ui, cid: Cid) -> bool {
    part(ui, cid).is_some_and(|held| held.armed)
}

/// Puts the keyboard on a button.
pub(crate) fn select(ui: &Ui, cid: Cid, index: usize) {
    let Some(held) = part(ui, cid) else {
        return;
    };
    change(ui, cid, |card| card.index = index);
    for (at, button) in held.buttons.iter().enumerate() {
        ui.tweak(*button, |widget| {
            if let Special::Shell(ShellPart::Button(part)) = &mut widget.sp {
                part.picked = at == index;
            }
        });
    }
}

/// Lets Force quit be pressed.
pub(crate) fn arm(ui: &Ui, cid: Cid) {
    let Some(held) = part(ui, cid) else {
        return;
    };
    if held.armed {
        return;
    }
    change(ui, cid, |card| card.armed = true);
    if let Some(force) = held.buttons.get(1) {
        ui.tweak(*force, |widget| {
            if let Special::Shell(ShellPart::Button(part)) = &mut widget.sp {
                part.enabled = true;
            }
        });
    }
    ui.schedule();
}

/// Presses a button, unless it is Force quit and the card is not armed.
pub(crate) fn activate(ui: &Rc<Ui>, cid: Cid, index: usize) {
    let Some(held) = part(ui, cid) else {
        return;
    };
    if index == 1 && !held.armed {
        return;
    }
    chosen(ui, cid, if index == 0 { answer::WAIT } else { answer::FORCE });
}

/// The user chose: the card's signal tells it, and the service that opened the card ends the alert.
pub(crate) fn chosen(ui: &Rc<Ui>, cid: Cid, choice: u32) {
    let Some(held) = part(ui, cid) else {
        return;
    };
    if let Some(obj) = ui.read(cid, |widget| widget.obj) {
        ui.announce(obj, |cx| cx.emit("chosen", vec![enumeration(choice)]));
    }
    if let Some(client) = held.client {
        alerts::finish(ui, client, choice);
    }
}

/// What the keyboard does to the card. Answers whether it used the key, which stops it there.
pub(crate) fn key(ui: &Rc<Ui>, cid: Cid, raw: &CanvasEvent) -> bool {
    if !raw.pressed {
        return false;
    }
    let Some(held) = part(ui, cid) else {
        return false;
    };
    match raw.key.as_str() {
        "Escape" => chosen(ui, cid, answer::WAIT),
        "Tab" | "ISO_Left_Tab" | "Left" | "Right" => select(ui, cid, (held.index + 1) % 2),
        "Return" | "KP_Enter" | "space" => activate(ui, cid, held.index),
        _ => return false,
    }
    ui.schedule();
    true
}

/// A button's colours follow whether it is enabled and whether the keyboard is on it.
pub(crate) fn refresh_button(ui: &Ui, cid: Cid, button: &ButtonPart) {
    let (fill, color) = if !button.enabled {
        ("#2a2a3a", "#6c7086")
    } else if button.picked {
        ("#89b4fa", "#11111b")
    } else {
        ("#45475a", "#cdd6f4")
    };
    ui.tweak(cid, |widget| widget.b.fill = fill.to_owned());
    ui.tweak(button.label, |widget| widget.t.color = color.to_owned());
}

/// A button puts its label in the middle.
pub(crate) fn arrange_button(ui: &Ui, cid: Cid, button: &ButtonPart) {
    let (width, height) = (ui.width_of(cid), ui.height_of(cid));
    let (label_width, label_height) = (ui.width_of(button.label), ui.height_of(button.label));
    ui.tweak(button.label, |widget| {
        widget.x = ((width - label_width) / 2.0).floor();
        widget.y = ((height - label_height) / 2.0).floor();
    });
}

/// What a button holds, for the class's members.
pub(crate) fn button<R>(ui: &Ui, cid: Cid, read: impl FnOnce(&ButtonPart) -> R) -> Option<R> {
    match super::part_of(ui, cid) {
        Some(ShellPart::Button(held)) => Some(read(&held)),
        _ => None,
    }
}

/// AlertChanges what a button holds.
pub(crate) fn change_button(ui: &Ui, cid: Cid, change: impl FnOnce(&mut ButtonPart)) {
    ui.tweak(cid, |widget| {
        if let Special::Shell(ShellPart::Button(part)) = &mut widget.sp {
            change(part);
        }
    });
}

/// The dialog's `choices`: the objects of its buttons.
pub(crate) fn choices(ui: &Ui, cx: &Cx<'_>, cid: Cid) -> Res<Val> {
    let buttons = read(ui, cid, |held| held.buttons.clone()).unwrap_or_default();
    let mut list = Vec::with_capacity(buttons.len());
    for button in buttons {
        if let Some(obj) = ui.read(button, |widget| widget.obj) {
            list.push(cx.handout(obj)?);
        }
    }
    Ok(Val::List(list))
}
