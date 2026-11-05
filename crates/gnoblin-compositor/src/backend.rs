//! Which backend a session drives.
//!
//! The backend is the one place a session touches hardware, so it is the one place
//! [`SessionConfig`](crate::SessionConfig) has to name a choice. It is an enum and not a trait
//! object because the choice is data: a supervisor reads it from a configuration
//! document, a test passes it in, and a later lane adds the DRM backend as one more
//! variant and one more arm of [`Backend::build`]. Nothing else in this crate has to
//! know which one ran.
//!
//! # Failure modes first
//!
//! 1. **A backend is selected that this build does not carry.** The run is refused
//!    with the variant named rather than falling back to a headless session nobody
//!    asked for: a session that renders nowhere is a worse answer than an error.
//!    Today every variant this crate declares is implemented, so the arm that will
//!    report this arrives with the first unimplemented one.

use gnoblin_backend::Backend as BackendTrait;
use gnoblin_wayland::State;

/// The backend a session drives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Backend {
    /// A virtual output with a software renderer: no DRM, no GPU, no session.
    ///
    /// It is what makes a session runnable anywhere — in CI, in a test and on a
    /// machine with no display — and its pixels are real, so what it proves about the
    /// frame path holds for a DRM session too.
    #[default]
    Headless,
}

impl Backend {
    /// The backend this selection names, ready for [`gnoblin_wayland::Server::new`].
    #[must_use]
    pub fn build(&self) -> Box<dyn BackendTrait<State>> {
        match self {
            Backend::Headless => Box::new(gnoblin_backend::headless::Headless::new()),
        }
    }

    /// The backend's name, for a log line and a startup report.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Backend::Headless => "headless",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Backend;

    #[test]
    fn the_default_backend_is_the_one_that_starts_anywhere() {
        assert_eq!(Backend::default(), Backend::Headless);
    }

    #[test]
    fn the_headless_selection_names_the_backend_it_builds() {
        // The two halves must agree: a variant that built one backend and named another
        // would mislabel every startup log.
        let backend = Backend::Headless;
        assert_eq!(backend.name(), backend.build().name());
    }
}
