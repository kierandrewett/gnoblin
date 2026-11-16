//! The document's `input` section reaching a session's input devices, through the running loop.
//!
//! The pure rules are tested in `gnoblin_backend::input_settings`, and the libinput calls in
//! `gnoblin-backend-drm`. This file is the part between them: a section given at start configures the devices
//! the backend announces, a section committed later reaches the devices the session already has and the ones
//! that arrive afterwards, and a device that goes away is forgotten. The headless backend records what each
//! virtual device was given, so the test reads back what reached it and does not trust what was sent.

use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::headless::{Headless, HeadlessHandle};
use gnoblin_backend::input_settings::{Accel, Setting};
use gnoblin_backend::{DeviceInfo, DeviceKind, InputEvent};
use gnoblin_config::{AccelProfile, Inherit, Input, Mouse, Touchpad};
use gnoblin_types::DeviceId;
use gnoblin_wayland::{Config, ControlHandle, LoopError, Server};

/// How long a test waits for the loop before failing.
const TIMEOUT: Duration = Duration::from_secs(10);

/// Stops the session when the worker finishes, including when it panics.
struct StopOnDrop(ControlHandle);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.stop();
    }
}

fn mouse_section(speed: f64) -> Input {
    Input {
        mouse: Some(Mouse {
            speed: Some(Inherit::Value(speed)),
            ..Mouse::default()
        }),
        ..Input::default()
    }
}

/// Waits until a virtual device holds exactly `expected`, or fails with what it held.
fn wait_for_settings(backend: &HeadlessHandle, device: DeviceId, expected: &[Setting]) {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let held = backend.device_settings(device);
        if held.as_deref() == Some(expected) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "device {device} never held {expected:?}; it holds {held:?}"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

/// Waits until the backend no longer holds a virtual device.
fn wait_for_removal(backend: &HeadlessHandle, device: DeviceId) {
    let deadline = Instant::now() + TIMEOUT;
    while backend.device_settings(device).is_some() {
        assert!(Instant::now() < deadline, "device {device} was never forgotten");
        thread::sleep(Duration::from_millis(5));
    }
}

/// Starts a session on `config` over a headless backend that already has `announced` devices queued, and runs
/// `body` on a worker.
fn run(
    config: Config,
    announced: Vec<DeviceInfo>,
    body: impl FnOnce(&ControlHandle, &HeadlessHandle) + Send + 'static,
) {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let config = Config {
        socket_path: Some(directory.path().join("wayland-0.sock")),
        ..config
    };
    let mut backend = Headless::new();
    for device in announced {
        backend.push_input(InputEvent::DeviceAdded { device });
    }
    let handle = backend.handle();
    let server = Server::new(Box::new(backend), config).expect("the session starts");
    let control = server.control();
    let worker = thread::spawn({
        let control = control.clone();
        move || {
            let _stop = StopOnDrop(control.clone());
            body(&control, &handle);
        }
    });
    let outcome: Result<(), LoopError> = server.run();
    if let Err(payload) = worker.join() {
        std::panic::resume_unwind(payload);
    }
    outcome.expect("the loop stops cleanly");
}

#[test]
fn a_section_given_at_start_configures_the_devices_that_arrive() {
    let mouse = DeviceInfo::new(DeviceId::new(1), "mouse", DeviceKind::Mouse);
    let keyboard = DeviceInfo::new(DeviceId::new(2), "keyboard", DeviceKind::Keyboard);
    let config = Config {
        input: Some(mouse_section(0.5)),
        ..Config::default()
    };
    run(config, vec![mouse, keyboard], |_control, backend| {
        wait_for_settings(backend, DeviceId::new(1), &[Setting::Speed(0.5)]);
        wait_for_settings(backend, DeviceId::new(2), &[]);
    });
}

#[test]
fn a_committed_section_reaches_the_devices_the_session_has_and_the_ones_that_arrive() {
    let first = DeviceInfo::new(DeviceId::new(1), "mouse", DeviceKind::Mouse);
    run(Config::default(), vec![first], |control, backend| {
        // Nothing was given at start, so the device holds nothing.
        wait_for_settings(backend, DeviceId::new(1), &[]);

        control.set_input_settings(mouse_section(0.25));
        wait_for_settings(backend, DeviceId::new(1), &[Setting::Speed(0.25)]);

        // A second mouse plugged in after the commit is configured when it arrives.
        backend.push_input(InputEvent::DeviceAdded {
            device: DeviceInfo::new(DeviceId::new(2), "second mouse", DeviceKind::Mouse),
        });
        wait_for_settings(backend, DeviceId::new(2), &[Setting::Speed(0.25)]);

        // A reload changes both.
        control.set_input_settings(Input {
            mouse: Some(Mouse {
                speed: Some(Inherit::Value(-0.5)),
                accel_profile: Some(Inherit::Value(AccelProfile::Flat)),
                ..Mouse::default()
            }),
            ..Input::default()
        });
        let expected = [Setting::Acceleration(Accel::Flat), Setting::Speed(-0.5)];
        wait_for_settings(backend, DeviceId::new(1), &expected);
        wait_for_settings(backend, DeviceId::new(2), &expected);
    });
}

#[test]
fn a_setting_removed_from_the_document_is_put_back_and_a_removed_device_is_forgotten() {
    let mouse = DeviceInfo::new(DeviceId::new(1), "mouse", DeviceKind::Mouse);
    let pad = DeviceInfo::new(DeviceId::new(2), "touchpad", DeviceKind::Touchpad);
    let config = Config {
        input: Some(Input {
            mouse: Some(Mouse {
                speed: Some(Inherit::Value(0.5)),
                ..Mouse::default()
            }),
            touchpad: Some(Touchpad {
                tap_to_click: Some(Inherit::Value(true)),
                ..Touchpad::default()
            }),
            ..Input::default()
        }),
        ..Config::default()
    };
    run(config, vec![mouse, pad], |control, backend| {
        wait_for_settings(backend, DeviceId::new(1), &[Setting::Speed(0.5)]);
        wait_for_settings(backend, DeviceId::new(2), &[Setting::TapToClick(true)]);

        // The mouse section is gone from the next document: the mouse is put back, the touchpad keeps its tap.
        control.set_input_settings(Input {
            touchpad: Some(Touchpad {
                tap_to_click: Some(Inherit::Value(true)),
                ..Touchpad::default()
            }),
            ..Input::default()
        });
        wait_for_settings(backend, DeviceId::new(1), &[]);
        wait_for_settings(backend, DeviceId::new(2), &[Setting::TapToClick(true)]);

        backend.push_input(InputEvent::DeviceRemoved { id: DeviceId::new(2) });
        wait_for_removal(backend, DeviceId::new(2));
        assert!(
            backend.device_settings(DeviceId::new(1)).is_some(),
            "the other device is still there"
        );
    });
}
