//! Example 36 (`examples/36-native-clients`), run for real from the three languages: the shipped binary serving a shell that passes a window,
//! as the core's `Client`, to the classes of `libgnoblin-std` that take one: `Decorations.insetOf`, `WindowMenus.show` and the clients of a
//! workspace (`add`, `contains`).
//!
//! What it asserts is what the shell logs about what the libraries answered. A live window is accepted by each member and the library
//! resolves it to the right window: the menu it opens is the window's (`menu.client` is the client's id). The failure modes are the ones a
//! shell can reach:
//!
//! - the window's id as text where a `Client` is taken, and an object of another class of the core (the root object), are refused by
//!   name in the two languages that can write them; a Rust shell cannot write either, because the typed API takes `&Client`;
//! - a window that has closed is still a `Client` whose last values can be read (the title in the line), and the library says it is gone
//!   (`E_NO_SUCH_NODE`, `Client#<id>.<member>: the client is gone`), the words the core uses for a write to the same window.
//!
//! A handle that was released is refused as stale; no language can pass one (a script's wrapper releases its handle when it is collected,
//! and a Rust `Client` releases it when it is dropped), so that case is proved over the object protocol itself, in
//! `crates/gnoblin-std-impl/tests/client_handles.rs`.

use crate::support::Mapper;
use crate::support::native_shell::{Gnoblin, Start, start_script, std_shells};
use crate::{copy_example, serving};

use super::native_std::{assert_one_core_one_std, start_std};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Language {
    Rust,
    TypeScript,
    Python,
}

fn session(language: Language) -> (Option<tempfile::TempDir>, Gnoblin) {
    match language {
        Language::Rust => (None, start_std(&std_shells().clients)),
        Language::TypeScript | Language::Python => {
            let dir = copy_example("36-native-clients");
            let file = if language == Language::TypeScript {
                "shell.ts"
            } else {
                "shell.py"
            };
            let mut session = start_script(Start::Compositor, &dir.path().join(file), &[]);
            serving(&mut session);
            (Some(dir), session)
        }
    }
}

/// The line the shell logged that holds `needle`, once it has.
#[track_caller]
fn logged(session: &mut Gnoblin, needle: &str) -> String {
    session.wait_log(needle, |line| line.contains(needle)).text
}

#[track_caller]
fn the_shell_passes_a_client(language: Language, session: &mut Gnoblin) {
    session.wait_log("the shell is running", |line| line.contains("clients: running"));
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("foot", "Terminal");

    // Each member takes the window the core made, and the library resolves it to the right one.
    logged(session, "clients: Terminal: the frame takes 0 0 0 0");
    logged(session, "clients: Terminal: it is in main: true");
    // The workspace told the shell of the window with the core's `Client`: the very object the core's own collection gives.
    logged(session, "clients: Terminal: the library's client is the core's: true");
    logged(session, "clients: Terminal: the menu is for the window: true");
    // The members the library adds to the core's `Client` and `Output`: the window's workspace and a move away and back, which a move to
    // the workspace it is in already refuses; the window's own style and its warnings; the keyboard move, refused before `Repositioning`
    // starts; and the workspace the output shows, which `show` changes.
    logged(session, "clients: Terminal: its workspace is main: true");
    logged(session, "clients: Terminal: moved to side: true, now in side: true");
    logged(session, "clients: Terminal: moved back: true, again refused: true");
    logged(session, "clients: Terminal: its own style holds 0.8: true");
    logged(session, "clients: Terminal: its style warnings are a list: true");
    logged(
        session,
        "clients: Terminal: keyboard move before Repositioning starts: false",
    );
    logged(session, "clients: Terminal: the output shows main: true");
    logged(session, "clients: Terminal: the output showed side: true");

    match language {
        Language::Rust => {
            let log = session.log_text();
            assert!(
                !log.contains("an id is refused") && !log.contains("another class is refused"),
                "the Rust types take a `Client` and nothing else, so there is nothing to refuse at run time: {log}"
            );
        }
        _ => {
            // An id where a `Client` is taken, and an object of another class: refused by the SDK, by name, before the library sees them.
            let id = logged(session, "clients: Terminal: an id is refused: ");
            let (error, text) = if language == Language::Python {
                ("TypeError", "got str")
            } else {
                ("TypeError", "got string")
            };
            assert!(
                id.contains(error) && id.contains("expected a Client") && id.contains(text),
                "an id is not a Client: {id}"
            );
            let other = logged(session, "clients: Terminal: another class is refused: ");
            assert!(
                other.contains(error) && other.contains("expected a Client") && other.contains("got Gnoblin"),
                "the root object is not a Client: {other}"
            );
        }
    }

    // The window closes. Its `Client` is still readable (the title is in the line), and the library says the window is gone.
    mapper.close_window(0);
    let gone = logged(session, "clients: Terminal: a closed window is refused: ");
    assert!(
        gone.contains("Client#") && gone.contains("the client is gone"),
        "the words the core uses for a window that is gone: {gone}"
    );
    if language == Language::Python {
        assert!(
            gone.contains("NodeGoneError"),
            "the window is gone, which Python spells NodeGoneError: {gone}"
        );
    }
    assert!(
        !session.log_text().contains("a closed window was answered"),
        "a closed window is never answered: {}",
        session.log_text()
    );
    assert_one_core_one_std(session);
    assert!(session.exited().is_none(), "{}", session.log_text());
}

#[test]
fn example_36_a_core_shell_passes_a_client_to_the_std_classes() {
    let (_dir, mut session) = session(Language::Rust);
    the_shell_passes_a_client(Language::Rust, &mut session);
}

#[test]
fn example_36_a_typescript_shell_passes_a_client_to_the_std_classes() {
    let (_dir, mut session) = session(Language::TypeScript);
    the_shell_passes_a_client(Language::TypeScript, &mut session);
}

#[test]
fn example_36_a_python_shell_passes_a_client_to_the_std_classes() {
    let (_dir, mut session) = session(Language::Python);
    the_shell_passes_a_client(Language::Python, &mut session);
    assert!(!session.log_text().contains("Traceback"), "{}", session.log_text());
}
