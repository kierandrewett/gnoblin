//! The ring: where a room's panels go round the viewer, from the windows' pixel sizes and the focus alone. Millimetres and radians.

use gnoblin_object::{Res, Val};

/// The dimensions of a ring.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RingParams {
    pub(crate) radius: f64,
    pub(crate) eye: f64,
    pub(crate) mm_per_px: f64,
    pub(crate) focus_scale: f64,
    pub(crate) focus_pull: f64,
    pub(crate) gap: f64,
    pub(crate) half_arc: f64,
    pub(crate) max_panels: f64,
    pub(crate) stack_step: f64,
}

impl Default for RingParams {
    fn default() -> RingParams {
        RingParams {
            radius: 1600.0,
            eye: 1400.0,
            mm_per_px: 0.75,
            focus_scale: 1.15,
            focus_pull: 100.0,
            gap: 3.0_f64.to_radians(),
            half_arc: 150.0_f64.to_radians(),
            max_panels: 12.0,
            stack_step: 250.0,
        }
    }
}

impl RingParams {
    /// The `ring_params` record.
    pub(crate) fn val(&self) -> Val {
        Val::Record {
            class: 0,
            fields: [
                self.radius,
                self.eye,
                self.mm_per_px,
                self.focus_scale,
                self.focus_pull,
                self.gap,
                self.half_arc,
                self.max_panels,
                self.stack_step,
            ]
            .into_iter()
            .map(Val::Double)
            .collect(),
        }
    }

    /// The ring a `ring_params` record holds; a field left out is the default's.
    pub(crate) fn of(value: &Val) -> Res<RingParams> {
        let fields = value.record()?;
        let defaults = RingParams::default();
        let at = |index: usize, default: f64| -> Res<f64> {
            match fields.get(index) {
                None | Some(Val::None) => Ok(default),
                Some(value) => value.double(),
            }
        };
        Ok(RingParams {
            radius: at(0, defaults.radius)?,
            eye: at(1, defaults.eye)?,
            mm_per_px: at(2, defaults.mm_per_px)?,
            focus_scale: at(3, defaults.focus_scale)?,
            focus_pull: at(4, defaults.focus_pull)?,
            gap: at(5, defaults.gap)?,
            half_arc: at(6, defaults.half_arc)?,
            max_panels: at(7, defaults.max_panels)?,
            stack_step: at(8, defaults.stack_step)?,
        })
    }
}

/// Where one panel goes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct RingPose {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) z: f64,
    /// The turn about y.
    pub(crate) yaw: f64,
    pub(crate) scale: f64,
    pub(crate) focused: bool,
    pub(crate) behind: u64,
}

impl RingPose {
    fn val(&self) -> Val {
        Val::Record {
            class: 0,
            fields: vec![
                Val::Double(self.x),
                Val::Double(self.y),
                Val::Double(self.z),
                Val::Double(self.yaw),
                Val::Double(self.scale),
                Val::Bool(self.focused),
                Val::Uint(self.behind),
            ],
        }
    }
}

/// The poses, in the views' order, and the order to list the views in, farthest first.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RingResult {
    pub(crate) poses: Vec<RingPose>,
    pub(crate) order: Vec<usize>,
}

impl RingResult {
    /// The `ring_result` record.
    pub(crate) fn val(&self) -> Val {
        Val::Record {
            class: 0,
            fields: vec![
                Val::List(self.poses.iter().map(RingPose::val).collect()),
                Val::List(self.order.iter().map(|at| Val::Uint(*at as u64)).collect()),
            ],
        }
    }
}

