//! `workspace_slide_binding`: the stock touchpad gesture. A three-finger swipe slides to the next or the previous workspace and follows the
//! fingers while they move. Fingers moving right bring the previous workspace in from the left, fingers moving left the next one from the
//! right. Lifting after `commit` of `travel` finishes the slide from where the fingers left it; lifting sooner, or a cancel, slides back. A
//! swipe that is more vertical than horizontal, or that has no workspace to reveal, moves nothing.
//!
//! A workspace is not a node, so the slide moves its windows: each window's `x` follows the fingers and goes back to where it rests when the
//! slide is over.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gnoblin_core::{BindingEvent, GestureBindingHandlers};
use gnoblin_object::server::Builder;
use gnoblin_object::{Fail, Val};
use gnoblin_std_abi::{GnoblinStdTransitionOptions, STD_EASING_EASE_OUT};

use super::binding::{BindStep, Flavour, PolicyBinding, gesture_handlers, members};
use crate::animation::engine::motion;
use crate::animation::table::commit_staged;
use crate::nodes;
use crate::object::record::look;
use crate::state::{Id, lookup_quiet};
use crate::ui::Ui;

/// Milliseconds the slide back takes, whatever `duration` is: a fixed 200 against the forward slide's 250.
const SETTLE_BACK_DURATION: f64 = 200.0;

/// The options of a slide.
#[derive(Clone, Copy, Debug)]
pub(super) struct Slide {
    travel: f64,
    commit: f64,
    duration: f64,
}

/// What one swipe knows between the fingers' first touch and their lift.
struct SlideFollow {
    from: Id,
    /// Where each window the slide has moved rests.
    rest: HashMap<u64, f64>,
    sign: i8,
    to: Option<Id>,
    progress: f64,
    /// Something else switched workspace under the fingers: the slide has nothing more to do.
    over: bool,
}

// ---- what the slide reads of the desktop ------------------------------------------------------------------------------------------

/// The workspace the output under the pointer shows, which a slide starts from.
fn active(ui: &Ui) -> Option<Id> {
    look(ui.ctx, |st| {
        Ok(st
            .current_desk()
            .and_then(|desk| st.desk(desk))
            .and_then(|desk| desk.shown))
    })
    .ok()
    .flatten()
}

/// The area windows are laid out in on the output the slide is on, or none while there is no screen.
fn area_width(ui: &Ui) -> Option<f64> {
    look(ui.ctx, |st| {
        Ok(st
            .current_desk()
            .and_then(|desk| st.area_of_desk(desk))
            .or_else(|| st.screen_area())
            .map(|area| area.width))
    })
    .ok()
    .flatten()
}

/// The workspace `at` places from the start of the list.
fn at(ui: &Ui, index: i64) -> Option<Id> {
    look(ui.ctx, |st| {
        Ok(usize::try_from(index)
            .ok()
            .and_then(|index| st.workspaces.get(index).copied()))
    })
    .ok()
    .flatten()
}

fn index_of(ui: &Ui, workspace: Id) -> Option<i64> {
    look(ui.ctx, |st| {
        Ok(st
            .workspaces
            .iter()
            .position(|id| *id == workspace)
            .and_then(|index| i64::try_from(index).ok()))
    })
    .ok()
    .flatten()
}

fn clients_of(ui: &Ui, workspace: Id) -> Vec<u64> {
    look(ui.ctx, |st| {
        Ok(st.ws.get(&workspace).map(|ws| ws.clients.clone()).unwrap_or_default())
    })
    .unwrap_or_default()
}

fn x_of(ui: &Ui, client: u64) -> f64 {
    nodes::number(&ui.g, client, "x")
}

// ---- the slide --------------------------------------------------------------------------------------------------------------------------

impl Slide {
    /// The handlers the core's matcher calls for a swipe of this slide.
    pub(super) fn handlers(self, ui: &Rc<Ui>) -> GestureBindingHandlers {
        let held: Rc<RefCell<Option<SlideFollow>>> = Rc::new(RefCell::new(None));
        let weak = Rc::downgrade(ui);
        let for_begin = (Rc::clone(&held), weak.clone());
        gesture_handlers(
            move |_| {
                let (held, weak) = &for_begin;
                let Some(ui) = weak.upgrade() else {
                    return Ok(false);
                };
                let Some(from) = active(&ui) else {
                    return Ok(false);
                };
                if area_width(&ui).is_none() {
                    return Ok(false);
                }
                let rest = clients_of(&ui, from)
                    .into_iter()
                    .map(|client| (client, x_of(&ui, client)))
                    .collect();
                *held.borrow_mut() = Some(SlideFollow {
                    from,
                    rest,
                    sign: 0,
                    to: None,
                    progress: 0.0,
                    over: false,
                });
                Ok(true)
            },
            move |phase, event| {
                let Some(ui) = weak.upgrade() else {
                    return;
                };
                let mut slot = held.borrow_mut();
                let Some(run) = slot.as_mut() else {
                    return;
                };
                match phase {
                    BindStep::Update => self.update(&ui, run, event),
                    BindStep::End => self.end(&ui, run, event),
                    BindStep::Cancel => self.cancel(&ui, run),
                }
                if phase != BindStep::Update {
                    *slot = None;
                }
            },
        )
    }

