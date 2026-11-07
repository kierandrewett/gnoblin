//! The compat adapters this session serves, built from their checked-in support matrices.
//!
//! Only `gnoblin-compat-host` links the adapter crates. It hands the compositor `Extension`
//! values and never names an adapter itself.
//! The mode is passed in by the caller; the compositor role
//! uses `Mode::default()` because no configuration key selects one yet. `Mode::Off` serves
//! nothing, so the globals are not advertised at all rather than advertised and refused.

use std::sync::Arc;

use gnoblin_compat_core::{DesktopSettings, Gate, MatrixError, Mode, Switch, settings_from_environment};
use gnoblin_compat_gtk_shell1::Redirect;
use gnoblin_compat_mutter_x11_interop::{Admission as InteropAdmission, Interop};
use gnoblin_compat_x11_xsettings::Owner as XsettingsOwner;
use gnoblin_compositor::{Extension, SessionConfig, XSettings};

/// Registers the adapters for `mode` on the session's Wayland configuration.
///
/// The XSETTINGS owner is built from the configuration's desktop settings and handed to the
/// X11 session, which publishes it when Xwayland runs. Only a session with an X server reads it.
///
/// # Errors
///
/// [`MatrixError`] when a checked-in manifest cannot be read, which is a build-time bug. Nothing
/// is registered in that case.
pub fn configure(config: &mut SessionConfig, mode: Mode) -> Result<(), MatrixError> {
    let extensions = extensions(mode)?;
    gnoblin_compositor::configure_compat_extensions(config, extensions);
    config.xsettings = xsettings(&settings_from_environment(), mode);
    Ok(())
}

/// The XSETTINGS owner for `settings` when `mode` is on, or [`None`] when it is off.
///
/// `Mode::Off` publishes nothing, so no owner exists and the X11 session has no settings to
/// publish. The owner answers the values from `settings` alone, never from a file a client wrote.
#[must_use]
pub fn xsettings(settings: &DesktopSettings, mode: Mode) -> Option<Box<dyn XSettings>> {
    let switch = Switch::from_enabled(mode != Mode::Off);
    switch
        .is_on()
        .then(|| Box::new(XsettingsOwner::new(settings.clone(), switch)) as Box<dyn XSettings>)
}

