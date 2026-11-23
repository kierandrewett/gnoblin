//! `displays` and `output_profiles`: `gnoblin.displays` and `gnoblin.outputs.profiles` of the core's root, as the lists they are in the
//! TypeScript library (`data/js/gnoblin-std/host/roots.ts`: `DisplayCollection`, and the profile collection `installRoots` makes).
//!
//! Both are the lists the library keeps (`displays.rs`), the ones `display_manager` also serves: a service here has the list, its length,
//! `add`, `remove`, `clear` and `by_id`, and `displays` has `replace`. The checks are the manager's, because the calls are.

use gnoblin_object::Fail;
use gnoblin_object::server::Builder;
use gnoblin_object::{Res, Val};

use super::display::display_of;
use super::manager::{clear_profiles, display_by_id, display_list, profile_by_id, profile_list};
use super::profile::profile_of;
use super::{LISTS, ListKind, edit, look};

/// Makes the object of a list findable by the turn that tells `changed`, and starts the desktop as the manager does.
fn open(cx: &gnoblin_object::server::Cx<'_>, kind: ListKind) -> Res<()> {
    let ctx = cx.ctx();
    crate::events::open(ctx).map_err(Fail::from)?;
    LISTS.with(|held| held.borrow_mut().insert((ctx, kind), cx.me()));
    Ok(())
}

pub(super) fn register(builder: &mut Builder) {
    let _ = builder
        .class::<()>("displays")
        .ctor(|cx, _| open(cx, ListKind::Displays))
        .get("list", |_, cx| display_list(cx))
        .get("length", |_, cx| {
            look(cx.ctx(), |st| Ok(st.disp.display_list.len() as u64)).map(Val::Uint)
        })
        .method("add", |_, cx, args| {
            let display = display_of(cx, &args[0])?;
            edit(cx.ctx(), |st| st.add_display(display))?;
            Ok(Val::None)
        })
        .method("remove", |_, cx, args| {
            let display = display_of(cx, &args[0])?;
            edit(cx.ctx(), |st| st.remove_display(display)).map(Val::Bool)
        })
        .method("clear", |_, cx, _| {
            edit(cx.ctx(), |st| {
                st.clear_displays();
                Ok(())
            })?;
            Ok(Val::None)
        })
        .method("replace", |_, cx, args| {
            let (old, next) = (display_of(cx, &args[0])?, display_of(cx, &args[1])?);
            edit(cx.ctx(), |st| st.replace_display(old, next))?;
            Ok(Val::None)
        })
        .method("by_id", |_, cx, args| display_by_id(cx, args[0].text()?));
    let _ = builder
        .class::<()>("output_profiles")
        .ctor(|cx, _| open(cx, ListKind::Profiles))
        .get("list", |_, cx| profile_list(cx))
        .get("length", |_, cx| {
            look(cx.ctx(), |st| Ok(st.disp.profile_list.len() as u64)).map(Val::Uint)
        })
        .method("add", |_, cx, args| {
            let profile = profile_of(cx, &args[0])?;
            edit(cx.ctx(), |st| st.add_profile(profile))?;
            Ok(Val::None)
        })
        .method("remove", |_, cx, args| {
            let profile = profile_of(cx, &args[0])?;
            edit(cx.ctx(), |st| {
                st.profile(profile)?;
                Ok(st.remove_profile(profile))
            })
            .map(Val::Bool)
        })
        .method("clear", |_, cx, _| {
            clear_profiles(cx.ctx())?;
            Ok(Val::None)
        })
        .method("by_id", |_, cx, args| profile_by_id(cx, args[0].text()?));
}
