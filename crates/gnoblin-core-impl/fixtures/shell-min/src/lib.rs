//! The smallest shell: core only. It prints when the lock is requested and panics when the tree attaches, so the scenario
//! tests can show a panic in a callback is contained and the next event still arrives.

use std::io::Write as _;

use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;

fn say(line: &str) {
    let _ = writeln!(std::io::stdout().lock(), "{line}");
}

#[gnoblin::main]
fn main(root: &Gnoblin) -> gnoblin_core::host::Result<()> {
    let g = root.host();
    g.log("shell-min init (core only)");
    g.on_signal_named("lock", "requested", |_event| say("[shell-min] lock.requested"))?;
    g.on_signal_named("tree", "attached", |_event| panic!("boom from the shell callback"))?;
    #[cfg(feature = "next-minor")]
    say(&format!("[shell-min] uptime_ms={:?}", g.uptime_ms()));
    Ok(())
}
