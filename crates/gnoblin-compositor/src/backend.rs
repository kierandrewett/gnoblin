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
//!    setting is named in the refusal and no other renderer is used in its place: a headless or
//!    nested session refuses a renderer this machine cannot create:
//!    `a_headless_session_that_cannot_make_the_renderer_is_refused_naming_the_setting` and
//!    `a_nested_session_with_a_gpu_renderer_contacts_its_parent_instead_of_refusing_the_renderer`.

use gnoblin_backend::Backend as BackendTrait;
use gnoblin_backend::headless::{Headless, HeadlessConfig, HeadlessHandle};
use gnoblin_backend::nested::{Nested, NestedConfig, NestedError};
use gnoblin_backend::render::renderer::RendererKind;
use gnoblin_config::RendererChoice;
use gnoblin_render_frame::frame::SecureTreatment;
use gnoblin_wayland::State;

use crate::Error;

/// Where a test hotplugs outputs of a headless session it started: filled when the session builds its backend, empty
/// before that and for a backend with no virtual outputs. Every clone shares one session.
#[derive(Clone, Debug, Default)]
pub struct OutputTap {
    handle: std::sync::Arc<std::sync::Mutex<Option<HeadlessHandle>>>,
}

impl OutputTap {
    /// A tap with no session behind it.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Connects a 60 Hz virtual output at scale one on the session's next turn. `false` when no headless session
    /// serves yet, or the size has a zero side.
    pub fn connect(&self, connector: &str, width: u32, height: u32) -> bool {
        let handle = self.handle.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        handle
            .as_ref()
            .is_some_and(|handle| handle.connect_virtual_output(connector, width, height).is_ok())
    }

    /// Disconnects an output on the session's next turn. `false` when no headless session serves yet.
    pub fn disconnect(&self, output: gnoblin_types::OutputId) -> bool {
        let handle = self.handle.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        handle.as_ref().is_some_and(|handle| {
            handle.disconnect_output(output);
            true
        })
    }

    fn fill(&self, handle: HeadlessHandle) {
        *self.handle.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(handle);
    }
}

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
    /// `treatment` is what a capture draws in place of a secure layer, and both backends take it.
    ///
    /// # Errors
    ///
    /// [`Error::Nested`] when the parent compositor cannot host the session,
    /// [`Error::BackendRefused`] when this build does not carry the selected backend, and
    /// [`Error::RendererRefused`] when the backend cannot draw with the named renderer.
    pub fn build(
        self,
        renderer: RendererChoice,
        treatment: SecureTreatment,
    ) -> Result<Box<dyn BackendTrait<State>>, Error> {
        self.build_tapped(renderer, treatment, None)
    }

    /// [`Backend::build`] that also hands a headless backend's hotplug handle to `tap`.
    ///
    /// # Errors
    ///
    /// The same refusals as [`Backend::build`].
    pub fn build_tapped(
        self,
        renderer: RendererChoice,
        treatment: SecureTreatment,
        tap: Option<&OutputTap>,
    ) -> Result<Box<dyn BackendTrait<State>>, Error> {
        match self {
            Backend::Headless => {
                let config = HeadlessConfig {
                    secure_treatment: treatment,
                    ..HeadlessConfig::from_setting(renderer)
                };
                match Headless::with_config(config) {
                    Ok(headless) => {
                        if let Some(tap) = tap {
                            tap.fill(headless.handle());
                        }
                        Ok(Box::new(headless))
                    }
                    Err(_) => Err(refused(
                        renderer,
                        "headless",
                        "the renderer could not be created on this machine; the log says why",
                    )),
                }
            }
            Backend::Nested(config) => {
                let config = NestedConfig {
                    secure_treatment: treatment,
                    renderer: RendererKind::from_setting(renderer),
                    ..config
                };
                match Nested::connect(&config) {
                    Ok(nested) => Ok(Box::new(nested)),
                    // The renderer is created for the first output, so a machine that cannot make it is refused
                    // naming the setting, exactly as a headless session is.
                    Err(NestedError::Render(_)) => Err(refused(
                        renderer,
                        "nested",
                        "the renderer could not be created on this machine; the log says why",
                    )),
                    Err(error) => Err(Error::Nested(error)),
                }
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
    use gnoblin_render_frame::frame::SecureTreatment;

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
            backend
                .build(RendererChoice::Auto, SecureTreatment::default())
                .expect("headless builds")
                .name()
        );
    }

    #[test]
    fn drm_is_refused_by_name() {
        match Backend::Drm.build(RendererChoice::Auto, SecureTreatment::default()) {
            Err(Error::BackendRefused { backend, .. }) => assert_eq!(backend, "drm"),
            Err(other) => panic!("drm refused with the wrong error: {other}"),
            Ok(_) => panic!("drm must not start a session in this build"),
        }
    }

    #[test]
    fn a_nested_session_without_a_parent_is_refused() {
        let socket = std::env::temp_dir().join(format!("gnoblin-no-parent-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket);
        match Backend::Nested(NestedConfig::new(&socket)).build(RendererChoice::Auto, SecureTreatment::default()) {
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
            assert!(
                Backend::Headless.build(choice, SecureTreatment::default()).is_ok(),
                "{choice} builds headless"
            );
        }
    }

    #[test]
    fn a_nested_session_with_a_gpu_renderer_contacts_its_parent_instead_of_refusing_the_renderer() {
        // The nested backend draws with every renderer the headless one does, so a GPU renderer is no longer refused
        // before the parent is reached: the failure here is the missing parent socket, and the renderer is only
        // refused (naming the setting) when the machine cannot create it.
        let socket = std::env::temp_dir().join(format!("gnoblin-renderer-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket);
        for choice in [RendererChoice::Vulkan, RendererChoice::Gles] {
            match Backend::Nested(NestedConfig::new(&socket)).build(choice, SecureTreatment::default()) {
                Err(Error::Nested(_)) => {}
                Err(other) => panic!("{choice} with no parent failed with the wrong error: {other}"),
                Ok(_) => panic!("a missing parent socket must not start a session"),
            }
        }
    }

    #[test]
    fn a_headless_session_that_cannot_make_the_renderer_is_refused_naming_the_setting() {
        // Whether this machine has a Vulkan device decides which half runs: either the renderer
        // is made, or the refusal names the setting, and nothing else is built in its place.
        match Backend::Headless.build(RendererChoice::Vulkan, SecureTreatment::default()) {
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
