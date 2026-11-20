//! `bar`: a strip along the top or the bottom of an output, over the windows, that takes its room from the windows there.
//!
//! The layouts place windows in what the bars leave (`insets`). A bar is a row of boxes as wide as its output; bars on one edge of one
//! output stack in the order they were made. A bar hides, and takes no room, while a fullscreen window covers its output (the shell's
//! `give_way_to`) or its own `hide_when` says so.

use std::rc::Rc;

use gnoblin_core::Rect;

use super::widget::{BarPart, Cid, Special, Ui, WidgetKind, en};

/// The bar's part of a canvas.
fn part(ui: &Ui, cid: Cid) -> Option<BarPart> {
    ui.read(cid, |widget| match &widget.sp {
        Special::Bar(part) => Some(part.clone()),
        _ => None,
    })
    .flatten()
}

fn place_of(ui: &Ui, output: u64) -> Rect {
    let node = ui.node(output);
    let number = |key: &str| node.get_f64(key).unwrap_or(0.0);
    Rect {
        x: number("x"),
        y: number("y"),
        width: number("width"),
        height: number("height"),
    }
}

/// Asks the shell a question of a bar and reads a boolean answer.
fn ask(closure: &gnoblin_object::Closure, args: &[gnoblin_object::Val]) -> Result<bool, String> {
    closure
        .call(0, args)
        .and_then(|value| value.bool())
        .map_err(|fail| fail.message)
}

impl Ui {
    /// The bars that are alive, in the order they were made.
    fn live_bars(&self) -> Vec<Cid> {
        self.w
            .borrow()
            .widgets
            .iter()
            .filter(|(_, widget)| widget.alive && widget.kind == WidgetKind::Bar)
            .map(|(cid, _)| *cid)
            .collect()
    }

    /// The output the bar sits on now: its own, or the primary one. `None` while there is none.
    fn bar_output(&self, own: u64) -> Result<Option<u64>, String> {
        if own != 0 {
            return Ok(Some(own));
        }
        self.primary_output()
    }

    /// The primary output, as the shell says it. `None` when it says none.
    fn primary_output(&self) -> Result<Option<u64>, String> {
        let Some(source) = self.w.borrow().bars_primary.clone() else {
            return Ok(None);
        };
        let node = source
            .call(0, &[])
            .and_then(|value| value.uint())
            .map_err(|fail| fail.message)?;
        Ok((node != 0).then_some(node))
    }

    /// Whether the bar is on show, and so takes room.
    pub(crate) fn bar_shown(&self, cid: Cid) -> Result<bool, String> {
        let Some(bar) = part(self, cid) else {
            return Ok(false);
        };
        if !self.alive(cid) {
            return Ok(false);
        }
        let output = self.bar_output(bar.output)?;
        if bar.output != 0 && !output.is_some_and(|node| self.node(node).is_alive()) {
            return Ok(false);
        }
        if let Some(hide) = &bar.hide_when
            && ask(hide, &[])?
        {
            return Ok(false);
        }
        let covered = self.w.borrow().bars_cover.clone();
        if let Some(covered) = covered
            && ask(&covered, &[gnoblin_object::Val::Uint(output.unwrap_or(0))])?
        {
            return Ok(false);
        }
        Ok(true)
    }

    /// The room the bars take from the top and the bottom of `output` (the primary output when it is zero).
    ///
    /// # Errors
    ///
    /// A question the shell was asked failed.
    pub(crate) fn bar_insets(&self, output: u64) -> Result<(f64, f64), String> {
        let on = if output != 0 {
            Some(output)
        } else {
            self.primary_output()?
        };
        let (mut top, mut bottom) = (0.0, 0.0);
        for cid in self.live_bars() {
            let Some(bar) = part(self, cid) else { continue };
            if !self.bar_shown(cid)? || self.bar_output(bar.output)? != on {
                continue;
            }
            if bar.edge == en::BAR_TOP {
                top += self.height_of(cid);
            } else {
                bottom += self.height_of(cid);
            }
        }
        Ok((top, bottom))
    }
}

/// Positions the bar on its output and updates its visibility.
pub(crate) fn refresh(ui: &Rc<Ui>, cid: Cid) -> Result<(), String> {
    let Some(bar) = part(ui, cid) else {
        return Ok(());
    };
    let shown = ui.bar_shown(cid)?;
    ui.tweak(cid, |widget| widget.visible = shown);
    let output = ui.bar_output(bar.output)?;
    let placed = match output {
        Some(node) if ui.node(node).is_alive() => Some(place_of(ui, node)),
        _ if bar.output == 0 => ui.primary_screen().map(|(width, height)| Rect {
            x: 0.0,
            y: 0.0,
            width,
            height,
        }),
        _ => None,
    };
    let Some(origin) = placed else {
        return Ok(());
    };
    ui.tweak(cid, |widget| widget.width = Some(origin.width));
    let mut before = 0.0;
    for other in ui.live_bars() {
        if other == cid {
            break;
        }
        let Some(theirs) = part(ui, other) else { continue };
        if theirs.edge == bar.edge && ui.bar_output(theirs.output)? == output && ui.bar_shown(other)? {
            before += ui.height_of(other);
        }
    }
    let height = ui.height_of(cid);
    ui.tweak(cid, |widget| {
        widget.x = origin.x;
        widget.y = if bar.edge == en::BAR_TOP {
            origin.y + before
        } else {
            origin.y + origin.height - before - height
        };
    });
    Ok(())
}
