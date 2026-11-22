//! Example 20 (`examples/20-native-object`), run for real from the three languages: the shipped binary on a headless backend serving a
//! shell that makes an `EntryModel` through the classes written from the catalog of `libgnoblin-std`, edits it, reads it back, hears its
//! `changed` signal from the library's turn, and calls a static function of the focus policy, against the libraries the session stages.
//!
//! - The Rust shell (`examples/20-native-object`) uses the typed Rust API (`gnoblin_std::objects`).
//! - The TypeScript shell (`examples/20-native-object/shell.ts`) imports `gnoblin-std/generated/objects`, a module the JavaScript runtime
//!   library carries, and the Python shell (`examples/20-native-object/shell.py`) imports `gnoblin.objects.std`, which the Python runtime
//!   library carries.
//!   Neither reads a file of the repository's `data/` trees for it: the classes are generated from the catalog when the runtime is built.
//!
//! What each asserts is what the shell logs about what the libraries answered, and that a chord the shell claimed edits the entry again and
//! is heard again, so the signal reaches a closure the shell holds and the entry outlives the call that made it. All three shells spell the
//! lines alike, so one function reads them. The TypeScript shell is also checked against the declarations (`gnoblinctl check --types`).

use std::path::Path;

use crate::support::native_shell::{Gnoblin, KEY_E, Start, Typist, start_script, std_shells};
use crate::{copy_example, serving};

use super::native_std::{assert_one_core_one_std, start_std};

/// What the three shells log, in order, and the chord that edits the entry again.
#[track_caller]
fn assert_the_entry_is_made_edited_heard_and_typed_into(session: &mut Gnoblin) {
    // Made, read: text counts characters, and the caret starts at the end.
    session.wait_log("the shell made an entry and read its properties", |line| {
        line.contains("object: made an entry of 5 characters with the caret at 5")
    });

    // Edited by calls that answer at once: the text, the selection as a record and the selected text.
    let edited = session.wait_log("the shell read back what it edited", |line| {
        line.contains("object: text ")
    });
    assert!(
        edited.text.contains(
            r#"object: text "héllo, world" selected "héllo, world" as Some(TextRange { start: 0, end: 12 })"#
        ),
        "{}",
        edited.text
    );

    // A function of the focus policy: two records and an enum go in, a record comes out.
    let decision = session.wait_log("the shell asked the focus policy", |line| {
        line.contains("object: a pointer entering a window under sloppy focus")
    });
    assert!(decision.text.contains("focus true, raise false"), "{}", decision.text);

    // The library tells `changed` from its next turn, to a closure the shell gave it.
    session.wait_log("the first change is heard", |line| {
        line.contains("object: heard changed #1")
    });
    session.wait_log("the shell is running", |line| line.contains("object: running"));

    // The chord the shell claimed edits the entry the shell kept, and the change is heard once more.
    let mut typist = Typist::connect(&session.wayland());
    typist.super_plus(KEY_E);
    let typed = session.wait_log("the chord typed into the entry", |line| {
        line.contains("object: typed, text is ")
    });
    assert!(
        typed.text.contains(r#"object: typed, text is "héllo, world!""#),
        "{}",
        typed.text
    );
    let heard: u32 = typed
        .text
        .rsplit_once("heard ")
        .and_then(|(_, rest)| rest.split_whitespace().next())
        .and_then(|count| count.parse().ok())
        .unwrap_or_else(|| panic!("the line says how many changes were heard: {}", typed.text));
    let next = heard + 1;
    session.wait_log("the typed change is heard", |line| {
        line.contains(&format!("object: heard changed #{next}"))
    });

    assert_one_core_one_std(session);
    assert!(session.exited().is_none(), "{}", session.log_text());
}

#[test]
fn example_20_a_std_shell_makes_edits_and_hears_an_entry_model_through_the_typed_object_api() {
    let mut session = start_std(&std_shells().object);
    assert_the_entry_is_made_edited_heard_and_typed_into(&mut session);
}

/// A script shell of the example `name` in a session of its own.
fn script_session(name: &str, file: &str) -> (tempfile::TempDir, Gnoblin) {
    let dir = copy_example(name);
    let mut session = start_script(Start::Compositor, &dir.path().join(file), &[]);
    serving(&mut session);
    (dir, session)
}

#[test]
fn example_20_a_typescript_shell_makes_edits_and_hears_an_entry_model_through_the_generated_classes() {
    let (_dir, mut session) = script_session("20-native-object", "shell.ts");
    assert_the_entry_is_made_edited_heard_and_typed_into(&mut session);
    assert!(!session.log_text().contains("raised"), "{}", session.log_text());
}

#[test]
fn example_20_a_python_shell_makes_edits_and_hears_an_entry_model_through_the_generated_classes() {
    let (_dir, mut session) = script_session("20-native-object", "shell.py");
    assert_the_entry_is_made_edited_heard_and_typed_into(&mut session);
    assert!(!session.log_text().contains("Traceback"), "{}", session.log_text());
}

/// The result of `gnoblinctl check FILE --types --json`: its exit code and the document.
fn check_types(file: &Path) -> (u8, serde_json::Value) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = gnoblinctl::run(
        [
            "check".as_ref(),
            file.as_os_str(),
            "--types".as_ref(),
            "--json".as_ref(),
        ]
        .into_iter()
        .map(std::ffi::OsString::from),
        gnoblinctl::SocketConnection::at(std::path::PathBuf::from("/nonexistent/gnoblin.sock")),
        &mut out,
        &mut err,
    );
    let text = String::from_utf8_lossy(&out).into_owned();
    let document = serde_json::from_str(text.lines().last().unwrap_or("{}")).unwrap_or_else(|error| {
        panic!(
            "`check --json` printed no document ({error}): {text}{}",
            String::from_utf8_lossy(&err)
        )
    });
    (code, document)
}

