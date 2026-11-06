//! The generated bindings for the vendored `mutter-x11-interop.xml`.
//!
//! Generated at build time by `wayland-scanner`, never checked in. The client half is
//! generated from the same file by the tests, so a client and the adapter can only agree
//! if they agree about the protocol.
//!
//! `wayland-scanner` describes each interface twice: as Rust data and as the C
//! `wl_interface` table, which contains raw pointers. There is no generation mode without
//! it, so this crate declares `unsafe_code = "deny"` (see `Cargo.toml`) and allows the
//! generated module and nothing else.

#[allow(unsafe_code)]
pub mod interop {
    use wayland_server;
    use wayland_server::protocol::*;

    pub mod __interfaces {
        use wayland_server::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/mutter-x11-interop.xml");
    }

    use self::__interfaces::*;

    wayland_scanner::generate_server_code!("protocols/mutter-x11-interop.xml");
}

pub use interop::mutter_x11_interop;
