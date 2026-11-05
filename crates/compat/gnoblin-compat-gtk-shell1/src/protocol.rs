//! The generated bindings for the vendored `gtk-shell.xml`.
//!
//! Generated at build time by `wayland-scanner` and never checked in, so the XML is the
//! only source: `wayland_scanner::generate_server_code!` expands the whole protocol into
//! this module during compilation, and the client half is generated from the same file by
//! the tests. Upstream's own protocol module also imports the core protocol's interfaces,
//! because `gtk_shell1.get_gtk_surface` names a `wl_surface` and
//! `gtk_surface1.titlebar_gesture` names a `wl_seat`.
//!
//! # Why this module allows unsafe code
//!
//! `wayland-scanner` describes each interface twice: once as Rust data the safe dispatcher
//! reads, and once as the C `wl_interface` table, which is raw pointers and an
//! `unsafe impl Sync` wrapper. There is no generation mode that omits the second half, and
//! the crate cannot inherit the workspace's `forbid(unsafe_code)` because `forbid` has no
//! exception. The allowance is therefore scoped to this module, the code below is a macro
//! expansion rather than anything written here, and no other module in this crate can
//! contain unsafe code.

#[allow(unsafe_code)]
pub mod gtk {
    use wayland_server;
    use wayland_server::protocol::*;

    pub mod __interfaces {
        use wayland_server::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/gtk-shell.xml");
    }

    use self::__interfaces::*;

    wayland_scanner::generate_server_code!("protocols/gtk-shell.xml");
}

pub use gtk::{gtk_shell1, gtk_surface1};
