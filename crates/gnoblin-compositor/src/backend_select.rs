//! Which backend a started session drives: the `compositor.backend` setting, the `GNOBLIN_BACKEND` variable and
//! what the machine offers.
//!
//! The backend is chosen by the machine: a session can take a display or sit in a window. This module is the one place the choice is made, as a pure function of three inputs (the
//! settings, the requested override and a description of the host), so every rule below is a test that runs on any
//! machine.
//!
//! # The rule for `auto`
//!
//! The first line that holds decides, and the decision is logged with its reason:
//!
//! 1. **A host Wayland compositor is reachable** (the socket `compositor.nested-display` or `WAYLAND_DISPLAY`
//!    names exists): `nested`. A running compositor owns the seat and the display, so DRM could not be taken; this
//!    is also what makes `gnoblin` started from a terminal on a GNOME or Sway desktop open a window instead of
//!    failing.
//! 2. **A login session with a seat, and a DRM node** (`XDG_SEAT`, or a `libseat` seatd socket, plus a
//!    `/dev/dri/card*`): `drm`. A display manager or a VT login with logind or seatd is how a session
//!    starts on hardware, and it is the only case with no host display. A build without the `drm` feature skips this
//!    rule.
//! 3. **Otherwise** `headless`: a session with nothing to draw on still has to run (CI, containers, services).
//!
//! `DISPLAY` alone (an X11 host) selects nothing: the nested backend is a Wayland client, and an X11 server cannot
//! host it. `auto` never falls back after a start fails: a `drm` pick that cannot open the seat is refused with the
//! reason (`Backend::build`), not replaced by a session nobody asked for. An explicit value is never second-guessed
//! at all.
//!
//! # Failure modes first
//!
//! 1. **A value no backend has.** `GNOBLIN_BACKEND=x11` is a start error naming the variable and the accepted
//!    values: `a_value_no_backend_has_is_refused_naming_the_variable_and_the_accepted_values`. The same mistake in
//!    the document is refused by the schema, naming the leaf (`gnoblin-config`'s `validation` tests).
//! 2. **`nested` with no host to nest in.** No `nested-display` and no `WAYLAND_DISPLAY` is a start error naming
//!    both, and a name that resolves to no socket names the path:
//!    `nested_without_a_host_display_is_refused_naming_both_sources`.
//! 3. **The environment beats the document, always.** The variable is for the session wrapper and for tests;
//!    the document is for the user: `the_environment_beats_the_document`.
//! 4. **`auto` picks DRM where a hardware session cannot start.** It needs both a seat and a node, so a container
//!    with `/dev/dri` but no seat, or a login session on a machine with no GPU, stays headless:
//!    `a_seat_without_a_gpu_and_a_gpu_without_a_seat_are_both_headless`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use gnoblin_backend::nested::{NestedConfig, OutputSpec};
use gnoblin_config::{BackendChoice, Compositor, Environment, NestedDecorations, Store};
use gnoblin_types::Size;

use crate::{Backend, SessionConfig};

use crate::log;

/// The environment variable that overrides `compositor.backend`.
pub const ENV: &str = "GNOBLIN_BACKEND";

/// The spellings [`ENV`] and `compositor.backend` accept, in the order the error names them.
const ACCEPTED: &str = "auto, headless, nested, drm";

/// Why no backend could be chosen.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SelectError {
    /// The override names no backend.
    #[error("{ENV}={value}: not a backend; accepted values are {ACCEPTED}")]
    UnknownValue {
        /// What the variable held.
        value: String,
    },
    /// `nested` was chosen and neither the setting nor the environment names a host.
    #[error(
        "the nested backend needs a host Wayland compositor: set compositor.nested-display, or run inside a session that sets WAYLAND_DISPLAY"
    )]
    NoHostDisplay,
    /// A host display is named relative to a runtime directory this process does not have.
    #[error("the host display {name} is a name, and XDG_RUNTIME_DIR is not set to look it up in")]
    NoRuntimeDir {
        /// The display name.
        name: String,
    },
    /// The host display is named, and nothing listens there.
    #[error("the host display {socket} does not exist: no Wayland compositor is serving it")]
    HostUnreachable {
        /// The socket that was looked for.
        socket: PathBuf,
    },
}