    /// Follows the fingers: picks the target workspace on the first horizontal move and places every window again.
    fn update(self, ui: &Rc<Ui>, run: &mut SlideFollow, event: &BindingEvent) {
        if run.over {
            return;
        }
        let now = active(ui);
        if now != Some(run.from) {
            if now != run.to {
                reset(ui, run);
            }
            run.over = true;
            return;
        }
        let (dx, dy) = (event.dx.unwrap_or(0.0), event.dy.unwrap_or(0.0));
        if dx == 0.0 {
            // No direction yet.
            return;
        }
        let sign: i8 = if dx > 0.0 { 1 } else { -1 };
        if run.to.is_none() && dy.abs() > dx.abs() {
            // Vertical so far: not a slide.
            return;
        }
        if sign != run.sign {
            reset(ui, run);
            let target = index_of(ui, run.from).and_then(|index| {
                let index = index - i64::from(sign);
                (index >= 0).then(|| at(ui, index)).flatten()
            });
            if let Some(target) = target {
                run.sign = sign;
                run.to = Some(target);
                for client in clients_of(ui, target) {
                    run.rest.insert(client, x_of(ui, client));
                    nodes::write_flag(&ui.g, client, "visible", true);
                }
            }
        }
        if run.to.is_some() {
            place(ui, run, (dx.abs() / self.travel).min(1.0));
        }
    }

    /// Finishes the slide through the shell's `show` when it passed `commit` and went mostly sideways. Otherwise it settles back.
    fn end(self, ui: &Rc<Ui>, run: &mut SlideFollow, event: &BindingEvent) {
        let Some(to) = run.to else {
            return;
        };
        if run.over || active(ui) != Some(run.from) {
            return;
        }
        let (dx, dy) = (event.dx.unwrap_or(0.0), event.dy.unwrap_or(0.0));
        let committed = run.progress >= self.commit && dx.abs() >= dy.abs();
        if !committed {
            settle_back(ui, run, to);
            return;
        }
        // Back to rest at once, so the slide that starts from the positions the layout made carries on from `progress` for the rest of
        // its time.
        for (client, rest) in &run.rest {
            if nodes::alive(&ui.g, *client) {
                nodes::write_num(&ui.g, *client, "x", *rest);
            }
        }
        let (duration, progress) = (self.duration * (1.0 - run.progress), run.progress);
        let shown = crate::object::record::edit(ui.ctx, |st| st.show_workspace(to, None, duration, progress));
        if let Err(fail) = shown {
            ui.warn(&format!("the workspace slide could not show the workspace: {fail}"));
        }
    }

    /// A cancelled gesture slides back like a short one.
    fn cancel(self, ui: &Rc<Ui>, run: &mut SlideFollow) {
        let Some(to) = run.to else {
            return;
        };
        if run.over || active(ui) != Some(run.from) {
            return;
        }
        settle_back(ui, run, to);
    }
}

/// Puts the windows where a slide of `progress` (0 to 1) in the direction of `run.sign` has them.
fn place(ui: &Ui, run: &mut SlideFollow, progress: f64) {
    let width = area_width(ui).unwrap_or(0.0);
    let sign = f64::from(run.sign);
    run.progress = progress;
    for client in clients_of(ui, run.from) {
        let rest = run.rest.get(&client).copied().unwrap_or_else(|| x_of(ui, client));
        nodes::write_num(&ui.g, client, "x", rest + sign * progress * width);
    }
    for client in run.to.map(|to| clients_of(ui, to)).unwrap_or_default() {
        let rest = run.rest.get(&client).copied().unwrap_or_else(|| x_of(ui, client));
        nodes::write_num(&ui.g, client, "x", rest + sign * (progress - 1.0) * width);
    }
}

/// Hides the workspace a slide had brought in and puts every window it moved back.
fn reset(ui: &Ui, run: &mut SlideFollow) {
    let Some(to) = run.to else {
        return;
    };
    for client in clients_of(ui, to) {
        if nodes::alive(&ui.g, client) {
            nodes::write_flag(&ui.g, client, "visible", false);
        }
    }
    for (client, rest) in &run.rest {
        if nodes::alive(&ui.g, *client) {
            nodes::write_num(&ui.g, *client, "x", *rest);
        }
    }
    run.to = None;
    run.sign = 0;
}

