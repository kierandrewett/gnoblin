//! Links the installed core without building it, and writes the generated half of the SDK.
//!
//! The link stub (`GNOBLIN_CORE_LIB_DIR` for a real library) is what makes a shell record `NEEDED libgnoblin-core.so.1`
//! and no copy of the core. See `gnoblin_core_abi::build_support` and `design/modular-framework.md` section 4.3. The
//! `panic = "abort"` refusal is in `src/lib.rs` (`cfg(panic)`): a build script cannot see the profile's panic setting.
//!
//! The schema's names, the typed node classes and the error kinds are rendered by `gnoblin-sdkgen` into `OUT_DIR` on every
//! build that the schema, the ABI crate or the generator changed; nothing of them is committed (`src/lib.rs` includes the file).

fn main() {
    gnoblin_core_abi::build_support::link_core(&[]);
    gnoblin_sdkgen::emit_for_build(&["rust"]);
}
