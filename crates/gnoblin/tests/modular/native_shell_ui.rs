//! Example 31 run for real from the three languages: the shipped binary serving a shell that starts the not-responding alert, puts the
//! stock lock screen to work, opens the launcher with a chord and flips the overview with another, with a real Wayland client that stops
//! answering the compositor's pings and a virtual keyboard that types.
//!
//! What is asserted is what the compositor's tree holds and what the shell logs:
//!
//! - A client that leaves the liveness ping unanswered gets the alert, which names its window. Force quit is off for the arming time, so
//!   Tab and Return at once close nothing, and Escape (Wait) closes the card. Force quit after the arming time asks the compositor to kill
//!   the client's process, which here is this test, so the press after arming is the replay's (`tests/shell_ui.rs` of the std crate).
//! - The session is locked the way the screensaver service locks it. The lock is engaged by the native screen, which draws the hint under
//!   the lock node; a password typed at it goes to the compositor and the screen shows what came back. The shipped binary checks the
//!   password with PAM, so a right password cannot be typed here: the right password and every other message are the replay's.
//! - `<Super>space` opens the launcher, typed text is painted, Return runs the command through the shell's callback.
//! - `<Super>o` flips the overview.
//!
//! One core and one std are mapped.

use std::path::Path;
use std::time::Duration;

use crate::support::lock_session::view;
use crate::support::native_shell::{
    Gnoblin, Home, KEY_A, KEY_B, KEY_ENTER, KEY_ESC, KEY_O, KEY_SPACE, KEY_TAB, Launch, Start, Typist,
    staged_library_path, start_script_in, std_shells,
};
use crate::support::{Mapper, snapshot, take_lock};
use crate::{copy_example, serving};

use super::native_std::assert_one_core_one_std;

/// The compositor pings a client and marks one that has not answered within this time as unresponsive.
const CHECK_ALIVE: &str = "[window-management]\ncheck-alive-timeout = 800\n";

fn home() -> Home {
    let home = Home::new();
    home.write("config/gnoblin/config.toml", CHECK_ALIVE);
    home
}

/// Whether some canvas of the tree paints `text`.
fn paints(ipc: &Path, text: &str) -> bool {
    snapshot(ipc).iter().any(|node| {
        node.class == "canvas"
            && node.props.iter().any(|prop| {
                prop.key == "content"
                    && gnoblin_paint::decode(&prop.value_bytes)
                        .is_ok_and(|list| format!("{:?}", list.ops).contains(&format!("text: {text:?}")))
            })
    })
}

#[track_caller]
fn the_shell_ui_works(session: &mut Gnoblin) {
    session.wait_log("the shell started", |line| line.contains("shell-ui: running"));
    assert!(
        session
            .log()
            .iter()
            .any(|line| line.text.contains("shell-ui: the lock screen is ready, shown false")),
        "{}",
        session.log_text()
    );
    let ipc = session.ipc();
    let mut mapper = Mapper::connect(&session.wayland());
    let mut typist = Typist::connect(&session.wayland());

    // A client that reads none of its events from here on answers no ping.
    mapper.map_window("org.example.stuck", "Stuck");
    let alert = "Stuck is not responding";
    session.wait_for("the alert opened", |_| paints(&ipc, alert));
    // Force quit is off for the arming time: Tab goes to it, Return closes nothing.
    typist.tap(KEY_TAB);
    typist.tap(KEY_ENTER);
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        paints(&ipc, alert),
        "Return on a Force quit that is not armed closed the alert"
    );
    assert!(
        session
            .find(|line| line.contains("shell-ui: the alert for window"))
            .is_none(),
        "{}",
        session.log_text()
    );
    // Escape waits: the card goes and the shell is told.
    typist.tap(KEY_ESC);
    session.wait_log("the alert ended", |line| {
        line.contains("shell-ui: the alert for window")
    });
    session.wait_for("the card is gone", |_| !paints(&ipc, alert));
    // The client answers again.
    mapper.roundtrip();

    // The launcher: a chord opens it, what is typed is painted, Return runs it.
    typist.super_plus(KEY_SPACE);
    session.wait_log("the launcher opened", |line| {
        line.contains("shell-ui: the launcher is open")
    });
    typist.tap(KEY_A);
    typist.tap(KEY_B);
    session.wait_for("the typed text is painted", |_| paints(&ipc, "ab"));
    typist.tap(KEY_ENTER);
    session.wait_log("the launcher ran the command", |line| {
        line.contains("shell-ui: the launcher ran ab")
    });
    session.wait_for("the launcher is gone", |_| !paints(&ipc, "ab"));

    // The overview flips with its chord.
    typist.super_plus(KEY_O);
    session.wait_log("the overview is on", |line| {
        line.contains("shell-ui: the overview is on")
    });
    typist.super_plus(KEY_O);
    session.wait_log("the overview is off", |line| {
        line.contains("shell-ui: the overview is off")
    });

    // The lock: engaged by the native screen, which draws its hint under the lock node.
    take_lock(&ipc);
    session.wait_for("the native screen engaged the lock", |_| {
        let seen = view(&ipc);
        seen.state == "locked" && seen.engaged && seen.texts.iter().any(|text| text == "Enter your password.")
    });
    typist.tap(KEY_A);
    typist.tap(KEY_B);
    typist.tap(KEY_ENTER);
    session.wait_for("the screen shows what the compositor answered", |_| {
        view(&ipc).texts.iter().any(|text| {
            text == "Wrong password." || text == "The password could not be checked." || text.starts_with("Too many")
        })
    });
    let seen = view(&ipc);
    assert!(
        seen.engaged && seen.state == "locked",
        "a wrong password does not unlock: {seen:?}"
    );
    assert!(
        !seen.texts.iter().any(|text| text == "Enter your password."),
        "the hint is replaced by the answer: {seen:?}"
    );
    assert_one_core_one_std(session);
    assert!(session.exited().is_none(), "{}", session.log_text());
}

#[test]
fn example_31_a_std_shell_raises_the_alert_opens_the_launcher_flips_the_overview_and_locks_with_its_own_screen() {
    let launch = Launch::compositor(&[])
        .with_arg(&std_shells().shell_ui)
        .with_env("LD_LIBRARY_PATH", &staged_library_path());
    let mut session = Gnoblin::launch(home(), launch);
    session.wait_serving();
    session.wait_log("the native shell serves in process", |line| {
        line.contains("serves in process")
    });
    the_shell_ui_works(&mut session);
}

#[test]
fn example_31_a_typescript_shell_raises_the_alert_opens_the_launcher_flips_the_overview_and_locks_with_its_own_screen()
{
    let dir = copy_example("31-native-shell-ui-ts");
    let mut session = start_script_in(home(), None, Start::Compositor, &dir.path().join("shell.ts"), &[]);
    serving(&mut session);
    the_shell_ui_works(&mut session);
}

#[test]
fn example_31_a_python_shell_raises_the_alert_opens_the_launcher_flips_the_overview_and_locks_with_its_own_screen() {
    let dir = copy_example("31-native-shell-ui-py");
    let mut session = start_script_in(home(), None, Start::Compositor, &dir.path().join("shell.py"), &[]);
    serving(&mut session);
    the_shell_ui_works(&mut session);
    assert!(!session.log_text().contains("Traceback"), "{}", session.log_text());
}
