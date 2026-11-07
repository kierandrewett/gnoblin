//! The dconf layer GTK reads its settings from, written by the services role.
//!
//! `gnoblin-compat-gtk-dconf-settings` renders the desktop's settings as a dconf keyfile and
//! compiles it. This module is the composition the `gnoblin` binary makes: it writes that layer
//! into the session's runtime directory when the mode is on, and the guardian points dconf at it
//! with `DCONF_PROFILE`. A switched-off mode writes nothing and exports nothing.
//!
//! The runtime directory is per login and is not persisted, so the layer is rewritten from the
//! configuration on every session start and nothing a toolkit wrote is read back.

use std::path::{Path, PathBuf};

use gnoblin_compat_core::{DesktopSettings, Mode, Switch, settings_from_environment};
use gnoblin_compat_gtk_dconf_settings::{Written, apply, publish};

use gnoblin_services::link::{Host, LinkedAgent, RequestId, RequestOutcome, Tick};

use crate::log;

/// The profile file name dconf reads `DCONF_PROFILE` from, beside the compiled database.
pub const PROFILE: &str = "profile";

/// The compiled database's file name, which the profile names.
pub const DATABASE: &str = "gnoblin.db";

/// The session's dconf profile directory, or [`None`] when the runtime directory is not set.
///
/// The guardian and the services role both compute it from `XDG_RUNTIME_DIR`, so the path the
/// layer is written to is the path the session exports.
#[must_use]
pub fn profile_directory() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(|runtime| PathBuf::from(runtime).join("gnoblin").join("dconf"))
}

/// The profile dconf reads: the Gnoblin layer first, then the user's own database for keys the
/// layer does not set.
#[must_use]
pub fn profile_text(directory: &Path) -> String {
    format!("file-db:{}\nuser-db:user\n", directory.join(DATABASE).display())
}

/// Writes the layer for `settings` into `directory` when `mode` is on, and its profile beside it.
///
/// A switched-off mode writes nothing, so an earlier session's layer is left for the next one to
/// rewrite and is never read while the mode is off, because the guardian exports no profile.
///
/// # Errors
///
/// The I/O error when the keyfile or the profile cannot be written.
pub fn write_layer(settings: &DesktopSettings, mode: Mode, directory: &Path) -> std::io::Result<Written> {
    let switch = Switch::from_enabled(mode != Mode::Off);
    let written = apply(&publish(settings), switch, directory)?;
    if switch.is_on() {
        std::fs::write(directory.join(PROFILE), profile_text(directory))?;
    }
    Ok(written)
}

/// The dconf layer as a linked agent of the services role.
///
/// It is not a bus agent: it writes its layer once when the role starts, and answers no request.
pub struct DconfLayer {
    mode: Mode,
}

impl DconfLayer {
    /// A layer for `mode`. It writes nothing until the role starts it.
    #[must_use]
    pub fn new(mode: Mode) -> DconfLayer {
        DconfLayer { mode }
    }
}

impl LinkedAgent for DconfLayer {
    fn start(&mut self, _host: &Host, _deadline: Tick) -> bool {
        let Some(directory) = profile_directory() else {
            log::warn!("[services] XDG_RUNTIME_DIR is not set, so no dconf layer is written");
            return false;
        };
        let settings = settings_from_environment();
        match write_layer(&settings, self.mode, &directory) {
            Ok(Written::Skipped) => true,
            Ok(written @ Written::Keyfile { compiled: true, .. }) => {
                log::info!(written = ?written, "[services] the dconf layer is compiled");
                true
            }
            Ok(written) => {
                log::warn!(written = ?written, "[services] the dconf layer is written but not compiled");
                true
            }
            Err(error) => {
                log::warn!(%error, "[services] the dconf layer cannot be written");
                false
            }
        }
    }

    fn answer(&self, _request: RequestId, _outcome: &RequestOutcome) {}
}

#[cfg(test)]
mod tests {
    use super::{DATABASE, PROFILE, write_layer};
    use gnoblin_compat_core::{DesktopSettings, Mode};

    fn keyfile(directory: &std::path::Path) -> String {
        std::fs::read_to_string(directory.join("keyfiles").join("gnoblin")).expect("the keyfile is written")
    }

    /// A configured cursor theme is the value the layer publishes, so GTK reads the cursor the
    /// configuration names.
    #[test]
    fn the_configured_cursor_theme_is_in_the_layer() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let settings = DesktopSettings {
            cursor_theme: "Breeze".to_owned(),
            cursor_size: 32,
            ..DesktopSettings::default()
        };
        write_layer(&settings, Mode::Portal, directory.path()).expect("the layer is written");
        let text = keyfile(directory.path());
        assert!(text.contains("cursor-theme='Breeze'"), "{text}");
        assert!(text.contains("cursor-size=32"), "{text}");
    }

    /// The profile names the compiled database the layer is written to, so dconf reads what the
    /// layer wrote and not the user's database alone.
    #[test]
    fn the_profile_names_the_layer_database() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        write_layer(&DesktopSettings::default(), Mode::Portal, directory.path()).expect("the layer is written");
        let profile = std::fs::read_to_string(directory.path().join(PROFILE)).expect("the profile is written");
        let database = directory.path().join(DATABASE);
        assert!(
            profile.starts_with(&format!("file-db:{}", database.display())),
            "{profile}"
        );
    }

    /// A switched-off mode writes no layer and no profile, so dconf has nothing from Gnoblin to read.
    #[test]
    fn a_switched_off_mode_writes_no_layer() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let target = directory.path().join("dconf");
        write_layer(&DesktopSettings::default(), Mode::Off, &target).expect("nothing to write");
        assert!(!target.exists());
    }
}
