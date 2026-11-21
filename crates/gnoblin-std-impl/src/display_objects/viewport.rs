//! `viewport`: a workspace seen through a camera.
//!
//! `data/js/gnoblin-std/display/viewport.ts` is the reference. The library keeps the viewport's state (`displays.rs`, `ViewportRec`):
//! what a display shows is the display's, so the content of a viewport that a display shows is read from the display.

use gnoblin_core_abi::GnoblinCtx;
use gnoblin_object::server::{Builder, Cx};
use gnoblin_object::{Fail, Obj, Res, Val};

use super::display::display_val;
use super::{VIEWPORTS, edit, look};
use crate::displays::{Hid, Vid, ViewportRec};
use crate::state::{self, State};
use crate::workspace_objects::{workspace_handle, workspace_value};

/// What a `viewport` object holds: the viewport the library calls `vid`.
pub(super) struct ViewportRef {
    ctx: GnoblinCtx,
    vid: Vid,
}

/// Makes the library's record of a viewport that `display` shows, or of one no display shows yet.
pub(super) fn new_rec(st: &mut State, display: Option<Hid>) -> Vid {
    let vid = state::next_id();
    st.disp.viewports.insert(
        vid,
        ViewportRec {
            content: None,
            camera: None,
            clear_color: "#000000".to_owned(),
            display,
        },
    );
    vid
}

/// The object of the viewport `vid`: the one a shell has, or a new one (the viewport of a display the library made has none until a shell
/// asks).
pub(super) fn viewport_obj(cx: &mut Cx<'_>, vid: Vid) -> Res<Obj> {
    let ctx = cx.ctx();
    let found = VIEWPORTS.with(|held| held.borrow().get(&(ctx, vid)).copied());
    if let Some(obj) = found
        && cx.class_of(obj).is_ok()
    {
        return Ok(obj);
    }
    let obj = cx.spawn("viewport", ViewportRef { ctx, vid })?;
    VIEWPORTS.with(|held| held.borrow_mut().insert((ctx, vid), obj));
    Ok(obj)
}

/// The object of the viewport `vid`, as a value to hand out.
pub(super) fn viewport_val(cx: &mut Cx<'_>, vid: Vid) -> Res<Val> {
    let obj = viewport_obj(cx, vid)?;
    cx.handout(obj)
}

/// The viewport an object of the class `viewport` stands for.
pub(super) fn viewport_of(cx: &Cx<'_>, value: &Val) -> Res<Vid> {
    cx.with_state(value.object()?, |held: &mut ViewportRef| held.vid)
}

fn gone() -> crate::error::Fail {
    crate::error::Fail::stale("the viewport does not exist")
}

/// What a viewport shows now: what its display shows (the workspace on the display's output, or the one it last showed or was given
/// while it has none), or the workspace it was given while no display shows it.
fn shown(st: &State, vid: Vid) -> crate::error::Res<Option<crate::state::Id>> {
    let rec = st.disp.viewports.get(&vid).ok_or_else(gone)?;
    let id = match rec.display {
        None => rec.content,
        Some(display) => match st.disp.displays.get(&display) {
            // A display the library dropped with its output shows nothing.
            None => None,
            Some(held) => st.display_shown(display).unwrap_or(held.content),
        },
    };
    Ok(id.filter(|id| st.ws.contains_key(id)))
}

pub(super) fn register(builder: &mut Builder) {
    let _ = builder
        .class::<ViewportRef>("viewport")
        .ctor(|cx, _inits| {
            let ctx = cx.ctx();
            let vid = edit(ctx, |st| Ok(new_rec(st, None)))?;
            VIEWPORTS.with(|held| held.borrow_mut().insert((ctx, vid), cx.me()));
            Ok(ViewportRef { ctx, vid })
        })
        .get("content", |held, cx| {
            match look(held.ctx, |st| shown(st, held.vid))? {
                Some(id) => workspace_value(cx, id),
                None => Ok(Val::None),
            }
        })
        .set("content", |held, cx, value| {
            if value.is_none() {
                edit(held.ctx, |st| {
                    let rec = st.disp.viewports.get_mut(&held.vid).ok_or_else(gone)?;
                    if rec.display.is_some() {
                        return Err(crate::error::Fail::type_error(
                            "Viewport.content: the viewport of a display shows the workspace it is given; give a Workspace",
                        ));
                    }
                    rec.content = None;
                    Ok(())
                })?;
                cx.changed("content");
                return Ok(());
            }
            let workspace = workspace_handle(cx, &value)?;
            edit(held.ctx, |st| {
                let display = st.disp.viewports.get(&held.vid).ok_or_else(gone)?.display;
                // A workspace that is not in `gnoblin.workspaces` cannot be shown, as `show` refuses it.
                if let Some(display) = display {
                    st.assign_display(display, workspace)?;
                }
                if let Some(rec) = st.disp.viewports.get_mut(&held.vid) {
                    rec.content = Some(workspace);
                }
                Ok(())
            })?;
            cx.changed("content");
            Ok(())
        })
        .get("camera", |held, cx| {
            let given = look(held.ctx, |st| {
                st.disp.viewports.get(&held.vid).map(|rec| rec.camera).ok_or_else(gone)
            })?;
            let obj = match given {
                Some(obj) if cx.class_of(obj).is_ok() => obj,
                // A viewport made without a camera has a flat one with no zoom, pan or roll.
                _ => {
                    let made = crate::scene::new_flat_camera(cx)?;
                    edit(held.ctx, |st| {
                        if let Some(rec) = st.disp.viewports.get_mut(&held.vid) {
                            rec.camera = Some(made);
                        }
                        Ok(())
                    })?;
                    made
                }
            };
            cx.handout(obj)
        })
        .set("camera", |held, cx, value| {
            let obj = value.object()?;
            if crate::scene::camera_state(held.ctx, obj).is_none() {
                return Err(Fail::type_error("Viewport.camera: must be a Camera2D or a Camera3D"));
            }
            edit(held.ctx, |st| {
                st.disp.viewports.get_mut(&held.vid).ok_or_else(gone)?.camera = Some(obj);
                Ok(())
            })?;
            cx.changed("camera");
            Ok(())
        })
        .get("clear_color", |held, _| {
            look(held.ctx, |st| {
                st.disp
                    .viewports
                    .get(&held.vid)
                    .map(|rec| rec.clear_color.clone())
                    .ok_or_else(gone)
            })
            .map(Val::Text)
        })
        .set("clear_color", |held, cx, value| {
            let color = value.text()?.to_owned();
            if color.is_empty() {
                return Err(Fail::type_error(
                    "Viewport.clear_color: must be a colour such as #101820",
                ));
            }
            edit(held.ctx, |st| {
                st.disp.viewports.get_mut(&held.vid).ok_or_else(gone)?.clear_color = color;
                Ok(())
            })?;
            cx.changed("clear_color");
            Ok(())
        })
        .get("display", |held, cx| {
            let display = look(held.ctx, |st| {
                st.disp
                    .viewports
                    .get(&held.vid)
                    .map(|rec| rec.display.filter(|display| st.disp.displays.contains_key(display)))
                    .ok_or_else(gone)
            })?;
            match display {
                Some(display) => display_val(cx, display),
                None => Ok(Val::None),
            }
        });
}
