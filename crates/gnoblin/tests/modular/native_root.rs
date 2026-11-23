//! Example 35 (`examples/35-native-root`), run for real from the three languages: the shipped binary serving a shell that uses the members
//! of the `gnoblin` root object of the core: the timers, the build identity, `find`, `motion` and the compositor's settings.
//!
//! What it asserts is what the shell logs about what the library answered, and in a session of the user's own settings what the shell reads
//! of them: `motion(200)` is 200 while the seat does not ask for reduced motion and 0 in a session whose accepted document asks for it
//! (`appearance.reduced-motion`, applied when the compositor starts and not only when a document is committed; a TypeScript shell commits a
//! document of its own, so there a commit that asks for it does the same), and `compositor.userShaders` is true until the setting is
//! switched off. A TypeScript shell that writes `compositor.userShaders` has the
//! runtime commit it, and the next login reads it. The failure modes are the ones a shell can reach: a timer that is cancelled never runs, a
//! kind the core does not list is refused with the kinds that work, a filter with a field a window lacks is refused with the fields that
//! exist, and a filter narrows the list. A Rust shell names the kind as a type, so a kind the core does not list is a build error there and
//! the shell logs the kinds it can ask for.

use std::path::Path;

use serde_json::json;

use crate::support::native_shell::{Gnoblin, Home, Launch, Start, shells, start_script_in};
use crate::support::{Mapper, snapshot};
use crate::{copy_example, serving};

use super::shader_filter::{accepted_document, home_with_document};

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

/// A home whose last accepted document switches the shaders off.
fn home_with_settings() -> Home {
    home_with_document(json!({"compositor": {"user-shaders": false}}), "").0
}

/// A home whose last accepted document asks for reduced motion (`appearance.reduced-motion`). A Rust and a Python shell commit no document, so
/// the compositor applies it from the document it started with and the seat asks for it before any window exists.
fn home_with_reduced_motion() -> Home {
    home_with_document(json!({"appearance": {"reduced-motion": true}}), "").0
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

#[test]
fn example_35_a_core_shell_sees_reduced_motion_the_seat_asks_for() {
    let (_dir, mut session) = session(Language::Rust, home_with_reduced_motion());
    the_root_answers(Language::Rust, &mut session, "0", "true");
}

/// A TypeScript shell owns the whole document it commits, built from what the script declares (no object of the shell carries `appearance`),
/// and commits it on every load, so the `appearance` of the document the compositor started with does not outlast that commit. Reduced motion
/// reaches such a session as a commit: here the owner's connection commits a document that asks for it once the shell has committed its own.
#[test]
fn example_35_a_typescript_shell_sees_reduced_motion_after_a_document_that_asks_for_it_is_committed() {
    let (_dir, mut session) = session(Language::TypeScript, Home::new());
    session.wait_log("the shell committed its own document", |line| {
        line.contains("the shell committed its configuration settings")
    });
    let ipc = session.ipc();
    let (major, minor, patch) = gnoblin_proto::IPC_VERSION;
    let mut owner = gnoblin_client::Client::connect(
        gnoblin_client::unix::UnixTransport::connect(&ipc).expect("the session listens"),
        gnoblin_proto::Version::new(major, minor, patch),
    )
    .expect("the handshake");
    owner
        .commit_config(br#"{"appearance":{"reduced-motion":true}}"#)
        .expect("the owner commits the document");
    session.wait_for("the seat asks for reduced motion", |_| {
        seat_asks_for_reduced_motion(&ipc)
    });
    the_root_answers(Language::TypeScript, &mut session, "0", "true");
}

/// Whether the main seat in the compositor's tree says the user asked for reduced motion.
fn seat_asks_for_reduced_motion(ipc: &Path) -> bool {
    snapshot(ipc)
        .iter()
        .find(|node| node.class == "seat")
        .is_some_and(|seat| {
            seat.props
                .iter()
                .any(|prop| prop.key == "reduced_motion" && prop.value_bool)
        })
}

#[test]
fn example_35_a_python_shell_sees_reduced_motion_the_seat_asks_for() {
    let (_dir, mut session) = session(Language::Python, home_with_reduced_motion());
    the_root_answers(Language::Python, &mut session, "0", "true");
    assert!(!session.log_text().contains("Traceback"), "{}", session.log_text());
}

/// The write path of `gnoblin.compositor.userShaders` through the core's own objects: the shell reads the setting, writes it, and reads what it
/// wrote while the compositor still holds the value it started with; the runtime commits it with the shell's document, the compositor keeps it as
/// the last accepted document, and the next login reads it from the seat.
#[test]
fn a_typescript_shell_writes_compositor_user_shaders_and_the_next_login_reads_it() {
    let dir = tempfile::tempdir().expect("a directory for the shell");
    let shell: &Path = &dir.path().join("shell.ts");
    std::fs::write(
        shell,
        "import { gnoblin } from \"gnoblin-core/generated/objects\";\n\
         console.log(`shaders: before = ${gnoblin.compositor.userShaders}`);\n\
         gnoblin.compositor.userShaders = false;\n\
         console.log(`shaders: after = ${gnoblin.compositor.userShaders}`);\n",
    )
    .expect("the shell");
    let mut first = start_script_in(Home::new(), None, Start::Compositor, shell, &[]);
    serving(&mut first);
    first.wait_log("the shell read the default", |line| {
        line.contains("shaders: before = true")
    });
    first.wait_log("the shell read what it wrote, before the next login", |line| {
        line.contains("shaders: after = false")
    });
    let ipc = first.ipc();
    first.wait_for("the first login commits compositor.user-shaders = false", |_| {
        accepted_document(&ipc).is_some_and(|document| document["compositor"]["user-shaders"] == false)
    });

    let mut second = start_script_in(first.into_home(), None, Start::Compositor, shell, &[]);
    serving(&mut second);
    second.wait_log("the next login starts with it switched off", |line| {
        line.contains("shaders: before = false")
    });
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
