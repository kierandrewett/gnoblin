//! The nested backend against a real host compositor (Sway or Weston), not this repository's own.
//!
//! Every other nested test is hosted by `gnoblin-wayland` on the headless backend, which is one implementation of the
//! protocols the nested backend speaks. This file runs the scenarios a headless host program can observe against a
//! second implementation, so a bug that only the repository's own host forgives shows. It cannot run by itself: it needs
//! a compositor to connect to, so every test is ignored until `scripts/real-host-conformance.sh` starts one and
//! names it.
//!
//! The host is named by `GNOBLIN_HOST_SOCKET` (a socket path, or a name in `XDG_RUNTIME_DIR`). Sway is told apart by
//! `SWAYSOCK`, which lets a test ask it for its window tree and change its output's scale. The clipboard scenarios
//! use `wl-copy` and `wl-paste` against the host's own data-control protocol.
//!
//! What a headless real host cannot show is named in `docs/backends.md`: it has no tablet, no pointer of the person's
//! to lock, and (here) no GPU for the dmabuf path.

mod nested_host;
mod stock;

use std::io::Read;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::nested::{DecorationAnswer, Nested, NestedConfig, NestedError};
use gnoblin_backend::{Backend, BackendEvent, Present, SelectionTarget};
use gnoblin_config::NestedDecorations;
use gnoblin_types::Scale;
use nested_host::{Probe, RED, TIMEOUT};

/// The host's socket, or a failed test that says how to start one.
fn host_socket() -> PathBuf {
    let named = std::env::var("GNOBLIN_HOST_SOCKET")
        .expect("GNOBLIN_HOST_SOCKET names the host: run scripts/real-host-conformance.sh");
    let path = PathBuf::from(&named);
    if path.is_absolute() {
        path
    } else {
        PathBuf::from(std::env::var("XDG_RUNTIME_DIR").expect("XDG_RUNTIME_DIR")).join(named)
    }
}

/// Whether the host is Sway, which has a command socket.
fn is_sway() -> bool {
    std::env::var_os("SWAYSOCK").is_some()
}

