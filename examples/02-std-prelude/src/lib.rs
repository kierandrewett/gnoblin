//! Example 2: a desktop shell on `gnoblin-std`.
//!
//! What it does, in twenty lines:
//!
//! - one floating workspace for every output (`workspace_per_output`), made now for the outputs there are and again for each output
//!   that appears; the library places each window the moment it joins a workspace, so no two windows open on the same spot;
//! - a window that opens joins the workspace of the output that has the pointer (the library's own rule, which this shell does not
//!   repeat: it only says so in the log);
//! - `Super+Return` starts a terminal (`foot`);
//! - `Super+Right` and `Super+Left` walk the workspaces on the output that shows the focused window.
//!
//! What it is not: it draws nothing. `readelf -d` shows `libgnoblin-std.so.1` as the only library of ours the shell names, and
//! `libgnoblin-std.so.1` records `libgnoblin-core.so.1` in its turn, so the process maps the chain shell, std, core, one of each.
//!
//! Build it with `cargo build -p gnoblin-example-std-prelude` and run it with `gnoblin ./target/debug/libstd_prelude.so`.

use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let std = g.std()?;
    std.workspace_per_output()?;

    std.clients()?.watch(|client| {
        let workspace = client
            .workspace()
            .ok()
            .flatten()
            .and_then(|workspace| workspace.id().ok())
            .unwrap_or_else(|| "none".to_owned());
        client.window().gnoblin().log(&format!(
            "client {} is on workspace {workspace}",
            client.window().id().0
        ));
    })?;

    g.bind("Super+Return", move || {
        if let Err(error) = g.spawn("foot") {
            g.log(&format!("cannot start a terminal: {error}"));
        }
    })?;

    for (chord, step) in [("Super+Right", 1), ("Super+Left", -1)] {
        g.bind(chord, move || {
            let walked = (|| -> gnoblin_std::Result<()> {
                let focused = g.clients()?.focused()?;
                if let Some(workspace) = focused.and_then(|client| client.workspace().ok().flatten()) {
                    workspace.walk(step)?;
                }
                Ok(())
            })();
            if let Err(error) = walked {
                g.log(&format!("cannot walk the workspaces: {error}"));
            }
        })?;
    }

    g.log("std prelude shell is running");
    Ok(())
}