/// A TypeScript compiler, when this machine has one: `$GNOBLIN_TSC`, or `tsc` on the path.
fn typescript_compiler() -> Option<std::path::PathBuf> {
    if let Some(explicit) = std::env::var_os("GNOBLIN_TSC").filter(|value| !value.is_empty()) {
        return Some(explicit.into());
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join("tsc"))
        .find(|candidate| candidate.is_file())
}

#[test]
fn example_20_the_typescript_shell_type_checks_against_the_generated_declarations_and_a_wrong_one_does_not() {
    let Some(_tsc) = typescript_compiler() else {
        eprintln!("no TypeScript compiler (GNOBLIN_TSC, or tsc on the path): the check is skipped");
        return;
    };
    let dir = copy_example("20-native-object");
    let shell = dir.path().join("shell.ts");
    let (code, document) = check_types(&shell);
    assert_eq!(code, 0, "{document}");
    assert_eq!(document["ok"], true, "{document}");
    assert_eq!(document["types"]["ran"], true, "the compiler ran: {document}");
    assert_eq!(document["types"]["passed"], true, "{document}");

    // The declarations are what is in force: a name the catalog does not have, a value of the wrong type and a field that is read-only
    // are each an error that names them.
    let wrong = std::fs::read_to_string(&shell)
        .expect("the example")
        .replace("entry.on(\"changed\"", "entry.on(\"nothing\"")
        .replace("{ mode: \"sloppy\" }", "{ mode: \"sideways\" }")
        .replace(
            "const entry = new EntryModel(\"héllo\");",
            "const entry = new EntryModel(\"héllo\");\nentry.length = 3;",
        );
    let broken = dir.path().join("broken.ts");
    std::fs::write(&broken, wrong).expect("a broken copy");
    let (code, document) = check_types(&broken);
    assert_ne!(code, 0, "{document}");
    let report = document["types"]["stdout"].as_str().unwrap_or_default();
    for wanted in ["\"nothing\"", "\"sideways\"", "read-only"] {
        assert!(
            report.contains(wanted),
            "the compiler does not complain of {wanted}:\n{document}"
        );
    }
}
