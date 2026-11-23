//! Example 38 (`examples/38-native-options`), run for real from the three languages: the shipped binary serving a shell that makes
//! std objects with the options record of their constructors.
//!
//! A constructor takes the properties its catalog names first, and then one optional record (`new Workspace("main", { name })` in
//! TypeScript, `Workspace("main", name=...)` in Python, `Workspace::new_with(&objects, "main", WorkspaceOptions { .. })` in Rust). Each
//! field sets the property of its name in the same create call. What can go wrong, and what this proves:
//!
//! - a field left out is sent anyway (`undefined`, `None`) and overwrites the library's default: a workspace made with one option reads
//!   the library's own gaps and the id as its name for the options it did not give;
//! - the options are set after the object is made, by calls of their own: a workspace made with every option reads them at once, and one
//!   the library refuses (a negative gap) fails the create call with the library's words;
//! - a class whose constructor does not take an option sets it after it makes the object (`OutputProfile` takes only the match): the id
//!   and the settings reach the object, and an empty id or a scale out of range fails the create call with the library's words and
//!   leaves no object in the list;
//! - the positional constructor of every shell so far changes: `Workspace("plain")` and `OutputProfile(match)` read what they always did.

use crate::support::native_shell::{Gnoblin, Start, start_script, std_shells};
use crate::{copy_example, serving};

use super::native_std::{assert_one_core_one_std, start_std};

/// What a language the example is written in is called in a failure.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Language {
    Rust,
    TypeScript,
    Python,
}

/// The session of the shell of the example in `language`.
fn session(language: Language) -> (Option<tempfile::TempDir>, Gnoblin) {
    match language {
        Language::Rust => (None, start_std(&std_shells().options)),
        Language::TypeScript | Language::Python => {
            let dir = copy_example("38-native-options");
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

/// The line the shell logs that starts with `ctoropts: ` and then `text`.
#[track_caller]
fn said(session: &mut Gnoblin, text: &str) -> String {
    let wanted = format!("ctoropts: {text}");
    session
        .wait_log(&format!("the shell said `{wanted}`"), |line| line.contains(&wanted))
        .text
}

#[track_caller]
fn the_options_are_set_as_the_objects_are_made(language: Language, session: &mut Gnoblin) {
    said(session, "running");

    // The constructor of every shell so far is unchanged: the library's name (an empty name reads as the id) and gaps.
    said(session, "plain workspace: id=plain name=\"plain\" gaps=0/0");
    // Every option reads back at once.
    said(session, "main workspace: id=main name=\"Main\" gaps=6/12 layout=true");
    // One option: the others keep the library's defaults, they are not overwritten by an omitted field.
    said(session, "half workspace: id=half name=\"Half\" gaps=0/0");

    // A value the library refuses fails the create call with the library's words, and the id is free for the next workspace.
    let gaps = said(session, "refused gaps:");
    assert!(
        gaps.contains("gaps.inner") && gaps.contains("at least 0"),
        "the library's words about the gap: {gaps}"
    );
    said(session, "bad workspace after the refusal: id=bad name=\"bad\" gaps=0/0");
    said(session, "workspaces hold main, plain");

    // A class whose constructor takes only the match: its id and settings are set as the object is made.
    said(session, "plain profile: id=HDMI-A-9 scale=unset");
    said(session, "profile with options: id=mine scale=2");
    let id = said(session, "refused id:");
    assert!(id.contains("must be a non-empty string"), "{id}");
    let scale = said(session, "refused scale:");
    assert!(scale.contains("scale must be a number from 0.5 to 4"), "{scale}");
    // Neither refused profile is in the list: only the two that were added.
    said(session, "profiles hold 2");
    said(session, "done");

    if language == Language::Rust {
        assert_one_core_one_std(session);
    } else {
        assert!(!session.log_text().contains("Traceback"), "{}", session.log_text());
    }
    assert!(session.exited().is_none(), "{}", session.log_text());
}

#[test]
fn example_38_a_std_shell_makes_objects_with_the_options_of_their_constructors() {
    let (_dir, mut session) = session(Language::Rust);
    the_options_are_set_as_the_objects_are_made(Language::Rust, &mut session);
}

#[test]
fn example_38_a_typescript_shell_makes_objects_with_the_options_of_their_constructors() {
    let (_dir, mut session) = session(Language::TypeScript);
    the_options_are_set_as_the_objects_are_made(Language::TypeScript, &mut session);
}

#[test]
fn example_38_a_python_shell_makes_objects_with_the_options_of_their_constructors() {
    let (_dir, mut session) = session(Language::Python);
    the_options_are_set_as_the_objects_are_made(Language::Python, &mut session);
}
