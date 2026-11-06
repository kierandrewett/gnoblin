//! Which backend a session drives.
//!
//! The backend is the one place a session touches hardware, so it is the one place
//! [`SessionConfig`](crate::SessionConfig) has to name a choice. It is an enum and not a trait
//! object because the choice is data: a supervisor reads it from a configuration
//! document, a test passes it in, and the DRM lane turns [`Backend::Drm`] from a
//! refusal into a session by changing one arm of [`Backend::build`]. Nothing else in
//! this crate has to know which one ran.
//!
//! # Failure modes first
//!
//! 1. **A backend is selected that this build does not carry.** The run is refused
//!    with the backend named rather than falling back to a headless session nobody
//!    asked for: a session that renders nowhere is a worse answer than an error.
//!    [`Backend::Drm`] is that case today: `drm_is_refused_by_name`.
//! 2. **A nested session cannot reach its parent.** The parent socket is named in the
//!    error and the compositor does not start a session with no output:
//!    `a_nested_session_without_a_parent_is_refused`.
//! 3. **The two halves disagree.** The name a startup log prints is the backend that
//!    was built: `the_headless_selection_names_the_backend_it_builds`.
//! 4. **A renderer is named that the backend cannot draw with.** The `compositor.renderer`
//!    setting is named in the refusal and no other renderer is used in its place: a nested
//!    session refuses `vulkan` and `gles`, and a headless one refuses a renderer this machine
//!    cannot create: `a_nested_session_refuses_a_gpu_renderer_naming_the_setting` and
//!    `a_headless_session_that_cannot_make_the_renderer_is_refused_naming_the_setting`.

use gnoblin_backend::Backend as BackendTrait;
use gnoblin_backend::headless::{Headless, HeadlessConfig};
use gnoblin_backend::nested::{Nested, NestedConfig};
use gnoblin_config::RendererChoice;
use gnoblin_wayland::State;

use crate::Error;

/// Builds the [`Error::RendererRefused`] for `renderer` on `backend`, naming the
/// `compositor.renderer` setting. `reason` says why that backend cannot draw with it.
fn refused(renderer: RendererChoice, backend: &'static str, reason: &'static str) -> Error {
    Error::RendererRefused {
        leaf: "compositor.renderer",
        renderer: renderer.as_str(),
        backend,
        reason,
    }
}

/// The backend a session drives.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Backend {
    /// A virtual output with a software renderer: no DRM, no GPU, no session.
    ///
    /// It is what makes a session runnable anywhere — in CI, in a test and on a
    /// machine with no display — and its pixels are real, so what it proves about the
    /// frame path holds for a DRM session too.
    #[default]
    Headless,
    /// A session hosted in another Wayland compositor: one toplevel per output, on the
    /// parent's socket.
    Nested(NestedConfig),
    /// Direct rendering on the hardware. The DRM backend is not wired into this build
    /// yet, so selecting it is refused by name rather than starting a different backend.
    Drm,
}

impl Backend {
    /// The backend this selection names, drawing with the renderer `renderer` selects, ready for
    /// [`gnoblin_wayland::Server::new`].
    ///
    /// `auto` is the renderer the backend draws with today: software for the headless and nested
    /// backends. A named renderer is never replaced by another.
    ///
    /// # Errors
    ///
    /// [`Error::Nested`] when the parent compositor cannot host the session,
    /// [`Error::BackendRefused`] when this build does not carry the selected backend, and
    /// [`Error::RendererRefused`] when the backend cannot draw with the named renderer.
    pub fn build(self, renderer: RendererChoice) -> Result<Box<dyn BackendTrait<State>>, Error> {
        match self {
            Backend::Headless => match Headless::with_config(HeadlessConfig::from_setting(renderer)) {
                Ok(headless) => Ok(Box::new(headless)),
                Err(_) => Err(refused(
                    renderer,
                    "headless",
                    "the renderer could not be created on this machine; the log says why",
                )),
            },
            Backend::Nested(config) => {
                if !matches!(renderer, RendererChoice::Auto | RendererChoice::Software) {
                    return Err(refused(renderer, "nested", "the nested backend draws in software only"));
                }
                let nested = Nested::connect(&config).map_err(Error::Nested)?;
                Ok(Box::new(nested))
            }
            Backend::Drm => Err(Error::BackendRefused {
                backend: "drm",
                reason: "the DRM backend is not wired into this build yet",
            }),
        }
    }

