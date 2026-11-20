//! The title text and the window buttons.

use std::rc::Rc;

use gnoblin_object::server::Cx;
use gnoblin_object::{Obj, Res};

use super::super::client::Win;
use super::super::drag::LEFT;
use super::super::titlebar::{TitlebarStyle, icon_default};
use super::super::window_frame::link_under;
use super::super::{ButtonPart, ButtonsPart, FrameId, FramePart, PartRole, actions, en};
use crate::ui::widget::{Cid, Special, Ui, WidgetKind, en as ui_en};

/// Makes a title text under `owner` that follows `frame`'s window: it takes the room a row leaves (`grow` 1) and is one line.
///
/// # Errors
///
/// The canvas cannot be made.
pub(crate) fn new_title(
    ui: &Rc<Ui>,
    cx: &mut Cx<'_>,
    frame: FrameId,
    owner: Obj,
    style: &TitlebarStyle,
) -> Res<(Obj, Cid)> {
    let made = ui.spawn_named(cx, "title_text", WidgetKind::Text, owner)?;
    init_title(ui, made.1, frame, style);
    Ok(made)
}

/// Gives a canvas the title's defaults: `style`'s size, colour and font, and it belongs to `frame`.
pub(crate) fn init_title(ui: &Ui, cid: Cid, frame: FrameId, style: &TitlebarStyle) {
    ui.tweak(cid, |widget| {
        widget.b.grow = 1.0;
        widget.t.size = style.title_size;
        widget.t.color = style.title_color.clone();
        widget.t.font = style.title_font.clone();
        widget.t.ellipsize = true;
        widget.sp = Special::Frame(FramePart {
            frame,
            role: PartRole::TitleText,
            drag: None,
        });
    });
}

/// Gives a canvas a window button's look and behaviour: the icon, size and colour, and which action it runs.
pub(crate) fn init_button(ui: &Ui, cid: Cid, frame: FrameId, name: u32, icon: &str, size: f64, color: &str) {
    ui.tweak(cid, |widget| {
        widget.i.icon = icon.to_owned();
        widget.i.size = size;
        widget.i.color = color.to_owned();
        widget.sp = Special::Frame(FramePart {
            frame,
            role: PartRole::Button(ButtonPart {
                name,
                restore_icon: None,
                base_icon: icon.to_owned(),
            }),
            drag: None,
        });
    });
}

/// Makes a window button under `owner`.
///
/// # Errors
///
/// The canvas cannot be made.
#[allow(clippy::too_many_arguments)]
pub(crate) fn new_button(
    ui: &Rc<Ui>,
    cx: &mut Cx<'_>,
    frame: FrameId,
    owner: Obj,
    name: u32,
    icon: &str,
    size: f64,
    color: &str,
) -> Res<(Obj, Cid)> {
    let made = ui.spawn_named(cx, "window_button", WidgetKind::Icon, owner)?;
    init_button(ui, made.1, frame, name, icon, size, color);
    Ok(made)
}

/// Fills the row `row` (the object `row.0`, its canvas `row.1`) with the buttons `names`, left to right, with the stock icons.
///
/// # Errors
///
/// A button cannot be made or put in the row.
pub(crate) fn new_buttons(ui: &Rc<Ui>, cx: &mut Cx<'_>, frame: FrameId, row: (Obj, Cid), names: Vec<u32>) -> Res<()> {
    ui.tweak(row.1, |widget| {
        widget.b.direction = ui_en::DIRECTION_ROW;
        widget.sp = Special::Frame(FramePart {
            frame,
            role: PartRole::Buttons(ButtonsPart {
                names: names.clone(),
                icons: Vec::new(),
                size: 16.0,
                color: "#cdd6f4".to_owned(),
                restore_icon: None,
            }),
            drag: None,
        });
    });
    for name in names {
        let button = new_button(ui, cx, frame, row.0, name, icon_default(name), 16.0, "#cdd6f4")?;
        link_under(ui, cx, row, button)?;
    }
    Ok(())
}

/// Changes what the buttons of a row show: the icons by name, the size and colour, and the maximise button's restore icon.
pub(crate) fn set_button_icons(ui: &Ui, row: Cid, change: impl FnOnce(&mut ButtonsPart)) {
    ui.tweak(row, |widget| {
        if let Special::Frame(FramePart {
            role: PartRole::Buttons(part),
            ..
        }) = &mut widget.sp
        {
            change(part);
        }
    });
    let Some((part, children)) = ui
        .read(row, |widget| match &widget.sp {
            Special::Frame(FramePart {
                role: PartRole::Buttons(part),
                ..
            }) => Some((part.clone(), widget.children.clone())),
            _ => None,
        })
        .flatten()
    else {
        return;
    };
    for child in children {
        ui.tweak(child, |widget| {
            if let Special::Frame(FramePart {
                role: PartRole::Button(button),
                ..
            }) = &mut widget.sp
            {
                widget.i.icon = part
                    .icons
                    .iter()
                    .find(|(name, _)| *name == button.name)
                    .map_or_else(|| icon_default(button.name).to_owned(), |(_, icon)| icon.clone());
                button.base_icon = widget.i.icon.clone();
                button.restore_icon = part.restore_icon.clone();
                widget.i.size = part.size;
                widget.i.color = part.color.clone();
            }
        });
    }
}

/// Updates the maximise icon when the window's maximised state changes.
pub(super) fn refresh(ui: &Ui, cid: Cid, win: &Win<'_>, button: &ButtonPart) {
    if button.name != en::BUTTON_MAXIMISE {
        return;
    }
    let Some(restore) = &button.restore_icon else {
        return;
    };
    let icon = if win.maximised() {
        restore.clone()
    } else {
        button.base_icon.clone()
    };
    ui.tweak(cid, |widget| widget.i.icon = icon);
}

/// The room a row of buttons takes: as wide as its buttons and gaps make it, and as high as the highest.
pub(super) fn natural(ui: &Ui, cid: Cid, vertical: bool) -> f64 {
    let (gap, padding, children) = ui
        .read(cid, |widget| (widget.b.gap, widget.b.padding, widget.children.clone()))
        .unwrap_or((0.0, 0.0, Vec::new()));
    if vertical {
        children.iter().map(|child| ui.height_of(*child)).fold(0.0, f64::max) + 2.0 * padding
    } else {
        let widths: f64 = children.iter().map(|child| ui.width_of(*child)).sum();
        widths + gap * (children.len().saturating_sub(1) as f64) + 2.0 * padding
    }
}

/// A press on a window button: a left press does what the button says through the window's actions. The press never reaches the
/// canvas under the button, so a button in a drag area is not a drag.
pub(super) fn press(ui: &Rc<Ui>, frame: FrameId, name: u32, button: u64, title: &str) -> bool {
    if button != LEFT {
        return true;
    }
    let done = match name {
        en::BUTTON_CLOSE => actions::close(ui, frame),
        en::BUTTON_MAXIMISE => actions::toggle_maximise(ui, frame),
        _ => actions::minimise(ui, frame),
    };
    if let Err(message) = done {
        ui.warn(&format!("{title}.onPress threw: {message}"));
    }
    true
}
