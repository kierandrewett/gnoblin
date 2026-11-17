//! Example 7: a custom input binding written entirely in a scripting language (`gnoblin ./shell.ts`, `gnoblin ./shell.py`). A
//! chord the script claims runs a closure of the script, which moves a real window; nothing of the example is Rust or C.

use crate::support::Mapper;
use crate::support::native_shell::{KEY_DOWN, KEY_RIGHT, Start, Typist, start_script, wait_for_window};
use crate::{copy_example, is_runtime_copy, near, serving};

/// Runs the example `shell` (`shell.ts` or `shell.py`), maps a window, and moves it with the two chords.
fn nudges(shell: &str, runtime_library: &str) {
    let dir = copy_example("07-script-binding");
    let mut session = start_script(Start::Compositor, &dir.path().join(shell), &[]);
    serving(&mut session);
    let files = session.mapped_files();
    assert!(
        files
            .iter()
            .any(|path| is_runtime_copy(path) && path.contains(runtime_library)),
        "the runtime of {shell} is mapped, from its own copy: {files:#?}"
    );

    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("w", "W");
    // The shell places nothing: the window is where the compositor put it.
    let start = wait_for_window(&mut session, "the window is in the tree", "w", |_| true);
    // The chords are claimed by the script, so the compositor sends them to it and not to the client.
    let mut typist = Typist::connect(&session.wayland());
    typist.super_plus(KEY_RIGHT);
    let right = wait_for_window(
        &mut session,
        "Super+Right moved the window 100 to the right",
        "w",
        |window| near(window.x, start.x + 100.0) && near(window.y, start.y),
    );
    typist.super_plus(KEY_DOWN);
    wait_for_window(&mut session, "Super+Down moved it 100 down", "w", |window| {
        near(window.x, right.x) && near(window.y, start.y + 100.0)
    });
    // It is the same window all along.
    assert_eq!(start.id, right.id);
}

#[test]
fn example_7_a_ts_binding_class_moves_a_real_window_when_its_chord_is_pressed() {
    nudges("shell.ts", "runtime-js");
}

#[test]
fn example_7_a_python_binding_function_moves_a_real_window_when_its_chord_is_pressed_with_libpython_mapped() {
    // The Python runtime links the system libpython, mapped for the life of the process once the runtime loads.
    let dir = copy_example("07-script-binding");
    let mut session = start_script(Start::Compositor, &dir.path().join("shell.py"), &[]);
    serving(&mut session);
    let files = session.mapped_files();
    assert!(
        files.iter().any(|path| path.contains("libpython3")),
        "libpython is mapped by a Python shell: {files:#?}"
    );
    drop(session);
    nudges("shell.py", "runtime-python");
}
