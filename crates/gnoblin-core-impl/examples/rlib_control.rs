//! Negative control for the "one core copy" test.
//!
//! A shell must reach the core through the SDK and the installed library. This example does the wrong thing on purpose: it
//! links `gnoblin-core-impl` as a Rust library, so the core's code, statics and exported `gnoblin_*` symbols are compiled
//! into this `cdylib`. `cargo xtask dist-check` and the scenario tests must flag it (a shell defining `gnoblin_*`), and
//! that is the only way to know the check can fail.
//!
//! It is an example of this package, not a workspace member, so no other crate has a dependency edge on the
//! implementation.
#![allow(unsafe_code)]

/// Keeps the core's exports alive in the linked output, as a real shell that called into the core would.
#[unsafe(no_mangle)]
pub extern "C" fn gnoshell_entry_v1() -> u32 {
    let (mut major, mut minor, mut patch) = (0, 0, 0);
    // SAFETY: the three pointers are live locals.
    unsafe { gnoblin_core_impl::gnoblin_abi_version(&mut major, &mut minor, &mut patch) };
    major
}
