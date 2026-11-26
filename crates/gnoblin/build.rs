//! Links `gnoblin` against `libgnoblin-core.so.1` without building it first, and tells the binary where to find it.
//!
//! Cargo cannot make a binary wait for a `cdylib`, so the link goes through a
//! generated stub that carries the SONAME. The binary records `NEEDED libgnoblin-core.so.1` once; at run time the dynamic
//! loader resolves that name to the real library. The binary calls the core (`gnoblin_main`, which serves the compositor
//! role), so the plain link records the entry: no `--no-as-needed` is needed for a library the code references.
//!
//! Where the loader looks, when `LD_LIBRARY_PATH` is unset, is the binary's run path, and `$ORIGIN` is the directory the
//! binary sits in:
//!
//! - `$ORIGIN`: the build tree. `cargo xtask stage` (and a workspace build) names the core `libgnoblin-core.so.1` next to
//!   `target/<profile>/gnoblin`, so the integration tests that run the built binary find it.
//! - `$ORIGIN/../lib` and `$ORIGIN/../lib64`: an install under a prefix the loader does not search (`dist install --prefix
//!   ~/gnoblin`), whose library directory is `lib` or `lib64`.
//!
//! A distribution package installs under `/usr`, which the loader searches, and wants no run path at all: set
//! `GNOBLIN_RPATH` to the empty string, or to the run path it prefers.
//!
//! `cargo build -p gnoblin` links a stub and does not build the core library, so a binary built that way stops at the loader's
//! `libgnoblin-core.so.1: cannot open shared object file` until the core sits beside it (`cargo xtask stage`, or a workspace build). This
//! script does not say so while it builds: a workspace build reaches it before the core's own script has named the library, so a warning
//! here was printed on every build, including the many that never run the binary. The steps are in CONTRIBUTING.md ("The core library
//! in the build tree").

use std::env;

/// The run path of a binary built without `GNOBLIN_RPATH`.
const DEFAULT_RPATH: &str = "$ORIGIN:$ORIGIN/../lib:$ORIGIN/../lib64";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=GNOBLIN_RPATH");
    println!(
        "cargo:rerun-if-env-changed={}",
        gnoblin_core_abi::build_support::LIB_DIR_VAR
    );
    // The binary only: a test target of this crate that links the core's implementation (`tests/support/core_rig.rs`) must see the one
    // definition of its exports, and a stub listed before it would take them.
    gnoblin_core_abi::build_support::link_core_into_bins(&[]);
    let rpath = env::var("GNOBLIN_RPATH").unwrap_or_else(|_| DEFAULT_RPATH.to_owned());
    if !rpath.is_empty() {
        // DT_RUNPATH, not DT_RPATH, so a caller's `LD_LIBRARY_PATH` can still point the binary at another core.
        println!("cargo:rustc-link-arg-bins=-Wl,--enable-new-dtags,-rpath,{rpath}");
    }
}
