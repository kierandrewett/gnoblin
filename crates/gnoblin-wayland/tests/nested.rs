//! The nested round trip: a session as a client of a session.
//!
//! These tests put the two halves of "Hosting sessions" (design/rust-compositor.md,
//! D4) on one socket: a real `gnoblin-wayland` [`Server`] playing the host with a
//! headless output, and the real [`Nested`] backend connecting to it as a hosted
//! session would. Nothing is simulated on either side of the wire — the host sends the
//! configure, the nested backend draws, commits a `wl_shm` buffer and the host
//! composes it — so what the assertions read from the host is what a person would see.
//!
//! ## What each side is
//!
//! - The **host** is the server's own loop, its window manager and its software
//!   renderer. It is asked about the world through its published snapshot and its
//!   screenshots, never from inside the loop.
//! - The **session** is a [`Nested`] backend, driven by a [`Probe`]: a `Host` that
//!   records the input it is handed and describes a frame of one colour, standing in
//!   for the session's own compositor.
//! - The **seam** is `Backend`: the session's frames, resizes and input are read
//!   through the same trait the headless backend implements, because a session must not
//!   be able to tell which backend its compositor runs on.
//!
//! ## Two threads, on purpose
//!
//! The host's calloop loop is not `Send`, so it runs on the test's thread while the
//! session runs on a worker — which is also the real arrangement: the host and the
//! session are different processes. The worker reaches the host only through the
//! control handle, exactly as `tests/real_client.rs` does.

use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::headless::Headless;
use gnoblin_backend::nested::{Nested, NestedConfig};
use gnoblin_backend::{Backend, BackendEvent, InputEvent, Present};
use gnoblin_output::DisplayMode;
use gnoblin_render_frame::frame::Solid;
use gnoblin_types::{DeviceId, OutputId, Point, Rect, Scale, Size};

mod nested_host;
use nested_host::*;

#[test]
fn a_nested_output_appears_as_a_mapped_toplevel_with_the_hosts_size() {
    let session = Session::start();
    session.run(|host| {
        let mut nested = Nested::connect(&NestedConfig::new(host.socket())).expect("the session connects");
        let mut probe = Probe::drawing(RED);
        let output = only_output(&nested);
        let connected = nested.dispatch(&mut probe);
        assert!(
            matches!(connected.as_slice(), [BackendEvent::OutputConnected(connected)] if connected.id == output.id),
            "the configure the connect waited for is an output appearing: {connected:?}"
        );

        draw(&mut nested, &mut probe);
        let observation = host.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));
        let window = observation.only().expect("one mapped window");
        assert_eq!(
            extent(window),
            output.logical_size(),
            "the host's configure is the size the session draws at"
        );
        assert_eq!(&observation.outputs(), &[window_owner(host)], "one host output");
    });
}

#[test]
fn a_host_resize_reconfigures_the_nested_output() {
    let session = Session::start();
    session.run(|host| {
        let mut nested = Nested::connect(&NestedConfig::new(host.socket())).expect("the session connects");
        let mut probe = Probe::drawing(RED);
        draw(&mut nested, &mut probe);
        let observation = host.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));
        let window = observation.only().expect("one mapped window").clone();
        let _ = nested.dispatch(&mut probe);

        let smaller = Rect::new(window.geometry.x, window.geometry.y, 640, 480);
        host.control.move_window(window.id, smaller);
        let deadline = Instant::now() + TIMEOUT;
        let mut events = Vec::new();
        while only_output(&nested).logical_size() != Size::new(640, 480) {
            assert!(
                Instant::now() < deadline,
                "the session never followed the host's resize; it is still {:?}",
                only_output(&nested).logical_size()
            );
            nested.roundtrip().expect("the host answers a round trip");
            events.extend(nested.dispatch(&mut probe));
        }
        assert!(
            events.iter().any(|event| matches!(
                event,
                BackendEvent::OutputResized { mode, .. }
                    if mode.width == 640 && mode.height == 480
            )),
            "the resize is reported as one output resize: {events:?}"
        );
        assert_eq!(
            extent(
                host.wait_for(|o| o.only().is_some_and(|w| w.geometry.width == 640))
                    .only()
                    .expect("the window")
            ),
            Size::new(640, 480),
            "the session's buffer followed the configure, so the host's window is the new size"
        );
    });
}