/// What the `compositor` section says about the backend, with the defaults of an unset leaf.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendSettings {
    /// `compositor.backend`.
    pub choice: BackendChoice,
    /// `compositor.nested-display`.
    pub nested_display: Option<String>,
    /// `compositor.nested-outputs`.
    pub nested_outputs: u32,
    /// `compositor.nested-width` and `compositor.nested-height`.
    pub nested_size: Size,
    /// `compositor.nested-decorations`.
    pub nested_decorations: NestedDecorations,
}

impl Default for BackendSettings {
    fn default() -> Self {
        Self {
            choice: BackendChoice::Auto,
            nested_display: None,
            nested_outputs: 1,
            nested_size: gnoblin_backend::nested::UNCONSTRAINED_SIZE,
            nested_decorations: NestedDecorations::Auto,
        }
    }
}

impl BackendSettings {
    /// The settings a `compositor` section states; a leaf it leaves out keeps its default. The schema bounds the
    /// numbers, so a value outside them cannot arrive; the conversions only guard a hand-built section.
    #[must_use]
    pub fn from_section(section: &Compositor) -> Self {
        let defaults = Self::default();
        let edge =
            |value: Option<i64>, fallback: u32| value.and_then(|value| u32::try_from(value).ok()).unwrap_or(fallback);
        Self {
            choice: section.backend.unwrap_or(defaults.choice),
            nested_display: section.nested_display.clone(),
            nested_outputs: edge(section.nested_outputs, defaults.nested_outputs).max(1),
            nested_size: Size::new(
                edge(section.nested_width, defaults.nested_size.width),
                edge(section.nested_height, defaults.nested_size.height),
            ),
            nested_decorations: section.nested_decorations.unwrap_or(defaults.nested_decorations),
        }
    }
}

/// What this process's environment offers, so the rule reads no environment itself.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostView {
    /// `WAYLAND_DISPLAY`: a name in the runtime directory or an absolute path.
    pub wayland_display: Option<OsString>,
    /// `XDG_RUNTIME_DIR`.
    pub runtime_dir: Option<PathBuf>,
    /// Whether the process is in a seated login session: `XDG_SEAT` or `SEATD_SOCK` is set.
    pub seat: bool,
    /// Whether the machine has a DRM card node.
    pub drm_node: bool,
}

impl HostView {
    /// The host as this process sees it.
    #[must_use]
    pub fn from_process() -> Self {
        let set = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
        Self {
            wayland_display: set("WAYLAND_DISPLAY"),
            runtime_dir: set("XDG_RUNTIME_DIR").map(PathBuf::from),
            seat: set("XDG_SEAT").is_some() || set("SEATD_SOCK").is_some(),
            drm_node: has_card_node(Path::new("/dev/dri")),
        }
    }
}

/// Whether `directory` holds a `card*` node.
fn has_card_node(directory: &Path) -> bool {
    std::fs::read_dir(directory).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().starts_with("card"))
    })
}

/// A decided backend and the reason, for the log line.
#[derive(Debug)]
pub struct Selection {
    /// The backend the session drives.
    pub backend: Backend,
    /// Why this one: which rule held, or what the user asked for.
    pub reason: &'static str,
}

/// The override [`ENV`] holds, if it is set and not empty.
///
/// # Errors
///
/// [`SelectError::UnknownValue`] for a spelling no backend has.
pub fn requested_from(value: Option<&str>) -> Result<Option<BackendChoice>, SelectError> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(None),
        Some(value) => BackendChoice::parse(value)
            .map(Some)
            .ok_or_else(|| SelectError::UnknownValue {
                value: value.to_owned(),
            }),
    }
}

