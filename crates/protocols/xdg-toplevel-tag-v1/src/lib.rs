//! XML protocol `xdg_toplevel_tag_v1` (`<protocol name="xdg_toplevel_tag_v1">`); interfaces served: `xdg_toplevel_tag_manager_v1`.
//!
//! `xdg_toplevel_tag_manager_v1`: a client names a toplevel with a tag and a description.
//!
//! A tag identifies what kind of window a toplevel is ("main", "settings"), so a shell can
//! tell windows apart without parsing titles. Smithay keeps the tag and description in the
//! toplevel's surface data, where they live as long as the toplevel. The compositor reads
//! the tag on each toplevel commit in `State::toplevel_committed`, stores it on the window,
//! and the session snapshot exposes it as `tag`. The description is kept by Smithay and is
//! not exposed; placement and focus do not read the tag.
//!
//! # Failure modes
//!
//! 1. **A tag that changes.** The last `set_tag` wins; the surface data holds one tag, and
//!    the window takes it on its next commit. Until that commit the snapshot shows the
//!    previous tag.
//! 2. **A tag or description outliving its toplevel.** The data sits on the toplevel's
//!    surface and goes with it, so a recycled surface cannot inherit another's tag.
//! 3. **A tag used for policy.** Not done: the window manager reads no tag, so a tag
//!    changes nothing about placement or focus.
//! 4. **A description shown.** Not exposed: only the tag reaches the snapshot.

use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::XdgToplevel;
use smithay::reexports::wayland_protocols::xdg::toplevel_tag::v1::server::xdg_toplevel_tag_manager_v1::XdgToplevelTagManagerV1;
use smithay::reexports::wayland_server::{GlobalDispatch, Resource};
use smithay::wayland::GlobalData;
use smithay::wayland::xdg_toplevel_tag::{XdgToplevelTagHandler, XdgToplevelTagManager};

use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};

gnoblin_log::logger!(log = "wayland");

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "xdg-toplevel-tag-v1",
    interfaces: &[ServedInterface::new("xdg_toplevel_tag_manager_v1", 1)],
    privilege: Privilege::Core,
};

/// Creates the `xdg_toplevel_tag_manager_v1` global.
pub fn register<D>(registrar: &Registrar<'_>) -> XdgToplevelTagManager
where
    D: XdgToplevelTagHandler + GlobalDispatch<XdgToplevelTagManagerV1, GlobalData> + 'static,
{
    XdgToplevelTagManager::new::<D>(registrar.display())
}

/// A client set the tag of `toplevel`. The tag is read on the next toplevel commit.
pub fn set_tag(toplevel: &XdgToplevel, tag: &str) {
    log::debug!(area = "xdg-toplevel-tag", toplevel = %toplevel.id(), %tag, "tag set; read on the next toplevel commit");
}

/// A client set the description of `toplevel`.
pub fn set_description(toplevel: &XdgToplevel, description: &str) {
    log::debug!(area = "xdg-toplevel-tag", toplevel = %toplevel.id(), %description, "description set");
}
