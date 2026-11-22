//! Decision D11, in a real session: the compositor's built-in window motion (the 200 ms fade a window has when it closes) is the std
//! library's policy, not core's. A shell that never loads std (core only) controls its own motion, so its windows land and leave at once;
//! a shell that loads std gets the desktop's look, because std sets the motion as it opens.
//!
//! What is read is the compositor's tree over the socket: a closing window keeps its node while it fades, and loses it in the frame
//! after the close when nothing fades it. Both sessions run the same compositor, the same stock library directory and the same client; the
//! only difference is whether the shell imports `gnoblin-std`.

use std::fs;
use std::time::{Duration, Instant};

use crate::serving;
use crate::support::native_shell::{Start, start_script, wait_for_window};
use crate::support::{Mapper, scene};

/// The built-in fade is 200 ms; a window that faded is still in the tree this long after its close, and
/// a window that did not is gone sooner. The boundary has the slack of a loaded machine on both sides.
const BOUNDARY: Duration = Duration::from_millis(120);

/// How long the closed window `a` stays in the tree of a session whose shell is `source` in `shell_name`, with the stock library of
/// `stock` (or the source tree's own).
fn how_long_a_closed_window_stays(shell_name: &str, source: &str, stock: Option<&str>) -> Duration {
    let dir = tempfile::tempdir().expect("a directory for the shell");
    let shell = dir.path().join(shell_name);
    fs::write(&shell, source).expect("the shell");
    let overrides: Vec<(&str, &str)> = stock.map(|stock| ("GNOBLIN_JS_STOCK_DIR", stock)).into_iter().collect();
    let mut session = start_script(Start::Compositor, &shell, &overrides);
    serving(&mut session);
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("a", "A");
    wait_for_window(&mut session, "the window is in the tree", "a", |_| true);
    // The window's own open fade runs out first (it is real time), so the only motion left is the close.
    std::thread::sleep(Duration::from_millis(600));
    let ipc = session.ipc();
    mapper.close_window(0);
    let closed = Instant::now();
    let deadline = closed + Duration::from_secs(10);
    loop {
        if !scene(&ipc).iter().any(|window| window.app_id == "a") {
            return closed.elapsed();
        }
        assert!(Instant::now() < deadline, "the closed window never left the tree");
        std::thread::sleep(Duration::from_millis(4));
    }
}

#[test]
fn a_core_only_shell_sees_no_built_in_fade_and_a_std_shell_still_fades_two_hundred_milliseconds() {
    let library = crate::no_std::core_only_library();
    let stock = library.path().display().to_string();
    // Core only: the shell imports `gnoblin-core` alone, so nothing sets the motion and the window goes in the frame after its close.
    let instant = how_long_a_closed_window_stays(
        "shell.js",
        "import { gnoblin } from \"gnoblin-core\";\nglobalThis.shell = gnoblin.clients.length;\n",
        Some(&stock),
    );
    assert!(
        instant < BOUNDARY,
        "a core-only session fades nothing, yet the closed window stayed {instant:?} in the tree"
    );
    // Std: importing the package opens the library, which sets the desktop's 200 ms motion, so the closed window fades out.
    let faded = how_long_a_closed_window_stays("shell.js", "import \"gnoblin-std\";\n", None);
    assert!(
        faded >= BOUNDARY,
        "a std session fades a closing window for 200 ms, yet the closed window left the tree after {faded:?}"
    );
    assert!(
        faded < Duration::from_secs(2),
        "the fade runs out: the closed window left the tree after {faded:?}"
    );
}
