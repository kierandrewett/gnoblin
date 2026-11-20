//! The application's tile: a rounded square with the application's first letter on a colour taken from its id, so the same application is
//! the same tile in every window and every session. The painter has no icon theme.

use gnoblin_paint::PaintOp;

use super::super::client::Win;
use super::super::{AppIconPart, FramePart, PartRole};
use crate::ui::paint;
use crate::ui::widget::{Cid, Special, Ui};

/// The colours a tile is taken from, all dark enough for white lettering.
const TILE_COLOURS: [&str; 8] = [
    "#5b6ee1", "#2f8f6b", "#b5533c", "#8a56ac", "#2d7d9a", "#a0762a", "#a8416a", "#4f7b38",
];

/// The letter and colour of an application id: the last part of `org.gnome.Nautilus` is `Nautilus`, so `N`.
pub(crate) fn tile_of(app_id: &str) -> (String, &'static str) {
    let name = app_id.split('.').rfind(|part| !part.is_empty()).unwrap_or("");
    let mut hash: u32 = 0;
    for ch in app_id.chars() {
        hash = hash.wrapping_mul(31).wrapping_add(ch as u32);
    }
    let letter = name
        .chars()
        .next()
        .map_or_else(|| "?".to_owned(), |first| first.to_uppercase().collect());
    (letter, TILE_COLOURS[hash as usize % TILE_COLOURS.len()])
}

/// Updates the letter and the tile colour from the window's current application id.
pub(super) fn refresh(ui: &Ui, cid: Cid, win: &Win<'_>) {
    let app_id = win.app_id();
    let (_, colour) = tile_of(&app_id);
    ui.tweak(cid, |widget| {
        widget.b.fill = colour.to_owned();
        if let Special::Frame(FramePart {
            role: PartRole::AppIcon(part),
            ..
        }) = &mut widget.sp
        {
            part.app_id = app_id;
        }
    });
}

/// Draws the tile and the application's initial.
pub(super) fn paint(ui: &Ui, cid: Cid, part: &AppIconPart) -> Result<Vec<PaintOp>, String> {
    let (width, height) = (ui.width_of(cid), ui.height_of(cid));
    let size = (width.min(height) * 0.62).floor();
    let mut ops = ui.box_ops(cid)?;
    ops.push(paint::text(
        ((width - (size * 0.6).floor()) / 2.0).floor(),
        ((height - (size * 1.5).floor()) / 2.0).floor(),
        size,
        &part.color,
        "sans",
        &tile_of(&part.app_id).0,
    )?);
    Ok(ops)
}
