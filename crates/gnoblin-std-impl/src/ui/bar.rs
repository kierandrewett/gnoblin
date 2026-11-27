//! `bar`: a strip along the top or the bottom of an output, over the windows, that takes its room from the windows there.
//!
//! The layouts place windows in what the bars leave (`insets`, which `sync_bars` hands the library as each output's room). A bar is a
//! row of boxes as wide as its output; bars on one edge of one output stack in the order they were made. A bar hides, and takes no
//! room, while a fullscreen window covers its output (the library's own answer, or the shell's `give_way_to`) or its own `hide_when`
//! says so.

use std::rc::Rc;

use gnoblin_core::Rect;

use super::widget::{BarPart, Cid, Special, Ui, WidgetKind, en};
use crate::state::State;

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

    /// The primary output: the one the shell's `on_outputs` says, else the library's (the one flagged primary, or the first). `None`
    /// when there is none.
    fn primary_output(&self) -> Result<Option<u64>, String> {
        let source = self.w.borrow().bars_primary.clone();
        let Some(source) = source else {
            return Ok(self.with_state(State::primary_output).flatten());
        };
        let node = source
            .call(0, &[])
            .and_then(|value| value.uint())
            .map_err(|fail| fail.message)?;
        Ok((node != 0).then_some(node))
    }

    /// Reads the library's state of the generation, when it is open and free. `None` otherwise.
    fn with_state<R>(&self, read: impl FnOnce(&State) -> R) -> Option<R> {
        let std = crate::state::find(self.ctx)?;
        let st = std.st.try_borrow().ok()?;
        Some(read(&st))
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
        let covering = match covered {
            Some(covered) => ask(&covered, &[gnoblin_object::Val::Uint(output.unwrap_or(0))])?,
            None => output.is_some_and(|node| self.with_state(|st| st.output_covered(node)).unwrap_or(false)),
        };
        if covering {
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

    /// Hands the library the room the bars take from each output, so the layouts place windows in what is left, and has the toolkit
    /// draw again when a bar came on show or gave way. It runs at the end of the toolkit's frame and at the start of the library's turn
    /// (`in_turn`): a window going fullscreen, a workspace shown or an output coming or going reaches the layouts in the same turn.
    pub(crate) fn sync_bars(self: &Rc<Self>, in_turn: bool) {
        let Some(std) = crate::state::find(self.ctx) else {
            return;
        };
        let Ok(outputs) = std.st.try_borrow().map(|st| st.outputs.clone()) else {
            return;
        };
        let mut shown = Vec::new();
        for cid in self.live_bars() {
            shown.push((
                cid,
                self.bar_shown(cid).unwrap_or_else(|message| {
                    self.warn(&format!("a bar's question threw: {message}"));
                    false
                }),
            ));
        }
        let mut changed = false;
        let handed = self.w.borrow().bars_room.clone();
        let mut room_now = handed.clone();
        room_now.retain(|output, _| outputs.contains(output));
        for output in outputs {
            let (top, bottom) = match self.bar_insets(output) {
                Ok(room) => room,
                Err(message) => {
                    self.warn(&format!("a bar's question threw: {message}"));
                    continue;
                }
            };
            // An output no bar takes room from, and none did, is the shell's own: its insets stay.
            if (top, bottom) == (0.0, 0.0) && !handed.contains_key(&output) {
                continue;
            }
            let Ok(mut st) = std.st.try_borrow_mut() else {
                continue;
            };
            if st.insets.get(&output).copied().unwrap_or((0.0, 0.0)) != (top, bottom) {
                st.set_insets(output, top, bottom);
                changed = true;
            }
            if (top, bottom) == (0.0, 0.0) {
                room_now.remove(&output);
            } else {
                room_now.insert(output, (top, bottom));
            }
        }
        self.w.borrow_mut().bars_room = room_now;
        if changed && !in_turn {
            crate::turn::schedule(&std);
        }
        let seen = std::mem::replace(&mut self.w.borrow_mut().bars_seen, shown.clone());
        if in_turn && seen != shown {
            self.schedule();
        }
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
