//! `gnoblinctl output call <id> apply`, which the compositor used to refuse with "does not serve that method".
//!
//! The call goes through the real `gnoblinctl` library entry, the real IPC server and the real session on a virtual
//! output, and it runs the same engine as a shell's `screen.apply` and a `zwlr_output_configuration_v1`: the layout
//! is resolved, the mode and scale are tested on the backend, the output is driven, and a refusal keeps what was
//! there. What it does not prove: a modeset on hardware (see `design/js-outputs.md`).

mod support;

use support::{Session, wait_for};

/// The object id of the first output `output list` prints, once the session publishes one.
#[track_caller]
fn first_output(session: &Session) -> u64 {
    wait_for("an output is listed", || {
        let list = session.ctl(&["output", "list"]).ok();
        list.out
            .lines()
            .next()
            .and_then(|line| line.split('\t').next())
            .and_then(|id| id.parse().ok())
    })
}

/// A numeric property of an output, as `output get` prints it.
#[track_caller]
fn number(session: &Session, id: u64, property: &str) -> f64 {
    let value = session.ctl(&["output", "get", &id.to_string(), property]).ok();
    value
        .out
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("`{property}` is a number, got {:?}", value.out))
}

/// The arguments of a full configuration of an output, as the method declares them.
fn apply_arguments(id: &str, scale: &str, transform: &str, mode: &str) -> Vec<String> {
    let mut arguments: Vec<String> = ["output", "call", id, "apply"].map(str::to_owned).to_vec();
    arguments.extend(
        [
            "x=0",
            "y=0",
            &format!("scale={scale}"),
            &format!("transform={transform}"),
            "primary=true",
            "enabled=true",
            &format!("mode={mode}"),
        ]
        .map(str::to_owned),
    );
    arguments
}

#[test]
fn apply_on_an_output_changes_its_scale_and_its_mode() {
    let session = Session::start();
    let id = first_output(&session);
    let text = id.to_string();
    let before = number(&session, id, "width");

    let arguments = apply_arguments(&text, "2", "0", "800x600@60");
    let command = session.ctl(&arguments.iter().map(String::as_str).collect::<Vec<_>>());
    assert_eq!(command.code, 0, "the apply is served: {}", command.err);

    wait_for("the new scale", || {
        ((number(&session, id, "scale") - 2.0).abs() < f64::EPSILON).then_some(())
    });
    // 800 physical pixels at scale 2 are 400 logical ones: the output is driven at the new mode, and the layout was
    // recomputed for it.
    wait_for("the new extent", || {
        ((number(&session, id, "width") - 400.0).abs() < f64::EPSILON).then_some(())
    });
    assert!(before > 400.0, "the output was wider before: {before}");
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_rotation_is_refused_with_its_reason_and_the_output_is_untouched() {
    let session = Session::start();
    let id = first_output(&session);
    let text = id.to_string();
    let before = number(&session, id, "width");

    let arguments = apply_arguments(&text, "1", "1", "800x600@60");
    let command = session.ctl(&arguments.iter().map(String::as_str).collect::<Vec<_>>());
    assert_ne!(command.code, 0, "no renderer draws a rotation");
    assert!(command.err.contains("normal"), "the reason is given: {}", command.err);
    assert!((number(&session, id, "width") - before).abs() < f64::EPSILON);
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_mode_the_engine_cannot_read_is_refused_and_the_output_is_untouched() {
    let session = Session::start();
    let id = first_output(&session);
    let text = id.to_string();
    let before = number(&session, id, "width");

    let arguments = apply_arguments(&text, "1", "0", "banana");
    let command = session.ctl(&arguments.iter().map(String::as_str).collect::<Vec<_>>());
    assert_ne!(command.code, 0, "`banana` is not a mode");
    assert!((number(&session, id, "width") - before).abs() < f64::EPSILON);
    session.stop().expect("the session stops cleanly");
}
