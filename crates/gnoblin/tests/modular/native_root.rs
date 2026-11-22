//! Example 35 (`examples/35-native-root`), run for real from the three languages: the shipped binary serving a shell that uses the members
//! of the `gnoblin` root object of the core: the timers, the build identity, `find`, `motion` and the compositor's settings.
//!
//! What it asserts is what the shell logs about what the library answered, and in a session of the user's own settings what the shell reads
//! of them: `motion(200)` is 200 while the seat does not ask for reduced motion, and `compositor.userShaders` is true until the setting
//! is switched off. The failure modes are the ones a shell can reach: a timer that is cancelled never runs, a kind the core does not list is
//! refused with the kinds that work, a filter with a field a window lacks is refused with the fields that exist, and a filter narrows the
//! list. A Rust shell names the kind as a type, so a kind the core does not list is a build error there and the shell logs the kinds it can ask for.

use std::path::Path;

use serde_json::json;

use crate::support::Mapper;
use crate::support::native_shell::{Gnoblin, Home, Launch, Start, shells, start_script_in};
use crate::{copy_example, serving};

use super::shader_filter::home_with_document;

/// A session of the shell of the example in `language`, in `home`.
fn session(language: Language, home: Home) -> (Option<tempfile::TempDir>, Gnoblin) {
    match language {
        Language::Rust => {
            let launch = Launch::compositor(&[]).with_arg(&shells().root);
            let mut session = Gnoblin::launch(home, launch);
            session.wait_serving();
            (None, session)
        }
        Language::TypeScript | Language::Python => {
            let dir = copy_example("35-native-root");
            let file = if language == Language::TypeScript {
                "shell.ts"
            } else {
                "shell.py"
            };
            let mut session = start_script_in(home, None, Start::Compositor, &dir.path().join(file), &[]);
            serving(&mut session);
            (Some(dir), session)
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Language {
    Rust,
    TypeScript,
    Python,
}

/// The line the shell logs when the window `title` appeared, once it has.
#[track_caller]
fn created(session: &mut Gnoblin, title: &str) -> String {
    let marker = format!("root: created {title}:");
    session
        .wait_log("the window was reported", |line| line.contains(&marker))
        .text
}

/// Everything the example's shell does on its own, before and after windows appear, with the settings the session has.
#[track_caller]
fn the_root_answers(language: Language, session: &mut Gnoblin, motion: &str, shaders: &str) {
    session.wait_log("the shell is running", |line| line.contains("root: running"));
    let log = session.log_text();

    // The build identity is a method, and says which API the build speaks.
    assert!(
        log.lines()
            .any(|line| line.contains("root: gnoblin ") && line.contains(" speaks api ") && !line.ends_with("api ")),
        "the build identity: {log}"
    );
    // A timer set with `timers.after` runs after its time on `timers.now`, and one that was cancelled never runs.
    session.wait_log("the timer ran", |line| {
        line.contains("root: the timer fired after its time: true")
    });
    assert!(
        log.contains("root: a cancelled timer is pending: false"),
        "a cancelled timer is not pending: {log}"
    );
    assert!(
        !session.log_text().contains("a cancelled timer ran"),
        "a cancelled timer never runs: {}",
        session.log_text()
    );
    match language {
        Language::Rust => assert!(
            log.contains("root: find lists the kinds Window, Output"),
            "the kinds a Rust shell can find: {log}"
        ),
        _ => {
            // A kind the core does not list is refused, and the refusal names the kinds that work.
            let refused = session.log_text();
            let line = refused
                .lines()
                .find(|line| line.contains("root: an unknown kind is refused:"))
                .unwrap_or_else(|| panic!("an unknown kind is refused: {refused}"));
            assert!(
                line.contains("Workspace"),
                "the refusal names what was asked for: {line}"
            );
            assert!(
                line.contains("the kinds that work are Window, Output"),
                "and what works: {line}"
            );
            // A filter with a field a window lacks is refused too, with the fields a window can be filtered by.
            let line = refused
                .lines()
                .find(|line| line.contains("root: an unknown field is refused:"))
                .unwrap_or_else(|| panic!("an unknown field is refused: {refused}"));
            let (wrong, right) = if language == Language::Python {
                ("no_such_field", "app_id")
            } else {
                ("noSuchField", "appId")
            };
            assert!(
                line.contains(wrong) && line.contains("known:") && line.contains(right),
                "{line}"
            );
        }
    }

    // A window that no filter keeps, and one that the filters narrow to.
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("other", "Elsewhere");
    let first = created(session, "Elsewhere");
    assert!(
        first.contains("windows = 1, appId foot = 0, title starts with Term = 0, outputs listed = true"),
        "a filter that matches no window lists none: {first}"
    );
    mapper.map_window("foot", "Terminal");
    let second = created(session, "Terminal");
    assert!(
        second.contains("windows = 2, appId foot = 1, title starts with Term = 1, outputs listed = true"),
        "a table and a function narrow the list the same way: {second}"
    );
    for line in [&first, &second] {
        assert!(
            line.contains(&format!("motion(200) = {motion}, userShaders = {shaders}")),
            "the seat's reduced motion and the compositor's shader setting: {line}"
        );
    }
    assert!(session.exited().is_none(), "{}", session.log_text());
}

#[test]
fn example_35_a_core_shell_reads_the_root_members_with_the_defaults() {
    let (_dir, mut session) = session(Language::Rust, Home::new());
    the_root_answers(Language::Rust, &mut session, "200", "true");
}

#[test]
fn example_35_a_typescript_shell_reads_the_root_members_with_the_defaults() {
    let (_dir, mut session) = session(Language::TypeScript, Home::new());
    the_root_answers(Language::TypeScript, &mut session, "200", "true");
}

#[test]
fn example_35_a_python_shell_reads_the_root_members_with_the_defaults() {
    let (_dir, mut session) = session(Language::Python, Home::new());
    the_root_answers(Language::Python, &mut session, "200", "true");
    assert!(!session.log_text().contains("Traceback"), "{}", session.log_text());
}

/// A home whose last accepted document switches the shaders off. Reduced motion has no startup path in a session: the compositor applies it
/// when a document is committed, so `motion` is proved against the seat in the library's own tests (`crates/gnoblin-core-impl/tests/services.rs`).
fn home_with_settings() -> Home {
    home_with_document(json!({"compositor": {"user-shaders": false}}), "").0
}

#[test]
fn example_35_a_core_shell_sees_switched_off_shaders() {
    let (_dir, mut session) = session(Language::Rust, home_with_settings());
    the_root_answers(Language::Rust, &mut session, "200", "false");
}

#[test]
fn example_35_a_typescript_shell_sees_switched_off_shaders() {
    let (_dir, mut session) = session(Language::TypeScript, home_with_settings());
    the_root_answers(Language::TypeScript, &mut session, "200", "false");
}

#[test]
fn example_35_a_python_shell_sees_switched_off_shaders() {
    let (_dir, mut session) = session(Language::Python, home_with_settings());
    the_root_answers(Language::Python, &mut session, "200", "false");
    assert!(!session.log_text().contains("Traceback"), "{}", session.log_text());
}

/// A core-only shell never loads std, and the root it reads has no kind std adds: `find("Workspace")` is refused there with the kinds that work.
#[test]
fn a_shell_that_loads_no_std_cannot_find_a_workspace() {
    let dir = tempfile::tempdir().expect("a directory for the shell");
    let shell: &Path = &dir.path().join("shell.ts");
    std::fs::write(
        shell,
        "import { gnoblin } from \"gnoblin-core/generated/objects\";\n\
         try { gnoblin.find(\"Workspace\"); console.log(\"found: workspaces\"); }\n\
         catch (error) { console.log(`refused: ${(error as Error).message}`); }\n",
    )
    .expect("the shell");
    let mut session = start_script_in(Home::new(), None, Start::Compositor, shell, &[]);
    serving(&mut session);
    let line = session
        .wait_log("find answered", |line| {
            line.contains("refused: ") || line.contains("found: workspaces")
        })
        .text;
    assert!(
        line.contains("refused: ") && line.contains("Workspace") && line.contains("Window, Output"),
        "{line}"
    );
}
