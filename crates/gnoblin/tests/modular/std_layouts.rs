//! Example 3 (std layouts) from a script, run for real: `gnoblin ./layouts.py` and `gnoblin ./layouts.ts` in a headless session.
//! The examples spell their chords `Super+1` to `Super+6`; the compositor's chord parser reads that spelling, so a key press of
//! the user reaches the binding and the library's layout moves real windows. What is asserted is the compositor's tree: monocle
//! gives every window one rectangle, and the master and stack, the grid and the monocle are different arrangements of the same windows.

use std::collections::BTreeMap;

use crate::support::native_shell::{KEY_1, KEY_3, KEY_5, Start, Typist, start_script, wait_for_window};
use crate::support::{Mapper, scene};
use crate::{copy_example, near, serving};

type Rects = BTreeMap<String, (f64, f64, f64, f64)>;

fn rects(session: &crate::support::native_shell::Gnoblin) -> Rects {
    scene(&session.ipc())
        .into_iter()
        .map(|window| (window.app_id, (window.x, window.y, window.width, window.height)))
        .collect()
}

fn same(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)) -> bool {
    near(a.0, b.0) && near(a.1, b.1) && near(a.2, b.2) && near(a.3, b.3)
}

/// Whether every window of `rects` has the one rectangle.
fn all_the_same(rects: &Rects) -> bool {
    let mut values = rects.values().copied();
    let Some(first) = values.next() else { return false };
    values.all(|rect| same(first, rect))
}

fn layouts(example: &str, shell: &str) {
    let dir = copy_example(example);
    let mut session = start_script(Start::Compositor, &dir.path().join(shell), &[]);
    serving(&mut session);
    let mut mapper = Mapper::connect(&session.wayland());
    for name in ["a", "b", "c"] {
        mapper.map_window(name, name);
    }
    for name in ["a", "b", "c"] {
        wait_for_window(&mut session, "the window is tiled", name, |_| true);
    }
    // Settle on the master and stack the workspace starts with: three windows of three different rectangles.
    let mut typist = Typist::connect(&session.wayland());
    typist.super_plus(KEY_1);
    let mut stacked = Rects::new();
    session.wait_for("Super+1 tiles the three windows as a master and stack", |session| {
        stacked = rects(session);
        stacked.len() == 3 && !all_the_same(&stacked)
    });

    // Super+5 is a monocle: every window has the one rectangle.
    typist.super_plus(KEY_5);
    session.wait_for("Super+5 gives every window one rectangle", |session| {
        let now = rects(session);
        now.len() == 3 && all_the_same(&now)
    });

    // Super+3 is a grid and Super+1 a master and stack again: tiled windows, and the monocle left.
    typist.super_plus(KEY_3);
    let mut grid = Rects::new();
    session.wait_for("Super+3 leaves the monocle for a grid", |session| {
        grid = rects(session);
        grid.len() == 3 && !all_the_same(&grid)
    });
    typist.super_plus(KEY_1);
    session.wait_for("Super+1 is the master and stack it was", |session| {
        let now = rects(session);
        now.len() == 3
            && now
                .iter()
                .all(|(name, rect)| stacked.get(name).is_some_and(|before| same(*before, *rect)))
    });
    assert!(
        grid.iter().any(|(name, rect)| !same(*rect, stacked[name])),
        "a grid and a master and stack are different arrangements: {grid:?} {stacked:?}"
    );
}

#[test]
fn example_3_the_python_layouts_shell_tiles_real_windows_for_its_super_n_chords() {
    layouts("03-python-std", "layouts.py");
}

#[test]
fn example_3_the_typescript_layouts_shell_tiles_real_windows_for_its_super_n_chords() {
    layouts("03-typescript-std", "layouts.ts");
}