    /// The backend's name, for a log line and a startup report.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Backend::Headless => "headless",
            Backend::Nested(_) => "nested",
            Backend::Drm => "drm",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Backend;
    use crate::Error;
    use gnoblin_backend::nested::NestedConfig;
    use gnoblin_config::RendererChoice;

    #[test]
    fn the_default_backend_is_the_one_that_starts_anywhere() {
        assert_eq!(Backend::default(), Backend::Headless);
    }

    #[test]
    fn the_headless_selection_names_the_backend_it_builds() {
        // The two halves must agree: a variant that built one backend and named another
        // would mislabel every startup log.
        let backend = Backend::Headless;
        let name = backend.name();
        assert_eq!(
            name,
            backend.build(RendererChoice::Auto).expect("headless builds").name()
        );
    }

    #[test]
    fn drm_is_refused_by_name() {
        match Backend::Drm.build(RendererChoice::Auto) {
            Err(Error::BackendRefused { backend, .. }) => assert_eq!(backend, "drm"),
            Err(other) => panic!("drm refused with the wrong error: {other}"),
            Ok(_) => panic!("drm must not start a session in this build"),
        }
    }

    #[test]
    fn a_nested_session_without_a_parent_is_refused() {
        let socket = std::env::temp_dir().join(format!("gnoblin-no-parent-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket);
        match Backend::Nested(NestedConfig::new(&socket)).build(RendererChoice::Auto) {
            Err(Error::Nested(_)) => {}
            Err(other) => panic!("nested refused with the wrong error: {other}"),
            Ok(_) => panic!("a missing parent socket must not start a session"),
        }
    }

    #[test]
    fn software_is_the_headless_default_and_an_explicit_choice() {
        // `auto` keeps today's behaviour, which for a headless session is software, and
        // `software` asks for the same thing by name.
        for choice in [RendererChoice::Auto, RendererChoice::Software] {
            assert!(Backend::Headless.build(choice).is_ok(), "{choice} builds headless");
        }
    }

    #[test]
    fn a_nested_session_refuses_a_gpu_renderer_naming_the_setting() {
        // The refusal comes before the parent is contacted, so no socket is needed.
        let socket = std::env::temp_dir().join(format!("gnoblin-renderer-{}.sock", std::process::id()));
        for choice in [RendererChoice::Vulkan, RendererChoice::Gles] {
            match Backend::Nested(NestedConfig::new(&socket)).build(choice) {
                Err(error @ Error::RendererRefused { leaf, backend, .. }) => {
                    assert_eq!((leaf, backend), ("compositor.renderer", "nested"));
                    assert!(
                        error
                            .to_string()
                            .starts_with(&format!("compositor.renderer = {choice}:")),
                        "the message names the setting and its value: {error}"
                    );
                }
                Err(other) => panic!("{choice} refused with the wrong error: {other}"),
                Ok(_) => panic!("a nested session draws in software only"),
            }
        }
    }

    #[test]
    fn a_headless_session_that_cannot_make_the_renderer_is_refused_naming_the_setting() {
        // Whether this machine has a Vulkan device decides which half runs: either the renderer
        // is made, or the refusal names the setting, and nothing else is built in its place.
        match Backend::Headless.build(RendererChoice::Vulkan) {
            Ok(backend) => println!("a Vulkan device is present: {} built", backend.name()),
            Err(Error::RendererRefused {
                leaf,
                renderer,
                backend,
                ..
            }) => {
                assert_eq!((leaf, renderer, backend), ("compositor.renderer", "vulkan", "headless"));
            }
            Err(other) => panic!("vulkan refused with the wrong error: {other}"),
        }
    }
}
