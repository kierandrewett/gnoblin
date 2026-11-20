//! Link flags of the std library.
//!
//! The library's identity is set here, as the core's is: the SONAME `libgnoblin-std.so.1` (Cargo derives the file name
//! `libgnoblin_std_impl.so`; dist installs the file under the SONAME), `-z nodelete` (a retired shell generation's
//! registrations may still point into the library, so it is never unmapped, decision D16), and `-z defs` (an unresolved symbol
//! fails the link, not the first call).
//!
//! The library needs the core, and says so the way every other consumer does: through the core SDK crate, whose build script links
//! the core's stub, so `NEEDED libgnoblin-core.so.1` is recorded and nothing of the core's internals is linked.
//!
//! The build tree is staged too: `target/<profile>/libgnoblin-std.so.1` is made a link to the library Cargo uplifts as
//! `libgnoblin_std_impl.so`, so a test or a shell run from the build tree finds the SONAME beside the other binaries.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rustc-cdylib-link-arg=-Wl,-soname,{}", gnoblin_std_abi::SONAME);
    println!("cargo:rustc-cdylib-link-arg=-Wl,-z,nodelete");
    println!("cargo:rustc-cdylib-link-arg=-Wl,-z,defs");
    // The core is the library beside this one in a build tree and in an install, so a std library loaded on its own (a tool that maps it,
    // `ldd`, `dist-check`) finds it. A shell that uses std is found through the loader's own search path (an install under `/usr`) or the
    // shell's run path; this run path serves the library's own dependency only.
    println!("cargo:rustc-cdylib-link-arg=-Wl,-rpath,$ORIGIN");
    println!("cargo:rerun-if-changed=build.rs");
    // The catalog of `gnoblin.std` the object protocol serves, written into `OUT_DIR` (nothing of it is committed).
    gnoblin_sdkgen::emit_for_build(&["objects"]);
    stage_soname_link();
}

/// Links `libgnoblin-std.so.1` to `libgnoblin_std_impl.so` in the profile directory this build writes to. The link may dangle
/// until Cargo has linked the library. A layout other than `<target>/<profile>/build/<package>/out` is left alone, and a failure
/// is a warning, never a failed build.
fn stage_soname_link() {
    let Some(out_dir) = env::var_os("OUT_DIR").map(PathBuf::from) else {
        return;
    };
    let Some(profile_dir) = out_dir.ancestors().nth(3) else {
        return;
    };
    if out_dir.ancestors().nth(2).and_then(Path::file_name) != Some("build".as_ref()) {
        return;
    }
    let link = profile_dir.join(gnoblin_std_abi::SONAME);
    let temporary = profile_dir.join(format!("{}.stage", gnoblin_std_abi::SONAME));
    let _ = fs::remove_file(&temporary);
    let linked =
        std::os::unix::fs::symlink("libgnoblin_std_impl.so", &temporary).and_then(|()| fs::rename(&temporary, &link));
    if let Err(error) = linked {
        println!(
            "cargo:warning=cannot stage {}: {error}; run `cargo xtask stage`",
            link.display()
        );
    }
}
