//! `cargo xtask stage`: make the build tree runnable by naming the core library the way the binary records it.
//!
//! `gnoblin` is linked against a stub that carries the SONAME `libgnoblin-core.so.1` (`crates/gnoblin/build.rs`), so at run
//! time the dynamic loader looks that name up. Cargo writes the real library as `target/<profile>/libgnoblin_core_impl.so`,
//! and nothing builds it when only `cargo build -p gnoblin` runs. This command builds `gnoblin-core-impl`, refuses a library
//! that `dist-check` would refuse, and links it into the same directory as the binary under the two names a consumer uses:
//!
//! ```text
//! target/<profile>/libgnoblin-core.so.1 -> libgnoblin_core_impl.so   (the SONAME; the binary's $ORIGIN run path finds it)
//! target/<profile>/libgnoblin-core.so   -> libgnoblin-core.so.1      (the linker name, for GNOBLIN_CORE_LIB_DIR=target/<profile>)
//! ```
//!
//! The links are relative, so a rebuilt library is picked up without staging again, and the tree can be moved. A workspace
//! build makes the SONAME link too (`gnoblin-core-impl/build.rs`); this command is the way to stage for `cargo build -p
//! gnoblin`, which does not build the library, and it is the check that what is staged is a shippable core.
//!
//! ```text
//! cargo xtask stage [--release | --profile NAME] [--no-build]
//! ```
//!
//! `--profile` names the directory under the target directory (`debug`, the default, or `release`), as for `dist`.
//! `--no-build` stages what a build already wrote. `CARGO_TARGET_DIR` is honoured.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use gnoblin_core_abi as abi;

use crate::dist_check;
use crate::elf;

pub const USAGE: &str = "usage: cargo xtask stage [--release | --profile NAME] [--no-build]";

/// What cargo names the core's `cdylib`.
const BUILT_NAME: &str = "libgnoblin_core_impl.so";
/// The linker name of the core: what `-lgnoblin-core` finds in a directory named by `GNOBLIN_CORE_LIB_DIR`.
const LINKER_NAME: &str = "libgnoblin-core.so";

#[derive(Debug, PartialEq, Eq)]
struct StageOptions {
    /// The directory under the target directory the build writes to.
    profile: String,
    build: bool,
    target_dir: PathBuf,
}

fn parse(args: &[String], target_dir: PathBuf) -> Result<StageOptions, String> {
    let mut profile = "debug".to_owned();
    let mut build = true;
    let mut rest = args.iter();
    while let Some(flag) = rest.next() {
        match flag.as_str() {
            "--release" => "release".clone_into(&mut profile),
            "--profile" => profile = rest.next().cloned().ok_or("`--profile` needs a value")?,
            "--no-build" => build = false,
            other => return Err(format!("unexpected argument `{other}`")),
        }
    }
    if profile.is_empty() || profile.contains('/') {
        return Err(format!("profile `{profile}` is not a directory name"));
    }
    Ok(StageOptions {
        profile,
        build,
        target_dir,
    })
}

/// The cargo arguments that select the profile whose output directory is `profile`.
fn cargo_profile_args(profile: &str) -> Vec<String> {
    match profile {
        "debug" => Vec::new(),
        "release" => vec!["--release".to_owned()],
        other => vec!["--profile".to_owned(), other.to_owned()],
    }
}

fn build(options: &StageOptions) -> Result<(), String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    println!(
        "[stage] cargo build -p gnoblin-core-impl {}",
        cargo_profile_args(&options.profile).join(" ")
    );
    let status = Command::new(&cargo)
        .args(["build", "-p", "gnoblin-core-impl"])
        .args(cargo_profile_args(&options.profile))
        .status()
        .map_err(|error| format!("cannot run cargo: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`cargo build -p gnoblin-core-impl` failed ({status})"))
    }
}

