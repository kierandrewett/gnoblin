//! Example 20 of `design/native-std-port.md`, run for real: the shipped binary on a headless backend serving a native shell that makes an
//! `EntryModel` through `gnoblin_std::objects` (the typed Rust API of the object protocol), edits it, reads it back, hears its `changed`
//! signal from the library's turn, and calls a static function of the focus policy, against the libraries the session stages.
//!
//! What it asserts is what the shell logs about what the libraries answered, and that a chord the shell claimed edits the entry again
//! and is heard again, so the signal reaches a closure the shell holds and the entry outlives the call that made it.

use crate::support::native_shell::{KEY_E, Typist, std_shells};

use super::native_std::{assert_one_core_one_std, start_std};

#[test]
fn example_20_a_std_shell_makes_edits_and_hears_an_entry_model_through_the_typed_object_api() {
    let mut session = start_std(&std_shells().object);

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

    assert_one_core_one_std(&session);
    assert!(session.exited().is_none(), "{}", session.log_text());
}
