//! The pointer drags the library runs itself: move, resize, tile swap and split resize, as classes of the catalog (`move_binding`,
//! `resize_binding`, `swap_binding` and `split_resize_binding`).
//!
//! The rules are `bindings/steps.rs`'s state machines, which read and write nothing, and `bindings/drags.rs`'s adapter that reads the window
//! the press landed on and writes what the machine answers (the window's rectangle, the rectangle it floats at, its snap, the order of
//! its workspace, the ratio of a layout's divider). What this module adds is the matcher's side: it turns the core's `binding_event` into
//! the step the rule is given, and supplies what the core has no source for, the work area.

use std::cell::RefCell;
use std::rc::Rc;

use gnoblin_core::host::Rect;
use gnoblin_core::{BindingEvent, PointerDragBindingHandlers};
use gnoblin_core_abi::GnoblinCtx;
use gnoblin_object::server::Builder;
use gnoblin_object::{Fail, Val};
use gnoblin_std_abi::{STD_CONSTRAINT_MINIMUM_STRIP, STD_CONSTRAINT_WORK_AREA};

use super::binding::{BindStep, Flavour, PolicyBinding, drag_handlers, members};
use crate::bindings::steps::{DragRule, GripChoice};
use crate::bindings::{DragCtx, DragHandler, RuleDrag};
use crate::builtin::Pt;
use crate::nodes;
use crate::object::record::{enumeration, look};
use crate::state::lookup_quiet;
use crate::ui::Ui;

/// Which rule a stock drag runs.
#[derive(Clone, Copy, Debug, PartialEq)]
enum StockRule {
    Move,
    Resize,
    Swap,
    Split,
}

/// The rule a stock drag runs, with its options: `snap` and `margin` are a move's, `grip` a resize's.
#[derive(Clone, Copy, Debug)]
pub(super) struct Stock {
    rule: StockRule,
    snap: bool,
    margin: f64,
    grip: u32,
}

impl Stock {
    fn of(rule: StockRule) -> Stock {
        Stock {
            rule,
            snap: true,
            margin: 4.0,
            grip: 0,
        }
    }

    fn rule(self) -> DragRule {
        match self.rule {
            StockRule::Move => DragRule::Move {
                snap: self.snap.then_some(self.margin),
            },
            StockRule::Resize => DragRule::Resize {
                grip: GripChoice::from_member(self.grip),
            },
            StockRule::Swap => DragRule::Swap,
            StockRule::Split => DragRule::Split,
        }
    }

    /// The handlers the core's matcher calls for a drag of this rule.
    pub(super) fn handlers(self, ui: &Rc<Ui>) -> PointerDragBindingHandlers {
        let held = Rc::new(RefCell::new(RuleDrag::new(self.rule())));
        let ctx = ui.ctx;
        let weak = Rc::downgrade(ui);
        let for_start = (Rc::clone(&held), weak.clone());
        drag_handlers(
            move |event| {
                let (held, weak) = &for_start;
                let Some(ui) = weak.upgrade() else {
                    return Ok(false);
                };
                let std = lookup_quiet(ctx).map_err(Fail::from)?;
                let step = step_of(&ui, event);
                Ok(held.borrow_mut().start(&std, &step))
            },
            move |phase, event| {
                let Some(ui) = weak.upgrade() else {
                    return;
                };
                let Ok(std) = lookup_quiet(ctx) else {
                    return;
                };
                let step = step_of(&ui, event);
                let mut rule = held.borrow_mut();
                match phase {
                    BindStep::Update => rule.update(&std, &step),
                    BindStep::End => rule.end(&std, &step),
                    BindStep::Cancel => rule.cancel(&std, &step),
                }
            },
        )
    }
}

/// The window a `client` of the core is: its node number.
pub(super) fn node_of(client: &gnoblin_core::Client) -> Option<u64> {
    client.id().ok()?.parse().ok()
}

/// The rectangle windows are kept inside while `client` is dragged: the work area of the output it is on, or the first screen's.
pub(super) fn work_area(ctx: GnoblinCtx, client: Option<u64>) -> Option<Rect> {
    look(ctx, |st| {
        Ok(client
            .and_then(|client| st.work_area_of_client(client))
            .or_else(|| st.screen_area()))
    })
    .ok()
    .flatten()
}

