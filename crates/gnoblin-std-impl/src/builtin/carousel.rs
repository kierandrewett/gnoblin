//! The carousel: cards on a perspective ring, the focused window at the front.
//!
//! A port of `Carousel3dLayout` and `ring` of `data/js/gnoblin-std/layouts/strip.ts`, which are the reference. Each placement is the
//! ring's projected rectangle together with the ring's depth (`z`) and its angle as a rotation about the vertical axis, so a renderer
//! that draws depth can use them. The depth fade of the ring is not applied: a window's opacity belongs to the rules.

use gnoblin_core::Rect;

use super::{ActionEnv, ActionResult, Imp, OptionSet, Val, plain};
use crate::error::{Fail, Res};
use crate::layout::{LayoutContext, Placement};

const TAU: f64 = 2.0 * std::f64::consts::PI;

/// The options of the ring, as the algorithm reads them.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RingShape {
    pub(crate) fov: f64,
    pub(crate) card_width: f64,
    pub(crate) card_height: f64,
}

impl RingShape {
    pub(crate) fn read(opts: &OptionSet) -> RingShape {
        RingShape {
            fov: opts.num("fov"),
            card_width: opts.num("card_width"),
            card_height: opts.num("card_height"),
        }
    }
}

/// One card of the ring: the projected rectangle, the depth fade, the angle round the ring and the depth.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RingCard {
    pub(crate) rect: Rect,
    pub(crate) opacity: f64,
    pub(crate) rotation: f64,
    pub(crate) z: f64,
}

/// The pure ring layout of `count` cards with the 1-based `focus` at the front, inside `area`. One slot per card.
///
/// # Errors
///
/// `focus` is not an index in `1..=count`.
pub(crate) fn ring_slots(count: usize, focus: usize, area: Rect, shape: RingShape) -> Res<Vec<RingCard>> {
    if count == 0 {
        return Ok(Vec::new());
    }
    if focus < 1 || focus > count {
        return Err(Fail::type_error(format!(
            "focus must be an index in 1..{count}, got {focus}"
        )));
    }
    let n = count as f64;
    let card_width = area.width * shape.card_width;
    let card_height = area.height * shape.card_height;
    let radius = (card_width * 1.25).max((n * card_width * 1.25) / TAU);
    let focal = (card_height / 2.0 / (shape.fov * std::f64::consts::PI / 180.0 / 2.0).tan()).max(1.0);
    let centre_z = -(focal + radius);
    let cx = area.x + area.width / 2.0;
    let cy = area.y + area.height / 2.0;
    let mut slots = Vec::with_capacity(count);
    for i in 1..=count {
        let mut k = (i as i64 - focus as i64).rem_euclid(count as i64) as f64;
        if k > n / 2.0 {
            k -= n; // signed offset from the front, in (-count/2, count/2]
        }
        let angle = (k * TAU) / n;
        let sideways = radius * angle.sin();
        let z = centre_z + radius * angle.cos();
        let scale = focal / -z;
        let width = card_width * scale;
        let height = card_height * scale;
        slots.push(RingCard {
            rect: Rect::new(cx + sideways * scale - width / 2.0, cy - height / 2.0, width, height),
            opacity: 0.4 + (0.6 * (1.0 + angle.cos())) / 2.0,
            rotation: (angle * 180.0) / std::f64::consts::PI,
            z,
        });
    }
    Ok(slots)
}

/// The built-in carousel layout.
pub(super) struct Carousel {
    opts: OptionSet,
}

impl Carousel {
    pub(super) fn new(opts: OptionSet) -> Carousel {
        Carousel { opts }
    }
}

impl Imp for Carousel {
    fn options(&self) -> &OptionSet {
        &self.opts
    }

    fn options_mut(&mut self) -> &mut OptionSet {
        &mut self.opts
    }

    fn compute(&mut self, cx: &LayoutContext) -> Vec<Placement> {
        let nodes = &cx.children;
        let index = cx
            .focused
            .and_then(|focused| nodes.iter().rposition(|node| *node == focused));
        let Ok(slots) = ring_slots(
            nodes.len(),
            index.map_or(1, |index| index + 1),
            cx.area,
            RingShape::read(&self.opts),
        ) else {
            return Vec::new();
        };
        nodes
            .iter()
            .zip(slots)
            .map(|(client, slot)| Placement {
                bounds: Some(slot.rect),
                z: slot.z,
                rotation: [0.0, (slot.rotation * std::f64::consts::PI) / 180.0, 0.0],
                ..plain(*client)
            })
            .collect()
    }

    fn act(&mut self, action: &str, args: &[Val], env: &ActionEnv) -> Res<ActionResult> {
        if action != "rotate" {
            return Err(Fail::no_such(format!("the carousel layout has no action `{action}`")));
        }
        let Some(Val::Num(steps)) = args.first() else {
            return Err(Fail::type_error("carousel.rotate needs a number of steps"));
        };
        let Some(at) = env
            .focused
            .and_then(|focused| env.clients.iter().position(|card| *card == focused))
        else {
            return Ok(ActionResult::nothing());
        };
        let count = env.clients.len() as i64;
        let index = (at as i64 + *steps as i64).rem_euclid(count);
        Ok(ActionResult {
            value: Val::None,
            changed: false,
            focus: env.clients.get(index as usize).copied(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape() -> RingShape {
        RingShape {
            fov: 40.0,
            card_width: 0.36,
            card_height: 0.6,
        }
    }

    #[test]
    fn the_front_card_is_centred_and_the_ring_has_one_slot_per_card() {
        let area = Rect::new(0.0, 0.0, 1000.0, 800.0);
        let slots = ring_slots(5, 1, area, shape()).expect("a ring");
        assert_eq!(slots.len(), 5);
        assert_eq!(slots[0].rotation, 0.0);
        // The front card is centred across the area.
        assert!((slots[0].rect.x + slots[0].rect.width / 2.0 - 500.0).abs() < 1e-9);
        assert!(slots[0].z > slots[2].z, "the front is nearer the camera than the back");
    }

    #[test]
    fn a_focus_outside_the_ring_is_refused_and_an_empty_ring_is_empty() {
        let area = Rect::new(0.0, 0.0, 1000.0, 800.0);
        assert!(ring_slots(3, 4, area, shape()).is_err());
        assert!(ring_slots(3, 0, area, shape()).is_err());
        assert!(ring_slots(0, 1, area, shape()).expect("an empty ring").is_empty());
    }
}
