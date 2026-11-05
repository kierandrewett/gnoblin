//! The generated halves of the Gnoblin-owned protocols.
//!
//! `protocols/` at the repository root is the only source of these three
//! protocols, and `wayland-scanner` reads it at compile time: the interface
//! descriptors, the object types and the request and event enums below are
//! generated, so nothing is checked in and a change to an XML cannot reach the
//! server without reaching the tests that generate their client from the same
//! file.
//!
//! Each protocol is generated once here and used by its module
//! ([`crate::protocols::window_frame`], [`crate::protocols::blur_fade`] and
//! [`crate::protocols::session_host`]), so no protocol has a second,
//! hand-written idea of its own wire.
//!
//! # Why the `unsafe_code` allowed here
//!
//! The scanner emits, per interface, `c_ptr: Some(unsafe { &C_INTERFACE })` for
//! the C descriptor a compositor using the C libraries needs. That `unsafe` is
//! machine-written and unavoidable, and a crate-wide `forbid` cannot be lifted
//! in one module, so this crate denies `unsafe_code` instead of forbidding it
//! and allows it here only. No hand-written module may carry the same allow.

#![allow(unsafe_code)]

/// `gnoblin_window_frame_v1`: the decoration renderer protocol.
pub(crate) mod window_frame {
    use smithay::reexports::wayland_server;
    use wayland_server::backend as wayland_backend;
    use wayland_server::protocol::wl_surface;

    pub(crate) mod __interfaces {
        use super::wayland_backend;
        use super::wayland_server::protocol::__interfaces::*;

        wayland_scanner::generate_interfaces!("../../protocols/gnoblin-window-frame-v1.xml");
    }

    #[allow(unused_imports)]
    use self::__interfaces::*;

    wayland_scanner::generate_server_code!("../../protocols/gnoblin-window-frame-v1.xml");
}

/// `gnoblin_blur_fade_v1`: animation opacity kept apart from glass alpha.
pub(crate) mod blur_fade {
    use smithay::reexports::wayland_server;
    use wayland_server::backend as wayland_backend;
    use wayland_server::protocol::wl_surface;

    pub(crate) mod __interfaces {
        use super::wayland_backend;
        use super::wayland_server::protocol::__interfaces::*;

        wayland_scanner::generate_interfaces!("../../protocols/gnoblin-blur-fade-v1.xml");
    }

    #[allow(unused_imports)]
    use self::__interfaces::*;

    wayland_scanner::generate_server_code!("../../protocols/gnoblin-blur-fade-v1.xml");
}

/// `gnoblin_session_host_v1`: the host compositor's channel to one session.
pub(crate) mod session_host {
    use smithay::reexports::wayland_server;
    use wayland_server::backend as wayland_backend;

    pub(crate) mod __interfaces {
        use super::wayland_backend;

        wayland_scanner::generate_interfaces!("../../protocols/gnoblin-session-host-v1.xml");
    }

    #[allow(unused_imports)]
    use self::__interfaces::*;

    wayland_scanner::generate_server_code!("../../protocols/gnoblin-session-host-v1.xml");
}
