//! What paints round a frame: the ring over its edge and the halo behind it.

use gnoblin_paint::PaintOp;

use super::super::client::Win;
use super::super::{BorderPart, FrameId, GlowPart};
use crate::ui::paint;
use crate::ui::widget::{Cid, Ui};

/// Two hex digits of `alpha`, from 0 to 1.
fn alpha_byte(alpha: f64) -> String {
    format!("{:02x}", (alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// The colour of a ring now: the shell's function of the window when it gave one, else the colour it has.
fn colour_of(ui: &Ui, frame: FrameId, part: &BorderPart) -> Result<String, String> {
    let (Some(of), Some(node)) = (&part.color_of, ui.frame_node(frame)) else {
        return Ok(part.color.clone());
    };
    of.call(0, &[Win::of(ui, node).record()])
        .and_then(|value| value.text().map(str::to_owned))
        .map_err(|fail| fail.message)
}

/// Draws the ring along the edge of the canvas, `thickness` wide and inside it.
pub(super) fn border(ui: &Ui, cid: Cid, frame: FrameId, part: &BorderPart) -> Result<Vec<PaintOp>, String> {
    let line = part.thickness;
    let colour = colour_of(ui, frame, part)?;
    Ok(vec![paint::outline(
        (
            line / 2.0,
            line / 2.0,
            ui.width_of(cid) - line,
            ui.height_of(cid) - line,
        ),
        part.radius,
        &colour,
        line,
    )?])
}

/// Draws the halo: rounded rectangles that grow outwards and fade, the outermost first, so it is strongest at the frame.
pub(super) fn glow(ui: &Ui, cid: Cid, part: &GlowPart) -> Result<Vec<PaintOp>, String> {
    let reach = ui
        .read(cid, |widget| widget.parent)
        .flatten()
        .map_or(0.0, |parent| super::layer_bleed(ui, parent));
    let (width, height) = (ui.width_of(cid), ui.height_of(cid));
    let steps = part.steps as f64;
    let mut ops = Vec::new();
    for ring in (1..=part.steps).rev() {
        let grow = reach * ring as f64 / steps;
        let alpha = part.strength / steps;
        ops.push(paint::rect(
            reach - grow,
            reach - grow,
            width - 2.0 * (reach - grow),
            height - 2.0 * (reach - grow),
            part.radius + grow,
            &format!("{}{}", part.color, alpha_byte(alpha)),
        )?);
    }
    Ok(ops)
}
