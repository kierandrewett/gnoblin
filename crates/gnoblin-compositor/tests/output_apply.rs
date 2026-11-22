//! `gnoblinctl output call <id> apply`.
//!
//! The call goes through the real `gnoblinctl` library entry, the real IPC server and the real session on a virtual
//! output, and it runs the same engine as a shell's `screen.apply` and a `zwlr_output_configuration_v1`: the layout
//! is resolved, the mode and scale are tested on the backend, the output is driven, and a refusal keeps what was
//! there. What it does not prove: a modeset on hardware (see `design/js/outputs.md`).

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
    supersampled_arguments(id, scale, transform, mode, "1")
}

/// The arguments of a full configuration of an output composed at `supersample` times its mode.
fn supersampled_arguments(id: &str, scale: &str, transform: &str, mode: &str, supersample: &str) -> Vec<String> {
    let mut arguments: Vec<String> = ["output", "call", id, "apply"].map(str::to_owned).to_vec();
    arguments.extend(
        [
            "x=0",
            "y=0",
            &format!("scale={scale}"),
            &format!("transform={transform}"),
            "enabled=true",
            &format!("mode={mode}"),
            &format!("supersample={supersample}"),
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
fn a_primary_argument_is_refused_naming_the_property_and_the_output_is_untouched() {
    // The layout derives the primary output, so an `apply` that carries `primary` would be asking for something it
    // never gets. It must say so instead of answering success.
    let session = Session::start();
    let id = first_output(&session);
    let text = id.to_string();
    let before = number(&session, id, "width");

    let mut arguments = apply_arguments(&text, "2", "0", "800x600@60");
    arguments.retain(|argument| !argument.starts_with("primary="));
    arguments.push("primary=false".to_owned());
    let command = session.ctl(&arguments.iter().map(String::as_str).collect::<Vec<_>>());
    assert_ne!(command.code, 0, "`primary` is not something `apply` sets");
    assert!(
        command.err.contains("primary"),
        "the property is named: {}",
        command.err
    );
    assert!(
        (number(&session, id, "width") - before).abs() < f64::EPSILON,
        "nothing was applied"
    );
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

/// Runs `gnoblinctl output call <id> apply` with the arguments.
fn call(session: &Session, arguments: &[String]) -> support::Command {
    session.ctl(&arguments.iter().map(String::as_str).collect::<Vec<_>>())
}

#[test]
fn apply_composes_an_output_at_a_supersample_factor_and_keeps_its_logical_size() {
    let session = Session::start();
    let id = first_output(&session);
    let text = id.to_string();
    assert!(
        (number(&session, id, "supersample") - 1.0).abs() < f64::EPSILON,
        "an output is composed at its mode until it is told otherwise"
    );
    let width = number(&session, id, "width");
    let height = number(&session, id, "height");

    for factor in ["2", "4", "1"] {
        let command = call(&session, &supersampled_arguments(&text, "1", "0", "", factor));
        assert_eq!(command.code, 0, "a factor of {factor} is served: {}", command.err);
        let wanted: f64 = factor.parse().expect("a number");
        wait_for("the new factor", || {
            ((number(&session, id, "supersample") - wanted).abs() < f64::EPSILON).then_some(())
        });
        // The desktop is the size it was: the factor changes the pixels behind it and nothing in front of it.
        assert!(
            (number(&session, id, "width") - width).abs() < f64::EPSILON,
            "width at {factor}"
        );
        assert!(
            (number(&session, id, "height") - height).abs() < f64::EPSILON,
            "height at {factor}"
        );
    }
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_supersample_the_virtual_size_cannot_honour_is_refused_naming_it_and_the_output_keeps_its_factor() {
    let session = Session::start();
    let id = first_output(&session);
    let text = id.to_string();
    let command = call(&session, &supersampled_arguments(&text, "1", "0", "800x600@60", "2"));
    assert_eq!(command.code, 0, "{}", command.err);
    wait_for("the factor", || {
        ((number(&session, id, "supersample") - 2.0).abs() < f64::EPSILON).then_some(())
    });

    // 800x600 at 1.001 is 800.8 pixels wide: not a whole surface.
    let refused = call(
        &session,
        &supersampled_arguments(&text, "1", "0", "800x600@60", "1.001"),
    );
    assert_ne!(refused.code, 0, "1.001 of 800 is not a whole width");
    assert!(
        refused.err.contains("supersample") || refused.out.contains("supersample"),
        "the refusal names the setting: out {:?} err {:?}",
        refused.out,
        refused.err
    );
    // Too large a surface is refused for the same reason.
    let huge = call(&session, &supersampled_arguments(&text, "1", "0", "800x600@60", "16"));
    assert_ne!(huge.code, 0, "800 at 16 is 12800 pixels, past the limit");
    assert!(
        huge.err.contains("supersample") || huge.out.contains("supersample"),
        "out {:?} err {:?}",
        huge.out,
        huge.err
    );
    assert!(
        (number(&session, id, "supersample") - 2.0).abs() < f64::EPSILON,
        "a refused apply keeps the factor it had"
    );
    session.stop().expect("the session stops cleanly");
}

/// What `output get` prints for a property, as text.
#[track_caller]
fn text(session: &Session, id: u64, property: &str) -> String {
    session
        .ctl(&["output", "get", &id.to_string(), property])
        .ok()
        .out
        .trim()
        .to_owned()
}

#[test]
fn an_apply_with_only_a_supersample_changes_only_the_factor() {
    // The owner's call: `gnoblinctl output call <id> apply supersample=2`, with nothing else, is served and every
    // setting that was left out keeps its value.
    let session = Session::start();
    let id = first_output(&session);
    let text_id = id.to_string();
    let full = apply_arguments(&text_id, "2", "0", "800x600@60");
    let command = call(&session, &full);
    assert_eq!(command.code, 0, "{}", command.err);
    wait_for("the new scale", || {
        ((number(&session, id, "scale") - 2.0).abs() < f64::EPSILON).then_some(())
    });
    let mode = text(&session, id, "mode");
    let width = number(&session, id, "width");

    let only = call(
        &session,
        &["output", "call", &text_id, "apply", "supersample=2"].map(str::to_owned),
    );
    assert_eq!(only.code, 0, "a call with only `supersample` is served: {}", only.err);
    wait_for("the factor", || {
        ((number(&session, id, "supersample") - 2.0).abs() < f64::EPSILON).then_some(())
    });
    assert!(
        (number(&session, id, "scale") - 2.0).abs() < f64::EPSILON,
        "the scale was left out"
    );
    assert_eq!(text(&session, id, "mode"), mode, "the mode was left out");
    assert!(
        (number(&session, id, "width") - width).abs() < f64::EPSILON,
        "so the desktop kept its size"
    );

    // The other way round: a scale alone keeps the factor the output was composed at.
    let scale = call(
        &session,
        &["output", "call", &text_id, "apply", "scale=1"].map(str::to_owned),
    );
    assert_eq!(scale.code, 0, "{}", scale.err);
    wait_for("the scale", || {
        ((number(&session, id, "scale") - 1.0).abs() < f64::EPSILON).then_some(())
    });
    assert!(
        (number(&session, id, "supersample") - 2.0).abs() < f64::EPSILON,
        "the factor was left out of this call"
    );
    assert_eq!(text(&session, id, "mode"), mode);
    session.stop().expect("the session stops cleanly");
}

#[test]
fn an_apply_with_no_key_at_all_changes_nothing() {
    let session = Session::start();
    let id = first_output(&session);
    let (mode, scale) = (text(&session, id, "mode"), number(&session, id, "scale"));
    let command = call(
        &session,
        &["output", "call", &id.to_string(), "apply"].map(str::to_owned),
    );
    assert_eq!(command.code, 0, "every parameter is optional: {}", command.err);
    assert_eq!(text(&session, id, "mode"), mode);
    assert!((number(&session, id, "scale") - scale).abs() < f64::EPSILON);
    session.stop().expect("the session stops cleanly");
}

#[test]
fn an_apply_with_only_x_is_served_and_keeps_y() {
    let session = Session::start();
    let id = first_output(&session);
    let y = number(&session, id, "y");
    let command = call(
        &session,
        &["output", "call", &id.to_string(), "apply", "x=0"].map(str::to_owned),
    );
    assert_eq!(command.code, 0, "{}", command.err);
    assert!((number(&session, id, "x")).abs() < f64::EPSILON);
    assert!((number(&session, id, "y") - y).abs() < f64::EPSILON, "`y` was left out");
    session.stop().expect("the session stops cleanly");
}

#[test]
fn an_apply_still_refuses_a_key_it_does_not_declare_and_a_value_of_the_wrong_type() {
    let session = Session::start();
    let id = first_output(&session);
    let text_id = id.to_string();
    let factor = number(&session, id, "supersample");

    let unknown = call(
        &session,
        &["output", "call", &text_id, "apply", "supersample=2", "sharpness=3"].map(str::to_owned),
    );
    assert_ne!(unknown.code, 0, "`sharpness` is not a parameter");
    assert!(unknown.err.contains("sharpness"), "the key is named: {}", unknown.err);

    let wrong = call(
        &session,
        &["output", "call", &text_id, "apply", "supersample=lots"].map(str::to_owned),
    );
    assert_ne!(wrong.code, 0, "`lots` is not a number");
    assert!(wrong.err.contains("supersample"), "the key is named: {}", wrong.err);

    let bad = call(
        &session,
        &["output", "call", &text_id, "apply", "supersample=0.5"].map(str::to_owned),
    );
    assert_ne!(bad.code, 0, "a factor under 1 is refused by the engine");
    assert!(
        (number(&session, id, "supersample") - factor).abs() < f64::EPSILON,
        "nothing was applied"
    );
    session.stop().expect("the session stops cleanly");
}
