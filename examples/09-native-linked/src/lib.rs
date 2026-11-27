//! Example 9: a native shell dynamically linked to the installed libraries.
//!
//! A shell is a shared object that links `libgnoblin-std.so.1` (which links `libgnoblin-core.so.1`) and carries no copy of
//! either. This one makes that visible: when it starts it logs
//!
//! - the ABI it was built for and the ABI of the library that is installed, for the core and for std, so an upgraded library
//!   shows in the log without the shell being rebuilt (see example 12);
//! - every `libgnoblin*` file its process maps, read from `/proc/self/maps`: one core, one std, and the shell's own copy.
//!
//! It then does what a small std shell does: one floating workspace per output, and a line in the log for each window the library
//! puts on a workspace.
//!
//! Check the links without running anything:
//!
//! ```text
//! cargo build -p gnoblin-example-native-linked
//! readelf -d target/debug/libnative_linked.so | grep NEEDED     # libgnoblin-std.so.1, libgnoblin-core.so.1
//! nm -D --defined-only target/debug/libnative_linked.so | grep gnoblin   # gnoshell_entry_v1 only
//! ```
//!
//! and run it against a prefix `cargo xtask dist install --prefix /tmp/gnoblin-prefix` made:
//!
//! ```text
//! /tmp/gnoblin-prefix/bin/gnoblin ./target/debug/libnative_linked.so
//! ```
//!
//! The launcher finds the core through its run path. A std shell carries no run path (it is found by its package, as an install
//! under `/usr` is), so for a private prefix put its `lib` directory on `LD_LIBRARY_PATH`:
//! `LD_LIBRARY_PATH=/tmp/gnoblin-prefix/lib /tmp/gnoblin-prefix/bin/gnoblin ./target/debug/libnative_linked.so`.

use std::collections::BTreeSet;
use std::fs;

use gnoblin_core::objects::ModelRoot;
use gnoblin_std::objects::{Desks, WorkspacePolicy, WorkspacePolicyMode};
use gnoblin_std::prelude::*;

/// The `libgnoblin*` files and the shell's own file this process maps, by path, read from `/proc/self/maps`.
fn mapped_libraries() -> BTreeSet<String> {
    let maps = fs::read_to_string("/proc/self/maps").unwrap_or_default();
    maps.lines()
        .filter_map(|line| line.split_whitespace().nth(5))
        .filter(|path| {
            let name = path.rsplit('/').next().unwrap_or(path);
            name.starts_with("libgnoblin") || name.contains("native_linked")
        })
        .map(str::to_owned)
        .collect()
}

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let (core_major, core_minor, core_patch) = gnoblin::installed_abi();
    let (std_major, std_minor, std_patch) = gnoblin_std::installed_abi();
    g.log(&format!(
        "linked: built for core ABI {}.{} and std ABI {}.{}; installed core ABI {core_major}.{core_minor}.{core_patch}, installed std ABI {std_major}.{std_minor}.{std_patch}",
        gnoblin::ABI_MAJOR,
        gnoblin::ABI_MINOR,
        gnoblin_std::ABI_MAJOR,
        gnoblin_std::ABI_MINOR
    ));
    for path in mapped_libraries() {
        g.log(&format!("linked: maps {path}"));
    }

    let objects = g.std()?.objects()?;
    Desks::get(&objects)?.set_workspaces(&WorkspacePolicy {
        mode: WorkspacePolicyMode::PerOutput,
        ..WorkspacePolicy::default()
    })?;
    let shell = g.clone();
    ModelRoot::get(&g.objects()?)?
        .clients()?
        .on_added(move |client, _| {
            let workspace = client
                .workspace()
                .ok()
                .flatten()
                .and_then(|workspace| workspace.id().ok())
                .unwrap_or_else(|| "none".to_owned());
            shell.log(&format!(
                "linked: client {} is on workspace {workspace}",
                client.id().unwrap_or_default()
            ));
        })?
        .keep();

    g.log("linked: running");
    Ok(())
}
