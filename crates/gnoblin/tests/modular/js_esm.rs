//! Example 4: a JavaScript ES-module shell (`gnoblin ./shell.js`) with `gnoblin-core`, `gnoblin-std` and a relative import.

use crate::support::Mapper;
use crate::support::native_shell::{Gnoblin, KEY_ENTER, Start, Typist, start_script, wait_for_window};
use crate::{copy_example, is_runtime_copy, screen_of, serving};

#[test]
fn example_4_a_js_shell_of_es_modules_tiles_real_windows_binds_a_chord_and_maps_the_runtime_and_the_std_library() {
    let dir = copy_example("04-js-esm");
    let mut session: Gnoblin = start_script(Start::Compositor, &dir.path().join("shell.js"), &[]);
    serving(&mut session);

    // One mapping of the core, the runtime's private copy as the one mapping beyond it, and the std library once: the shell imports
    // `gnoblin-std`, whose stock layouts are the library's (`builtin_*`), which the runtime opens the first time one is made.
    let files = session.mapped_files();
    let runtimes: Vec<&String> = files.iter().filter(|path| is_runtime_copy(path)).collect();
    assert_eq!(runtimes.len(), 1, "one mapping of the JavaScript runtime in {files:#?}");
    let std: Vec<&String> = files
        .iter()
        .filter(|path| path.contains("gnoblin-std") || path.contains("gnoblin_std"))
        .collect();
    assert_eq!(
        std.len(),
        1,
        "the std library is mapped once by a shell that imports it: {files:#?}"
    );
    assert!(
        files.iter().all(|path| !path.contains("libpython")),
        "a JavaScript shell maps no Python: {files:#?}"
    );

    // `import { Workspace, MasterStackLayout } from "gnoblin-std"`: the windows are tiled master and stack, by the shell and not by
    // the compositor's own window management.
    let (width, height) = screen_of(&session);
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("first", "First");
    let first = wait_for_window(&mut session, "one window fills the work area", "first", |window| {
        window.width > width * 0.9 && window.height > height * 0.9
    });
    mapper.map_window("second", "Second");
    let master = wait_for_window(&mut session, "the master keeps the left side", "first", |window| {
        window.width < width * 0.6 && window.width > width * 0.3
    });
    let stack = wait_for_window(
        &mut session,
        "the other window takes the right side",
        "second",
        |window| window.x > master.x + master.width * 0.5 && window.width < width * 0.6,
    );
    assert_eq!(first.id, master.id, "the first window is the same window, tiled again");
    assert!(
        (stack.height - master.height).abs() < 1.0,
        "a master and one stacked window are as tall as each other: {master:?} {stack:?}"
    );

    // `bindLaunchers(gnoblin.input.primary)` of the relative module: `Super+Return` starts a terminal.
    let mut typist = Typist::connect(&session.wayland());
    assert!(session.noted("terminal").is_none());
    typist.super_plus(KEY_ENTER);
    session.wait_noted("terminal");
}
