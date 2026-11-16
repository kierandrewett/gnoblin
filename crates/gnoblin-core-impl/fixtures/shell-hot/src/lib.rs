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

/// Whether it is safe to unload this library: the answer the host's opt-in `try_unload` asks for. Only the build labelled
/// `safe` says yes; every other label answers no, which is how the tests show a module that does not prove itself is kept.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn gnoshell_unload_safe() -> u32 {
    u32::from(LABEL == "safe")
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
