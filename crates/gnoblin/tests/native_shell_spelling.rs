//! The plus form of a chord (`Super+Return`) claimed by a Rust, a Python and a TypeScript shell in a real session
//! (`design/modular-framework.md`, section 31.2).
//!
//! The compositor's chord parser reads `Super+Return` as it reads `<Super>Return`, and no SDK rewrites the text on the way. So each shell below claims the chord as written, in a session that is the shipped binary on a
//! headless backend, and the proof is what a user would see: the chord typed on a virtual keyboard reaches the shell, which starts
//! the stand-in terminal that leaves a file behind. A claim the compositor refused would leave no file.

mod support;
use support::Mapper;
use support::native_shell::*;

/// Types the chord once the shell holds the tree (it has seen a window) and waits for the terminal the shell starts on it.
fn the_chord_starts_a_terminal(mut session: Gnoblin) {
    session.wait_serving();
    session.wait_log("the shell serves in process", |line| line.contains("serves in process"));
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("anchor", "Anchor");
    wait_for_window(&mut session, "the shell has seen a window", "anchor", |_| true);
    assert!(session.noted("terminal").is_none());
    session.wait_log("the shell has claimed the chord", |line| {
        line.contains("claimed Super+Return")
    });
    // A claim is a call over the tree link and takes effect when the compositor has read it: type until the shell answers (a chord
    // typed before the claim reaches a client, which has nothing bound to it, and is lost).
    let mut typist = Typist::connect(&session.wayland());
    session.wait_for("the chord reaches the shell and it starts the terminal", |session| {
        if session.noted("terminal").is_none() {
            typist.super_plus(KEY_ENTER);
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        session.noted("terminal").is_some()
    });
    assert!(session.exited().is_none(), "{}", session.log_text());
}

#[test]
fn a_rust_shell_claims_the_plus_form_and_the_chord_reaches_it() {
    the_chord_starts_a_terminal(Gnoblin::start(Start::Compositor, &shells().plus, &[], &[]));
}

#[test]
fn a_python_shell_claims_the_plus_form_and_the_chord_reaches_it() {
    let directory = tempfile::tempdir().expect("a directory for the shell");
    let shell = directory.path().join("shell.py");
    std::fs::write(
        &shell,
        "from gnoblin import gnoblin\n\n\n@gnoblin.bind(\"Super+Return\")\ndef terminal():\n    gnoblin.spawn(\"foot\")\n\n\n\
         gnoblin.log(\"python shell claimed Super+Return\", \"info\")\n",
    )
    .expect("the shell");
    the_chord_starts_a_terminal(start_script(Start::Compositor, &shell, &[]));
}

#[test]
fn a_typescript_shell_claims_the_plus_form_and_the_chord_reaches_it() {
    let directory = tempfile::tempdir().expect("a directory for the shell");
    let shell = directory.path().join("shell.ts");
    std::fs::write(
        &shell,
        "import { gnoblin } from \"gnoblin-core\";\n\n\
         gnoblin.input.primary.bind(\"Super+Return\", () => {\n    gnoblin.spawn(\"foot\");\n});\n\
         console.log(\"typescript shell claimed Super+Return\");\n",
    )
    .expect("the shell");
    the_chord_starts_a_terminal(start_script(Start::Compositor, &shell, &[]));
}
