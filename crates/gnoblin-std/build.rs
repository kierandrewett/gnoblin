//! Links the installed std library without building it.
//!
//! The link stub (`GNOBLIN_STD_LIB_DIR` for a real library) is what makes a std shell record `NEEDED libgnoblin-std.so.1` and no copy of
//! the library. See `gnoblin_std_abi::build_support` and `design/modular-framework.md` section 4.3. The core is linked the same way
//! by the `gnoblin-core` crate this one depends on, so the shell records `NEEDED libgnoblin-core.so.1` once and the chain is
//! shell -> std -> core.

fn main() {
    gnoblin_std_abi::build_support::link_std();
}
