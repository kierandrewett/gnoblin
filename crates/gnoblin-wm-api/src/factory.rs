//! [`WindowManagerFactory`]: how a configuration names the provider a session starts with.

use std::sync::Arc;

use crate::manager::WindowManager;
use crate::null::NullWm;

/// Builds the window-manager provider a session starts with. A factory rather than a provider, so a configuration stays
/// `Clone` and the provider is made on the thread that runs the session.
#[derive(Clone)]
pub struct WindowManagerFactory(Arc<dyn Fn() -> Box<dyn WindowManager> + Send + Sync>);

impl WindowManagerFactory {
    /// A factory that calls `make` once for the session.
    pub fn new(make: impl Fn() -> Box<dyn WindowManager> + Send + Sync + 'static) -> Self {
        Self(Arc::new(make))
    }

    /// The provider a session starts with: the factory's, or [`NullWm`] without one.
    #[must_use]
    pub fn start(factory: Option<Self>) -> Box<dyn WindowManager> {
        match factory {
            Some(factory) => (factory.0)(),
            None => Box::<NullWm>::default(),
        }
    }
}

impl std::fmt::Debug for WindowManagerFactory {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("WindowManagerFactory").finish_non_exhaustive()
    }
}
