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

use gnoblin_backend::Backend as BackendTrait;
use gnoblin_backend::nested::{Nested, NestedConfig};
use gnoblin_wayland::State;

use crate::Error;

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
    /// The backend this selection names, ready for [`gnoblin_wayland::Server::new`].
    ///
    /// # Errors
    ///
    /// [`Error::Nested`] when the parent compositor cannot host the session, and
    /// [`Error::BackendRefused`] when this build does not carry the selected backend.
    pub fn build(self) -> Result<Box<dyn BackendTrait<State>>, Error> {
        match self {
            Backend::Headless => Ok(Box::new(gnoblin_backend::headless::Headless::new())),
            Backend::Nested(config) => {
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
        assert_eq!(name, backend.build().expect("headless builds").name());
    }

    #[test]
    fn drm_is_refused_by_name() {
        match Backend::Drm.build() {
            Err(Error::BackendRefused { backend, .. }) => assert_eq!(backend, "drm"),
            Err(other) => panic!("drm refused with the wrong error: {other}"),
            Ok(_) => panic!("drm must not start a session in this build"),
        }
    }

    #[test]
    fn a_nested_session_without_a_parent_is_refused() {
        let socket = std::env::temp_dir().join(format!("gnoblin-no-parent-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket);
        match Backend::Nested(NestedConfig::new(&socket)).build() {
            Err(Error::Nested(_)) => {}
            Err(other) => panic!("nested refused with the wrong error: {other}"),
            Ok(_) => panic!("a missing parent socket must not start a session"),
        }
    }
}
