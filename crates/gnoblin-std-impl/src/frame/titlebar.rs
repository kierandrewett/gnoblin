//! The stock titlebar: its style, how it is built from the same parts a shell can use (a drag area, a title and the window buttons), how it
//! follows its window, and `fill`, which puts one in a server frame's top outlet.

use std::borrow::Borrow;
use std::rc::Rc;

use gnoblin_object::server::Cx;
use gnoblin_object::{Fail, Obj, Res, Val};

use super::window_frame::{link_under, outlet};
use super::{BarPart, FrameId, FramePart, FrameSlot, PartRole, Shape, en};
use crate::object::record::{field, number_field, record, text_field};
use crate::ui::widget::{Cid, Special, Ui, WidgetKind, en as ui_en};

/// The icon of each button when a style says nothing: by name from the icon theme, in the order minimise, maximise, close.
const DEFAULT_ICONS: [&str; 3] = [
    "window-minimize-symbolic",
    "window-maximize-symbolic",
    "window-close-symbolic",
];

/// The three window buttons, which a style can order, leave out or give other icons.
pub(crate) const ALL_BUTTONS: [u32; 3] = [en::BUTTON_MINIMISE, en::BUTTON_MAXIMISE, en::BUTTON_CLOSE];

/// The titlebar's appearance and layout. Lengths are logical pixels.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TitlebarStyle {
    pub(crate) height: f64,
    pub(crate) padding: f64,
    pub(crate) gap: f64,
    pub(crate) radius: f64,
    pub(crate) focused_fill: String,
    pub(crate) unfocused_fill: String,
    pub(crate) title_size: f64,
    pub(crate) title_color: String,
    pub(crate) title_font: String,
    pub(crate) buttons: Vec<u32>,
    pub(crate) button_size: f64,
    pub(crate) button_color: String,
    pub(crate) icons: [String; 3],
}

impl Default for TitlebarStyle {
    fn default() -> TitlebarStyle {
        TitlebarStyle {
            height: 28.0,
            padding: 6.0,
            gap: 8.0,
            radius: 0.0,
            focused_fill: "#414559".to_owned(),
            unfocused_fill: "#232634".to_owned(),
            title_size: 13.0,
            title_color: "#c6d0f5".to_owned(),
            title_font: "sans".to_owned(),
            buttons: ALL_BUTTONS.to_vec(),
            button_size: 16.0,
            button_color: "#cdd6f4".to_owned(),
            icons: DEFAULT_ICONS.map(str::to_owned),
        }
    }
}

impl TitlebarStyle {
    /// The icon of the button `name`.
    pub(crate) fn icon(&self, name: u32) -> &str {
        &self.icons[(name as usize).saturating_sub(1).min(2)]
    }
}

/// The icon a button has when nothing says otherwise.
pub(crate) fn icon_default(name: u32) -> &'static str {
    DEFAULT_ICONS[(name as usize).saturating_sub(1).min(2)]
}

/// A `button_icons` value.
pub(crate) fn icons_val(icons: &[String; 3]) -> Val {
    record(icons.iter().cloned().map(Val::Text).collect())
}

/// The icons of a `button_icons` value; one left out is the stock icon.
pub(crate) fn icons_of(value: impl Borrow<Val>) -> Res<[String; 3]> {
    let fields = value.borrow().record()?;
    Ok([
        text_field(fields, 0, DEFAULT_ICONS[0])?,
        text_field(fields, 1, DEFAULT_ICONS[1])?,
        text_field(fields, 2, DEFAULT_ICONS[2])?,
    ])
}

/// A `titlebar_style` value.
pub(crate) fn style_val(style: &TitlebarStyle) -> Val {
    record(vec![
        Val::Double(style.height),
        Val::Double(style.padding),
        Val::Double(style.gap),
        Val::Double(style.radius),
        Val::Text(style.focused_fill.clone()),
        Val::Text(style.unfocused_fill.clone()),
        Val::Double(style.title_size),
        Val::Text(style.title_color.clone()),
        Val::Text(style.title_font.clone()),
        Val::List(
            style
                .buttons
                .iter()
                .map(|name| Val::Enum { class: 0, value: *name })
                .collect(),
        ),
        Val::Double(style.button_size),
        Val::Text(style.button_color.clone()),
        icons_val(&style.icons),
    ])
}