/// Checks the library Cargo wrote in `dir` and links it under the SONAME and the linker name. Returns the SONAME link.
fn stage_dir(dir: &Path) -> Result<PathBuf, String> {
    let built = dir.join(BUILT_NAME);
    let core = elf::read(&built).map_err(|error| {
        format!(
            "{}: {error}; build it with `cargo build -p gnoblin-core-impl` (or drop `--no-build`)",
            built.display()
        )
    })?;
    let problems = dist_check::check_core(&core);
    if !problems.is_empty() {
        return Err(format!(
            "{} is not a shippable core, so it is not staged:\n  {}",
            built.display(),
            problems.join("\n  ")
        ));
    }
    let soname = dir.join(abi::SONAME);
    crate::dist::write_link(&soname, BUILT_NAME).map_err(|error| error.to_string())?;
    crate::dist::write_link(&dir.join(LINKER_NAME), abi::SONAME).map_err(|error| error.to_string())?;
    Ok(soname)
}

pub fn run(args: &[String]) -> ExitCode {
    let target_dir = std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| PathBuf::from("target"), PathBuf::from);
    let options = match parse(args, target_dir) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("[stage] {message}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let staged = (|| {
        if options.build {
            build(&options)?;
        }
        stage_dir(&options.target_dir.join(&options.profile))
    })();
    match staged {
        Ok(soname) => {
            println!(
                "[stage] {} -> {BUILT_NAME}: `gnoblin` in {} finds its core through $ORIGIN",
                soname.display(),
                soname
                    .parent()
                    .map_or_else(String::new, |dir| dir.display().to_string())
            );
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("[stage] {message}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{cc, exports_source, good_core, scratch};

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn the_profile_is_a_directory_name_and_defaults_to_debug() {
        let target = PathBuf::from("target");
        assert_eq!(parse(&[], target.clone()).unwrap().profile, "debug");
        assert_eq!(parse(&args(&["--release"]), target.clone()).unwrap().profile, "release");
        assert_eq!(
            parse(&args(&["--profile", "ci"]), target.clone()).unwrap().profile,
            "ci"
        );
        assert!(!parse(&args(&["--no-build"]), target.clone()).unwrap().build);
        assert!(parse(&args(&["--profile"]), target.clone()).is_err());
        assert!(parse(&args(&["--profile", "a/b"]), target.clone()).is_err());
        assert!(parse(&args(&["--frob"]), target).is_err());
    }

    #[test]
    fn the_cargo_arguments_follow_the_directory() {
        assert!(cargo_profile_args("debug").is_empty());
        assert_eq!(cargo_profile_args("release"), ["--release"]);
        assert_eq!(cargo_profile_args("ci"), ["--profile", "ci"]);
    }

    #[test]
    fn a_built_core_is_linked_under_the_soname_and_the_linker_name() {
        let dir = scratch("stage");
        std::fs::rename(good_core(&dir), dir.join(BUILT_NAME)).unwrap();
        let soname = stage_dir(&dir).unwrap();
        assert_eq!(soname, dir.join(abi::SONAME));
        assert_eq!(std::fs::read_link(&soname).unwrap(), Path::new(BUILT_NAME));
        assert_eq!(
            std::fs::read_link(dir.join(LINKER_NAME)).unwrap(),
            Path::new(abi::SONAME)
        );
        // Staging again replaces the links, and the SONAME link resolves to the library itself.
        stage_dir(&dir).unwrap();
        assert!(elf::read(&soname).unwrap().soname.as_deref() == Some(abi::SONAME));
    }

    #[test]
    fn a_core_with_the_test_driver_or_a_missing_library_is_not_staged() {
        let dir = scratch("stage-bad");
        let missing = stage_dir(&dir).unwrap_err();
        assert!(missing.contains("cargo build -p gnoblin-core-impl"), "{missing}");

        cc(
            &dir,
            "driver.c",
            &exports_source("void gnoblin_test_main(void) {}\n", ""),
            &[
                "-shared",
                "-fPIC",
                "-Wl,-soname,libgnoblin-core.so.1",
                "-Wl,-z,nodelete",
                "-o",
                BUILT_NAME,
            ],
        );
        let refused = stage_dir(&dir).unwrap_err();
        assert!(refused.contains("gnoblin_test_main"), "{refused}");
        assert!(!dir.join(abi::SONAME).exists());
    }
}
