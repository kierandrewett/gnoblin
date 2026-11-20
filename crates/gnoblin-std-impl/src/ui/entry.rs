//! `entry`: a line of text you type into.
//!
//! The entry edits an `entry_model` object it owns (`object/entry_model.rs`); it draws the text, the selection and the caret from the
//! model and from the width of the text before them, which three hidden texts under it measure. Give it the keyboard and it reads the
//! keys: letters type, BackSpace and Delete remove, Left, Right, Home and End move the caret (with Shift held they select),
//! Control+A selects all, and Return calls `on_submit` with the text. Each key it uses it stops, so a parent does not hear it too.

use std::rc::Rc;

use gnoblin_object::{Obj, Res, Val};
use gnoblin_paint::PaintOp;

use super::input::CanvasEvent;
use super::paint;
use super::widget::{Cid, EntryPart, Special, Ui, WidgetKind};
use crate::object::{entry_model::EntryModel, with_lib};

/// The entry's part of a canvas.
fn part(ui: &Ui, cid: Cid) -> Option<EntryPart> {
    ui.read(cid, |widget| match &widget.sp {
        Special::Entry(part) => Some((**part).clone()),
        _ => None,
    })
    .flatten()
}

/// What an entry has by the time its object is made: the model it edits and the three hidden texts it measures with.
///
/// # Errors
///
/// A canvas cannot be made.
pub(crate) fn build(ui: &Rc<Ui>, cx: &mut gnoblin_object::server::Cx<'_>, me: Obj, cid: Cid) -> Res<()> {
    let model = cx.spawn("entry_model", EntryModel::empty())?;
    cx.set_parent(model, me)?;
    // Hidden texts the painter measures: the whole line, what lies before the caret, and before the anchor.
    let mut probes = [0; 3];
    for probe in &mut probes {
        let (_, made) = ui.spawn(cx, WidgetKind::Text, me)?;
        ui.tweak(made, |widget| widget.visible = false);
        *probe = made;
    }
    for probe in probes {
        ui.link(cid, probe)?;
    }
    let [line, head, mark] = probes;
    ui.tweak(cid, |widget| {
        widget.b.padding = 12.0;
        widget.sp = Special::Entry(Box::new(EntryPart {
            model,
            line,
            head,
            mark,
            secret: false,
            size: 14.0,
            color: "#cdd6f4".to_owned(),
            font: "sans".to_owned(),
            selection_fill: "#585b70".to_owned(),
            on_submit: None,
            shift: false,
            control: false,
        }));
    });
    Ok(())
}

/// What is drawn for `text`: the text itself, or a `*` for each character of a secret.
pub(crate) fn shown(secret: bool, text: &str) -> String {
    if secret {
        "*".repeat(text.chars().count())
    } else {
        text.to_owned()
    }
}

/// The model's text.
pub(crate) fn model_text(ui: &Ui, model: Obj) -> String {
    let ids = ui.ids().model;
    let ctx = ui.ctx;
    with_lib(|lib| lib.get(ctx, model, ids.text))
        .ok()
        .and_then(|value| value.text().map(str::to_owned).ok())
        .unwrap_or_default()
}

/// A number the model holds: its length, its caret or its anchor.
fn model_number(ui: &Ui, model: Obj, pick: impl FnOnce(&super::widget::ModelIds) -> u32) -> u64 {
    let member = pick(&ui.ids().model);
    let ctx = ui.ctx;
    with_lib(|lib| lib.get(ctx, model, member))
        .ok()
        .and_then(|value| value.uint().ok())
        .unwrap_or(0)
}

/// The text before the position `at`.
fn model_before(ui: &Ui, model: Obj, at: u64) -> String {
    let member = ui.ids().model.before;
    let ctx = ui.ctx;
    let at = i64::try_from(at).unwrap_or(i64::MAX);
    with_lib(|lib| lib.call(ctx, model, member, vec![Val::Int(at)]))
        .ok()
        .and_then(|value| value.text().map(str::to_owned).ok())
        .unwrap_or_default()
}

/// Calls a method of the model.
fn model_call(ui: &Ui, model: Obj, pick: impl FnOnce(&super::widget::ModelIds) -> u32, args: Vec<Val>) {
    let member = pick(&ui.ids().model);
    let ctx = ui.ctx;
    if let Err(fail) = with_lib(|lib| lib.call(ctx, model, member, args)) {
        ui.warn(&format!("Entry: the model refused: {fail}"));
    }
}

/// The model's text, caret and anchor.
fn snapshot(ui: &Ui, model: Obj) -> (String, u64, u64) {
    (
        model_text(ui, model),
        model_number(ui, model, |ids| ids.caret),
        model_number(ui, model, |ids| ids.anchor),
    )
}