/// Plays the windows back to rest. The incoming workspace's windows are hidden again when the slide back ends, unless the shell showed that
/// workspace meanwhile.
fn settle_back(ui: &Rc<Ui>, run: &SlideFollow, to: Id) {
    let width = area_width(ui).unwrap_or(0.0);
    let duration = motion(&ui.g, SETTLE_BACK_DURATION);
    let sign = f64::from(run.sign);
    let incoming = clients_of(ui, to);
    let from: Vec<u64> = clients_of(ui, run.from);
    let rest = |client: u64| run.rest.get(&client).copied().unwrap_or_else(|| x_of(ui, client));
    let outgoing: Vec<(u64, f64)> = from.iter().map(|client| (*client, rest(*client))).collect();
    let arriving: Vec<(u64, f64)> = incoming.iter().map(|client| (*client, rest(*client))).collect();
    let write = |ui: &Ui| {
        for (client, at) in &outgoing {
            nodes::write_num(&ui.g, *client, "x", *at);
        }
        for (client, at) in &arriving {
            nodes::write_num(&ui.g, *client, "x", *at - sign * width);
        }
    };
    request_transition(ui, duration, write);
    let weak = Rc::downgrade(ui);
    let timer = ui.g.after(duration.max(1.0) as u32, move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        // The workspace came in for good meanwhile.
        if active(&ui) == Some(to) {
            return;
        }
        for (client, at) in &arriving {
            if nodes::alive(&ui.g, *client) {
                nodes::write_flag(&ui.g, *client, "visible", false);
                nodes::write_num(&ui.g, *client, "x", *at);
            }
        }
    });
    if let Err(error) = timer {
        ui.warn(&format!("the workspace slide could not set its timer: {error}"));
    }
}

/// Runs `change` and plays what it wrote over `duration` milliseconds along the ease-out curve.
fn request_transition(ui: &Ui, duration: f64, change: impl FnOnce(&Ui)) {
    let Ok(std) = lookup_quiet(ui.ctx) else {
        change(ui);
        return;
    };
    let raw = GnoblinStdTransitionOptions {
        size: size_of::<GnoblinStdTransitionOptions>() as u32,
        autoplay: 1,
        easing: STD_EASING_EASE_OUT,
        reserved: 0,
        duration_ms: duration,
        delay_ms: 0.0,
    };
    if let Err(fail) = std.st.borrow_mut().transition_begin(&raw) {
        ui.warn(&format!(
            "the workspace slide could not begin a transition: {}",
            fail.message
        ));
        change(ui);
        return;
    }
    change(ui);
    if let Err(fail) = commit_staged(&std) {
        ui.warn(&format!("the workspace slide's transition failed: {}", fail.message));
    }
}

pub(super) fn register(builder: &mut Builder) {
    let check = |name: &str, value: f64, ok: bool| -> Result<f64, Fail> {
        if ok {
            Ok(value)
        } else {
            Err(Fail::type_error(format!(
                "WorkspaceSlideBinding {name} is out of range, got {value}"
            )))
        }
    };
    let _ = members(
        builder
            .class::<PolicyBinding>("workspace_slide_binding")
            .ctor(move |_cx, inits| {
                let travel = inits.take("travel").map_or(Ok(400.0), |value| value.double())?;
                let commit = inits.take("commit").map_or(Ok(0.25), |value| value.double())?;
                let duration = inits.take("duration").map_or(Ok(250.0), |value| value.double())?;
                Ok(PolicyBinding::new(
                    inits
                        .take("trigger")
                        .map_or_else(|| Ok("Swipe3".to_owned()), |value| value.text().map(str::to_owned))?,
                    inits.take("priority").map_or(Ok(0.0), |value| value.double())?,
                    Flavour::Slide(Slide {
                        travel: check("travel", travel, travel > 0.0 && travel.is_finite())?,
                        commit: check("commit", commit, (0.0..=1.0).contains(&commit))?,
                        duration: check("duration", duration, duration >= 0.0 && duration.is_finite())?,
                    }),
                ))
            }),
    )
    .get("travel", |h, _| option(h, |slide| slide.travel))
    .set("travel", |h, _, value| {
        let by = value.double()?;
        if by.is_nan() || by <= 0.0 || by.is_infinite() {
            return Err(Fail::type_error(
                "WorkspaceSlideBinding travel must be a finite number above 0",
            ));
        }
        change(h, |slide| slide.travel = by)
    })
    .get("commit", |h, _| option(h, |slide| slide.commit))
    .set("commit", |h, _, value| {
        let by = value.double()?;
        if !(0.0..=1.0).contains(&by) {
            return Err(Fail::type_error(
                "WorkspaceSlideBinding commit must be a number from 0 to 1",
            ));
        }
        change(h, |slide| slide.commit = by)
    })
    .get("duration", |h, _| option(h, |slide| slide.duration))
    .set("duration", |h, _, value| {
        let by = value.double()?;
        if by.is_nan() || by < 0.0 || by.is_infinite() {
            return Err(Fail::type_error(
                "WorkspaceSlideBinding duration must be a finite number of milliseconds of at least 0",
            ));
        }
        change(h, |slide| slide.duration = by)
    });
}

fn option(binding: &PolicyBinding, read: impl FnOnce(&Slide) -> f64) -> Result<Val, Fail> {
    match &binding.flavour {
        Flavour::Slide(slide) => Ok(Val::Double(read(slide))),
        _ => Ok(Val::None),
    }
}

fn change(binding: &mut PolicyBinding, edit: impl FnOnce(&mut Slide)) -> Result<(), Fail> {
    binding.open()?;
    if let Flavour::Slide(slide) = &mut binding.flavour {
        edit(slide);
    }
    Ok(())
}
