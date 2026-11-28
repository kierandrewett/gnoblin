//! Example 2: a desktop shell on `gnoblin-std`.
//!
//! What it does, in twenty lines:
//!
//! - one floating workspace for every output (the desk's `per_output` workspace policy), made now for the outputs there are and again for each output
//!   that appears; the library places each window the moment it joins a workspace, so no two windows open on the same spot;
//! - a window that opens joins the workspace of the output that has the pointer (the library's own rule, which this shell does not
//!   repeat: it only says so in the log);
//! - a window takes the keyboard when it opens and when it is clicked, and a click raises it, with no line here: the library starts its focus
//!   policy when a shell makes its workspaces (`FocusPolicy::stop` ends it for a shell with a policy of its own);
//! - `<Super>Return` starts a terminal (`foot`);
//! - `<Super>Right` and `<Super>Left` walk the workspaces on the output that shows the focused window.
//!
//! What it is not: it draws nothing. `readelf -d` shows `libgnoblin-std.so.1` as the only library of ours the shell names, and
//! `libgnoblin-std.so.1` records `libgnoblin-core.so.1` in its turn, so the process maps the chain shell, std, core, one of each.
//!
//! Build it with `cargo build -p gnoblin-example-std-prelude` and run it with `gnoblin ./target/debug/libstd_prelude.so`.

use gnoblin_core::Gnoblin;
use gnoblin_std::prelude::*;
use gnoblin_std::{Desks, WorkspacePolicy, WorkspacePolicyMode};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.objects()?.std()?;
    Desks::get(&objects)?.set_workspaces(&WorkspacePolicy {
        mode: WorkspacePolicyMode::PerOutput,
        ..WorkspacePolicy::default()
    })?;

    let model = Gnoblin::get(&g.objects()?)?;
    let shell = g.clone();
    model
        .clients()?
        .on_added(move |client, _| {
            let workspace = client
                .workspace()
                .ok()
                .flatten()
                .and_then(|workspace| workspace.id().ok())
                .unwrap_or_else(|| "none".to_owned());
            shell.log(&format!(
                "client {} is on workspace {workspace}",
                client.id().unwrap_or_default()
            ));
        })?
        .keep();

    g.bind("<Super>Return", move || {
        if let Err(error) = g.spawn(&["foot".to_owned()], None) {
            g.log(&format!("cannot start a terminal: {error}"));
        }
    })?;

    for (chord, step) in [("<Super>Right", 1), ("<Super>Left", -1)] {
        g.bind(chord, move || {
            let walked = (|| -> gnoblin_std::Result<()> {
                let focused = Gnoblin::get(&g.objects()?)?.clients()?.focused()?;
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