/// The host socket the nested backend connects to: the setting, else `WAYLAND_DISPLAY`.
fn host_socket(settings: &BackendSettings, host: &HostView) -> Result<PathBuf, SelectError> {
    let display = match (&settings.nested_display, &host.wayland_display) {
        (Some(name), _) => OsString::from(name),
        (None, Some(name)) => name.clone(),
        (None, None) => return Err(SelectError::NoHostDisplay),
    };
    let path = Path::new(&display);
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    host.runtime_dir
        .as_ref()
        .map(|directory| directory.join(path))
        .ok_or_else(|| SelectError::NoRuntimeDir {
            name: display.to_string_lossy().into_owned(),
        })
}

/// The nested backend that connects to `socket`, with the settings' outputs, starting size and window frame.
fn nested(settings: &BackendSettings, socket: PathBuf) -> Backend {
    let mut config = NestedConfig::new(socket);
    config.outputs = (1..=settings.nested_outputs.max(1))
        .map(|index| OutputSpec::new(format!("NESTED-{index}")))
        .collect();
    config.initial_size = settings.nested_size;
    config.decorations = settings.nested_decorations;
    Backend::Nested(config)
}

/// Decides the backend. `requested` is the [`ENV`] override and beats `settings.choice`; `exists` says whether a
/// path is a live socket, so a test needs no compositor.
///
/// # Errors
///
/// [`SelectError`] when `nested` is chosen with no reachable host.
pub fn select(
    settings: &BackendSettings,
    requested: Option<BackendChoice>,
    host: &HostView,
    exists: impl Fn(&Path) -> bool,
) -> Result<Selection, SelectError> {
    let (choice, asked) = match requested {
        Some(choice) => (choice, "asked for by GNOBLIN_BACKEND"),
        None => (settings.choice, "asked for by compositor.backend"),
    };
    match choice {
        BackendChoice::Headless => Ok(Selection {
            backend: Backend::Headless,
            reason: asked,
        }),
        BackendChoice::Drm => Ok(Selection {
            backend: Backend::Drm,
            reason: asked,
        }),
        BackendChoice::Nested => {
            let socket = host_socket(settings, host)?;
            if !exists(&socket) {
                return Err(SelectError::HostUnreachable { socket });
            }
            Ok(Selection {
                backend: nested(settings, socket),
                reason: asked,
            })
        }
        BackendChoice::Auto => {
            // A host display that is named but does not exist is not a host: it falls through to the next rule, the
            // way a stale WAYLAND_DISPLAY left in an environment must not decide the session.
            if let Ok(socket) = host_socket(settings, host)
                && exists(&socket)
            {
                return Ok(Selection {
                    backend: nested(settings, socket),
                    reason: "auto: a host Wayland compositor is serving the display",
                });
            }
            // A build without the `drm` feature has no hardware backend to pick, so a seated login session falls through to
            // the next rule instead of being refused for a backend nobody asked for by name.
            if cfg!(feature = "drm") && host.seat && host.drm_node {
                return Ok(Selection {
                    backend: Backend::Drm,
                    reason: "auto: a login session with a seat and a DRM node",
                });
            }
            Ok(Selection {
                backend: Backend::Headless,
                reason: "auto: no host compositor, and no seat with a DRM node",
            })
        }
    }
}

/// Whether `path` is a Unix socket.
fn is_socket(path: &Path) -> bool {
    use std::os::unix::fs::FileTypeExt;
    std::fs::metadata(path).is_ok_and(|metadata| metadata.file_type().is_socket())
}

