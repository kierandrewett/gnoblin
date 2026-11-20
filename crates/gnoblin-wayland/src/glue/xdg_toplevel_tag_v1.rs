//! `gnoblin-wayland-xdg-toplevel-tag-v1`: the tag is read on the next toplevel commit.

use gnoblin_wayland_protocols::xdg_toplevel_tag_v1;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::XdgToplevel;
use smithay::wayland::xdg_toplevel_tag::XdgToplevelTagHandler;

use crate::State;

impl XdgToplevelTagHandler for State {
    fn set_tag(&mut self, toplevel: XdgToplevel, tag: String) {
        xdg_toplevel_tag_v1::set_tag(&toplevel, &tag);
    }

    fn set_description(&mut self, toplevel: XdgToplevel, description: String) {
        xdg_toplevel_tag_v1::set_description(&toplevel, &description);
    }
}
