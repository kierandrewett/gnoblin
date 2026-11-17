//! Links `gnoblin` against `libgnoblin-core.so.1` without building it first, and tells the binary where to find it.
//!
//! `design/modular-framework.md` section 4.3: Cargo cannot make a binary wait for a `cdylib`, so the link goes through a
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

use std::env;

/// The run path of a binary built without `GNOBLIN_RPATH`.
const DEFAULT_RPATH: &str = "$ORIGIN:$ORIGIN/../lib:$ORIGIN/../lib64";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=GNOBLIN_RPATH");
    gnoblin_core_abi::build_support::link_core(&[]);
    let rpath = env::var("GNOBLIN_RPATH").unwrap_or_else(|_| DEFAULT_RPATH.to_owned());
    if !rpath.is_empty() {
        // DT_RUNPATH, not DT_RPATH, so a caller's `LD_LIBRARY_PATH` can still point the binary at another core.
        println!("cargo:rustc-link-arg-bins=-Wl,--enable-new-dtags,-rpath,{rpath}");
    }
}
