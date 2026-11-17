//! Dividers: the edge between two tiles that a split-resize drag moves.
//!
//! A layout with dividers (the [dwindle](crate::DwindleLayout)'s splits) answers [`Layout::find_split`](crate::Layout::find_split)
//! with the [`Split`] a drag that pressed a client at a point would move. A [`Divider`] says how the pointer's travel maps to a ratio:
//! dragging `span` pixels along its [`SplitAxis`] moves `ratio` by one, in the direction `direction` says, and the layout holds the ratio to
//! `min` and `max`. The [`SplitResizeBinding`](crate::SplitResizeBinding) is the drag that drives one.
#![allow(unsafe_code)]

use gnoblin_core::NodeId;
use gnoblin_std_abi::{self as abi, GnoblinStdSplit};

use crate::bindings::Point;
use crate::client::AsClient;
use crate::error::Result;
use crate::workspace::Workspace;

/// The pointer axis that carries a divider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitAxis {
    /// The pointer's horizontal travel.
    X,
    /// The pointer's vertical travel.
    Y,
}

/// One divider as a layout describes it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Divider {
    /// The axis of the pointer's travel that moves the divider.
    pub axis: SplitAxis,
    /// Pixels of pointer travel that change `ratio` by 1. Above zero.
    pub span: f64,
    /// `1.0` when moving the pointer right or down grows `ratio`, `-1.0` when it shrinks it.
    pub direction: f64,
    /// The lowest ratio the layout allows.
    pub min: f64,
    /// The highest ratio the layout allows.
    pub max: f64,
    /// The divider's position now, as a ratio.
    pub ratio: f64,
}

impl Divider {
    /// The ratio the pointer's travel from `start` to `at` carries the divider to, from `start_ratio`, held to `min..=max`.
    pub fn follow(&self, start_ratio: f64, start: Point, at: Point) -> f64 {
        let travel = match self.axis {
            SplitAxis::X => at.x - start.x,
            SplitAxis::Y => at.y - start.y,
        };
        (start_ratio + self.direction * travel / self.span).clamp(self.min, self.max)
    }
}

/// The dividers one drag moves: the nearest one, and optionally a second on the other axis that moves with it (a window resized by both
/// of the dividers that bound it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Split {
    /// Names this drag's dividers to the layout (`set_split`, `reset_split`). A layout keeps one drag at a time.
    pub token: u64,
    /// The divider the drag moves.
    pub divider: Divider,
    /// A second divider on the other axis that the same drag moves, each by the pointer's travel along its own axis.
    pub cross: Option<Divider>,
}

fn axis_of(raw: u32) -> SplitAxis {
    if raw == abi::STD_AXIS_Y {
        SplitAxis::Y
    } else {
        SplitAxis::X
    }
}

fn raw_axis(axis: SplitAxis) -> u32 {
    match axis {
        SplitAxis::X => abi::STD_AXIS_X,
        SplitAxis::Y => abi::STD_AXIS_Y,
    }
}

impl Split {
    /// The table's description of this split.
    pub(crate) fn to_raw(self) -> GnoblinStdSplit {
        let cross = self.cross;
        GnoblinStdSplit {
            size: size_of::<GnoblinStdSplit>() as u32,
            flags: if cross.is_some() { abi::STD_SPLIT_CROSS } else { 0 },
            token: self.token,
            axis: raw_axis(self.divider.axis),
            direction: if self.divider.direction < 0.0 { -1 } else { 1 },
            span: self.divider.span,
            min: self.divider.min,
            max: self.divider.max,
            ratio: self.divider.ratio,
            cross_axis: cross.map_or(0, |c| raw_axis(c.axis)),
            cross_direction: cross.map_or(1, |c| if c.direction < 0.0 { -1 } else { 1 }),
            cross_span: cross.map_or(0.0, |c| c.span),
            cross_min: cross.map_or(0.0, |c| c.min),
            cross_max: cross.map_or(0.0, |c| c.max),
            cross_ratio: cross.map_or(0.0, |c| c.ratio),
        }
    }

    /// A split from the table's description.
    fn from_raw(raw: &GnoblinStdSplit) -> Split {
        let divider = |axis: u32, direction: i32, span: f64, min: f64, max: f64, ratio: f64| Divider {
            axis: axis_of(axis),
            span,
            direction: f64::from(direction.signum()),
            min,
            max,
            ratio,
        };
        Split {
            token: raw.token,
            divider: divider(raw.axis, raw.direction, raw.span, raw.min, raw.max, raw.ratio),
            cross: (raw.flags & abi::STD_SPLIT_CROSS != 0).then(|| {
                divider(
                    raw.cross_axis,
                    raw.cross_direction,
                    raw.cross_span,
                    raw.cross_min,
                    raw.cross_max,
                    raw.cross_ratio,
                )
            }),
        }
    }
}

impl Workspace {
    /// The divider that a drag which pressed `client` at `at` would move, from this workspace's layout: `None` when the layout has no
    /// divider there (a grid, a monocle, a layout with none at all).
    ///
    /// # Errors
    ///
    /// The workspace is gone, or the generation retired.
    pub fn find_split(&self, client: impl AsClient, at: Point) -> Result<Option<Split>> {
        let std = self.std();
        let mut raw = Split {
            token: 0,
            divider: Divider {
                axis: SplitAxis::X,
                span: 1.0,
                direction: 1.0,
                min: 0.0,
                max: 1.0,
                ratio: 0.0,
            },
            cross: None,
        }
        .to_raw();
        let NodeId(id) = client.client_id();
        // SAFETY: `workspace_find_split` fills the struct it is given up to its `size`; `raw` is a live local.
        let found =
            std.check(unsafe { (std.api().workspace_find_split)(std.ctx(), self.raw(), id, at.x, at.y, &mut raw) })?;
        Ok((found == 1).then(|| Split::from_raw(&raw)))
    }

    /// Moves the dividers of `split` (the drag the layout named by its token) to `ratio` and, when it has a second one, `cross_ratio`.
    /// The workspace computes again.
    ///
    /// # Errors
    ///
    /// The workspace is gone, a ratio is not finite, or the generation retired.
    pub fn set_split(&self, split: &Split, ratio: f64, cross_ratio: Option<f64>) -> Result<()> {
        let std = self.std();
        let cross = cross_ratio.unwrap_or(f64::NAN);
        // SAFETY: `workspace_set_split` takes the context, a workspace handle, a token and two numbers.
        std.check(unsafe { (std.api().workspace_set_split)(std.ctx(), self.raw(), split.token, ratio, cross) })
            .map(|_| ())
    }

    /// Puts the dividers of `split` back as they were when the layout returned them: a drag that was cancelled.
    ///
    /// # Errors
    ///
    /// The workspace is gone, or the generation retired.
    pub fn reset_split(&self, split: &Split) -> Result<()> {
        let std = self.std();
        // SAFETY: `workspace_reset_split` takes the context, a workspace handle and a token.
        std.check(unsafe { (std.api().workspace_reset_split)(std.ctx(), self.raw(), split.token) })
            .map(|_| ())
    }
}