/// Places panels of the pixel widths `widths` round a ring. The focused one (`focus`, an index) is at the front, with its neighbours in
/// order on either side (the right holds the extra one), wrapping round; with no focus the first is at the front, not enlarged. The panels
/// shrink together until both sides fit in `half_arc`, gaps included, and the panels past `max_panels` stack behind the ring panel at the
/// same angle, a tier further back for every full turn.
pub(crate) fn arrange(widths: &[f64], focus: Option<usize>, p: &RingParams) -> RingResult {
    let n = widths.len();
    let mut poses = vec![RingPose::default(); n];
    let mut order = Vec::with_capacity(n);
    if n == 0 {
        return RingResult { poses, order };
    }
    let focus_at = focus.filter(|at| *at < n);
    let has_focus = focus_at.is_some();
    let limit = if p.max_panels.is_finite() && p.max_panels >= 1.0 {
        p.max_panels.floor() as usize
    } else {
        1
    };
    let m = n.min(limit);
    let mut seq: Vec<usize> = (0..n).collect();
    if let Some(at) = focus_at
        && at >= m
    {
        seq.swap(m - 1, at);
    }
    let centre_slot = focus_at
        .and_then(|at| seq[..m].iter().position(|slot| *slot == at))
        .unwrap_or(0);
    let wrap = |k: isize| -> usize { k.rem_euclid(m as isize) as usize };
    let nr = m / 2;
    let right: Vec<usize> = (1..=nr).map(|j| seq[wrap(centre_slot as isize + j as isize)]).collect();
    let left: Vec<usize> = (1..m - nr)
        .map(|j| seq[wrap(centre_slot as isize - j as isize)])
        .collect();
    let centre = seq[centre_slot];
    let span = |i: usize, focused: bool| -> f64 {
        (if focused { p.focus_scale } else { 1.0 }) * widths[i].max(0.0) * p.mm_per_px / p.radius
    };
    let a_centre = span(centre, has_focus);
    let mut shrink: f64 = 1.0;
    for side in [&right, &left] {
        let need = a_centre / 2.0 + side.iter().map(|i| span(*i, false)).sum::<f64>();
        let allowed = p.half_arc - side.len() as f64 * p.gap;
        if need > allowed {
            shrink = shrink.min(allowed / need);
        }
    }
    let mut phi = vec![0.0_f64; n];
    for (sign, side) in [(1.0, &right), (-1.0, &left)] {
        let mut edge = shrink * a_centre / 2.0;
        for i in side {
            let half = shrink * span(*i, false) / 2.0;
            let at = edge + p.gap + half;
            phi[*i] = sign * at;
            edge = at + half;
        }
    }
    let mut put = |i: usize, angle: f64, radius: f64, focused: bool, tier: u64| {
        poses[i] = RingPose {
            x: radius * angle.sin(),
            y: p.eye,
            z: -radius * angle.cos(),
            yaw: -angle,
            scale: shrink * (if focused { p.focus_scale } else { 1.0 }) * p.mm_per_px,
            focused,
            behind: tier,
        };
    };
    for &i in &seq[..m] {
        let focused = has_focus && i == centre;
        put(
            i,
            phi[i],
            if focused { p.radius - p.focus_pull } else { p.radius },
            focused,
            0,
        );
    }
    // The extras sit behind the ring panel at the same angle, a tier further back for every full turn of the ring.
    let mut tiers: Vec<Vec<usize>> = Vec::new();
    for j in 1..=n - m {
        let tier = (j - 1) / m + 1;
        let i = seq[m + j - 1];
        put(
            i,
            phi[seq[wrap(j as isize - 1)]],
            p.radius + p.stack_step * tier as f64,
            false,
            tier as u64,
        );
        if tiers.len() <= tier {
            tiers.resize(tier + 1, Vec::new());
        }
        tiers[tier].push(i);
    }
    for tier in tiers.iter().skip(1).rev() {
        order.extend(tier.iter().copied());
    }
    let mut ring: Vec<usize> = seq[..m]
        .iter()
        .copied()
        .filter(|i| !(has_focus && *i == centre))
        .collect();
    ring.sort_unstable();
    order.extend(ring);
    if has_focus {
        order.push(centre);
    }
    RingResult { poses, order }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(got: f64, want: f64) {
        assert!(
            (got - want).abs() <= 1e-6 * want.abs().max(1.0),
            "got {got}, want {want}"
        );
    }

    #[test]
    fn one_panel_is_ahead_and_a_focused_one_larger_and_closer() {
        let plain = arrange(&[800.0], None, &RingParams::default());
        near(plain.poses[0].z, -1600.0);
        near(plain.poses[0].scale, 0.75);
        let focused = arrange(&[800.0], Some(0), &RingParams::default());
        near(focused.poses[0].z, -1500.0);
        near(focused.poses[0].scale, 0.8625);
        assert!(arrange(&[], None, &RingParams::default()).order.is_empty());
    }

    #[test]
    fn three_panels_form_a_flat_arc_with_the_focused_one_last() {
        let got = arrange(&[800.0; 3], Some(1), &RingParams::default());
        near(got.poses[0].x, -703.8364939904852);
        near(got.poses[0].z, -1436.8765394866678);
        near(got.poses[0].yaw, 0.45548487755982986);
        near(got.poses[2].x, 703.8364939904852);
        assert_eq!(got.order, [0, 2, 1]);
    }
}
