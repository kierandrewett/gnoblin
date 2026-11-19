//! A running transition does not outlive its generation: the `tween` node it made in the compositor's tree goes when the shell reloads.
//!
//! A transition plays on a node the compositor holds (a `tween` node that its timed writes join, which carries its play state). The
//! library that runs the transition belongs to the generation that asked for it, so when a reload retires the generation the library destroys
//! the tween nodes of the transitions still running (`State::anim_release`). Proved against the shipped binary on a headless backend, with
//! a real Wayland window animated by a TypeScript shell for a minute, and the tree the compositor publishes read over its socket.

use std::fs;
use std::path::Path;

use crate::save;
use crate::support::native_shell::{Start, start_script, wait_for_window};
use crate::support::{Mapper, snapshot};

/// A shell that, a second after the first window it hears of is added (a transition started in `onAdded`, before the window is shown, was seen to leave no tween node in the tree), starts a transition of the window's opacity that lasts a minute. The `generation` tag
/// tells the two generations' logs apart.
fn shell_text(generation: &str, animate: bool) -> String {
    let body = if animate {
        "gnoblin.timers.after(1000, () => { gnoblin.requestTransition(() => { client.opacity = 0.5; }, { duration: 60000 }); });"
    } else {
        ""
    };
    format!(
        r#"import {{ CollectionListener, gnoblin }} from "gnoblin-core";
import type {{ Client }} from "gnoblin-core";
import "gnoblin-std";

// generation {generation}
class Animator extends CollectionListener<Client> {{
    onAdded(client: Client): void {{
        {body}
    }}
}}
gnoblin.clients.addListener(new Animator());
"#
    )
}

/// How many `tween` nodes the compositor's tree holds.
fn tweens(ipc: &Path) -> usize {
    snapshot(ipc).iter().filter(|node| node.class == "tween").count()
}

#[test]
fn a_running_transitions_tween_node_is_destroyed_when_its_generation_is_retired_by_a_reload() {
    let dir = tempfile::tempdir().expect("a directory for the shell");
    let shell = dir.path().join("shell.ts");
    fs::write(&shell, shell_text("first", true)).expect("the shell");
    let mut session = start_script(Start::Compositor, &shell, &[]);
    crate::serving(&mut session);

    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("a", "A");
    wait_for_window(&mut session, "the window", "a", |window| window.width > 0.0);
    let ipc = session.ipc();
    session.wait_for("the transition's tween node is in the tree", |_| tweens(&ipc) == 1);

    // The reload: a new generation that animates nothing. The old one is retired, and the transition it had running goes with it.
    let reloads = |session: &crate::support::native_shell::Gnoblin| {
        session
            .log()
            .iter()
            .filter(|line| line.text.contains("the shell reloaded"))
            .count()
    };
    let before = reloads(&session);
    save(&shell, &shell_text("second", false));
    session.wait_for("the shell reloads", |session| reloads(session) > before);
    session.wait_for("the retired generation's tween node is gone", |_| tweens(&ipc) == 0);
    mapper.roundtrip();
}
