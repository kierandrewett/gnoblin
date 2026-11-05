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
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::reexports::wayland_server::Resource;
use smithay::wayland::xdg_toplevel_tag::{XdgToplevelTagHandler, XdgToplevelTagManager};
use tracing::debug;

use crate::State;

/// Creates the `xdg_toplevel_tag_manager_v1` global.
pub(crate) fn global(display: &DisplayHandle) -> XdgToplevelTagManager {
    XdgToplevelTagManager::new::<State>(display)
}

impl XdgToplevelTagHandler for State {
    fn set_tag(&mut self, toplevel: XdgToplevel, tag: String) {
        debug!(toplevel = %toplevel.id(), %tag, "[xdg-toplevel-tag] tag set; read on the next toplevel commit");
    }

    fn set_description(&mut self, toplevel: XdgToplevel, description: String) {
        debug!(toplevel = %toplevel.id(), %description, "[xdg-toplevel-tag] description set");
    }
}
