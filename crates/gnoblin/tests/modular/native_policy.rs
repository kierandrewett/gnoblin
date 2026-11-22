//! Example 32 run for real from the three languages: the shipped binary serving a shell that starts the window policy of `libgnoblin-std`,
//! with a real Wayland client that asks to go fullscreen, to maximise and to leave maximised.
//!
//! What is asserted is what the shell logs and what the compositor's tree holds: a listener of the shell hears each request first and
//! logs it, the fullscreen request is taken by the listener (so the stock answer does not run and the window stays out of fullscreen), the
//! maximise request is answered by the stock policy (the window fills the screen), and the request to leave maximised ends it. One core and one std are mapped.

use crate::support::native_shell::{Gnoblin, Start, start_script, std_shells};
use crate::support::{Mapper, Window, scene};
use crate::{copy_example, screen_of, serving};

use super::native_std::{assert_one_core_one_std, start_std};

/// How many lines of the log hold `needle`.
fn count(session: &Gnoblin, needle: &str) -> usize {
    session.log().iter().filter(|line| line.text.contains(needle)).count()
}

/// The window of `app` as the compositor holds it.
fn window_of(session: &Gnoblin, app: &str) -> Window {
    scene(&session.ipc())
        .into_iter()
        .find(|window| window.app_id == app)
        .unwrap_or_else(|| panic!("no window of {app}"))
}

#[track_caller]
fn a_window_asks_and_the_policy_answers(session: &mut Gnoblin) {
    session.wait_log("the shell started the policy", |line| line.contains("policy: running"));
    // The gesture bindings: made, set with ways and actions, installed, and the way of a gesture told, as the replays of the std crate hold
    // the gestures themselves to the TypeScript library's. A touchpad is not something this session can give a shell.
    for needed in [
        "policy: the gesture bindings are installed",
        "policy: an action that is not one is refused",
    ] {
        assert!(
            session.log().iter().any(|line| line.text.contains(needed)),
            "the shell never said {needed:?}: {}",
            session.log_text()
        );
    }
    let directions: Vec<String> = session
        .log()
        .iter()
        .filter_map(|line| {
            let rest = line.text.split("policy: gesture direction ").nth(1)?;
            rest.split_whitespace().next().map(str::to_owned)
        })
        .collect();
    assert_eq!(
        directions,
        ["left", "none", "right"],
        "a swipe of -100 and 5 goes left, one of 1 and 1 has not gone the default 48 units, and with a threshold of 0.5 it has and the tie goes \
         to the horizontal axis: {}",
        session.log_text()
    );
    let (width, height) = screen_of(session);
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("plain", "plain");
    session.wait_for("the window is in the tree", |session| scene(&session.ipc()).len() == 1);
    let plain = window_of(session, "plain");

    // The listener of the shell hears a request first, and one it takes is not answered by the stock policy.
    mapper.fullscreen(0);
    session.wait_log("the listener refuses fullscreen", |line| {
        line.contains(&format!("policy: request fullscreen of window {}", plain.id))
    });
    session.wait_log("the listener took it", |line| {
        line.contains("policy: fullscreen refused")
    });
    mapper.roundtrip();
    let after = window_of(session, "plain");
    assert!(
        (after.width, after.height) != (width, height),
        "a fullscreen request the shell took leaves the window as it was: {}",
        session.log_text()
    );

    // A request the listener leaves alone is answered by the stock policy.
    mapper.maximize(0);
    session.wait_log("the listener hears the maximise request", |line| {
        line.contains(&format!("policy: request maximize of window {}", plain.id))
    });
    session.wait_for("the window fills the work area", |session| {
        let window = window_of(session, "plain");
        window.width >= width * 0.9 && window.height >= height * 0.8
    });
    mapper.unmaximize(0);
    session.wait_for("the window no longer fills the work area", |session| {
        let window = window_of(session, "plain");
        window.width < width * 0.9
    });
    assert_eq!(count(session, "fullscreen refused"), 1, "{}", session.log_text());
    assert_one_core_one_std(session);
    assert!(session.exited().is_none(), "{}", session.log_text());
}

#[test]
fn example_32_a_std_shell_starts_the_window_policy_and_a_client_asks_for_fullscreen_and_maximise() {
    let mut session = start_std(&std_shells().policy);
    a_window_asks_and_the_policy_answers(&mut session);
}

fn script_session(name: &str, file: &str) -> (tempfile::TempDir, Gnoblin) {
    let dir = copy_example(name);
    let mut session = start_script(Start::Compositor, &dir.path().join(file), &[]);
    serving(&mut session);
    (dir, session)
}

#[test]
fn example_32_a_typescript_shell_starts_the_window_policy_and_a_client_asks_for_fullscreen_and_maximise() {
    let (_dir, mut session) = script_session("32-native-policy", "shell.ts");
    a_window_asks_and_the_policy_answers(&mut session);
}

#[test]
fn example_32_a_python_shell_starts_the_window_policy_and_a_client_asks_for_fullscreen_and_maximise() {
    let (_dir, mut session) = script_session("32-native-policy", "shell.py");
    a_window_asks_and_the_policy_answers(&mut session);
    assert!(!session.log_text().contains("Traceback"), "{}", session.log_text());
}