fn extensions(mode: Mode) -> Result<Vec<Arc<dyn Extension>>, MatrixError> {
    if mode == Mode::Off {
        return Ok(Vec::new());
    }
    let redirect = Redirect::for_mode(mode)?;
    let admission = InteropAdmission::load(Gate::new(mode, Default::default(), Default::default()))?;
    let interop = Interop::new(admission);
    Ok(vec![Arc::new(redirect), Arc::new(interop)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;
    use std::time::{Duration, Instant};

    use gnoblin_compositor::{Compositor, StopHandle};
    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::wl_registry;
    use wayland_client::{Connection, Dispatch, QueueHandle};

    /// Takes the registry's globals and nothing else.
    struct Probe;

    impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Probe {
        fn event(
            _: &mut Self,
            _: &wl_registry::WlRegistry,
            _: wl_registry::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    /// Runs a session built with `mode` on its own socket, and returns the interfaces its
    /// registry advertises.
    fn advertised(mode: Mode) -> Vec<String> {
        let directory = tempfile::tempdir().expect("a temporary directory");
        // The compositor refuses an IPC socket in a directory anyone else can enter.
        std::fs::set_permissions(
            directory.path(),
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
        )
        .expect("the directory mode");
        let wayland = directory.path().join("wayland.sock");
        let stop = StopHandle::new();
        let mut config = SessionConfig {
            ipc_socket: Some(directory.path().join("ipc.sock")),
            stop: Some(stop.clone()),
            ..SessionConfig::default()
        };
        config.wayland.socket_path = Some(wayland.clone());
        configure(&mut config, mode).expect("the adapters load");
        let thread = std::thread::spawn(move || Compositor::run(config));
        let deadline = Instant::now() + Duration::from_secs(10);
        let connection = loop {
            match UnixStream::connect(&wayland) {
                Ok(stream) => break Connection::from_socket(stream).expect("a wayland connection"),
                Err(_) if !thread.is_finished() && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(error) => panic!("the session never listened: {error}"),
            }
        };
        let (globals, queue) = registry_queue_init::<Probe>(&connection).expect("the registry");
        let names = globals
            .contents()
            .with_list(|list| list.iter().map(|global| global.interface.clone()).collect::<Vec<_>>());
        drop(queue);
        drop(globals);
        drop(connection);
        stop.request();
        thread.join().expect("the session thread").expect("the session ran");
        names
    }

    #[test]
    fn a_session_in_the_default_mode_advertises_gtk_shell1_and_the_x11_interop() {
        let names = advertised(Mode::default());
        assert!(names.iter().any(|name| name == "gtk_shell1"), "{names:?}");
        assert!(names.iter().any(|name| name == "mutter_x11_interop"), "{names:?}");
    }

    #[test]
    fn a_session_with_the_adapters_off_advertises_neither() {
        let names = advertised(Mode::Off);
        assert!(!names.iter().any(|name| name == "gtk_shell1"), "{names:?}");
        assert!(!names.iter().any(|name| name == "mutter_x11_interop"), "{names:?}");
    }

    /// Desktop settings whose cursor theme no default uses, so a test reads its own value.
    fn settings_with_cursor(theme: &str) -> DesktopSettings {
        DesktopSettings {
            cursor_theme: theme.to_owned(),
            ..DesktopSettings::default()
        }
    }

    #[test]
    fn the_configured_cursor_theme_is_the_value_the_owner_publishes() {
        let mut owner =
            xsettings(&settings_with_cursor("Gnoblin-Test"), Mode::default()).expect("an owner in the default mode");
        let values = owner.settings();
        let cursor = (
            gnoblin_compat_x11_xsettings::CURSOR_THEME.to_owned(),
            gnoblin_xwayland::xsettings::Setting::Text("Gnoblin-Test".to_owned()),
        );
        assert!(values.contains(&cursor), "{values:?}");
    }

    #[test]
    fn with_the_mode_off_no_xsettings_owner_exists() {
        assert!(xsettings(&DesktopSettings::default(), Mode::Off).is_none());
        let mut config = SessionConfig::default();
        configure(&mut config, Mode::Off).expect("the adapters load");
        assert!(config.xsettings.is_none());
    }

    /// The display numbers whose socket under `/tmp/.X11-unix` accepts a connection. A socket
    /// left by a server that has died refuses one, so it is not counted as a live display.
    fn x11_displays() -> std::collections::BTreeSet<u32> {
        let Ok(entries) = std::fs::read_dir("/tmp/.X11-unix") else {
            return std::collections::BTreeSet::new();
        };
        entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let number: u32 = entry.file_name().to_str()?.strip_prefix('X')?.parse().ok()?;
                UnixStream::connect(entry.path()).is_ok().then_some(number)
            })
            .collect()
    }

    /// Takes `len` bytes at `at`, failing rather than panicking when the blob is too short.
    fn take<'a>(blob: &'a [u8], at: &mut usize, len: usize) -> Result<&'a [u8], String> {
        let end = at
            .checked_add(len)
            .ok_or_else(|| format!("a read of {len} bytes overflows"))?;
        let bytes = blob
            .get(*at..end)
            .ok_or_else(|| format!("a {len}-byte read at {} runs past {} bytes", *at, blob.len()))?;
        *at = end;
        Ok(bytes)
    }

    fn array<const N: usize>(blob: &[u8], at: &mut usize) -> Result<[u8; N], String> {
        <[u8; N]>::try_from(take(blob, at, N)?).map_err(|error| error.to_string())
    }

    /// The bytes of padding that follow a field of `len` bytes so the next one is four-byte aligned.
    fn padding(len: usize) -> usize {
        (4 - len % 4) % 4
    }

    /// Reads an XSETTINGS blob as the spec lays it out: a byte order (0 least significant byte
    /// first, 1 most significant first), three pad bytes, a serial, a setting count, then per
    /// setting a type, a pad byte, a name length, the name padded to four bytes, a last-change
    /// serial and the value. Text, integers and colours are returned as text. Every read is
    /// bounds-checked, so a short or malformed blob is an error.
    fn parse_blob(blob: &[u8]) -> Result<Vec<(String, String)>, String> {
        let mut at = 0;
        let little = match array::<1>(blob, &mut at)?[0] {
            0 => true,
            1 => false,
            other => return Err(format!("an unknown byte order {other}")),
        };
        let u16_at = |at: &mut usize| -> Result<u16, String> {
            let bytes = array::<2>(blob, at)?;
            Ok(if little {
                u16::from_le_bytes(bytes)
            } else {
                u16::from_be_bytes(bytes)
            })
        };
        let u32_at = |at: &mut usize| -> Result<u32, String> {
            let bytes = array::<4>(blob, at)?;
            Ok(if little {
                u32::from_le_bytes(bytes)
            } else {
                u32::from_be_bytes(bytes)
            })
        };
        take(blob, &mut at, 3)?;
        u32_at(&mut at)?;
        let count = u32_at(&mut at)?;
        let mut values = Vec::new();
        for _ in 0..count {
            let kind = array::<1>(blob, &mut at)?[0];
            take(blob, &mut at, 1)?;
            let name_len = usize::from(u16_at(&mut at)?);
            let name = String::from_utf8(take(blob, &mut at, name_len)?.to_vec()).map_err(|error| error.to_string())?;
            take(blob, &mut at, padding(name_len))?;
            u32_at(&mut at)?;
            let value = match kind {
                0 => (u32_at(&mut at)? as i32).to_string(),
                1 => {
                    let len = usize::try_from(u32_at(&mut at)?).map_err(|error| error.to_string())?;
                    let text =
                        String::from_utf8(take(blob, &mut at, len)?.to_vec()).map_err(|error| error.to_string())?;
                    take(blob, &mut at, padding(len))?;
                    text
                }
                2 => {
                    let red = u16_at(&mut at)?;
                    let green = u16_at(&mut at)?;
                    let blue = u16_at(&mut at)?;
                    let alpha = u16_at(&mut at)?;
                    format!("{red},{green},{blue},{alpha}")
                }
                other => return Err(format!("an unknown setting type {other}")),
            };
            values.push((name, value));
        }
        Ok(values)
    }

    #[test]
    fn an_x11_client_reads_the_cursor_theme_the_session_publishes() {
        use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};
        use x11rb::rust_connection::RustConnection;

        if gnoblin_xwayland::probe(std::env::var_os("PATH").as_deref()).is_err() {
            eprintln!("skipped: no Xwayland binary on PATH, so no X11 session can start here");
            return;
        }
        let directory = tempfile::tempdir().expect("a temporary directory");
        std::fs::set_permissions(
            directory.path(),
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
        )
        .expect("the directory mode");
        let stop = StopHandle::new();
        let mut config = SessionConfig {
            ipc_socket: Some(directory.path().join("ipc.sock")),
            stop: Some(stop.clone()),
            // The default refuses byte-swapped clients, which an Xwayland without the switch
            // cannot serve; the test allows them so the session starts on any Xwayland.
            xwayland: Some(gnoblin_xwayland::Settings {
                allow_byte_swapped_clients: true,
                ..gnoblin_xwayland::Settings::default()
            }),
            ..SessionConfig::default()
        };
        config.wayland.socket_path = Some(directory.path().join("wayland.sock"));
        configure(&mut config, Mode::default()).expect("the adapters load");
        config.xsettings = xsettings(
            &DesktopSettings {
                cursor_size: 32,
                ..settings_with_cursor("Gnoblin-Test")
            },
            Mode::default(),
        );

        let before = x11_displays();
        let thread = std::thread::spawn(move || Compositor::run(config));
        let deadline = Instant::now() + Duration::from_secs(30);
        let display = loop {
            if let Some(display) = x11_displays().difference(&before).next().copied() {
                break format!(":{display}");
            }
            assert!(Instant::now() < deadline, "no X11 socket appeared");
            std::thread::sleep(Duration::from_millis(50));
        };
        let connection = loop {
            match RustConnection::connect(Some(&display)) {
                Ok((connection, _)) => break connection,
                Err(error) => {
                    assert!(Instant::now() < deadline, "the X11 client could not connect: {error}");
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        };
        let selection = connection
            .intern_atom(false, b"_XSETTINGS_S0")
            .expect("an atom request")
            .reply()
            .expect("the selection atom")
            .atom;
        let settings = connection
            .intern_atom(false, b"_XSETTINGS_SETTINGS")
            .expect("an atom request")
            .reply()
            .expect("the settings atom")
            .atom;
        // The manager publishes after the window manager is up, and the property is empty until
        // its first update, so the client polls for up to 5 s for a blob that has settings. The
        // XSETTINGS spec keeps `_XSETTINGS_SETTINGS` on the selection owner's window, not root.
        let published = Instant::now() + Duration::from_secs(5);
        let mut last = String::from("no selection owner was seen");
        let values = loop {
            let owner = connection
                .get_selection_owner(selection)
                .expect("a selection request")
                .reply()
                .expect("the selection owner")
                .owner;
            if owner != 0 {
                let blob = connection
                    .get_property(false, owner, settings, AtomEnum::ANY, 0, 16384)
                    .expect("a property request")
                    .reply()
                    .expect("the settings property")
                    .value;
                match parse_blob(&blob) {
                    Ok(values) if !values.is_empty() => break values,
                    Ok(_) => last = "the blob has no settings yet".to_owned(),
                    Err(error) => last = error,
                }
            }
            assert!(Instant::now() < published, "no XSETTINGS were published: {last}");
            std::thread::sleep(Duration::from_millis(50));
        };
        let has = |name: &str, value: &str| values.iter().any(|(n, v)| n == name && v == value);
        assert!(has("Gtk/CursorThemeName", "Gnoblin-Test"), "{values:?}");
        assert!(has("Gtk/CursorThemeSize", "32"), "{values:?}");

        drop(connection);
        stop.request();
        thread.join().expect("the session thread").expect("the session ran");
    }
}