#[test]
fn a_frame_drawn_by_the_nested_compositor_appears_in_the_hosts_screenshot() {
    let session = Session::start();
    session.run(|host| {
        let mut nested = Nested::connect(&NestedConfig::new(host.socket())).expect("the session connects");
        let mut probe = Probe::drawing(RED);
        draw(&mut nested, &mut probe);

        let observation = host.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));
        let window = observation.only().expect("one mapped window").clone();
        // The host suggests no size, so the session picks the whole output; the test needs a part of the host's output
        // the window does not cover, so the window is made smaller and the session follows.
        host.control
            .move_window(window.id, Rect::new(window.geometry.x, window.geometry.y, 640, 480));
        let deadline = Instant::now() + TIMEOUT;
        while only_output(&nested).logical_size() != Size::new(640, 480) {
            assert!(
                Instant::now() < deadline,
                "the session never followed the host's resize"
            );
            nested.roundtrip().expect("the host answers a round trip");
            let _ = nested.dispatch(&mut probe);
        }
        let output = only_output(&nested);
        draw(&mut nested, &mut probe);
        let window = host
            .wait_for(|observation| observation.only().is_some_and(|window| window.geometry.width == 640))
            .only()
            .expect("the window")
            .clone();
        let host_output = window_owner(host);
        let elsewhere = outside(&window, HEADLESS_SCREEN);

        // The session's red is inside its window, and the host's own black is where the
        // session is not — which is what makes "at the right place" an assertion rather
        // than "somewhere on the screen".
        host.wait_for_colour(host_output, elsewhere, BLACK_PIXEL);
        host.wait_for_colour(host_output, centre(&window), RED_PIXEL);
        let screenshot = host.control.screenshot(host_output).expect("a screenshot");
        assert_eq!(
            screenshot.pixel(elsewhere),
            Some(BLACK_PIXEL),
            "the host's own background is at {elsewhere:?}, where the session is not"
        );
        assert_eq!(
            session_screenshot(&mut nested, output.id).map(|image| image.size()),
            Some(output.logical_size()),
            "the session's own readback is what it drew"
        );
    });
}

#[test]
fn input_injected_on_the_host_seat_arrives_as_input_events() {
    let session = Session::start();
    session.run(|host| {
        let mut nested = Nested::connect(&NestedConfig::new(host.socket())).expect("the session connects");
        let mut probe = Probe::drawing(RED);
        draw(&mut nested, &mut probe);
        let observation = host.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));
        let window = observation.only().expect("one mapped window").clone();
        let host_output = window_owner(host);
        let at = centre(&window);

        host.control.inject_input(InputEvent::PointerMotion {
            device: DeviceId::VIRTUAL,
            output: host_output,
            position: at,
        });
        host.control.inject_input(InputEvent::PointerButton {
            device: DeviceId::VIRTUAL,
            button: BTN_LEFT,
            pressed: true,
        });

        let expected = Point::new(at.x - window.geometry.x, at.y - window.geometry.y);
        let _ = pump_until(&mut nested, &mut probe, |probe| {
            probe.motions().iter().any(|(_, position)| *position == expected)
                && probe.inputs.iter().any(|event| {
                    matches!(
                        event,
                        InputEvent::PointerButton {
                            button: BTN_LEFT,
                            pressed: true,
                            ..
                        }
                    )
                })
        });
        let session_output = only_output(&nested).id;
        assert!(
            probe
                .motions()
                .iter()
                .any(|(output, position)| *output == session_output && *position == expected),
            "a host position arrives as a position on the session's output at {expected:?}: {:?}",
            probe.motions()
        );
        // The host's seat has a pointer, so the session's seat lists one with the capabilities a pointer has, and the
        // press names that device rather than the virtual one.
        let pointer = probe
            .inputs
            .iter()
            .find_map(|event| match event {
                InputEvent::DeviceAdded { device } if device.caps.contains(gnoblin_backend::DeviceCaps::POINTER) => {
                    Some(device.id)
                }
                _ => None,
            })
            .expect("the host's pointer is a device of the session's seat");
        assert_ne!(pointer, DeviceId::VIRTUAL);
        assert!(
            probe.inputs.contains(&InputEvent::PointerButton {
                device: pointer,
                button: BTN_LEFT,
                pressed: true,
            }),
            "the press arrived from the pointer device: {:?}",
            probe.inputs
        );
    });
}

