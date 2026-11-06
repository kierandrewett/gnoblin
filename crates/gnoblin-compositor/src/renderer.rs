//! The renderer a session draws with, from the `compositor.renderer` configuration setting.
//!
//! A document that does not state the setting keeps `auto`, which is the renderer the selected
//! backend draws with today. A value the schema does not accept is refused there, naming the
//! leaf, and never arrives here. What does arrive here is a renderer this machine cannot
//! create: that is refused at startup by [`Backend::build`](crate::Backend::build) with the
//! setting named, never by drawing with a different renderer from the one written.

use std::path::Path;

use gnoblin_config::{RendererChoice, SearchPaths, Store};
use gnoblin_log::{debug_in, warn_in};

use crate::SessionConfig;

/// Sets `config.renderer` from the last accepted configuration in `store`, read from `source`.
pub fn configure_from_store(config: &mut SessionConfig, store: &Store, source: &Path) {
    match store.load(source) {
        Ok(stored) => {
            apply(
                config,
                stored.config.compositor.as_ref().and_then(|section| section.renderer),
            );
            debug_in!("compositor", "[renderer] loaded from the last accepted configuration");
        }
        Err(error) => warn_in!("compositor", %error, "[renderer] configuration not readable; the default is used"),
    }
}

/// [`configure_from_store`] for the process's own state directory and configuration path.
///
/// With no configuration file there is no accepted document, so the default is used.
pub fn configure_from_environment(config: &mut SessionConfig) {
    let Some(store) = Store::from_env() else {
        debug_in!(
            "compositor",
            "[renderer] no state directory is set; the default is used"
        );
        return;
    };
    match gnoblin_config::locate(&SearchPaths::from_env()) {
        Ok(path) => configure_from_store(config, &store, path.path()),
        Err(error) => debug_in!("compositor", %error, "[renderer] no configuration file; the default is used"),
    }
}

/// Sets the renderer to what the section states. A leaf the section leaves out is [`None`],
/// which a session reads as `auto`.
fn apply(config: &mut SessionConfig, setting: Option<RendererChoice>) {
    config.renderer = setting;
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
        apply(&mut config, section.renderer);
        config.renderer
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
}
