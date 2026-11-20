//! Writes the call of every function of `libgnoblin-std`'s table.
//!
//! `gnoblin-sdkgen` renders it from the std ABI crate into `OUT_DIR` on every build that the ABI or the generator changed;
//! nothing of it is committed (`src/lib.rs` includes the file).

fn main() {
    gnoblin_sdkgen::emit_for_build(&["std-dyn"]);
}
