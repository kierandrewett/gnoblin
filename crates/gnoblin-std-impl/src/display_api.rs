//! The table functions of output profiles and displays: what a shell that keeps its own objects calls to make them, list them and
//! reconcile them with the outputs. The model and the reconciling are `displays.rs`.
#![allow(unsafe_code)]

use std::cell::RefCell;

use gnoblin_core_abi::{self as core_abi, GnoblinCtx};
use gnoblin_std_abi as abi;
use gnoblin_std_abi::{
    GnoblinStdDisplay, GnoblinStdDisplayInfo, GnoblinStdDisplaySpec, GnoblinStdMembers, GnoblinStdProfile,
    GnoblinStdProfileSpec, GnoblinStdSelector, GnoblinStdWorkspace,
};

use crate::displays::{Match, NeighbourSide, PlaceTarget, Position, ProfileSettings, Sel};
use crate::error::{Fail, Res};
use crate::state::State;
use crate::table::{call, fill_out, lent, mutate, owned, put, read, read_ext};

thread_local! {
    /// The members a list function lent, valid until the next call on this thread.
    static LENT: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
    /// The display id `display_info` lent.
    static LENT_ID: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Lends `items` to the caller as the members of `out`.
///
/// # Safety
///
/// `out` must be null or writable.
unsafe fn lend(items: Vec<u64>, out: *mut GnoblinStdMembers) -> Res<()> {
    let (ptr, len) = LENT.with(|lent| {
        *lent.borrow_mut() = items;
        let lent = lent.borrow();
        (lent.as_ptr(), lent.len())
    });
    // SAFETY: the caller guarantees `out` is null or writable.
    unsafe {
        put(
            out,
            GnoblinStdMembers {
                ptr: if len == 0 { core::ptr::null() } else { ptr },
                len,
            },
        )
    }
}

fn some(text: String) -> Option<String> {
    (!text.is_empty()).then_some(text)
}

/// The settings a profile spec asks for, checked.
///
/// # Safety
///
/// The strings of `spec` must be valid for the call.
unsafe fn settings_of(spec: &GnoblinStdProfileSpec) -> Res<ProfileSettings> {
    let scale = (spec.flags & abi::STD_PROFILE_HAS_SCALE != 0).then_some(spec.scale);
    if let Some(scale) = scale
        && !(scale.is_finite() && (0.5..=4.0).contains(&scale))
    {
        return Err(Fail::type_error("scale must be a number from 0.5 to 4"));
    }
    let supersample = (spec.flags & abi::STD_PROFILE_HAS_SUPERSAMPLE != 0).then_some(spec.supersample);
    if let Some(factor) = supersample
        && !(factor.is_finite() && (1.0..=16.0).contains(&factor))
    {
        return Err(Fail::type_error("supersample must be a number from 1 to 16"));
    }
    let side = match spec.position {
        abi::STD_POSITION_RIGHT_OF => Some(NeighbourSide::RightOf),
        abi::STD_POSITION_LEFT_OF => Some(NeighbourSide::LeftOf),
        abi::STD_POSITION_ABOVE => Some(NeighbourSide::Above),
        abi::STD_POSITION_BELOW => Some(NeighbourSide::Below),
        _ => None,
    };
    let position = match (spec.position, side) {
        (abi::STD_POSITION_NONE, _) => None,
        (abi::STD_POSITION_ABSOLUTE, _) => {
            if !(spec.x.is_finite() && spec.y.is_finite()) {
                return Err(Fail::type_error("position must be two finite numbers"));
            }
            Some(Position::Absolute(spec.x, spec.y))
        }
        (_, Some(side)) => {
            let target = match spec.target_kind {
                abi::STD_SELECT_PROFILE => PlaceTarget::Profile(spec.target),
                abi::STD_SELECT_OUTPUT => PlaceTarget::Output(spec.target),
                other => {
                    return Err(Fail::type_error(format!(
                        "position target kind {other} is not a profile or an output"
                    )));
                }
            };
            Some(Position::Relative {
                side,
                align: spec.align,
                target,
            })
        }
        (other, None) => return Err(Fail::type_error(format!("position kind {other} is not known"))),
    };
    // SAFETY: the caller guarantees the strings of `spec` are valid for the call.
    let (mode, transform) = unsafe { (owned(&spec.mode)?, owned(&spec.transform)?) };
    Ok(ProfileSettings {
        mode: some(mode),
        scale,
        supersample,
        transform: some(transform),
        vrr: (spec.flags & abi::STD_PROFILE_HAS_VRR != 0).then_some(spec.flags & abi::STD_PROFILE_VRR_ON != 0),
        enabled: (spec.flags & abi::STD_PROFILE_HAS_ENABLED != 0)
            .then_some(spec.flags & abi::STD_PROFILE_ENABLED_ON != 0),
        position,
    })
}

pub(crate) unsafe extern "C" fn profile_new(
    ctx: GnoblinCtx,
    spec: *const GnoblinStdProfileSpec,
    out: *mut GnoblinStdProfile,
) -> i32 {
    call("profile_new", || {
        // SAFETY: the caller guarantees `spec` is a `GnoblinStdProfileSpec` and its strings are valid for the call.
        let spec = unsafe { read_ext(spec, "profile spec") }?;
        // SAFETY: as above.
        let (id, matcher, settings) = unsafe {
            (
                owned(&spec.id)?,
                Match {
                    connector: some(owned(&spec.connector)?),
                    make: some(owned(&spec.make)?),
                    model: some(owned(&spec.model)?),
                    serial: some(owned(&spec.serial)?),
                },
                settings_of(&spec)?,
            )
        };
        if id.is_empty() {
            return Err(Fail::type_error("a profile needs an id"));
        }
        if matcher.connector.is_none() && matcher.make.is_none() && matcher.model.is_none() && matcher.serial.is_none()
        {
            return Err(Fail::type_error(
                "a profile needs at least one of connector, make, model and serial to match",
            ));
        }
        mutate(ctx, |st| {
            let handle = st.new_profile(id, matcher, settings);
            // SAFETY: the caller guarantees `out` is null or writable.
            unsafe { put(out, handle) }?;
            Ok(core_abi::E_OK)
        })
    })
}

pub(crate) unsafe extern "C" fn profile_set(
    ctx: GnoblinCtx,
    profile: GnoblinStdProfile,
    spec: *const GnoblinStdProfileSpec,
) -> i32 {
    call("profile_set", || {
        // SAFETY: the caller guarantees `spec` is a `GnoblinStdProfileSpec` and its strings are valid for the call.
        let spec = unsafe { read_ext(spec, "profile spec") }?;
        // SAFETY: as above.
        let settings = unsafe { settings_of(&spec) }?;
        mutate(ctx, |st| {
            st.set_profile(profile, settings)?;
            Ok(core_abi::E_OK)
        })
    })
}

pub(crate) unsafe extern "C" fn profile_add(ctx: GnoblinCtx, profile: GnoblinStdProfile) -> i32 {
    call("profile_add", || {
        mutate(ctx, |st| st.add_profile(profile).map(|_| core_abi::E_OK))
    })
}

pub(crate) unsafe extern "C" fn profile_remove(ctx: GnoblinCtx, profile: GnoblinStdProfile) -> i32 {
    call("profile_remove", || {
        mutate(ctx, |st| {
            st.profile(profile)?;
            Ok(i32::from(st.remove_profile(profile)))
        })
    })
}

pub(crate) unsafe extern "C" fn profile_list(ctx: GnoblinCtx, out: *mut GnoblinStdMembers) -> i32 {
    call("profile_list", || {
        let items = read(ctx, |st| Ok(st.disp.profile_list.clone()))?;
        // SAFETY: the caller guarantees `out` is null or writable.
        unsafe { lend(items, out) }?;
        Ok(core_abi::E_OK)
    })
}

fn check_mode(mode: u32) -> Res<()> {
    match mode {
        abi::STD_PRESENT_SINGLE | abi::STD_PRESENT_MIRROR | abi::STD_PRESENT_SPAN => Ok(()),
        other => Err(Fail::type_error(format!(
            "display mode {other} is not single, mirror or span"
        ))),
    }
}

pub(crate) unsafe extern "C" fn display_new(
    ctx: GnoblinCtx,
    spec: *const GnoblinStdDisplaySpec,
    out: *mut GnoblinStdDisplay,
) -> i32 {
    call("display_new", || {
        // SAFETY: the caller guarantees `spec` is a `GnoblinStdDisplaySpec` and its strings and selectors are valid for the call.
        let spec = unsafe { read_ext(spec, "display spec") }?;
        // SAFETY: as above.
        let id = unsafe { owned(&spec.id) }?;
        check_mode(spec.mode)?;
        if id.is_empty() {
            return Err(Fail::type_error("a display needs an id"));
        }
        let given: &[GnoblinStdSelector] = if spec.selectors.is_null() || spec.n_selectors == 0 {
            &[]
        } else {
            // SAFETY: the caller guarantees `n_selectors` readable selectors at `selectors`.
            unsafe { std::slice::from_raw_parts(spec.selectors, spec.n_selectors) }
        };
        if given.is_empty() {
            return Err(Fail::type_error("a display presents at least one output or profile"));
        }
        if spec.mode == abi::STD_PRESENT_SINGLE && given.len() > 1 {
            return Err(Fail::type_error(format!(
                "\"single\" presents one output; use \"mirror\" or \"span\" for {}",
                given.len()
            )));
        }
        let selectors = given
            .iter()
            .map(|one| match one.kind {
                abi::STD_SELECT_PROFILE => Ok(Sel::Profile(one.target)),
                abi::STD_SELECT_OUTPUT => Ok(Sel::Output(one.target)),
                other => Err(Fail::type_error(format!(
                    "selector kind {other} is not a profile or an output"
                ))),
            })
            .collect::<Res<Vec<Sel>>>()?;
        mutate(ctx, |st| {
            let wanted = (spec.wanted != 0).then_some(spec.wanted);
            let handle = st.new_display(id, spec.mode, selectors, wanted);
            // SAFETY: the caller guarantees `out` is null or writable.
            unsafe { put(out, handle) }?;
            Ok(core_abi::E_OK)
        })
    })
}

pub(crate) unsafe extern "C" fn display_set_mode(ctx: GnoblinCtx, display: GnoblinStdDisplay, mode: u32) -> i32 {
    call("display_set_mode", || {
        check_mode(mode)?;
        mutate(ctx, |st| {
            st.set_display_mode(display, mode)?;
            Ok(core_abi::E_OK)
        })
    })
}

pub(crate) unsafe extern "C" fn display_add(ctx: GnoblinCtx, display: GnoblinStdDisplay) -> i32 {
    call("display_add", || {
        mutate(ctx, |st| st.add_display(display).map(|_| core_abi::E_OK))
    })
}

pub(crate) unsafe extern "C" fn display_remove(ctx: GnoblinCtx, display: GnoblinStdDisplay) -> i32 {
    call("display_remove", || {
        mutate(ctx, |st| st.remove_display(display).map(i32::from))
    })
}

pub(crate) unsafe extern "C" fn display_replace(
    ctx: GnoblinCtx,
    old: GnoblinStdDisplay,
    next: GnoblinStdDisplay,
) -> i32 {
    call("display_replace", || {
        mutate(ctx, |st| st.replace_display(old, next).map(|()| core_abi::E_OK))
    })
}

pub(crate) unsafe extern "C" fn display_clear(ctx: GnoblinCtx) -> i32 {
    call("display_clear", || {
        mutate(ctx, |st| {
            st.clear_displays();
            Ok(core_abi::E_OK)
        })
    })
}

pub(crate) unsafe extern "C" fn display_list(ctx: GnoblinCtx, out: *mut GnoblinStdMembers) -> i32 {
    call("display_list", || {
        let items = read(ctx, |st| Ok(st.disp.display_list.clone()))?;
        // SAFETY: the caller guarantees `out` is null or writable.
        unsafe { lend(items, out) }?;
        Ok(core_abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn display_info(
    ctx: GnoblinCtx,
    display: GnoblinStdDisplay,
    out: *mut GnoblinStdDisplayInfo,
) -> i32 {
    call("display_info", || {
        let (info, id) = read(ctx, |st: &State| {
            let rec = st.display(display)?;
            let mut flags = 0;
            if rec.implicit {
                flags |= abi::STD_DISPLAY_IMPLICIT;
            }
            if rec.listed {
                flags |= abi::STD_DISPLAY_LISTED;
            }
            let shown = match st.display_shown(display) {
                Some(shown) => {
                    flags |= abi::STD_DISPLAY_HAS_DESK;
                    shown.unwrap_or(0)
                }
                None => 0,
            };
            Ok((
                GnoblinStdDisplayInfo {
                    size: size_of::<GnoblinStdDisplayInfo>() as u32,
                    flags,
                    mode: rec.mode,
                    reserved: 0,
                    id: gnoblin_core_abi::GnoblinStr {
                        ptr: core::ptr::null(),
                        len: 0,
                    },
                    shown,
                    content: rec.content.unwrap_or(0),
                    wanted: rec.wanted.unwrap_or(0),
                },
                rec.id.clone(),
            ))
        })?;
        let mut info = info;
        LENT_ID.with(|held| {
            *held.borrow_mut() = id;
            info.id = lent(&held.borrow());
        });
        // SAFETY: the caller guarantees `out` is null or a `GnoblinStdDisplayInfo` whose first field is its size.
        unsafe { fill_out(out, &info, "display info") }?;
        Ok(core_abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn display_outputs(
    ctx: GnoblinCtx,
    display: GnoblinStdDisplay,
    out: *mut GnoblinStdMembers,
) -> i32 {
    call("display_outputs", || {
        let items = read(ctx, |st| {
            st.display(display)?;
            Ok(st.display_outputs(display))
        })?;
        // SAFETY: the caller guarantees `out` is null or writable.
        unsafe { lend(items, out) }?;
        Ok(core_abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn display_at(ctx: GnoblinCtx, x: f64, y: f64, out: *mut GnoblinStdDisplay) -> i32 {
    call("display_at", || {
        let found = mutate(ctx, |st| Ok(st.display_at(x, y).unwrap_or(0)))?;
        // SAFETY: the caller guarantees `out` is null or writable.
        unsafe { put(out, found) }?;
        Ok(core_abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn display_assign(
    ctx: GnoblinCtx,
    display: GnoblinStdDisplay,
    workspace: GnoblinStdWorkspace,
) -> i32 {
    call("display_assign", || {
        mutate(ctx, |st| {
            st.assign_display(display, workspace)?;
            Ok(core_abi::E_OK)
        })
    })
}

pub(crate) unsafe extern "C" fn display_snapshot(ctx: GnoblinCtx, out: *mut u64) -> i32 {
    call("display_snapshot", || {
        let token = mutate(ctx, |st| Ok(st.snapshot_displays()))?;
        // SAFETY: the caller guarantees `out` is null or writable.
        unsafe { put(out, token) }?;
        Ok(core_abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn display_restore(ctx: GnoblinCtx, token: u64) -> i32 {
    call("display_restore", || {
        mutate(ctx, |st| {
            st.restore_displays(token);
            Ok(core_abi::E_OK)
        })
    })
}

pub(crate) unsafe extern "C" fn display_reflow(ctx: GnoblinCtx) -> i32 {
    call("display_reflow", || {
        mutate(ctx, |st| {
            st.reflow_displays();
            Ok(core_abi::E_OK)
        })
    })
}