/// The style a `titlebar_style` value says; a field left out is the stock one.
pub(crate) fn style_of(value: impl Borrow<Val>) -> Res<TitlebarStyle> {
    let fields = value.borrow().record()?;
    let stock = TitlebarStyle::default();
    let finite = |at: usize, default: f64, what: &str| -> Res<f64> {
        let number = number_field(fields, at, default)?;
        if number.is_finite() && number >= 0.0 {
            Ok(number)
        } else {
            Err(Fail::type_error(format!(
                "a titlebar style's {what} is a finite number of at least 0"
            )))
        }
    };
    let buttons = match field(fields, 9) {
        Some(list) => list.list()?.iter().map(Val::enum_value).collect::<Res<Vec<u32>>>()?,
        None => stock.buttons.clone(),
    };
    let icons = match field(fields, 12) {
        Some(icons) => icons_of(icons)?,
        None => stock.icons.clone(),
    };
    Ok(TitlebarStyle {
        height: finite(0, stock.height, "height")?,
        padding: finite(1, stock.padding, "padding")?,
        gap: finite(2, stock.gap, "gap")?,
        radius: finite(3, stock.radius, "radius")?,
        focused_fill: text_field(fields, 4, &stock.focused_fill)?,
        unfocused_fill: text_field(fields, 5, &stock.unfocused_fill)?,
        title_size: finite(6, stock.title_size, "title size")?,
        title_color: text_field(fields, 7, &stock.title_color)?,
        title_font: text_field(fields, 8, &stock.title_font)?,
        buttons,
        button_size: finite(10, stock.button_size, "button size")?,
        button_color: text_field(fields, 11, &stock.button_color)?,
        icons,
    })
}

/// The style a titlebar follows: its own, or the decorations'.
pub(crate) fn bar_style(ui: &Ui, bar: Cid) -> TitlebarStyle {
    let own = match super::part_of(ui, bar) {
        Some(FramePart {
            role: PartRole::Titlebar(part),
            ..
        }) => part.style,
        _ => None,
    };
    own.unwrap_or_else(|| ui.frames.borrow().deco.style.clone())
}

/// Updates the bar's size, style, title, focus fill and visibility from its window.
pub(crate) fn refresh(ui: &Rc<Ui>, cid: Cid, frame: FrameId, part: &BarPart) -> Result<(), String> {
    let Some(node) = ui.frame_node(frame) else {
        return Ok(());
    };
    let style = bar_style(ui, cid);
    let win = super::client::Win::of(ui, node);
    let fill = if win.focused() {
        style.focused_fill.clone()
    } else {
        style.unfocused_fill.clone()
    };
    let (width, fullscreen) = (win.width(), win.fullscreen());
    ui.tweak(cid, |widget| {
        widget.height = Some(style.height);
        if widget.parent.is_none() {
            // Mounted on the window's node: above the content, whose origin is the node's corner, and as wide as it.
            widget.width = Some(width);
            widget.x = 0.0;
            widget.y = -style.height;
        } else {
            // In an outlet: the outlet's row places the bar and gives it the room.
            widget.b.grow = 1.0;
        }
        widget.b.padding = style.padding;
        widget.b.gap = style.gap;
        widget.b.radius = style.radius;
        widget.b.fill = fill;
        widget.visible = !fullscreen;
    });
    ui.tweak(part.title, |widget| {
        widget.t.size = style.title_size;
        widget.t.color = style.title_color.clone();
    });
    for (name, button) in &part.buttons {
        ui.tweak(*button, |widget| {
            widget.i.icon = style.icon(*name).to_owned();
            widget.i.size = style.button_size;
            widget.i.color = style.button_color.clone();
        });
    }
    Ok(())
}

