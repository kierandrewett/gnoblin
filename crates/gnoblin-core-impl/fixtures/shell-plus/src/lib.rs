//! A shell that claims `Super+Return` in the plus form and starts a terminal on it.
//!
//! The SDK passes the chord to the compositor as it is written, so the session this shell runs in proves the compositor's chord
//! parser reads the plus form: if it did not, the claim would be refused and the terminal would never start. The test
//! (`crates/gnoblin/tests/native_shell_spelling.rs`) types the chord on a virtual keyboard.

use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    g.bind("Super+Return", move || {
        if let Err(error) = g.spawn("foot") {
            g.log(&format!("cannot start a terminal: {error}"));
        }
    })?;
    g.log("shell-plus claimed Super+Return");
    Ok(())
}
