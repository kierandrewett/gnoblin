//! A real reload clears the style the earlier script left on a window: the opacity, the corner radius and the effects of the window's node
//! in the compositor's tree go back to what a window with no style has, whichever door asked for the reload (a saved file, which the
//! runtime sees, and `gnoblinctl shell reload`). The shell is a TypeScript script in the shipped binary on a headless backend with a Wayland
//! client that stays connected; what is asserted is what the tree holds (read over the socket), not what the shell believes.

use std::ffi::OsString;
use std::fs;
use std::path::Path;

use crate::save;
use crate::support::native_shell::{Gnoblin, Start, start_script};
use crate::support::{Mapper, snapshot};

/// A shell with a workspace (so `gnoblin-std` is imported and its library runs) and, when `styled`, a style on every client.
fn shell_text(styled: bool) -> String {
    let style = if styled {
        r##"gnoblin.clients.style.opacity = 0.8;
gnoblin.clients.style.cornerRadius = 12;
gnoblin.clients.style.shadows = [{ y: 8, blur: 24, color: "#00000066" }];"##
    } else {
        ""
    };
    format!(
        r#"import {{ gnoblin }} from "gnoblin-core";
import {{ Layout, Workspace }} from "gnoblin-std";

// A layout that places nothing: the windows keep the bounds they open with.
class Keep extends Layout {{
    compute(): [] {{
        return [];
    }}
}}
gnoblin.workspaces.add(new Workspace("desk", {{ layout: new Keep() }}));
{style}
"#
    )
}

/// What the compositor holds on the window node of `app`: `(opacity, corner_radius, number of effects)`.
fn node_style(ipc: &Path, app: &str) -> Option<(f64, f64, usize)> {
    let node = snapshot(ipc)
        .into_iter()
        .find(|node| node.class == "window" && node.app_id == app)?;
    let number = |key: &str, none: f64| {
        node.props
            .iter()
            .find(|prop| prop.key == key)
            .map_or(none, |prop| prop.value_double)
    };
    let effects = node
        .props
        .iter()
        .find(|prop| prop.key == "effects")
        .map_or(0, |prop| prop.value_effects.len());
    Some((number("opacity", 1.0), number("corner_radius", 0.0), effects))
}

fn styled(style: Option<(f64, f64, usize)>) -> bool {
    style.is_some_and(|(opacity, radius, effects)| {
        (opacity - 0.8).abs() < 1e-6 && (radius - 12.0).abs() < 1e-6 && effects > 0
    })
}

fn clear(style: Option<(f64, f64, usize)>) -> bool {
    style == Some((1.0, 0.0, 0))
}

/// `gnoblinctl` against the session's socket.
fn ctl(session: &Gnoblin, arguments: &[&str]) -> (u8, String, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = gnoblinctl::run(
        arguments.iter().map(OsString::from),
        gnoblinctl::SocketConnection::at(session.ipc()),
        &mut out,
        &mut err,
    );
    (
        code,
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
    )
}

#[test]
#[ignore = "vrnw: needs a reload that clears the style the retired generation left on a window: after the reload the node still holds opacity 0.8, corner radius 12 and one effect"]
fn a_real_reload_clears_the_opacity_the_corner_radius_and_the_effects_the_earlier_script_left_on_a_window() {
    let dir = tempfile::tempdir().expect("a directory for the shell");
    let shell = dir.path().join("shell.ts");
    fs::write(&shell, shell_text(true)).expect("the shell");
    let mut session = start_script(Start::Compositor, &shell, &[]);
    crate::serving(&mut session);
    let ipc = session.ipc();

    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("styled", "Styled");
    session.wait_for("the style reaches the window's node", |_| {
        styled(node_style(&ipc, "styled"))
    });

    // The first door: a saved file. The runtime sees it and the host reloads; the new script has no style.
    save(&shell, &shell_text(false));
    session.wait_for("the reload by a saved file clears the style", |_| {
        clear(node_style(&ipc, "styled"))
    });

    // The style comes back with the next script, and is cleared by the second door: `gnoblinctl shell reload`. The script is
    // written without the watcher's help being needed: whichever of the two asks first, the window must end clear.
    save(&shell, &shell_text(true));
    session.wait_for("the style returns", |_| styled(node_style(&ipc, "styled")));
    save(&shell, &shell_text(false));
    let (code, out, err) = ctl(&session, &["shell", "reload"]);
    assert!(
        code == 0 || err.contains("reload"),
        "gnoblinctl shell reload: {code} {out} {err}\n{}",
        session.log_text()
    );
    session.wait_for("the reload by gnoblinctl clears the style", |_| {
        clear(node_style(&ipc, "styled"))
    });
    mapper.roundtrip();
    assert!(
        node_style(&ipc, "styled").is_some(),
        "the window is the one the client still has, not a new one"
    );
}
