//! A shell for the reload scenarios. Every generation registers one callback that prints its label when the lock is
//! requested. The generation labelled `bad` registers first and then fails its `init`, which is how the tests prove a
//! half-initialised generation is cleaned up and the running one keeps serving.

use std::io::Write as _;

use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;

const LABEL: &str = env!("FIXTURE_LABEL");

fn say(line: &str) {
    let _ = writeln!(std::io::stdout().lock(), "{line}");
}

#[gnoblin::main]
fn main(g: &Gnoblin) -> Result<(), String> {
    g.log(&format!("shell-hot {LABEL} init"));
    g.on_signal_named("lock", "requested", |_event| {
        say(&format!("[shell-hot {LABEL}] lock.requested"))
    })
    .map_err(|error| error.to_string())?;
    if LABEL == "bad" {
        return Err("simulated init failure (bad shell)".to_owned());
    }
    Ok(())
}
