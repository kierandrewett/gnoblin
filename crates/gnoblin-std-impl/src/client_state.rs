//! What the library remembers about a client beyond the tree: the room its frame takes, the sizes it allows itself, the rectangle it
//! floats at, and the zone it is snapped to.
//!
//! The TypeScript library keeps the same four things as properties of a client (`frameInset`, `minSize` and `maxSize`,
//! `floatingBounds`) and as a record of `gnoblin-std/snap`. Here they are the library's, so every language's shell reads and writes
//! the one record, and the library's refit of a window to a new work area (`fit.rs`) can honour them.
#![allow(unsafe_code)]

use gnoblin_core::Rect;
use gnoblin_core_abi::{self as abi, GnoblinCtx, GnoblinNode};
use gnoblin_std_abi::{
    GnoblinStdClientState, GnoblinStdRect, STD_CLIENT_HAS_FLOATING, STD_CLIENT_HAS_MAX, STD_CLIENT_HAS_MIN,
    STD_SNAP_LEFT, STD_SNAP_NONE, STD_SNAP_RIGHT, STD_SNAP_TOP,
};

use crate::error::Fail;
use crate::floating::{Inset, grow, shrink};
use crate::nodes;
use crate::state::{NodeId, State};
use crate::table::{call, fill_out, mutate, read, read_ext};

/// Where a snapped client floated before it snapped.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Snap {
    pub(crate) zone: u32,
    pub(crate) floating: Rect,
}

/// The part of the work area a window tiles to in `zone`: the halves are whole pixels. `None` for a zone that does not tile.
pub(crate) fn tile_rect(zone: u32, area: &Rect) -> Option<Rect> {
    let half = (area.width / 2.0).floor();
    match zone {
        STD_SNAP_LEFT => Some(Rect::new(area.x, area.y, half, area.height)),
        STD_SNAP_RIGHT => Some(Rect::new(area.x + half, area.y, area.width - half, area.height)),
        STD_SNAP_TOP => Some(*area),
        _ => None,
    }
}

fn rect_out(rect: &Rect) -> GnoblinStdRect {
    GnoblinStdRect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }
}

fn rect_in(rect: &GnoblinStdRect) -> Rect {
    Rect::new(rect.x, rect.y, rect.width, rect.height)
}

/// The rectangle `rect` becomes as a frame inside `area`: the frame (the content with `inset` round it) shrunk to fit and moved the
/// least distance that puts all of it inside, then the content inside that.
pub(crate) fn fit_content(rect: &Rect, inset: &Inset, area: &Rect) -> Rect {
    shrink(&crate::fit::fit_into(&grow(rect, inset), area), inset)
}

impl State {
    /// The room the client's frame takes round its content.
    pub(crate) fn inset_of(&self, client: NodeId) -> Inset {
        self.rec.get(&client).map(|rec| rec.inset).unwrap_or_default()
    }

    /// `snapClient`: tiles the client to `zone` of `area` (less its frame) when edge tiling is on. A maximised client floats first,
    /// so unsnapping returns it to the rectangle it had before; a snapped client keeps the rectangle it first floated at.
    pub(crate) fn snap_client(&mut self, client: NodeId, zone: u32, area: &Rect, edge_tiling: bool) -> bool {
        let Some(rect) = edge_tiling.then(|| tile_rect(zone, area)).flatten() else {
            return false;
        };
        let g = self.g.clone();
        let inset = self.inset_of(client);
        let maximised = nodes::flag(&g, client, "maximised");
        let own = nodes::rect_of(&g, client);
        let Some(rec) = self.rec.get_mut(&client) else {
            return false;
        };
        let floating = match rec.snap {
            Some(snap) => snap.floating,
            None if maximised => rec.float_rect.unwrap_or(own),
            None => own,
        };
        rec.snap = Some(Snap { zone, floating });
        if maximised {
            nodes::write_flag(&g, client, "maximised", false);
        }
        nodes::write_rect(&g, client, &shrink(&rect, &inset));
        true
    }

    /// `unsnapClient`: puts a snapped client back where it floated. With `forget` the client is left where it is.
    pub(crate) fn unsnap_client(&mut self, client: NodeId, forget: bool) -> bool {
        let Some(snap) = self.rec.get_mut(&client).and_then(|rec| rec.snap.take()) else {
            return false;
        };
        if !forget {
            nodes::write_rect(&self.g.clone(), client, &snap.floating);
        }
        true
    }

    /// The `GnoblinStdClientState` of a client.
    fn client_state_of(&self, client: NodeId) -> Option<GnoblinStdClientState> {
        let rec = self.rec.get(&client)?;
        let mut flags = 0;
        let (min_width, min_height) = rec.min.map_or((0.0, 0.0), |size| {
            flags |= STD_CLIENT_HAS_MIN;
            size
        });
        let (max_width, max_height) = rec.max.map_or((0.0, 0.0), |size| {
            flags |= STD_CLIENT_HAS_MAX;
            size
        });
        let floating = rec.float_rect.map_or_else(
            || rect_out(&Rect::new(0.0, 0.0, 0.0, 0.0)),
            |rect| {
                flags |= STD_CLIENT_HAS_FLOATING;
                rect_out(&rect)
            },
        );
        let (snap_zone, snap_floating) = rec.snap.map_or((STD_SNAP_NONE, Rect::new(0.0, 0.0, 0.0, 0.0)), |snap| {
            (snap.zone, snap.floating)
        });
        Some(GnoblinStdClientState {
            size: size_of::<GnoblinStdClientState>() as u32,
            flags,
            inset_top: rec.inset.top,
            inset_right: rec.inset.right,
            inset_bottom: rec.inset.bottom,
            inset_left: rec.inset.left,
            min_width,
            min_height,
            max_width,
            max_height,
            floating,
            snap_zone,
            reserved: 0,
            snap_floating: rect_out(&snap_floating),
        })
    }

