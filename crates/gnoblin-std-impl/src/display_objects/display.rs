//! `display`: one viewport presented on one output (`single`), copied to several (`mirror`) or stretched over their union (`span`).
//!
//! `data/js/gnoblin-std/display/display.ts` is the reference. The library's display (`displays.rs`) holds the selectors, the mode and
//! what the display presents now; this class checks what a shell writes and gives the objects of what the library holds.

use gnoblin_core_abi::GnoblinCtx;
use gnoblin_object::server::{Builder, Cx};
use gnoblin_object::{ErrorCode, Fail, Obj, Res, Val};
use gnoblin_std_abi as abi;

use super::profile::{profile_of, profile_val};
use super::viewport::{new_rec, viewport_obj, viewport_of, viewport_val};
use super::{DISPLAYS, edit, look};
use crate::displays::{Hid, Sel};
use crate::object::record::{enumeration, field, record};
use crate::state::State;

/// What a `display` object holds: the display the library calls `hid`.
pub(super) struct DisplayRef {
    ctx: GnoblinCtx,
    hid: Hid,
}

/// The object of the display `hid`: the one a shell has, or a new one (a display the library made for an output has none until a shell
/// asks).
pub(super) fn display_obj(cx: &mut Cx<'_>, hid: Hid) -> Res<Obj> {
    let ctx = cx.ctx();
    let found = DISPLAYS.with(|held| held.borrow().get(&(ctx, hid)).copied());
    if let Some(obj) = found
        && cx.class_of(obj).is_ok()
    {
        return Ok(obj);
    }
    let obj = cx.spawn("display", DisplayRef { ctx, hid })?;
    DISPLAYS.with(|held| held.borrow_mut().insert((ctx, hid), obj));
    Ok(obj)
}

/// The object of the display `hid`, as a value to hand out.
pub(super) fn display_val(cx: &mut Cx<'_>, hid: Hid) -> Res<Val> {
    let obj = display_obj(cx, hid)?;
    cx.handout(obj)
}

/// The display an object of the class `display` stands for.
pub(super) fn display_of(cx: &Cx<'_>, value: &Val) -> Res<Hid> {
    cx.with_state(value.object()?, |held: &mut DisplayRef| held.hid)
}

/// The value of a presentation mode: the library counts them from zero, a catalog enum from one.
fn mode_val(mode: u32) -> Val {
    enumeration(mode + 1)
}

fn selector_val(cx: &mut Cx<'_>, selector: Sel) -> Res<Val> {
    Ok(match selector {
        Sel::Profile(profile) => record(vec![profile_val(cx, profile)?, Val::None]),
        Sel::Output(node) => record(vec![Val::None, Val::Uint(node)]),
    })
}

/// The viewport of a display, made when the library made the display and no shell has asked for one yet.
fn viewport_of_display(st: &mut State, hid: Hid) -> crate::error::Res<u64> {
    if let Some(vid) = st.display(hid)?.viewport {
        return Ok(vid);
    }
    let vid = new_rec(st, Some(hid));
    if let Some(rec) = st.disp.displays.get_mut(&hid) {
        rec.viewport = Some(vid);
    }
    Ok(vid)
}