/// Runs a `swaymsg` command and returns what it printed.
fn swaymsg(arguments: &[&str]) -> String {
    let output = Command::new("swaymsg").args(arguments).output().expect("swaymsg runs");
    assert!(
        output.status.success(),
        "swaymsg {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Connects to the host, or `None` for a host that has no seat: the session needs a `wl_seat` to take input from, and says
/// so by name instead of running with none (a headless Weston offers none).
fn connect() -> Option<(Nested, Probe)> {
    match Nested::connect(&NestedConfig::new(host_socket())) {
        Ok(nested) => Some((nested, Probe::drawing(RED))),
        Err(NestedError::MissingGlobal("wl_seat")) => {
            println!("skipped: the host offers no wl_seat, which the session refused by name");
            None
        }
        Err(other) => panic!("the session did not connect to the real host: {other}"),
    }
}

/// Serves the session's connection until `done` holds for what it collected.
fn pump(nested: &mut Nested, probe: &mut Probe, done: impl Fn(&[BackendEvent], &Probe) -> bool) -> Vec<BackendEvent> {
    let deadline = Instant::now() + TIMEOUT;
    let mut events = Vec::new();
    loop {
        nested.roundtrip().expect("the host answers a round trip");
        events.extend(nested.dispatch(probe));
        if done(&events, probe) {
            return events;
        }
        assert!(
            Instant::now() < deadline,
            "the real host never reached the expected state: {events:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

/// Serves the session's connection until `finished` says the thread it waits for is done.
fn serve_while(nested: &mut Nested, probe: &mut Probe, finished: impl Fn() -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !finished() {
        nested.roundtrip().expect("the host answers a round trip");
        let _ = nested.dispatch(probe);
        assert!(
            Instant::now() < deadline,
            "the thread the test waits for never finished"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

/// Draws until a frame was really presented, which is when the host asked for one.
fn draw_rendered(nested: &mut Nested, probe: &mut Probe) {
    let output = <Nested as Backend<Probe>>::outputs(nested)[0].id;
    let deadline = Instant::now() + TIMEOUT;
    loop {
        nested.roundtrip().expect("the host answers a round trip");
        let _ = nested.dispatch(probe);
        if nested.render(probe, output).expect("a frame") == Present::Rendered {
            return;
        }
        assert!(Instant::now() < deadline, "the real host never asked for a frame");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "needs a real host: scripts/real-host-conformance.sh"]
fn the_real_host_sizes_and_scales_the_window() {
    let Some((mut nested, mut probe)) = connect() else {
        return;
    };
    let _ = nested.dispatch(&mut probe);
    let outputs = <Nested as Backend<Probe>>::outputs(&nested);
    assert_eq!(outputs.len(), 1, "one window is one output");
    let output = outputs[0].clone();
    assert!(
        output.mode.width > 0 && output.mode.height > 0,
        "the host gave the window a size: {output:?}"
    );
    assert!(output.scale.as_f64() >= 1.0, "and a scale: {output:?}");
    println!(
        "real host: {}x{} at scale {} (fractional scale offered: {})",
        output.mode.width,
        output.mode.height,
        output.scale.as_f64(),
        nested.fractional_scale_offered()
    );
}

#[test]
#[ignore = "needs a real host: scripts/real-host-conformance.sh"]
fn the_real_host_paces_frames_and_shows_ours() {
    let Some((mut nested, mut probe)) = connect() else {
        return;
    };
    draw_rendered(&mut nested, &mut probe);
    // Two frames in a row: the second needs the host's frame callback for the first.
    draw_rendered(&mut nested, &mut probe);
    let output = <Nested as Backend<Probe>>::outputs(&nested)[0].id;
    let image = <Nested as Backend<Probe>>::screenshot(&mut nested, output).expect("a readback");
    assert_eq!(
        image.pixel(gnoblin_types::Point::new(5, 5)),
        Some([255, 0, 0, 255]),
        "the session drew red"
    );
}

#[test]
#[ignore = "needs a real host: scripts/real-host-conformance.sh"]
fn sway_shows_the_window_with_its_title_and_app_id() {
    if !is_sway() {
        println!("skipped: the host is not Sway");
        return;
    }
    let Some((mut nested, mut probe)) = connect() else {
        return;
    };
    draw_rendered(&mut nested, &mut probe);
    let tree = swaymsg(&["-t", "get_tree"]);
    assert!(
        tree.contains("\"org.gnoblin.Session\""),
        "sway lists the session's app id: {tree}"
    );
    assert!(tree.contains("Gnoblin session (NESTED-1)"), "and its title");
}

#[test]
#[ignore = "needs a real host: scripts/real-host-conformance.sh"]
fn sway_answers_server_side_so_the_session_draws_no_frame() {
    if !is_sway() {
        println!("skipped: the host is not Sway");
        return;
    }
    let config = NestedConfig {
        decorations: NestedDecorations::Auto,
        ..NestedConfig::new(host_socket())
    };
    let mut nested = Nested::connect(&config).expect("the session connected to sway");
    let mut probe = Probe::drawing(RED);
    draw_rendered(&mut nested, &mut probe);
    // Sway sizes a tiled window after the first configure, which leaves the size to the window: let that one arrive.
    for _ in 0..30 {
        nested.roundtrip().expect("the host answers a round trip");
        let _ = nested.dispatch(&mut probe);
        thread::sleep(Duration::from_millis(20));
    }
    draw_rendered(&mut nested, &mut probe);
    let output = <Nested as Backend<Probe>>::outputs(&nested)[0].clone();
    println!(
        "real host: sway answered {:?} to the request for a server-side frame",
        nested.decoration_answer(output.id)
    );
    assert_eq!(
        nested.decoration_answer(output.id),
        Some(DecorationAnswer::Server),
        "wlroots draws the frame when a window asks it to"
    );
    assert!(
        !nested.frame_drawn(output.id),
        "so the session draws none, and the window is its output exactly"
    );
    // The tree holds one `window_rect` per window; the session's is the one that is its output's size. Sway draws its own
    // title bar above that rectangle, which is why the rectangle and not the container is compared.
    let tree = swaymsg(&["-t", "get_tree"]);
    let number = |after: &str, key: &str| -> Option<u32> {
        let rest = after.split(&format!("\"{key}\":")).nth(1)?;
        rest.trim_start()
            .split(|c: char| !c.is_ascii_digit())
            .next()?
            .parse()
            .ok()
    };
    let rectangles: Vec<(Option<u32>, Option<u32>)> = tree
        .split("\"window_rect\"")
        .skip(1)
        .map(|rest| (number(rest, "width"), number(rest, "height")))
        .collect();
    assert!(
        rectangles.contains(&(Some(output.logical_size().width), Some(output.logical_size().height))),
        "sway's window rectangle is the session's output: {rectangles:?} against {:?}",
        output.logical_size()
    );
}

#[test]
#[ignore = "needs a real host: scripts/real-host-conformance.sh"]
fn sway_states_a_whole_scale_and_a_fractional_one_rounded_up() {
    if !is_sway() {
        println!("skipped: the host is not Sway");
        return;
    }
    let Some((mut nested, mut probe)) = connect() else {
        return;
    };
    draw_rendered(&mut nested, &mut probe);
    let _ = nested.dispatch(&mut probe);
    let before = <Nested as Backend<Probe>>::outputs(&nested)[0].clone();
    // The first quoted string after the first "name" key of the output list is the output's name.
    let outputs = swaymsg(&["-t", "get_outputs", "--raw"]);
    let name = outputs
        .split("\"name\"")
        .nth(1)
        .and_then(|rest| rest.split('"').nth(1))
        .expect("sway has an output")
        .to_owned();
    println!(
        "real host: scale {} before; fractional scale offered: {}",
        before.scale.as_f64(),
        nested.fractional_scale_offered()
    );

    swaymsg(&["output", &name, "scale", "2"]);
    pump(&mut nested, &mut probe, |events, _| {
        events
            .iter()
            .any(|event| matches!(event, BackendEvent::OutputResized { scale, .. } if *scale == Scale::integral(2)))
    });
    let doubled = <Nested as Backend<Probe>>::outputs(&nested)[0].clone();
    assert_eq!(
        doubled.scale,
        Scale::integral(2),
        "the session follows sway's whole scale"
    );
    assert_eq!(
        (doubled.mode.width, doubled.mode.height),
        (doubled.logical_size().width * 2, doubled.logical_size().height * 2),
        "the mode is the logical size at the new scale"
    );
    draw_rendered(&mut nested, &mut probe);

    // A fractional scale: a host that cannot state it through the protocol states the next whole scale up through
    // wl_output, and the session follows that, with a mode that is still the logical size at the scale it has.
    swaymsg(&["output", &name, "scale", "1.5"]);
    for _ in 0..30 {
        nested.roundtrip().expect("the host answers a round trip");
        let _ = nested.dispatch(&mut probe);
        thread::sleep(Duration::from_millis(20));
    }
    let after = <Nested as Backend<Probe>>::outputs(&nested)[0].clone();
    println!("real host: after scale 1.5 the session is at {}", after.scale.as_f64());
    assert!(
        after.scale == Scale::integral(2) || after.scale == Scale::new(3, 2),
        "a fractional host scale reaches the session exactly or as the whole scale above: {:?}",
        after.scale
    );
    swaymsg(&["output", &name, "scale", &before.scale.as_f64().to_string()]);
}

/// Gives sway's keyboard focus to the session's window.
fn focus_session_on_sway() {
    swaymsg(&["[app_id=\"org.gnoblin.Session\"]", "focus"]);
}

#[test]
#[ignore = "needs a real host: scripts/real-host-conformance.sh"]
fn sways_clipboard_is_pasted_in_the_session() {
    if !is_sway() {
        println!("skipped: the host is not Sway");
        return;
    }
    let Some((mut nested, mut probe)) = connect() else {
        return;
    };
    draw_rendered(&mut nested, &mut probe);
    focus_session_on_sway();
    pump(&mut nested, &mut probe, |_, _| true);

    // Host to session: wl-copy sets the selection through the host's data control.
    // wl-copy forks and leaves the child holding the selection, so its own process ends at once.
    let copied = Command::new("wl-copy")
        .args(["--type", "text/plain;charset=utf-8", "hello from sway"])
        .env("WAYLAND_DISPLAY", host_socket())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("wl-copy runs");
    assert!(copied.success(), "wl-copy set the host's clipboard");
    pump(&mut nested, &mut probe, |_, probe| {
        probe
            .selections
            .iter()
            .any(|(target, types)| *target == SelectionTarget::Clipboard && types.is_some())
    });
    let (mut reader, writer) = std::io::pipe().expect("a pipe");
    let writer: OwnedFd = writer.into();
    <Nested as Backend<Probe>>::read_host_selection(
        &mut nested,
        SelectionTarget::Clipboard,
        "text/plain;charset=utf-8",
        writer,
    )
    .expect("the host's selection can be read");
    let bytes = thread::scope(|scope| {
        let handle = scope.spawn(move || {
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).expect("the pasted bytes");
            bytes
        });
        serve_while(&mut nested, &mut probe, || handle.is_finished());
        handle.join().expect("the reader joins")
    });
    assert_eq!(
        String::from_utf8_lossy(&bytes),
        "hello from sway",
        "the session read sway's clipboard"
    );

    // Session to host is not run here: `set_selection` needs the serial of an input event, and a headless Sway has no
    // input device, so the session has none to name and Sway ignores the request. That direction is proven against
    // this repository's own host, which has the keyboard focus to give (`tests/selection_bridge.rs`).
    println!("skipped: the session's copy cannot reach a host with no input device");
}

#[test]
#[ignore = "needs a real host: scripts/real-host-conformance.sh"]
fn a_host_with_no_seat_is_refused_by_name() {
    match Nested::connect(&NestedConfig::new(host_socket())) {
        Err(NestedError::MissingGlobal(name)) => assert_eq!(name, "wl_seat"),
        Ok(_) => println!("skipped: the host has a seat"),
        Err(other) => panic!("a host with no seat is refused by name, not with {other}"),
    }
}
