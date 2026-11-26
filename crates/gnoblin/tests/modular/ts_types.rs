//! Example 5: a TypeScript shell of three files (`gnoblin ./shell.ts`): types erased, columns and rows, and a runtime error
//! reported at the line of the `.ts` file that threw.

use crate::support::Mapper;
use crate::support::native_shell::{KEY_T, KEY_X, Start, Typist, start_script, wait_for_window};
use crate::{copy_example, near, screen_of, serving};

#[test]
fn example_5_a_ts_shell_of_three_files_tiles_columns_then_rows_and_reports_an_error_at_the_ts_line() {
    let dir = copy_example("05-ts-types");
    let geometry = std::fs::read_to_string(dir.path().join("geometry.ts")).expect("geometry.ts");
    let throw_line = geometry
        .lines()
        .position(|line| line.contains("throw new Error(reason)"))
        .map(|index| index + 1)
        .expect("the example throws in geometry.ts");
    let mut session = start_script(Start::Compositor, &dir.path().join("shell.ts"), &[]);
    serving(&mut session);
    let (width, height) = screen_of(&session);
    let gap = 24.0;

    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("a", "A");
    mapper.map_window("b", "B");
    // Two windows in two columns of the screen, `Gap.Large` (24) apart.
    let half = (width - gap) / 2.0;
    wait_for_window(&mut session, "the first window takes the left column", "a", |window| {
        near(window.x, 0.0) && near(window.y, 0.0) && near(window.width, half) && near(window.height, height)
    });
    wait_for_window(&mut session, "the second takes the right column", "b", |window| {
        near(window.x, half + gap) && near(window.width, half) && near(window.height, height)
    });

    // `Super+t` switches to rows: the second window below the first.
    let mut typist = Typist::connect(&session.wayland());
    typist.super_plus(KEY_T);
    let rows = (height - gap) / 2.0;
    wait_for_window(&mut session, "the first window takes the top row", "a", |window| {
        near(window.x, 0.0) && near(window.y, 0.0) && near(window.width, width) && near(window.height, rows)
    });
    wait_for_window(&mut session, "the second takes the bottom row", "b", |window| {
        near(window.y, rows + gap) && near(window.width, width) && near(window.height, rows)
    });

    // `Super+x` throws in `geometry.ts`: the report names that file and the line of the `throw`, not a line of stripped JavaScript.
    typist.super_plus(KEY_X);
    session.wait_log("the error is reported at the line of geometry.ts", |line| {
        line.contains(&format!("geometry.ts:{throw_line}:"))
    });
    // The library reports the message on one line and the frames under it, so the line that names the file is a frame and the message is
    // in the report that line belongs to.
    assert!(
        session.log_text().contains("example 5 stops here"),
        "{}",
        session.log_text()
    );
    // The shell goes on after the error: a window that opens is placed by it, in the third row.
    mapper.map_window("c", "C");
    let third = (height - 2.0 * gap) / 3.0;
    wait_for_window(&mut session, "the shell still tiles after the error", "c", |window| {
        near(window.height, third) && near(window.width, width)
    });
}
