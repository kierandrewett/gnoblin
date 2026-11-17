//! [`Carousel3dLayout`]: cards on a perspective ring, the focused window at the front.
//!
//! A port of `Carousel3dLayout` and `ring` of `data/js/gnoblin-std/layouts/strip.ts`, which are the reference. Each placement is the
//! ring's projected rectangle together with the ring's depth (`z`) and its angle as a rotation about the vertical axis, so a renderer
//! that draws depth can use them. The depth fade of the ring is not applied: a window's opacity belongs to the rules.

use gnoblin_core::Rect;

use crate::error::{Error, ErrorKind, Result};
use crate::layout::{Layout, LayoutContext, Placement};
use crate::workspace::Workspace;

const TAU: f64 = 2.0 * std::f64::consts::PI;

/// The options of [`ring`], which [`Carousel3dLayout`] lays its cards out with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RingOptions {
    /// Vertical field of view in degrees. Default 40.
    pub fov: f64,
    /// A card's width as a fraction of the area's. Default 0.36.
    pub card_width: f64,
    /// A card's height as a fraction of the area's. Default 0.6.
    pub card_height: f64,
}

impl Default for RingOptions {
    fn default() -> RingOptions {
        RingOptions {
            fov: 40.0,
            card_width: 0.36,
            card_height: 0.6,
        }
    }
}

/// One card of the ring: the projected rectangle, the depth fade, the angle round the ring and the depth.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RingSlot {
    /// The projected rectangle.
    pub rect: Rect,
    /// Fade from 0.4 at the back to 1 at the front.
    pub opacity: f64,
    /// Degrees: 0 at the front, 180 at the back, positive to the right of the front card.
    pub rotation: f64,
    /// Negative in front of the camera, more so for the back of the ring.
    pub z: f64,
}

/// The pure ring layout of `count` cards with the 1-based `focus` at the front, inside `area`. One slot per card.
///
/// # Errors
///
/// `focus` is not an index in `1..=count`.
pub fn ring(count: usize, focus: usize, area: Rect, options: RingOptions) -> Result<Vec<RingSlot>> {
    if count == 0 {
        return Ok(Vec::new());
    }
    if focus < 1 || focus > count {
        return Err(Error::new(
            ErrorKind::Type,
            format!("focus must be an index in 1..{count}, got {focus}"),
        ));
    }
    let n = count as f64;
    let card_width = area.width * options.card_width;
    let card_height = area.height * options.card_height;
    let radius = (card_width * 1.25).max((n * card_width * 1.25) / TAU);
    let focal = (card_height / 2.0 / (options.fov * std::f64::consts::PI / 180.0 / 2.0).tan()).max(1.0);
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
        slots.push(RingSlot {
            rect: Rect::new(cx + sideways * scale - width / 2.0, cy - height / 2.0, width, height),
            opacity: 0.4 + (0.6 * (1.0 + angle.cos())) / 2.0,
            rotation: (angle * 180.0) / std::f64::consts::PI,
            z,
        });
    }
    Ok(slots)
}

/// Cards on a ring, the focused window at the front; with no focus among the windows the first is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Carousel3dLayout {
    options: RingOptions,
}

impl Carousel3dLayout {
    /// A carousel with a 40 degree field of view and cards 0.36 of the area wide and 0.6 high.
    pub fn new() -> Carousel3dLayout {
        Carousel3dLayout::default()
    }

    /// A carousel with `options`.
    pub fn with_options(options: RingOptions) -> Carousel3dLayout {
        Carousel3dLayout { options }
    }

    /// The ring's options.
    pub fn options(&self) -> RingOptions {
        self.options
    }

    /// Moves the keyboard `steps` cards round the ring (the workspace's windows in its order), wrapping at both ends. Nothing happens
    /// while no window has the keyboard.
    ///
    /// # Errors
    ///
    /// The workspace is gone, or the generation retired.
    pub fn rotate(&self, workspace: &Workspace, steps: i64) -> Result<()> {
        let cards = workspace.clients()?.to_vec()?;
        let Some(at) = cards
            .iter()
            .position(|card| card.window().is_focused().unwrap_or(false))
        else {
            return Ok(());
        };
        let count = cards.len() as i64;
        let index = (at as i64 + steps).rem_euclid(count);
        cards[index as usize].window().focus()?;
        Ok(())
    }
}

impl Layout for Carousel3dLayout {
    fn compute(&mut self, cx: &LayoutContext) -> Vec<Placement> {
        let nodes = &cx.children;
        let index = cx
            .focused
            .as_ref()
            .and_then(|focused| nodes.iter().rposition(|node| node == focused));
        let Ok(slots) = ring(nodes.len(), index.map_or(1, |index| index + 1), cx.area, self.options) else {
            return Vec::new();
        };
        nodes
            .iter()
            .zip(slots)
            .map(|(client, slot)| {
                Placement::new(client).bounds(slot.rect).z(slot.z).rotation([
                    0.0,
                    (slot.rotation * std::f64::consts::PI) / 180.0,
                    0.0,
                ])
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_front_card_is_centred_and_the_ring_has_one_slot_per_card() {
        let area = Rect::new(0.0, 0.0, 1000.0, 800.0);
        let slots = ring(5, 1, area, RingOptions::default()).expect("a ring");
        assert_eq!(slots.len(), 5);
        assert_eq!(slots[0].rotation, 0.0);
        // The front card is centred across the area.
        assert!((slots[0].rect.x + slots[0].rect.width / 2.0 - 500.0).abs() < 1e-9);
        assert!(slots[0].z > slots[2].z, "the front is nearer the camera than the back");
    }

    #[test]
    fn a_focus_outside_the_ring_is_refused_and_an_empty_ring_is_empty() {
        let area = Rect::new(0.0, 0.0, 1000.0, 800.0);
        assert!(ring(3, 4, area, RingOptions::default()).is_err());
        assert!(ring(3, 0, area, RingOptions::default()).is_err());
        assert_eq!(ring(0, 1, area, RingOptions::default()), Ok(Vec::new()));
    }
}