    /// Keeps the frame inset, the size limits and the floating rectangle `state` carries. A new inset marks the workspace dirty:
    /// a layout places the frame, so the content moves with what the frame takes.
    fn set_client_state(&mut self, client: NodeId, state: &GnoblinStdClientState) -> Result<(), Fail> {
        let inset = Inset {
            top: state.inset_top,
            right: state.inset_right,
            bottom: state.inset_bottom,
            left: state.inset_left,
        };
        let finite = [
            inset.top,
            inset.right,
            inset.bottom,
            inset.left,
            state.min_width,
            state.min_height,
            state.max_width,
            state.max_height,
            state.floating.x,
            state.floating.y,
            state.floating.width,
            state.floating.height,
        ]
        .iter()
        .all(|n| n.is_finite());
        if !finite
            || [inset.top, inset.right, inset.bottom, inset.left]
                .iter()
                .any(|n| *n < 0.0)
        {
            return Err(Fail::type_error(
                "a client's frame inset must be four finite numbers of at least 0, and its limits and rectangle finite",
            ));
        }
        let Some(rec) = self.rec.get_mut(&client) else {
            return Err(Fail::new(abi::E_NO_SUCH_NODE, format!("node {client} is not a client")));
        };
        let changed = rec.inset != inset;
        rec.inset = inset;
        rec.min = (state.flags & STD_CLIENT_HAS_MIN != 0).then_some((state.min_width, state.min_height));
        rec.max = (state.flags & STD_CLIENT_HAS_MAX != 0).then_some((state.max_width, state.max_height));
        rec.float_rect = (state.flags & STD_CLIENT_HAS_FLOATING != 0).then(|| rect_in(&state.floating));
        if changed && let Some(workspace) = rec.workspace.and_then(|id| self.ws.get_mut(&id)) {
            workspace.dirty = true;
        }
        Ok(())
    }
}

pub(crate) unsafe extern "C" fn client_state(
    ctx: GnoblinCtx,
    client: GnoblinNode,
    out: *mut GnoblinStdClientState,
) -> i32 {
    call("client_state", || {
        let state = read(ctx, |st| {
            st.client_state_of(client)
                .ok_or_else(|| Fail::new(abi::E_NO_SUCH_NODE, format!("node {client} is not a client")))
        })?;
        // SAFETY: the caller guarantees `out` is null or a `GnoblinStdClientState` whose first field is its size.
        unsafe { fill_out(out, &state, "client state") }?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn client_set_state(
    ctx: GnoblinCtx,
    client: GnoblinNode,
    state: *const GnoblinStdClientState,
) -> i32 {
    call("client_set_state", || {
        // SAFETY: the caller guarantees `state` is null or a `GnoblinStdClientState` whose first field is its size.
        let state = unsafe { read_ext(state, "client state") }?;
        mutate(ctx, |st| st.set_client_state(client, &state).map(|()| abi::E_OK))
    })
}

pub(crate) unsafe extern "C" fn client_snap(
    ctx: GnoblinCtx,
    client: GnoblinNode,
    zone: u32,
    area: *const GnoblinStdRect,
    edge_tiling: u32,
) -> i32 {
    call("client_snap", || {
        // SAFETY: the caller guarantees `area` is null or a readable `GnoblinStdRect`.
        let area = match unsafe { area.as_ref() } {
            Some(area) => rect_in(area),
            None => return Err(Fail::err("client_snap: a null area")),
        };
        mutate(ctx, |st| {
            if !st.rec.contains_key(&client) || !nodes::alive(&st.g, client) {
                return Err(Fail::new(abi::E_NO_SUCH_NODE, format!("node {client} is not a client")));
            }
            Ok(i32::from(st.snap_client(client, zone, &area, edge_tiling != 0)))
        })
    })
}

pub(crate) unsafe extern "C" fn client_unsnap(ctx: GnoblinCtx, client: GnoblinNode, forget: u32) -> i32 {
    call("client_unsnap", || {
        mutate(ctx, |st| {
            if !st.rec.contains_key(&client) {
                return Err(Fail::new(abi::E_NO_SUCH_NODE, format!("node {client} is not a client")));
            }
            Ok(i32::from(st.unsnap_client(client, forget != 0)))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_halves_are_whole_pixels_and_the_top_zone_takes_the_area() {
        let area = Rect::new(0.0, 0.0, 1001.0, 800.0);
        assert_eq!(tile_rect(STD_SNAP_LEFT, &area), Some(Rect::new(0.0, 0.0, 500.0, 800.0)));
        assert_eq!(
            tile_rect(STD_SNAP_RIGHT, &area),
            Some(Rect::new(500.0, 0.0, 501.0, 800.0))
        );
        assert_eq!(tile_rect(STD_SNAP_TOP, &area), Some(area));
        assert_eq!(tile_rect(STD_SNAP_NONE, &area), None);
    }

    #[test]
    fn a_frame_stays_inside_the_area() {
        let inset = Inset {
            top: 30.0,
            right: 2.0,
            bottom: 2.0,
            left: 2.0,
        };
        let area = Rect::new(0.0, 0.0, 1000.0, 800.0);
        let content = fit_content(&Rect::new(-50.0, -50.0, 300.0, 200.0), &inset, &area);
        assert_eq!(grow(&content, &inset).x, 0.0);
        assert_eq!(grow(&content, &inset).y, 0.0);
    }
}