#[test]
fn a_frame_waits_for_the_hosts_frame_callback() {
    let session = Session::start();
    session.run(|host| {
        let mut nested = Nested::connect(&NestedConfig::new(host.socket())).expect("the session connects");
        let mut probe = Probe::drawing(RED);
        let output = only_output(&nested).id;

        assert_eq!(
            nested.render(&mut probe, output).expect("a first frame"),
            Present::Rendered,
            "the first frame is drawn, because no callback is outstanding"
        );
        assert_eq!(
            nested.render(&mut probe, output).expect("a second frame"),
            Present::Nothing,
            "the host has not answered the first frame, so it has not asked for a second"
        );

        // The answer arrives on the wire, and only then is another frame drawn. The loop
        // is bounded because a host that never answered would be the failure this test
        // exists to catch.
        let deadline = Instant::now() + TIMEOUT;
        loop {
            assert!(
                Instant::now() < deadline,
                "the host never asked for a second frame after the first one it was shown"
            );
            nested.roundtrip().expect("the host answers a round trip");
            if nested.render(&mut probe, output).expect("a third frame") == Present::Rendered {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
    });
}

#[test]
fn killing_the_host_reports_a_disconnect_without_panicking() {
    let session = Session::start();
    session.run(|host| {
        let mut nested = Nested::connect(&NestedConfig::new(host.socket())).expect("the session connects");
        let mut probe = Probe::drawing(RED);
        draw(&mut nested, &mut probe);
        host.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));

        // The worker stops the host, which ends its loop and closes every client
        // connection. `Session::run` joins this worker after the loop returns.
        host.control.stop();

        let deadline = Instant::now() + TIMEOUT;
        let mut events = Vec::new();
        loop {
            assert!(
                Instant::now() < deadline,
                "the session never noticed the host had gone; events: {events:?}"
            );
            match nested.roundtrip() {
                Ok(()) => events.extend(nested.dispatch(&mut probe)),
                Err(_) => {
                    events.extend(nested.dispatch(&mut probe));
                    break;
                }
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            events.contains(&BackendEvent::Disconnected),
            "the session was told its parent is gone: {events:?}"
        );
        assert!(
            session_outputs(&nested).is_empty(),
            "and it has no output left to draw on"
        );
        // A frame after the disconnect is a refusal, not a panic.
        assert!(
            nested.render(&mut probe, OutputId::new(0)).is_err(),
            "rendering an output that is gone is an error the caller handles"
        );
        assert!(
            nested.roundtrip().is_err(),
            "and every later round trip says the same thing"
        );
    });
}

/// The nested output is composed at twice its mode when asked: the logical size and the host's window stay as they
/// were, a capture is the hi-res frame, and what the host shows is the frame averaged down to the mode.
#[test]
fn a_nested_output_can_be_supersampled() {
    let session = Session::start();
    session.run(|host| {
        let mut nested = Nested::connect(&NestedConfig::new(host.socket())).expect("the session connects");
        // A rounded red rectangle on black: its curved edge is where a box average differs from a copy.
        let shape = Solid {
            corner_radius: 40.0,
            ..Solid::new(Rect::new(100, 100, 200, 120), RED)
        };
        let mut probe = Probe::drawing_shape(shape);
        draw_rendered(&mut nested, &mut probe);
        let observation = host.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));
        let window = observation.only().expect("one mapped window").clone();
        host.control
            .move_window(window.id, Rect::new(window.geometry.x, window.geometry.y, 640, 480));
        let deadline = Instant::now() + TIMEOUT;
        while only_output(&nested).logical_size() != Size::new(640, 480) {
            assert!(
                Instant::now() < deadline,
                "the session never followed the host's resize"
            );
            nested.roundtrip().expect("the host answers a round trip");
            let _ = nested.dispatch(&mut probe);
        }
        let output = only_output(&nested);
        let window = host
            .wait_for(|observation| observation.only().is_some_and(|window| window.geometry.width == 640))
            .only()
            .expect("the window")
            .clone();
        let host_output = window_owner(host);

        // A change the parent's window owns is refused, a factor that is not whole is refused, and nothing changed.
        let mode_change = DisplayMode::new(800, 600, output.mode.refresh_mhz);
        assert_eq!(
            Backend::<Probe>::configure_output(&mut nested, output.id, mode_change, output.scale, Scale::IDENTITY),
            Err(gnoblin_backend::BackendError::Unsupported(output.id)),
            "the parent's window is the mode"
        );
        assert_eq!(
            Backend::<Probe>::test_output(
                &mut nested,
                output.id,
                output.mode,
                output.scale,
                Scale::new(1001, 1000)
            ),
            Err(gnoblin_backend::BackendError::Supersample(output.id)),
            "1.001 of 640 is not a whole width"
        );
        assert_eq!(only_output(&nested).supersample, Scale::IDENTITY);

        draw_rendered(&mut nested, &mut probe);
        let plain = Backend::<Probe>::capture(&mut nested, output.id, false).expect("a readback");
        assert_eq!(
            plain.size(),
            Size::new(640, 480),
            "without a factor a capture is the mode"
        );

        Backend::<Probe>::configure_output(&mut nested, output.id, output.mode, output.scale, Scale::integral(2))
            .expect("a factor of two on a window of 640x480 is a whole 1280x960");
        let events = nested.dispatch(&mut probe);
        assert!(
            events.iter().any(|event| matches!(
                event,
                BackendEvent::OutputResized { supersample, .. } if *supersample == Scale::integral(2)
            )),
            "the change is reported: {events:?}"
        );
        let supersampled = only_output(&nested);
        assert_eq!(supersampled.supersample, Scale::integral(2));
        assert_eq!(
            supersampled.logical_size(),
            Size::new(640, 480),
            "the logical size does not change"
        );
        assert_eq!(
            supersampled.mode, output.mode,
            "and neither does the mode the host's window has"
        );
        draw_rendered(&mut nested, &mut probe);

        let hires = Backend::<Probe>::capture(&mut nested, output.id, false).expect("a readback");
        assert_eq!(
            hires.size(),
            Size::new(1280, 960),
            "a capture is the frame the compositor composed"
        );
        let shown = Backend::<Probe>::capture_display(&mut nested, output.id, false).expect("the displayed frame");
        assert_eq!(shown.size(), Size::new(640, 480), "the display shows the mode");
        assert_eq!(
            shown.data(),
            hires.downsample(Size::new(640, 480)).data(),
            "the displayed frame is the exact box average of the hi-res one"
        );

        // On the curve of the rectangle's corner there are pixels the average gives a colour in between: the host's
        // own screenshot must show those, which proves the averaged frame is what reached the parent's buffer.
        let mut partial = Vec::new();
        for y in 100..140 {
            for x in 100..140 {
                let pixel = shown.colour_at(x, y).expect("a pixel");
                if pixel[0] != 0 && pixel[0] != 255 {
                    partial.push((x, y, pixel));
                }
            }
        }
        assert!(
            partial.len() >= 8,
            "the corner has edge pixels, found {}",
            partial.len()
        );
        for (x, y, pixel) in partial.into_iter().step_by(3).take(4) {
            host.wait_for_colour(
                host_output,
                Point::new(window.geometry.x + x, window.geometry.y + y),
                pixel,
            );
        }

        Backend::<Probe>::configure_output(&mut nested, output.id, output.mode, output.scale, Scale::IDENTITY)
            .expect("back to no factor");
        draw_rendered(&mut nested, &mut probe);
        assert_eq!(
            Backend::<Probe>::capture(&mut nested, output.id, false).map(|image| image.size()),
            Some(Size::new(640, 480)),
            "a capture is the mode again"
        );
    });
}