/// What the rule is told at a step, from what the matcher told the binding.
fn step_of(ui: &Ui, event: &BindingEvent) -> DragCtx {
    let client = event.client.as_ref().and_then(node_of);
    let position = Pt {
        x: event.position.x,
        y: event.position.y,
    };
    let origin = Pt {
        x: position.x - event.dx.unwrap_or(0.0),
        y: position.y - event.dy.unwrap_or(0.0),
    };
    let rect = client.map(|client| nodes::rect_of(&ui.g, client));
    DragCtx {
        client,
        position,
        origin,
        local: event
            .local_position
            .as_ref()
            .map(|local| Pt { x: local.x, y: local.y })
            .or_else(|| {
                rect.map(|rect| Pt {
                    x: position.x - rect.x,
                    y: position.y - rect.y,
                })
            }),
        button: i64::try_from(event.button.unwrap_or(0)).unwrap_or(0),
        work_area: work_area(ui.ctx, client),
        constraint: if event.drag_constraint.as_deref() == Some("minimum_strip") {
            STD_CONSTRAINT_MINIMUM_STRIP
        } else {
            STD_CONSTRAINT_WORK_AREA
        },
        edge_tiling: event.edge_tiling.unwrap_or(false),
    }
}

/// What a stock drag keeps of its options.
fn stock<R>(binding: &PolicyBinding, read: impl FnOnce(Stock) -> R) -> Result<R, Fail> {
    match &binding.flavour {
        Flavour::Stock(stock) => Ok(read(*stock)),
        _ => Err(Fail::type_error("not a stock drag")),
    }
}

fn edit(binding: &mut PolicyBinding, change: impl FnOnce(&mut Stock)) -> Result<(), Fail> {
    binding.open()?;
    match &mut binding.flavour {
        Flavour::Stock(stock) => {
            change(stock);
            Ok(())
        }
        _ => Err(Fail::type_error("not a stock drag")),
    }
}

pub(super) fn register(builder: &mut Builder) {
    let _ = members(builder.class::<PolicyBinding>("move_binding").ctor(|_cx, inits| {
        Ok(PolicyBinding::new(
            inits.take("trigger").map_or_else(
                || Ok("Super+MouseLeft+Drag".to_owned()),
                |value| value.text().map(str::to_owned),
            )?,
            inits.take("priority").map_or(Ok(0.0), |value| value.double())?,
            Flavour::Stock(Stock {
                snap: inits.take("snap").map_or(Ok(true), |value| value.bool())?,
                margin: inits.take("snap_margin").map_or(Ok(4.0), |value| value.double())?,
                ..Stock::of(StockRule::Move)
            }),
        ))
    }))
    .get("snap", |h, _| stock(h, |stock| Val::Bool(stock.snap)))
    .set("snap", |h, _, value| {
        let on = value.bool()?;
        edit(h, |stock| {
            stock.snap = on;
        })
    })
    .get("snap_margin", |h, _| stock(h, |stock| Val::Double(stock.margin)))
    .set("snap_margin", |h, _, value| {
        let by = value.double()?;
        if by.is_nan() || by < 0.0 {
            return Err(Fail::type_error("snap_margin is a distance of 0 or more"));
        }
        edit(h, |stock| {
            stock.margin = by;
        })
    });
    let _ = members(builder.class::<PolicyBinding>("resize_binding").ctor(|_cx, inits| {
        Ok(PolicyBinding::new(
            inits.take("trigger").map_or_else(
                || Ok("Super+MouseRight+Drag".to_owned()),
                |value| value.text().map(str::to_owned),
            )?,
            inits.take("priority").map_or(Ok(0.0), |value| value.double())?,
            Flavour::Stock(Stock {
                grip: inits
                    .take("grip")
                    .map_or(Ok(0), |value| value.enum_value().map(|place| place.saturating_sub(1)))?,
                ..Stock::of(StockRule::Resize)
            }),
        ))
    }))
    .get("grip", |h, _| stock(h, |stock| enumeration(stock.grip + 1)))
    .set("grip", |h, _, value| {
        let place = value.enum_value()?.saturating_sub(1);
        edit(h, |stock| {
            stock.grip = place;
        })
    });
    let _ = members(builder.class::<PolicyBinding>("swap_binding").ctor(|_cx, inits| {
        Ok(PolicyBinding::new(
            inits.take("trigger").map_or_else(
                || Ok("Super+MouseLeft+Drag".to_owned()),
                |value| value.text().map(str::to_owned),
            )?,
            inits.take("priority").map_or(Ok(0.0), |value| value.double())?,
            Flavour::Stock(Stock::of(StockRule::Swap)),
        ))
    }));
    let _ = members(
        builder
            .class::<PolicyBinding>("split_resize_binding")
            .ctor(|_cx, inits| {
                Ok(PolicyBinding::new(
                    inits.take("trigger").map_or_else(
                        || Ok("Super+MouseRight+Drag".to_owned()),
                        |value| value.text().map(str::to_owned),
                    )?,
                    inits.take("priority").map_or(Ok(0.0), |value| value.double())?,
                    Flavour::Stock(Stock::of(StockRule::Split)),
                ))
            }),
    );
}
