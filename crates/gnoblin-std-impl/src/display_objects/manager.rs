//! `display_manager`: `gnoblin.displays` and `gnoblin.outputs.profiles`, and what changes them.
//!
//! `data/js/gnoblin-std/display_manager/manager.ts` is the reference. The lists are the library's (`displays.rs`): this service hands out
//! their objects and passes a shell's changes to them.

use gnoblin_object::server::{Builder, Cx};
use gnoblin_object::{Res, Val};

use super::display::{display_of, display_val};
use super::profile::{profile_of, profile_val};
use super::{MANAGERS, edit, look};
use crate::displays::Hid;

fn displays_val(cx: &mut Cx<'_>, list: Vec<Hid>) -> Res<Vec<Val>> {
    list.into_iter().map(|hid| display_val(cx, hid)).collect()
}

fn maybe_display(cx: &mut Cx<'_>, hid: Option<Hid>) -> Res<Val> {
    match hid {
        Some(hid) => display_val(cx, hid),
        None => Ok(Val::None),
    }
}

pub(super) fn register(builder: &mut Builder) {
    let _ = builder
        .class::<()>("display_manager")
        .ctor(|cx, _| {
            let ctx = cx.ctx();
            crate::events::open(ctx).map_err(gnoblin_object::Fail::from)?;
            MANAGERS.with(|held| held.borrow_mut().insert(ctx, cx.me()));
            Ok(())
        })
        .get("displays", |_, cx| {
            let list = look(cx.ctx(), |st| Ok(st.disp.display_list.clone()))?;
            displays_val(cx, list).map(Val::List)
        })
        .get("profiles", |_, cx| {
            let list = look(cx.ctx(), |st| Ok(st.disp.profile_list.clone()))?;
            list.into_iter()
                .map(|hid| profile_val(cx, hid))
                .collect::<Res<Vec<_>>>()
                .map(Val::List)
        })
        .method("add_profile", |_, cx, args| {
            let profile = profile_of(cx, &args[0])?;
            edit(cx.ctx(), |st| st.add_profile(profile))?;
            Ok(Val::None)
        })
        .method("remove_profile", |_, cx, args| {
            let profile = profile_of(cx, &args[0])?;
            edit(cx.ctx(), |st| {
                st.profile(profile)?;
                Ok(st.remove_profile(profile))
            })
            .map(Val::Bool)
        })
        .method("profile_named", |_, cx, args| {
            let id = args[0].text()?.to_owned();
            let found = look(cx.ctx(), |st| {
                Ok(st
                    .disp
                    .profile_list
                    .iter()
                    .copied()
                    .find(|hid| st.disp.profiles.get(hid).is_some_and(|rec| rec.id == id)))
            })?;
            match found {
                Some(hid) => profile_val(cx, hid),
                None => Ok(Val::None),
            }
        })
        .method("add_display", |_, cx, args| {
            let display = display_of(cx, &args[0])?;
            edit(cx.ctx(), |st| st.add_display(display))?;
            Ok(Val::None)
        })
        .method("remove_display", |_, cx, args| {
            let display = display_of(cx, &args[0])?;
            edit(cx.ctx(), |st| st.remove_display(display)).map(Val::Bool)
        })
        .method("replace_display", |_, cx, args| {
            let (old, next) = (display_of(cx, &args[0])?, display_of(cx, &args[1])?);
            edit(cx.ctx(), |st| st.replace_display(old, next))?;
            Ok(Val::None)
        })
        .method("clear_displays", |_, cx, _| {
            edit(cx.ctx(), |st| {
                st.clear_displays();
                Ok(())
            })?;
            Ok(Val::None)
        })
        .method("display_named", |_, cx, args| {
            let id = args[0].text()?.to_owned();
            let found = look(cx.ctx(), |st| {
                Ok(st
                    .disp
                    .display_list
                    .iter()
                    .copied()
                    .find(|hid| st.disp.displays.get(hid).is_some_and(|rec| rec.id == id)))
            })?;
            maybe_display(cx, found)
        })
        .method("display_at", |_, cx, args| {
            let (x, y) = (args[0].double()?, args[1].double()?);
            let found = edit(cx.ctx(), |st| Ok(st.display_at(x, y)))?;
            maybe_display(cx, found)
        })
        .method("reflow", |_, cx, _| {
            edit(cx.ctx(), |st| {
                st.reflow_displays();
                Ok(())
            })?;
            Ok(Val::None)
        })
        .method("snapshot", |_, cx, _| {
            edit(cx.ctx(), |st| Ok(st.snapshot_displays())).map(Val::Uint)
        })
        .method("restore", |_, cx, args| {
            let token = args[0].uint()?;
            edit(cx.ctx(), |st| {
                st.restore_displays(token);
                Ok(())
            })?;
            Ok(Val::None)
        });
}
