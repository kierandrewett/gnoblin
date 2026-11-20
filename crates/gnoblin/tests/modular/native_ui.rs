//! Example 22 run for real: the shipped binary on a headless backend serving a native shell that makes a `Text` and an `Entry` of the
//! toolkit of `libgnoblin-std`, mounts them over the windows and gives the entry the keyboard.
//!
//! What it asserts is what the shell logs: the entry is a node the library made and holds the keyboard, keys the test's virtual
//! keyboard sends reach it (the compositor delivers them to the focused canvas, the library types them), and Return calls the shell's
//! closure with what was typed. One core and one std are mapped.

use crate::support::native_shell::{KEY_A, KEY_B, KEY_ENTER, Typist, std_shells};

use super::native_std::{assert_one_core_one_std, start_std};

#[test]
fn example_22_a_std_shell_types_into_a_toolkit_entry_and_hears_return() {
    let mut session = start_std(&std_shells().ui);
    session.wait_log("the shell made its entry", |line| {
        line.contains("ui: the entry is node") && line.contains("holds the keyboard")
    });
    let mut typist = Typist::connect(&session.wayland());
    typist.tap(KEY_A);
    typist.tap(KEY_B);
    typist.tap(KEY_ENTER);
    let heard = session.wait_log("Return is heard with what was typed", |line| {
        line.contains("ui: submitted")
    });
    assert!(heard.text.contains(r#"ui: submitted "ab""#), "{}", heard.text);
    assert_one_core_one_std(&session);
    assert!(session.exited().is_none(), "{}", session.log_text());
}
