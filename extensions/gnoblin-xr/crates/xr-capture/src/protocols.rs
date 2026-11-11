//! Client bindings for `gnoblin_view_source_v1`, generated from the repository's own XML. The
//! path is relative to this crate, which sits four directories below the repository root. The
//! extension workspace is its own package, so it cannot reach the root's `protocols/` any other
//! way. The `ext_image_*` interfaces the protocol names come from `wayland-protocols`.

pub mod gnoblin_view_source_v1 {
    use wayland_client;
    pub use wayland_protocols::ext::image_capture_source::v1::client::ext_image_capture_source_v1;

    pub mod __interfaces {
        use super::wayland_client::backend as wayland_backend;
        use wayland_protocols::ext::image_capture_source::v1::client::__interfaces::*;

        wayland_scanner::generate_interfaces!("../../../../protocols/gnoblin-view-source-v1.xml");
    }

    #[allow(unused_imports)]
    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("../../../../protocols/gnoblin-view-source-v1.xml");
}
