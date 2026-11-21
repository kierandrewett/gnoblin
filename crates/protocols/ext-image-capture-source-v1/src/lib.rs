//! XML protocol `ext_image_capture_source_v1` (`<protocol name="ext_image_capture_source_v1">`); interfaces served: `ext_output_image_capture_source_manager_v1`, `ext_foreign_toplevel_image_capture_source_manager_v1`.
//!
//! `ext_image_capture_source_v1`: the source objects a client names a capture by.
//!
//! An output source names an output and a toplevel source names a window; the compositor remembers
//! each source under its object id so a capture session can resolve it later. Smithay serves both
//! managers and the source object. This crate creates the two globals for the clients the roster
//! calls privileged, and names the window a published toplevel identifier stands for. What a
//! source names is recorded by the compositor's handlers, which need its state and so sit in
//! `gnoblin-wayland`'s `glue/`.
//!
//! The managers are one capability with `ext_image_copy_capture_manager_v1`
//! (`gnoblin_wayland_api::access::Global::ImageCopyCapture`), so a configuration that turns the
//! capture protocol off removes all of them.

use gnoblin_types::WindowId;
use gnoblin_wayland_api::access::Global;
use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};
use smithay::wayland::image_capture_source::{
    OutputCaptureSourceHandler, OutputCaptureSourceState, ToplevelCaptureSourceHandler, ToplevelCaptureSourceState,
};

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "ext-image-capture-source-v1",
    interfaces: &[
        ServedInterface::new("ext_output_image_capture_source_manager_v1", 1),
        ServedInterface::new("ext_foreign_toplevel_image_capture_source_manager_v1", 1),
    ],
    privilege: Privilege::Roster(Global::ImageCopyCapture),
};

/// The two source managers a session holds.
pub struct CaptureSources {
    /// The output capture sources.
    pub output: OutputCaptureSourceState,
    /// The toplevel capture sources.
    pub toplevel: ToplevelCaptureSourceState,
}

/// Creates the output and toplevel source manager globals, in that order, visible to the clients the roster calls
/// privileged.
pub fn register<D>(registrar: &Registrar<'_>) -> CaptureSources
where
    D: OutputCaptureSourceHandler + ToplevelCaptureSourceHandler,
{
    CaptureSources {
        output: OutputCaptureSourceState::new_with_filter::<D, _>(
            registrar.display(),
            registrar.filter(Global::ImageCopyCapture),
        ),
        toplevel: ToplevelCaptureSourceState::new_with_filter::<D, _>(
            registrar.display(),
            registrar.filter(Global::ImageCopyCapture),
        ),
    }
}

/// The window a published toplevel identifier names.
///
/// The foreign-toplevel list publishes each window under `gnoblin-` followed by the window's sixteen hex
/// digits; a handle that does not parse is one this session did not publish, which is refused rather than
/// guessed at.
#[must_use]
pub fn window_of_identifier(identifier: &str) -> Option<WindowId> {
    let digits = identifier.strip_prefix("gnoblin-")?;
    u64::from_str_radix(digits, 16).ok().map(WindowId::new)
}

#[cfg(test)]
mod tests {
    use super::window_of_identifier;
    use gnoblin_types::WindowId;

    #[test]
    fn an_identifier_the_list_published_names_its_window() {
        assert_eq!(
            window_of_identifier("gnoblin-000000000000002a"),
            Some(WindowId::new(42))
        );
    }

    #[test]
    fn an_identifier_the_list_did_not_publish_names_no_window() {
        assert_eq!(window_of_identifier("other-000000000000002a"), None);
        assert_eq!(window_of_identifier("gnoblin-not-hex"), None);
        assert_eq!(window_of_identifier(""), None);
    }
}
