//! The compat adapters this session serves, built from their checked-in support matrices.
//!
//! Only this binary links the adapter crates. It hands the compositor `Extension` values and
//! never names an adapter itself. The mode is passed in by the caller; the compositor role
//! uses `Mode::default()` because no configuration key selects one yet. `Mode::Off` serves
//! nothing, so the globals are not advertised at all rather than advertised and refused.

use std::sync::Arc;

use gnoblin_compat_core::{Gate, MatrixError, Mode};
use gnoblin_compat_gtk_shell1::Redirect;
use gnoblin_compat_mutter_x11_interop::{Admission as InteropAdmission, Interop};
use gnoblin_compositor::{Extension, SessionConfig};

/// Registers the adapters for `mode` on the session's Wayland configuration.
///
/// # Errors
///
/// [`MatrixError`] when a checked-in manifest cannot be read, which is a build-time bug. Nothing
/// is registered in that case.
pub fn configure(config: &mut SessionConfig, mode: Mode) -> Result<(), MatrixError> {
    let extensions = extensions(mode)?;
    gnoblin_compositor::configure_compat_extensions(config, extensions);
    Ok(())
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
}