/// Sets the session's backend from the last accepted configuration in `store` (read from `source`), the
/// environment override and the host.
///
/// # Errors
///
/// [`SelectError`] for an override no backend has, and for `nested` with no reachable host.
pub fn configure_from_store(
    config: &mut SessionConfig,
    store: &Store,
    source: &Path,
    requested: Option<&str>,
    host: &HostView,
) -> Result<(), SelectError> {
    let settings = match store.load(source) {
        Ok(stored) => stored
            .config
            .compositor
            .as_ref()
            .map(BackendSettings::from_section)
            .unwrap_or_default(),
        Err(error) if error.is_absent() => {
            log::debug!(area = "backend", %error, "no configuration file; the defaults are used");
            BackendSettings::default()
        }
        Err(error) => {
            log::warn!(area = "backend", %error, "configuration not readable; the defaults are used");
            BackendSettings::default()
        }
    };
    apply(config, &settings, requested, host)
}

/// [`configure_from_store`] for the process's own state directory, configuration path, environment and host.
///
/// With no configuration file the settings are the defaults, and the environment override still applies.
///
/// # Errors
///
/// The same refusals as [`configure_from_store`].
pub fn configure_from_environment(config: &mut SessionConfig) -> Result<(), SelectError> {
    let requested = std::env::var(ENV).ok();
    let host = HostView::from_process();
    let Some(store) = Store::from_env() else {
        log::debug!(area = "backend", "no state directory is set; the defaults are used");
        return apply(config, &BackendSettings::default(), requested.as_deref(), &host);
    };
    match gnoblin_config::locate(&Environment::from_env()) {
        Ok(path) => configure_from_store(config, &store, path.path(), requested.as_deref(), &host),
        Err(error) => {
            log::debug!(area = "backend", %error, "no configuration file; the defaults are used");
            apply(config, &BackendSettings::default(), requested.as_deref(), &host)
        }
    }
}