/// The renderer's own answers a nested session must give exactly as the headless backend does for the same renderer:
/// one trait, one answer, whichever backend carries the renderer.
fn assert_answers_like_headless(nested: &mut Nested, kind: gnoblin_backend::render::renderer::RendererKind) {
    use gnoblin_backend::headless::HeadlessConfig;
    let output = only_output(nested).id;
    let mut headless = Headless::with_config(HeadlessConfig {
        renderer: kind,
        ..HeadlessConfig::default()
    })
    .expect("a headless backend with the renderer the nested one made");
    let reference = Backend::<Probe>::outputs(&headless)[0].id;
    assert_eq!(
        Backend::<Probe>::effect_kinds(nested),
        Backend::<Probe>::effect_kinds(&headless),
        "the effects the renderer draws"
    );
    assert_eq!(
        Backend::<Probe>::gpu_views(nested),
        Backend::<Probe>::gpu_views(&headless),
        "whether a View is drawn on the GPU"
    );
    let offers = |backend: &mut dyn Backend<Probe>| {
        backend.dmabuf().map(|support| {
            let mut formats: Vec<_> = support
                .formats
                .iter()
                .map(|format| (format.code as u32, u64::from(format.modifier)))
                .collect();
            formats.sort_unstable();
            formats
        })
    };
    assert_eq!(
        offers(nested),
        offers(&mut headless),
        "the dmabuf formats clients are offered"
    );
    assert_eq!(
        Backend::<Probe>::dmabuf_capture_support(nested, output).is_some(),
        Backend::<Probe>::dmabuf_capture_support(&mut headless, reference).is_some(),
        "whether a capture can be written into a client's GPU buffer"
    );
    assert_eq!(
        Backend::<Probe>::max_virtual_edge(nested, output).min(16384),
        Backend::<Probe>::max_virtual_edge(&mut headless, reference).min(16384),
        "the texture limit of the renderer"
    );
}

