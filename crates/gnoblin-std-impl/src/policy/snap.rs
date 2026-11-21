//! `snap`: where a dragged window tiles to. The pointer near the left or the right edge of the work area names the half it tiles to, near the
//! top edge all of it; left and right win over top at a corner. A window is snapped with `client_state.snap_to` and its record is
//! `client_state.snap`; the move binding snaps a window on a drop when the seat's `edge_tiling` is on.

use gnoblin_object::server::Builder;
use gnoblin_object::{Fail, Val};
use gnoblin_std_abi::STD_SNAP_MARGIN;

use super::stack::{rect_in, rect_val};
use crate::bindings::steps::edge_zone;
use crate::client_state::tile_rect;
use crate::object::record::{enumeration, record};

pub(super) fn register(builder: &mut Builder) {
    let _ = builder
        .class::<()>("snap")
        .ctor(|_cx, _inits| Ok(()))
        .get("margin", |_, _| Ok(Val::Double(f64::from(STD_SNAP_MARGIN))))
        .function("edge_zone", |_cx, args| {
            let area = rect_in(&args[2])?;
            let margin = match args.get(3) {
                None | Some(Val::None) => f64::from(STD_SNAP_MARGIN),
                Some(value) => value.double()?,
            };
            if margin.is_nan() || margin < 0.0 {
                return Err(Fail::type_error("the margin is a distance of 0 or more"));
            }
            Ok(edge_zone(args[0].double()?, args[1].double()?, area, margin)
                .map_or(Val::None, |zone| record(vec![enumeration(zone)])))
        })
        .function("tile_rect", |_cx, args| {
            let zone = args[0].enum_value()?;
            tile_rect(zone, &rect_in(&args[1])?)
                .map(|rect| rect_val(&rect))
                .ok_or_else(|| Fail::type_error(format!("{zone} is not a snap zone")))
        });
}
