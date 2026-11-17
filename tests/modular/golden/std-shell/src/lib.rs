//! Golden shell of example 12, on std (`design/modular-framework.md` 5.8 and 12).
//!
//! Built once against ABI 1.0 of the core and of std, and stored as a binary (see the core-only one for the idea). It logs the
//! versions it was built for and the versions it finds installed, takes one floating workspace per output from std, and logs
//! where std places each client, so a test sees a later `libgnoblin-std` and a later core serve a shell that was not recompiled.

use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let (core_major, core_minor, core_patch) = gnoblin::installed_abi();
    let (std_major, std_minor, std_patch) = gnoblin_std::installed_abi();
    g.log(&format!(
        "golden-std-shell: built for core ABI {}.{} and std ABI {}.{}; installed core ABI {core_major}.{core_minor}.{core_patch}, installed std ABI {std_major}.{std_minor}.{std_patch}",
        gnoblin::ABI_MAJOR,
        gnoblin::ABI_MINOR,
        gnoblin_std::ABI_MAJOR,
        gnoblin_std::ABI_MINOR
    ));

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
            "golden-std-shell: client {} is on workspace {workspace}",
            client.window().id().0
        ));
    })?;

    g.log("golden-std-shell: running");
    Ok(())
}