pub(super) fn register(builder: &mut Builder) {
    let _ = builder
        .class::<DisplayRef>("display")
        .ctor(|cx, inits| {
            let ctx = cx.ctx();
            let given = inits
                .take("selectors")
                .ok_or_else(|| Fail::type_error("Display.selectors: must list at least one OutputProfile or Output"))?;
            let entries = given.list()?;
            if entries.is_empty() {
                return Err(Fail::type_error(
                    "Display.selectors: must list at least one OutputProfile or Output",
                ));
            }
            let outputs = look(ctx, |st| Ok(st.outputs.clone()))?;
            let mut selectors = Vec::new();
            for entry in entries {
                let fields = entry.record()?;
                selectors.push(match (field(fields, 0), field(fields, 1)) {
                    (Some(profile), None) => Sel::Profile(profile_of(cx, profile)?),
                    (None, Some(output)) => {
                        let node = output.uint()?;
                        if !outputs.contains(&node) {
                            return Err(Fail::type_error(
                                "Display.selectors: each entry must be an OutputProfile or an Output",
                            ));
                        }
                        Sel::Output(node)
                    }
                    _ => {
                        return Err(Fail::type_error(
                            "Display.selectors: each entry must be an OutputProfile or an Output",
                        ));
                    }
                });
            }
            let (hid, vid) = edit(ctx, |st| {
                st.disp.numbered += 1;
                let id = format!("display-{}", st.disp.numbered);
                let hid = st.new_display(id, abi::STD_PRESENT_SINGLE, selectors, None);
                let vid = new_rec(st, Some(hid));
                if let Some(rec) = st.disp.displays.get_mut(&hid) {
                    rec.viewport = Some(vid);
                }
                Ok((hid, vid))
            })?;
            let _ = viewport_obj(cx, vid)?;
            DISPLAYS.with(|held| held.borrow_mut().insert((ctx, hid), cx.me()));
            Ok(DisplayRef { ctx, hid })
        })
        .get("selectors", |held, cx| {
            let selectors = look(held.ctx, |st| st.display(held.hid).map(|rec| rec.selectors.clone()))?;
            selectors
                .into_iter()
                .map(|selector| selector_val(cx, selector))
                .collect::<Res<Vec<_>>>()
                .map(Val::List)
        })
        .get("id", |held, _| {
            look(held.ctx, |st| st.display(held.hid).map(|rec| rec.id.clone())).map(Val::Text)
        })
        .set("id", |held, cx, value| {
            let id = value.text()?.to_owned();
            if id.is_empty() {
                return Err(Fail::type_error("Display.id: must be a non-empty string"));
            }
            edit(held.ctx, |st| {
                let rec = st.display(held.hid)?;
                if rec.listed && rec.id != id {
                    return Err(crate::error::Fail::refused(
                        ErrorCode::InvalidState,
                        format!(
                            "Display \"{}\".id: cannot change while the display is in the manager",
                            rec.id
                        ),
                    ));
                }
                if let Some(rec) = st.disp.displays.get_mut(&held.hid) {
                    rec.id = id;
                }
                Ok(())
            })?;
            cx.changed("id");
            Ok(())
        })
        .get("mode", |held, _| {
            look(held.ctx, |st| st.display(held.hid).map(|rec| mode_val(rec.mode)))
        })
        .set("mode", |held, cx, value| {
            let mode = value
                .enum_value()?
                .checked_sub(1)
                .ok_or_else(|| Fail::type_error("Display.mode: must be \"single\", \"mirror\" or \"span\""))?;
            edit(held.ctx, |st| st.set_display_mode(held.hid, mode))?;
            cx.changed("mode");
            Ok(())
        })
        .get("outputs", |held, _| {
            look(held.ctx, |st| {
                st.display(held.hid)?;
                Ok(Val::List(
                    st.display_outputs(held.hid).into_iter().map(Val::Uint).collect(),
                ))
            })
        })
        .get("viewport", |held, cx| {
            let vid = edit(held.ctx, |st| viewport_of_display(st, held.hid))?;
            viewport_val(cx, vid)
        })
        .set("viewport", |held, cx, value| {
            let next = viewport_of(cx, &value)?;
            edit(held.ctx, |st| {
                let old = st.display(held.hid)?.viewport;
                if old == Some(next) {
                    return Ok(());
                }
                let rec = st
                    .disp
                    .viewports
                    .get(&next)
                    .ok_or_else(|| crate::error::Fail::stale("the viewport does not exist"))?;
                if let Some(owner) = rec.display {
                    let id = st.disp.displays.get(&owner).map_or("", |rec| rec.id.as_str());
                    return Err(crate::error::Fail::type_error(format!(
                        "Display.viewport: already shown by display \"{id}\""
                    ))
                    .with_code(ErrorCode::Busy));
                }
                let content = rec.content;
                if let Some(old) = old
                    && let Some(rec) = st.disp.viewports.get_mut(&old)
                {
                    rec.display = None;
                }
                if let Some(rec) = st.disp.viewports.get_mut(&next) {
                    rec.display = Some(held.hid);
                }
                if let Some(rec) = st.disp.displays.get_mut(&held.hid) {
                    rec.viewport = Some(next);
                    // The workspace the viewport holds is the one the display shows from now on, when it has an output to show it on.
                    if let Some(workspace) = content {
                        rec.wanted = Some(workspace);
                        rec.content = Some(workspace);
                    }
                }
                Ok(())
            })?;
            cx.changed("viewport");
            Ok(())
        })
        .get("implicit", |held, _| {
            look(held.ctx, |st| st.display(held.hid).map(|rec| rec.implicit)).map(Val::Bool)
        })
        .get("listed", |held, _| {
            look(held.ctx, |st| st.display(held.hid).map(|rec| rec.listed)).map(Val::Bool)
        });
}
