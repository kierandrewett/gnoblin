//! Links the installed core without building it.
//!
//! The link stub (`GNOBLIN_CORE_LIB_DIR` for a real library) is what makes a shell record `NEEDED libgnoblin-core.so.1`
//! and no copy of the core. See `gnoblin_core_abi::build_support` and `design/modular-framework.md` section 4.3. The
//! `panic = "abort"` refusal is in `src/lib.rs` (`cfg(panic)`): a build script cannot see the profile's panic setting.

fn main() {
    gnoblin_core_abi::build_support::link_core(&[]);
}
