//! A shell for the reload scenarios. Every generation registers one callback that prints its label when the lock is
//! requested. The generation labelled `bad` registers first and then fails its `init`, which is how the tests prove a
//! half-initialised generation is cleaned up and the running one keeps serving; the one labelled `panic` registers first
//! and then panics in `init`.
//!
//! Two features build the other ways a replacement can be unusable: `wrong-abi` exports an entry descriptor built for another
//! ABI major, and `newer-symbol` calls a symbol the core does not export, as a shell built against a newer core would.

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

#[cfg(feature = "newer-symbol")]
#[allow(unsafe_code)]
unsafe extern "C" {
    /// A function of a later core: this core does not export it, so the library does not load.
    fn gnoblin_future_call() -> i32;
}

#[cfg_attr(not(feature = "wrong-abi"), gnoblin::main)]
#[cfg_attr(feature = "wrong-abi", allow(dead_code))]
fn main(g: &Gnoblin) -> Result<(), String> {
    g.log(&format!("shell-hot {LABEL} init"));
    g.on_signal_named("lock", "requested", |_event| {
        say(&format!("[shell-hot {LABEL}] lock.requested"))
    })
    .map_err(|error| error.to_string())?;
    if LABEL == "bad" {
        return Err("simulated init failure (bad shell)".to_owned());
    }
    if LABEL == "panic" {
        panic!("simulated panic in init");
    }
    #[cfg(feature = "newer-symbol")]
    {
        // SAFETY: never reached: the dynamic loader refuses the library for this undefined symbol before `init` runs.
        #[allow(unsafe_code)]
        let _ = unsafe { gnoblin_future_call() };
    }
    Ok(())
}

/// An entry descriptor built for ABI major 2, minor 3: what a shell written for a future core exports.
#[cfg(feature = "wrong-abi")]
mod foreign {
    use gnoblin_core::__private::{GnoblinShellEntry, GnoblinStr, entry};

    static ENTRY: GnoblinShellEntry = {
        let mut descriptor = entry(GnoblinStr::borrowed("shell-hot-wrong-abi"));
        descriptor.abi_major = 2;
        descriptor.abi_minor_built = 3;
        descriptor
    };

    #[allow(unsafe_code)]
    #[unsafe(no_mangle)]
    pub extern "C" fn gnoshell_entry_v1() -> *const GnoblinShellEntry {
        &ENTRY
    }
}