/// Selects with the real socket test and stores the result in `config`.
fn apply(
    config: &mut SessionConfig,
    settings: &BackendSettings,
    requested: Option<&str>,
    host: &HostView,
) -> Result<(), SelectError> {
    let requested = requested_from(requested)?;
    let selection = select(settings, requested, host, is_socket)?;
    log::info!(
        area = "backend",
        backend = selection.backend.name(),
        reason = selection.reason,
        "backend selected"
    );
    config.backend = selection.backend;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(wayland: Option<&str>, seat: bool, drm_node: bool) -> HostView {
        HostView {
            wayland_display: wayland.map(OsString::from),
            runtime_dir: Some(PathBuf::from("/run/user/1000")),
            seat,
            drm_node,
        }
    }

    fn name(selection: Result<Selection, SelectError>) -> &'static str {
        selection.expect("a backend is chosen").backend.name()
    }

    fn always(_: &Path) -> bool {
        true
    }

    fn never(_: &Path) -> bool {
        false
    }

    #[test]
    fn a_host_compositor_beats_a_seat() {
        // A desktop session has a seat too, and DRM would fail on it: the host display is what decides.
        let host = host(Some("wayland-0"), true, true);
        assert_eq!(name(select(&BackendSettings::default(), None, &host, always)), "nested");
    }

    #[cfg(feature = "drm")]
    #[test]
    fn a_login_session_with_a_seat_and_a_node_is_drm() {
        let host = host(None, true, true);
        assert_eq!(name(select(&BackendSettings::default(), None, &host, always)), "drm");
    }

    #[cfg(not(feature = "drm"))]
    #[test]
    fn a_login_session_with_a_seat_and_a_node_is_headless_in_a_build_without_drm() {
        let host = host(None, true, true);
        assert_eq!(
            name(select(&BackendSettings::default(), None, &host, always)),
            "headless"
        );
    }

    #[test]
    fn a_seat_without_a_gpu_and_a_gpu_without_a_seat_are_both_headless() {
        for host in [
            host(None, true, false),
            host(None, false, true),
            host(None, false, false),
        ] {
            assert_eq!(
                name(select(&BackendSettings::default(), None, &host, always)),
                "headless"
            );
        }
    }

    #[test]
    fn a_stale_host_display_is_not_a_host() {
        // WAYLAND_DISPLAY left over in an environment names a socket nobody serves.
        let host = host(Some("wayland-9"), true, true);
        let expected = if cfg!(feature = "drm") { "drm" } else { "headless" };
        assert_eq!(name(select(&BackendSettings::default(), None, &host, never)), expected);
        let host = self::host(Some("wayland-9"), false, false);
        assert_eq!(
            name(select(&BackendSettings::default(), None, &host, never)),
            "headless"
        );
    }

    #[test]
    fn display_alone_selects_nothing() {
        // The nested backend is a Wayland client; an X11 server cannot host it. DISPLAY is not even read.
        assert_eq!(
            name(select(
                &BackendSettings::default(),
                None,
                &host(None, false, false),
                always
            )),
            "headless"
        );
    }

    #[test]
    fn the_environment_beats_the_document() {
        let settings = BackendSettings {
            choice: BackendChoice::Drm,
            ..BackendSettings::default()
        };
        let host = host(Some("wayland-0"), true, true);
        assert_eq!(
            name(select(&settings, Some(BackendChoice::Headless), &host, always)),
            "headless"
        );
        assert_eq!(name(select(&settings, None, &host, always)), "drm");
    }

    #[test]
    fn an_explicit_choice_is_not_second_guessed() {
        // `drm` on a machine with no seat is still drm: the start refuses it with the reason.
        let settings = BackendSettings {
            choice: BackendChoice::Drm,
            ..BackendSettings::default()
        };
        assert_eq!(name(select(&settings, None, &host(None, false, false), always)), "drm");
        let settings = BackendSettings {
            choice: BackendChoice::Headless,
            ..BackendSettings::default()
        };
        assert_eq!(
            name(select(&settings, None, &host(Some("wayland-0"), true, true), always)),
            "headless"
        );
    }

    #[test]
    fn nested_without_a_host_display_is_refused_naming_both_sources() {
        let settings = BackendSettings {
            choice: BackendChoice::Nested,
            ..BackendSettings::default()
        };
        let error = select(&settings, None, &host(None, false, false), always).expect_err("no host");
        assert_eq!(error, SelectError::NoHostDisplay);
        let text = error.to_string();
        assert!(
            text.contains("compositor.nested-display") && text.contains("WAYLAND_DISPLAY"),
            "{text}"
        );
    }

    #[test]
    fn nested_with_a_display_nobody_serves_names_the_socket() {
        let settings = BackendSettings {
            choice: BackendChoice::Nested,
            ..BackendSettings::default()
        };
        let error = select(&settings, None, &host(Some("wayland-9"), false, false), never).expect_err("unreachable");
        assert_eq!(
            error,
            SelectError::HostUnreachable {
                socket: PathBuf::from("/run/user/1000/wayland-9")
            }
        );
    }

    #[test]
    fn the_setting_names_the_host_before_the_environment_does() {
        let settings = BackendSettings {
            choice: BackendChoice::Nested,
            nested_display: Some("/tmp/host.sock".to_owned()),
            nested_outputs: 2,
            nested_size: Size::new(1280, 720),
            nested_decorations: NestedDecorations::Off,
        };
        let selection = select(&settings, None, &host(Some("wayland-0"), false, false), always).expect("nested");
        let Backend::Nested(config) = selection.backend else {
            panic!("nested was chosen");
        };
        assert_eq!(config.socket, PathBuf::from("/tmp/host.sock"));
        assert_eq!(
            config
                .outputs
                .iter()
                .map(|spec| spec.connector.as_str())
                .collect::<Vec<_>>(),
            ["NESTED-1", "NESTED-2"]
        );
        assert_eq!(config.initial_size, Size::new(1280, 720));
        assert_eq!(
            config.decorations,
            NestedDecorations::Off,
            "the setting reaches the nested backend"
        );
    }

    #[test]
    fn a_nested_session_draws_its_own_frame_unless_the_setting_says_off() {
        assert_eq!(BackendSettings::default().nested_decorations, NestedDecorations::Auto);
        let settings = BackendSettings {
            choice: BackendChoice::Nested,
            ..BackendSettings::default()
        };
        let selection = select(&settings, None, &host(Some("wayland-0"), false, false), always).expect("nested");
        let Backend::Nested(config) = selection.backend else {
            panic!("nested was chosen");
        };
        assert_eq!(
            config.decorations,
            NestedDecorations::Auto,
            "the shipped default is auto"
        );
    }

    #[test]
    fn a_relative_display_with_no_runtime_directory_is_refused() {
        let settings = BackendSettings {
            choice: BackendChoice::Nested,
            ..BackendSettings::default()
        };
        let host = HostView {
            wayland_display: Some(OsString::from("wayland-0")),
            ..HostView::default()
        };
        assert_eq!(
            select(&settings, None, &host, always).expect_err("no runtime dir"),
            SelectError::NoRuntimeDir {
                name: "wayland-0".to_owned()
            }
        );
    }

    #[test]
    fn a_value_no_backend_has_is_refused_naming_the_variable_and_the_accepted_values() {
        let error = requested_from(Some("x11")).expect_err("not a backend");
        let text = error.to_string();
        assert!(text.contains("GNOBLIN_BACKEND") && text.contains("x11"), "{text}");
        for accepted in ["auto", "headless", "nested", "drm"] {
            assert!(text.contains(accepted), "{text} must list {accepted}");
        }
    }

    #[test]
    fn every_accepted_spelling_parses_and_an_empty_override_is_no_override() {
        for spelling in ACCEPTED.split(", ") {
            assert!(
                requested_from(Some(spelling)).expect("accepted").is_some(),
                "{spelling}"
            );
        }
        assert_eq!(requested_from(None), Ok(None));
        assert_eq!(requested_from(Some("")), Ok(None));
        assert_eq!(requested_from(Some("  ")), Ok(None));
    }

    #[test]
    fn a_section_states_the_settings_and_an_empty_one_keeps_the_defaults() {
        assert_eq!(
            BackendSettings::from_section(&Compositor::default()),
            BackendSettings::default()
        );
        let section = Compositor {
            backend: Some(BackendChoice::Nested),
            nested_outputs: Some(3),
            nested_width: Some(1920),
            nested_height: Some(1080),
            nested_display: Some("wayland-1".to_owned()),
            nested_decorations: Some(NestedDecorations::Off),
            ..Compositor::default()
        };
        let settings = BackendSettings::from_section(&section);
        assert_eq!(settings.choice, BackendChoice::Nested);
        assert_eq!(settings.nested_outputs, 3);
        assert_eq!(settings.nested_size, Size::new(1920, 1080));
        assert_eq!(settings.nested_display.as_deref(), Some("wayland-1"));
        assert_eq!(settings.nested_decorations, NestedDecorations::Off);
    }

    #[test]
    fn only_a_card_node_counts_as_a_gpu() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        assert!(!has_card_node(directory.path()));
        std::fs::write(directory.path().join("renderD128"), b"").expect("a render node");
        assert!(!has_card_node(directory.path()), "a render node cannot drive a display");
        std::fs::write(directory.path().join("card0"), b"").expect("a card node");
        assert!(has_card_node(directory.path()));
        assert!(!has_card_node(&directory.path().join("missing")));
    }

    #[test]
    fn the_start_applies_the_choice_to_the_session() {
        let mut config = SessionConfig::default();
        apply(
            &mut config,
            &BackendSettings::default(),
            Some("headless"),
            &host(Some("wayland-0"), true, true),
        )
        .expect("headless");
        assert_eq!(config.backend, Backend::Headless);
        let error = apply(
            &mut config,
            &BackendSettings::default(),
            Some("bogus"),
            &HostView::default(),
        )
        .expect_err("refused");
        assert!(matches!(error, SelectError::UnknownValue { .. }));
    }
}
