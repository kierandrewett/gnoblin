//! A document the runtime commits reaches the session's input devices.
//!
//! The pure rules and the libinput calls are tested where they live; this drives the part between them with a real
//! session: the runtime's privileged connection commits a document, the compositor's commit path hands the `input`
//! section to the backend, and the headless backend records what each virtual device was given. A document that no
//! longer names a setting puts the device back, which is what a reload after deleting a line of `shell.ts` must do.

mod support;

use gnoblin_backend::input_settings::Setting;
use gnoblin_backend::{DeviceInfo, DeviceKind, InputEvent};
use gnoblin_client::Client;
use gnoblin_client::unix::UnixTransport;
use gnoblin_compositor::OutputTap;
use gnoblin_types::DeviceId;
use support::{Session, wait_for};

/// The runtime's connection to `ends`.
fn runtime(ends: &mut Vec<(String, std::os::unix::net::UnixStream)>) -> Client<UnixTransport> {
    let (_, stream) = ends.pop().expect("the runtime's end");
    Client::connect(
        UnixTransport::from_stream(stream).expect("a transport"),
        gnoblin_compositor::version(),
    )
    .expect("the handshake")
}

/// Waits until `device` holds exactly `expected`.
#[track_caller]
fn wait_for_settings(tap: &OutputTap, device: DeviceId, expected: &[Setting]) {
    wait_for(&format!("device {device} holds {expected:?}"), || {
        (tap.device_settings(device).as_deref() == Some(expected)).then_some(())
    });
}

#[test]
fn a_committed_document_configures_the_devices_and_a_removed_line_puts_them_back() {
    let tap = OutputTap::new();
    let (_session, mut ends) = Session::start_privileged_configured(&["shell"], {
        let tap = tap.clone();
        move |config| config.output_tap = Some(tap)
    });
    let mut runtime = runtime(&mut ends);

    let mouse = DeviceId::new(21);
    let touchpad = DeviceId::new(22);
    wait_for("the session serves", || {
        tap.push_input(InputEvent::DeviceAdded {
            device: DeviceInfo::new(mouse, "mouse", DeviceKind::Mouse),
        })
        .then_some(())
    });
    assert!(tap.push_input(InputEvent::DeviceAdded {
        device: DeviceInfo::new(touchpad, "touchpad", DeviceKind::Touchpad),
    }));
    wait_for_settings(&tap, mouse, &[]);
    wait_for_settings(&tap, touchpad, &[]);

    runtime
        .commit_config(br#"{"input":{"mouse":{"speed":0.5},"touchpad":{"tap-to-click":true}}}"#)
        .expect("the runtime commits");
    wait_for_settings(&tap, mouse, &[Setting::Speed(0.5)]);
    wait_for_settings(&tap, touchpad, &[Setting::TapToClick(true)]);

    // The next document drops the mouse's line and keeps the touchpad's.
    runtime
        .commit_config(br#"{"input":{"touchpad":{"tap-to-click":true}}}"#)
        .expect("the runtime commits again");
    wait_for_settings(&tap, mouse, &[]);
    wait_for_settings(&tap, touchpad, &[Setting::TapToClick(true)]);

    // And a document with no `input` section at all puts every device back.
    runtime
        .commit_config(br#"{}"#)
        .expect("the runtime commits an empty document");
    wait_for_settings(&tap, touchpad, &[]);
}
