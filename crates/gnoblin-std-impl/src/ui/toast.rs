//! `toast`: a card with one message that fades in, holds, fades out and destroys itself.
//!
//! The card sits in the screen's top-right corner, over the windows. Its opacity follows its age, so the toolkit asks for a turn
//! every frame while one is up and no timer of the card's own is needed.

use std::rc::Rc;

use gnoblin_object::server::Cx;
use gnoblin_object::{Obj, Res};

use super::update::now_ms;
use super::widget::{Cid, Kind, Special, ToastPart, Ui, en};

/// The card's width in pixels.
const WIDTH: f64 = 320.0;
/// The card's height in pixels.
const HEIGHT: f64 = 64.0;
/// The gap in pixels between the card and the screen's right edge, plus the card's width.
const RIGHT_INSET: f64 = 340.0;
/// The distance in pixels from the top of the screen, below the bar.
const TOP: f64 = 44.0;
/// The card's corner radius in pixels.
const RADIUS: f64 = 10.0;
/// The space in pixels between the card's edge and its text.
const PADDING: f64 = 14.0;
/// The card's background colour.
const FILL: &str = "#45475a";

/// Makes the card of a toast whose object is the one `cx` is about: its look, its message, and its place over the windows.
///
/// # Errors
///
/// A canvas cannot be made, or the compositor refuses the card's mounting.
pub(crate) fn build(ui: &Rc<Ui>, cx: &mut Cx<'_>, me: Obj, cid: Cid) -> Res<()> {
    // 340 is the width the card takes when the tree has no screen yet; the card then sits at x = 0.
    let screen = ui.primary_screen().unwrap_or((RIGHT_INSET, 0.0));
    let (_, message) = ui.spawn(cx, Kind::Text, me)?;
    ui.tweak(message, |widget| widget.b.grow = 1.0);
    ui.tweak(cid, |widget| {
        widget.x = screen.0 - RIGHT_INSET;
        widget.y = TOP;
        widget.width = Some(WIDTH);
        widget.height = Some(HEIGHT);
        widget.b.radius = RADIUS;
        widget.b.padding = PADDING;
        widget.b.direction = en::DIRECTION_ROW;
        widget.b.fill = FILL.to_owned();
        widget.opacity = 0.0;
        widget.sp = Special::Toast(ToastPart {
            message,
            fade_in: 120.0,
            hold: 4000.0,
            fade_out: 300.0,
            clock: None,
            start: None,
        });
    });
    ui.link(cid, message)?;
    ui.mount_top(cid)
}

/// Updates the fade from the toast's age and ends it after the fade-out.
pub(crate) fn refresh(ui: &Rc<Ui>, cid: Cid) -> Result<(), String> {
    let Some(part) = ui
        .read(cid, |widget| match &widget.sp {
            Special::Toast(part) => Some(part.clone()),
            _ => None,
        })
        .flatten()
    else {
        return Ok(());
    };
    let now = match &part.clock {
        Some(clock) => clock
            .call(0, &[])
            .and_then(|value| value.double())
            .map_err(|fail| fail.message)?,
        None => now_ms(),
    };
    let start = match part.start {
        Some(start) => start,
        None => {
            ui.tweak(cid, |widget| {
                if let Special::Toast(held) = &mut widget.sp {
                    held.start = Some(now);
                }
            });
            now
        }
    };
    let age = now - start;
    let ToastPart {
        fade_in,
        hold,
        fade_out,
        ..
    } = part;
    if age < fade_in {
        ui.tweak(cid, |widget| widget.opacity = age / fade_in);
    } else if age < fade_in + hold {
        ui.tweak(cid, |widget| widget.opacity = 1.0);
    } else if age < fade_in + hold + fade_out {
        ui.tweak(cid, |widget| widget.opacity = 1.0 - (age - fade_in - hold) / fade_out);
    } else {
        ui.kill(cid);
    }
    Ok(())
}