/// Makes the parts of the titlebar `bar` (the object `bar.0` and its canvas `bar.1`): a title in a drag area that grows, then the buttons
/// the style names, in the order of the nodes the fixtures hold.
///
/// # Errors
///
/// A canvas cannot be made or put under another.
pub(crate) fn build(
    ui: &Rc<Ui>,
    cx: &mut Cx<'_>,
    frame: FrameId,
    bar: (Obj, Cid),
    style: Option<TitlebarStyle>,
) -> Res<()> {
    let frame_obj = {
        let world = ui.frames.borrow();
        world
            .frames
            .get(&frame)
            .map(|rec| rec.obj)
            .ok_or_else(|| Fail::stale("the frame was destroyed"))?
    };
    let resolved = style.clone().unwrap_or_else(|| ui.frames.borrow().deco.style.clone());
    let drag = ui.frames.borrow_mut().mint_drag(frame);
    let title = super::parts::new_title(ui, cx, frame, frame_obj, &resolved)?;
    let area = ui.spawn_named(cx, "box", WidgetKind::Box, frame_obj)?;
    ui.tweak(area.1, |widget| {
        widget.b.direction = ui_en::DIRECTION_ROW;
        widget.b.grow = 1.0;
    });
    link_under(ui, cx, area, title)?;
    let mut buttons = Vec::with_capacity(resolved.buttons.len());
    let mut made = Vec::with_capacity(resolved.buttons.len());
    ui.tweak(bar.1, |widget| {
        widget.b.direction = ui_en::DIRECTION_ROW;
        widget.height = Some(resolved.height);
        widget.b.padding = resolved.padding;
        widget.b.gap = resolved.gap;
        widget.b.radius = resolved.radius;
    });
    link_under(ui, cx, bar, area)?;
    for name in &resolved.buttons {
        let button = super::parts::new_button(
            ui,
            cx,
            frame,
            frame_obj,
            *name,
            resolved.icon(*name),
            resolved.button_size,
            &resolved.button_color,
        )?;
        buttons.push((*name, button.1));
        made.push(button);
    }
    ui.tweak(bar.1, |widget| {
        widget.sp = Special::Frame(FramePart {
            frame,
            role: PartRole::Titlebar(BarPart {
                style,
                area: area.1,
                title: title.1,
                buttons,
            }),
            drag: Some(drag),
        });
    });
    for button in made {
        link_under(ui, cx, bar, button)?;
    }
    Ok(())
}

/// Adds a titlebar to the top outlet of a server frame, and sets the outlet's height to the style's. `false` for a frame the client draws.
///
/// # Errors
///
/// The titlebar cannot be made or put in the outlet.
pub(crate) fn fill(ui: &Rc<Ui>, cx: &mut Cx<'_>, frame: FrameId, style: Option<TitlebarStyle>) -> Res<bool> {
    if ui.frame_kind(frame) != Some(super::FrameKind::Server) {
        return Ok(false);
    }
    let frame_obj = ui
        .frames
        .borrow()
        .frames
        .get(&frame)
        .map(|rec| rec.obj)
        .ok_or_else(|| Fail::stale("the frame was destroyed"))?;
    let bar = ui.spawn_named(cx, "titlebar", WidgetKind::Box, frame_obj)?;
    // The bar is made before the outlet it goes in: the order of the nodes is the fixtures'.
    ui.tweak(bar.1, |widget| widget.b.direction = ui_en::DIRECTION_ROW);
    build(ui, cx, frame, bar, style.clone())?;
    let height = style.unwrap_or_else(|| ui.frames.borrow().deco.style.clone()).height;
    let top = outlet(ui, cx, frame, FrameSlot::Top)?;
    ui.tweak_outlet(top.1, |held| {
        if let Shape::Edge(edge) = &mut held.shape {
            edge.size = Some(height);
            edge.bar = Some(bar.1);
        }
    });
    link_under(ui, cx, top, bar)?;
    Ok(true)
}