/// Updates the hidden measuring texts from the model.
pub(crate) fn refresh(ui: &Rc<Ui>, cid: Cid) -> Result<(), String> {
    let Some(part) = part(ui, cid) else {
        return Ok(());
    };
    let (text, caret, anchor) = snapshot(ui, part.model);
    for probe in [part.line, part.head, part.mark] {
        ui.tweak(probe, |widget| {
            widget.t.size = part.size;
            widget.t.font.clone_from(&part.font);
        });
    }
    let before_caret = model_before(ui, part.model, caret);
    let before_anchor = model_before(ui, part.model, anchor);
    ui.tweak(part.line, |widget| widget.t.text = shown(part.secret, &text));
    ui.tweak(part.head, |widget| widget.t.text = shown(part.secret, &before_caret));
    ui.tweak(part.mark, |widget| widget.t.text = shown(part.secret, &before_anchor));
    Ok(())
}

/// What an entry paints: its background, the selection's highlight, the text, and the caret when it holds the keyboard.
pub(crate) fn ops(ui: &Ui, cid: Cid) -> Result<Vec<PaintOp>, String> {
    let mut ops = ui.box_ops(cid)?;
    let Some(part) = part(ui, cid) else {
        return Ok(ops);
    };
    let padding = ui.read(cid, |widget| widget.b.padding).unwrap_or(0.0);
    let height = ui.height_of(cid);
    let line = ui.metrics(part.line);
    let top = ((height - line.height) / 2.0).floor().max(0.0);
    let caret_x = padding + ui.metrics(part.head).width;
    let (text, caret, anchor) = snapshot(ui, part.model);
    if anchor != caret {
        let mark_x = padding + ui.metrics(part.mark).width;
        let left = caret_x.min(mark_x);
        ops.push(paint::rect(
            left,
            top,
            (caret_x - mark_x).abs(),
            line.height,
            0.0,
            &part.selection_fill,
        )?);
    }
    if !text.is_empty() {
        ops.push(paint::text(
            padding,
            top,
            part.size,
            &part.color,
            &part.font,
            &shown(part.secret, &text),
        )?);
    }
    if ui.focused(cid) {
        ops.push(paint::line(caret_x, top, caret_x, top + line.height, 1.5, &part.color)?);
    }
    Ok(ops)
}

/// Whether a key's text may be typed: not a control character.
fn typed(text: &str) -> bool {
    text.chars().any(|c| {
        let code = u32::from(c);
        code >= 0x20 && code != 0x7f && !(0x80..=0x9f).contains(&code)
    })
}

/// What an entry does with a key by itself. Answers whether it used the key, which stops the key there.
pub(crate) fn on_key(ui: &Rc<Ui>, cid: Cid, raw: &CanvasEvent) -> bool {
    let Some(entry) = part(ui, cid) else {
        return false;
    };
    // A modifier's own events say whether Shift or Control is held for the keys that follow.
    match raw.key.as_str() {
        "Shift_L" | "Shift_R" => {
            ui.tweak(cid, |widget| {
                if let Special::Entry(part) = &mut widget.sp {
                    part.shift = raw.pressed;
                }
            });
            return false;
        }
        "Control_L" | "Control_R" => {
            ui.tweak(cid, |widget| {
                if let Special::Entry(part) = &mut widget.sp {
                    part.control = raw.pressed;
                }
            });
            return false;
        }
        _ => {}
    }
    if !raw.pressed {
        // The key coming up types nothing.
        return false;
    }
    let model = entry.model;
    let extend = Val::Bool(entry.shift);
    let before = snapshot(ui, model);
    match raw.key.as_str() {
        "Return" | "KP_Enter" => {
            let text = before.0.clone();
            if let Some(submit) = &entry.on_submit
                && let Err(fail) = submit.call(0, &[Val::Text(text.clone())])
            {
                ui.warn(&format!("Entry.on_key threw: {fail}"));
            }
            if let Some(obj) = ui.read(cid, |widget| widget.obj) {
                ui.announce(obj, |cx| cx.emit("submit", vec![Val::Text(text)]));
            }
        }
        "BackSpace" => model_call(ui, model, |ids| ids.backspace, Vec::new()),
        "Delete" | "KP_Delete" => model_call(ui, model, |ids| ids.delete, Vec::new()),
        "Left" => model_call(ui, model, |ids| ids.left, vec![extend]),
        "Right" => model_call(ui, model, |ids| ids.right, vec![extend]),
        "Home" => model_call(ui, model, |ids| ids.home, vec![extend]),
        "End" => model_call(ui, model, |ids| ids.end, vec![extend]),
        key => {
            if entry.control && (key == "a" || key == "A") {
                model_call(ui, model, |ids| ids.select_all, Vec::new());
            } else if entry.control || raw.text.is_empty() || !typed(&raw.text) {
                return false;
            } else {
                model_call(ui, model, |ids| ids.insert, vec![Val::Text(raw.text.clone())]);
            }
        }
    }
    if snapshot(ui, model) != before
        && let Some(obj) = ui.read(cid, |widget| widget.obj)
    {
        ui.announce(obj, |cx| {
            cx.changed("text");
            cx.emit("changed", Vec::new());
        });
    }
    ui.schedule();
    true
}