#[test]
fn a_gles_nested_session_draws_the_same_frame_and_answers_the_gpu_questions_like_headless() {
    use gnoblin_backend::render::renderer::RendererKind;
    let session = Session::start();
    session.run(|host| {
        let Some(mut nested) = connect_drawing_with(host, RendererKind::Gles) else {
            return;
        };
        let mut probe = Probe::drawing(RED);
        let output = only_output(&nested);
        draw_rendered(&mut nested, &mut probe);
        let host_output = window_owner(host);
        let window = host
            .wait_for(|observation| observation.only().is_some_and(|window| window.mapped))
            .only()
            .expect("one mapped window")
            .clone();
        host.wait_for_colour(host_output, centre(&window), RED_PIXEL);
        assert_eq!(
            session_screenshot(&mut nested, output.id).and_then(|image| image.colour_at(1, 1)),
            Some(RED_PIXEL),
            "the GLES readback is the frame the host received"
        );
        assert!(
            Backend::<Probe>::gpu_views(&nested),
            "GLES draws a View on the GPU, nested as on any backend"
        );
        assert_answers_like_headless(&mut nested, RendererKind::Gles);
        assert_ne!(
            Backend::<Probe>::footprint(&nested, output.id),
            gnoblin_render_frame::footprint::Footprint::default(),
            "a GLES output holds memory the footprint reports"
        );
    });
}

