//! Turns a layout's reply into a canonical layout, or refuses it.

use std::collections::BTreeSet;

use gnoblin_types::WindowId;

use crate::error::{LayoutFailure, ReplyError, Subject};
use crate::fallback::fallback;
use crate::placement::{Camera, Handled, LayoutEffect, Origin, Params, Placement, Reply};
use crate::transform::BOUND;

/// A reply that passed every check, in canonical order.
///
/// Placements and effects are sorted by window id, so two runs over the same
/// input produce the same value.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    /// One placement per placed window, sorted by window id.
    pub placements: Vec<Placement>,
    /// The camera the placements are projected through.
    pub camera: Camera,
    /// Side effects, sorted by window id.
    pub effects: Vec<LayoutEffect>,
    /// Whether the layout answered or the fallback was used.
    pub origin: Origin,
}

/// Accepts or refuses `reply` for the run described by `params`.
///
/// A [`Reply::Declined`] becomes the built-in fallback. A
/// [`Reply::Handled`] is checked in full: every tiled window must be placed or
/// floated exactly once, no unknown window may appear, every number must be
/// finite and bounded, every transform invertible, and the camera well formed.
/// Any failure refuses the whole reply, so a layout is never applied in part.
pub fn settle(params: &Params, reply: Reply) -> Result<Layout, LayoutFailure> {
    match reply {
        Reply::Declined => Ok(fallback(params)),
        Reply::Handled(handled) => validate(params, handled).map_err(LayoutFailure::RefusedReply),
    }
}

fn validate(params: &Params, handled: Handled) -> Result<Layout, ReplyError> {
    let tiled: BTreeSet<WindowId> = params.windows().iter().map(|window| window.id).collect();

    let mut placed = BTreeSet::new();
    for placement in &handled.placements {
        if !tiled.contains(&placement.window) {
            return Err(ReplyError::UnknownWindow(placement.window));
        }
        if !placed.insert(placement.window) {
            return Err(ReplyError::DuplicatePlacement(placement.window));
        }
        check_placement(placement)?;
    }

    let mut floated = BTreeSet::new();
    for effect in &handled.effects {
        let window = effect.window();
        if !tiled.contains(&window) {
            return Err(ReplyError::UnknownWindow(window));
        }
        if !floated.insert(window) {
            return Err(ReplyError::DuplicateEffect(window));
        }
        if placed.contains(&window) {
            return Err(ReplyError::FloatedWindowPlaced(window));
        }
    }

    for window in &tiled {
        if !placed.contains(window) && !floated.contains(window) {
            return Err(ReplyError::MissingWindow(*window));
        }
    }

    check_camera(&handled.camera)?;

    let mut placements = handled.placements;
    placements.sort_by_key(|placement| placement.window);
    let mut effects = handled.effects;
    effects.sort_by_key(LayoutEffect::window);

    Ok(Layout {
        placements,
        camera: handled.camera,
        effects,
        origin: Origin::Reply,
    })
}

fn check_placement(placement: &Placement) -> Result<(), ReplyError> {
    let subject = Subject::Window(placement.window);
    check_length(subject, "width", placement.width)?;
    check_length(subject, "height", placement.height)?;
    if placement.width < 0.0 || placement.height < 0.0 {
        return Err(ReplyError::NegativeSize(placement.window));
    }
    if !placement.opacity.is_finite() {
        return Err(ReplyError::NonFinite {
            subject,
            field: "opacity",
        });
    }
    if !(0.0..=1.0).contains(&placement.opacity) {
        return Err(ReplyError::OpacityOutOfRange(placement.window));
    }
    placement.transform.check(subject)
}

fn check_length(subject: Subject, field: &'static str, value: f64) -> Result<(), ReplyError> {
    if !value.is_finite() {
        Err(ReplyError::NonFinite { subject, field })
    } else if value.abs() > BOUND {
        Err(ReplyError::OutOfBounds { subject, field })
    } else {
        Ok(())
    }
}

fn check_camera(camera: &Camera) -> Result<(), ReplyError> {
    let Camera::Perspective {
        fov_degrees,
        near,
        far,
        pose,
    } = *camera
    else {
        return Ok(());
    };
    if !fov_degrees.is_finite() {
        return Err(ReplyError::NonFinite {
            subject: Subject::Camera,
            field: "fov",
        });
    }
    if !(fov_degrees > 0.0 && fov_degrees < 180.0) {
        return Err(ReplyError::FieldOfViewOutOfRange);
    }
    if !near.is_finite() {
        return Err(ReplyError::NonFinite {
            subject: Subject::Camera,
            field: "near",
        });
    }
    if near <= 0.0 {
        return Err(ReplyError::NearPlaneInvalid);
    }
    if !far.is_finite() {
        return Err(ReplyError::NonFinite {
            subject: Subject::Camera,
            field: "far",
        });
    }
    if far <= near {
        return Err(ReplyError::FarPlaneInvalid);
    }
    pose.check(Subject::Camera)
}
