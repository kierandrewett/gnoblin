//! Example 21 of `design/native-std-port.md`, run for real: the shipped binary on a headless backend serving a native shell that uses the
//! typed object model of the core (`gnoblin_core::objects`), with a real Wayland client that maps and closes windows.
//!
//! What it asserts is what the shell logs about what the library told it: the windows there at the start, a selection (`where`) that
//! a window joins when its application id matches and not otherwise, and the close
//! of a window heard through the `gnoblin` object with its last title still readable.

use crate::support::Mapper;
use crate::support::native_shell::{Gnoblin, Start, shells};

#[test]
fn example_21_a_core_shell_follows_the_windows_through_the_object_model() {
    let mut session = Gnoblin::start(Start::Compositor, &shells().model, &[], &[]);
    session.wait_serving();
    session.wait_log("the shell is running", |line| line.contains("model: running"));

    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("other", "Elsewhere");
    mapper.map_window("foot", "Terminal");
    session.wait_log("the terminal joined the selection", |line| {
        line.contains("model: a terminal joined")
    });
    assert!(
        !session.log_text().contains("a terminal joined: Elsewhere"),
        "a window of another application is not in the selection: {}",
        session.log_text()
    );

    mapper.close_window(1);
    session.wait_log("the close was heard through the gnoblin object", |line| {
        line.contains("closed, last titled \"Terminal\"")
    });
    assert!(session.exited().is_none(), "{}", session.log_text());
}