#[test]
fn a_vulkan_nested_session_draws_the_same_frame_and_answers_the_gpu_questions_like_headless() {
    use gnoblin_backend::render::renderer::RendererKind;
    let session = Session::start();
    session.run(|host| {
        let Some(mut nested) = connect_drawing_with(host, RendererKind::Vulkan) else {
            return;
        };
        let mut probe = Probe::drawing(RED);
        let output = only_output(&nested);
        draw_rendered(&mut nested, &mut probe);
        let host_output = window_owner(host);
        let window = host
            .wait_for(|observation| observation.only().is_some_and(|window| window.mapped))
            .only()
            .expect("one mapped window")
            .clone();
        host.wait_for_colour(host_output, centre(&window), RED_PIXEL);
        assert_eq!(
            session_screenshot(&mut nested, output.id).and_then(|image| image.colour_at(1, 1)),
            Some(RED_PIXEL),
            "the Vulkan readback is the frame the host received"
        );
        assert_answers_like_headless(&mut nested, RendererKind::Vulkan);
    });
}

#[test]
fn closing_the_last_window_ends_the_session_and_closing_one_of_two_does_not() {
    let session = Session::start();
    session.run(|host| {
        let config = NestedConfig {
            outputs: vec![
                gnoblin_backend::nested::OutputSpec::new("NESTED-1"),
                gnoblin_backend::nested::OutputSpec::new("NESTED-2"),
            ],
            ..NestedConfig::new(host.socket())
        };
        let mut nested = Nested::connect(&config).expect("the session connects");
        let mut probe = Probe::drawing(RED);
        assert_eq!(session_outputs(&nested).len(), 2, "one output per window");
        let first = session_outputs(&nested)[0].id;
        draw(&mut nested, &mut probe);
        host.wait_for(|observation| observation.windows().filter(|window| window.mapped).count() == 2);
        let _ = nested.dispatch(&mut probe);

        // The first window goes: that output is removed, and the session is still one.
        close_host_window(host, |window| window.title.contains("NESTED-1"));
        let events = collect_until(&mut nested, &mut probe, |events| {
            events
                .iter()
                .any(|event| matches!(event, BackendEvent::OutputDisconnected(_)))
        });
        assert!(
            events.contains(&BackendEvent::OutputDisconnected(first)),
            "the closed window's output goes: {events:?}"
        );
        assert!(
            !events.contains(&BackendEvent::SessionEnded),
            "one window left, so the session goes on: {events:?}"
        );
        assert_eq!(session_outputs(&nested).len(), 1);

        // The last window goes: the session ends, once.
        close_host_window(host, |window| window.title.contains("NESTED-2"));
        let events = collect_until(&mut nested, &mut probe, |events| {
            events.contains(&BackendEvent::SessionEnded)
        });
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, BackendEvent::SessionEnded))
                .count(),
            1,
            "closing the last window is one session end: {events:?}"
        );
        assert!(session_outputs(&nested).is_empty());
        assert!(
            nested.dispatch(&mut probe).is_empty(),
            "an ended session reports nothing more"
        );
    });
}

