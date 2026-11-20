//! The renderer a session draws with, from the `compositor.renderer` configuration setting, and the memory bounds
//! (`compositor.gpu-memory-budget-mib`, `compositor.client-memory-limit-mib`, `compositor.client-surface-limit`), and whether
//! the tree accepts a user shader effect (`compositor.user-shaders`, on unless the document says off).
//!
//! A document that does not state the setting keeps `auto`, which is the renderer the selected
//! backend draws with today. A value the schema does not accept is refused there, naming the
//! leaf, and never arrives here. What does arrive here is a renderer this machine cannot
//! create: that is refused at startup by [`Backend::build`](crate::Backend::build) with the
//! setting named, never by drawing with a different renderer from the one written.

use std::path::Path;

use gnoblin_config::{Compositor, Environment, RendererChoice, Store};
use gnoblin_wayland::MemoryLimits;

use crate::SessionConfig;

use crate::log;

/// The memory bounds a `compositor` section sets: a key it leaves out keeps the built-in default, and a stated zero turns
/// that bound off. The values are mebibytes in the document and bytes in the session.
pub(crate) fn limits_of(section: &Compositor) -> MemoryLimits {
    const MIB: u64 = 1 << 20;
    let defaults = MemoryLimits::default();
    let mebibytes = |stated: Option<i64>, default: u64| {
        stated
            .and_then(|mib| u64::try_from(mib).ok())
            .map_or(default, |mib| mib.saturating_mul(MIB))
    };
    MemoryLimits {
        budget_bytes: mebibytes(section.gpu_memory_budget_mib, defaults.budget_bytes),
        client_bytes: mebibytes(section.client_memory_limit_mib, defaults.client_bytes),
        client_surfaces: section
            .client_surface_limit
            .and_then(|count| u32::try_from(count).ok())
            .unwrap_or(defaults.client_surfaces),
    }
}

/// Sets the renderer and the capture treatment from the last accepted configuration in `store`, read from `source`.
pub fn configure_from_store(config: &mut SessionConfig, store: &Store, source: &Path) {
    match store.load(source) {
        Ok(stored) => {
            let section = stored.config.compositor.clone().unwrap_or_default();
            config.wayland.gpu_limits = limits_of(&section);
            config.wayland.user_shaders = section.user_shaders.unwrap_or(true);
            apply(
                config,
                stored.config.compositor.as_ref().and_then(|section| section.renderer),
                stored
                    .config
                    .capture
                    .as_ref()
                    .and_then(|section| section.secure_treatment),
            );
            log::debug!(area = "renderer", "loaded from the last accepted configuration");
        }
        Err(error) if error.is_absent() => {
            log::debug!(area = "renderer", %error, "no configuration file; the default is used")
        }
        Err(error) => log::warn!(area = "renderer", %error, "configuration not readable; the default is used"),
    }
}

/// [`configure_from_store`] for the process's own state directory and configuration path.
///
/// With no configuration file there is no accepted document, so the default is used.
pub fn configure_from_environment(config: &mut SessionConfig) {
    let Some(store) = Store::from_env() else {
        log::debug!(area = "renderer", "no state directory is set; the default is used");
        return;
    };
    match gnoblin_config::locate(&Environment::from_env()) {
        Ok(path) => configure_from_store(config, &store, path.path()),
        Err(error) => log::debug!(area = "renderer", %error, "no configuration file; the default is used"),
    }
}

/// Sets the renderer and the capture treatment to what the sections state. A leaf a section
/// leaves out is [`None`], which a session reads as the default: `auto` for the renderer and
/// `mask` for secure layers.
fn apply(
    config: &mut SessionConfig,
    renderer: Option<RendererChoice>,
    secure_treatment: Option<gnoblin_config::CaptureSecureTreatment>,
) {
    config.renderer = renderer;
    config.secure_treatment = secure_treatment;
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_config::Compositor;

    /// The renderer a session gets from a `compositor` section whose `renderer` is `renderer`.
    fn from_section(renderer: Option<RendererChoice>) -> Option<RendererChoice> {
        let mut config = SessionConfig::default();
        let section = Compositor {
            renderer,
            ..Compositor::default()
        };
        apply(&mut config, section.renderer, None);
        config.renderer
    }

    #[test]
    fn the_memory_bounds_are_the_defaults_until_the_section_states_them() {
        assert_eq!(limits_of(&Compositor::default()), MemoryLimits::default());
        let stated = Compositor {
            gpu_memory_budget_mib: Some(512),
            client_memory_limit_mib: Some(0),
            client_surface_limit: Some(10),
            ..Compositor::default()
        };
        assert_eq!(
            limits_of(&stated),
            MemoryLimits {
                budget_bytes: 512 << 20,
                client_bytes: 0,
                client_surfaces: 10,
            },
            "mebibytes become bytes and a stated zero turns the bound off"
        );
    }

    #[test]
    fn an_absent_setting_is_left_to_the_backend() {
        assert_eq!(from_section(None), None);
        assert_eq!(SessionConfig::default().renderer, None);
    }

    #[test]
    fn a_stated_setting_is_the_session_renderer() {
        for choice in [
            RendererChoice::Auto,
            RendererChoice::Vulkan,
            RendererChoice::Gles,
            RendererChoice::Software,
        ] {
            assert_eq!(from_section(Some(choice)), Some(choice));
        }
    }

    #[test]
    fn an_absent_capture_setting_is_the_default_mask() {
        let mut config = SessionConfig::default();
        apply(&mut config, None, None);
        assert_eq!(config.secure_treatment, None);
    }

    #[test]
    fn a_stated_capture_setting_is_the_session_treatment() {
        use gnoblin_config::CaptureSecureTreatment;
        for treatment in [CaptureSecureTreatment::Mask, CaptureSecureTreatment::Omit] {
            let mut config = SessionConfig::default();
            apply(&mut config, None, Some(treatment));
            assert_eq!(config.secure_treatment, Some(treatment));
        }
    }
}