#[test]
fn a_handle_opens_and_closes_output_windows_and_closing_them_all_is_not_a_session_end() {
    let session = Session::start();
    session.run(|host| {
        let mut nested = Nested::connect(&NestedConfig::new(host.socket())).expect("the session connects");
        let mut probe = Probe::drawing(RED);
        let first = only_output(&nested).id;
        let _ = nested.dispatch(&mut probe);
        let handle = nested.handle();

        handle.connect_output(gnoblin_backend::nested::OutputSpec::new("NESTED-2"));
        let mut events = Vec::new();
        let deadline = Instant::now() + TIMEOUT;
        while session_outputs(&nested).len() < 2 {
            assert!(Instant::now() < deadline, "the second window never became an output");
            nested.roundtrip().expect("the host answers a round trip");
            events.extend(nested.dispatch(&mut probe));
        }
        let second = events
            .iter()
            .find_map(|event| match event {
                BackendEvent::OutputConnected(output) if output.connector == "NESTED-2" => Some(output.id),
                _ => None,
            })
            .expect("the new window is reported as an output");
        assert_ne!(second, first, "an output never reuses an identity");
        draw(&mut nested, &mut probe);
        host.wait_for(|observation| observation.windows().filter(|window| window.mapped).count() == 2);

        handle.disconnect_output(first);
        handle.disconnect_output(second);
        let mut events = Vec::new();
        while !session_outputs(&nested).is_empty() {
            assert!(Instant::now() < deadline, "the windows never closed");
            nested.roundtrip().expect("the host answers a round trip");
            events.extend(nested.dispatch(&mut probe));
        }
        assert!(events.contains(&BackendEvent::OutputDisconnected(first)), "{events:?}");
        assert!(events.contains(&BackendEvent::OutputDisconnected(second)), "{events:?}");
        assert!(
            !events.contains(&BackendEvent::SessionEnded),
            "unplugging windows through a handle is not the person ending the session: {events:?}"
        );
        host.wait_for(|observation| observation.windows().next().is_none());
    });
}

#[test]
fn a_host_scale_change_reaches_the_nested_output() {
    let session = Session::start();
    session.run(|host| {
        let mut nested = Nested::connect(&NestedConfig::new(host.socket())).expect("the session connects");
        let mut probe = Probe::drawing(RED);
        draw(&mut nested, &mut probe);
        host.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));
        let _ = nested.dispatch(&mut probe);
        assert_eq!(
            only_output(&nested).scale,
            Scale::IDENTITY,
            "the host starts at scale one"
        );

        host.control.set_global_scale(Some(2));
        let events = collect_until(&mut nested, &mut probe, |events| {
            events
                .iter()
                .any(|event| matches!(event, BackendEvent::OutputResized { scale, .. } if *scale == Scale::integral(2)))
        });
        let output = only_output(&nested);
        assert_eq!(
            output.scale,
            Scale::integral(2),
            "the session follows the host's scale: {events:?}"
        );
        assert_eq!(
            (output.mode.width, output.mode.height),
            (output.logical_size().width * 2, output.logical_size().height * 2),
            "the mode is the window's logical size at the new scale"
        );
        draw_rendered(&mut nested, &mut probe);
        assert_eq!(
            session_screenshot(&mut nested, output.id).map(|image| image.size()),
            Some(Size::new(output.mode.width, output.mode.height)),
            "the session draws at the new scale"
        );
    });
}

#[test]
fn the_nested_window_asks_the_host_to_keep_its_shortcuts_while_it_has_the_keyboard() {
    let session = Session::start();
    session.run(|host| {
        let mut nested = Nested::connect(&NestedConfig::new(host.socket())).expect("the session connects");
        let mut probe = Probe::drawing(RED);
        let output = only_output(&nested).id;
        assert!(
            nested.shortcuts_inhibited(output).is_some(),
            "the host offers the inhibitor, so the session asked for it"
        );
        draw(&mut nested, &mut probe);
        let window = host
            .wait_for(|observation| observation.only().is_some_and(|window| window.mapped))
            .only()
            .expect("one mapped window")
            .clone();
        let _ = host.control.focus(window.id);
        let deadline = Instant::now() + TIMEOUT;
        while nested.shortcuts_inhibited(output) != Some(true) {
            assert!(
                Instant::now() < deadline,
                "the host never granted the inhibitor: {:?}",
                nested.shortcuts_inhibited(output)
            );
            nested.roundtrip().expect("the host answers a round trip");
            let _ = nested.dispatch(&mut probe);
            thread::sleep(Duration::from_millis(5));
        }
    });
}
