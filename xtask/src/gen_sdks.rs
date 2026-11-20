//! `cargo xtask gen-sdks [--out DIR] [--check] [--update] [--target NAME]`: the generator of every language SDK, by hand.
//!
//! Nothing the generator writes is committed (`crates/gnoblin-sdkgen` is the library; its `build.rs` callers write the files a
//! build embeds into `OUT_DIR`, and `cargo xtask dist` writes the editor files into a prefix). This command is for a person and
//! for CI:
//!
//! - with no flag, it writes a readable copy of every generated file (Rust formatted by the pinned `rustfmt`) to `target/sdk`,
//!   or to `--out DIR`;
//! - `--check` proves the generator is sound: every target renders, a second render is byte-identical, the Python it writes
//!   compiles, the public surface of the catalog matches `crates/gnoblin-schema/schema/lock/*.lock` and the C ABI matches
//!   `abi.lock` and the golden shells (what `abi-check` proves). It writes nothing;
//! - `--update` records an additive change of the catalog's surface in the lock files, as `abi-check --update` does for the ABI.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use gnoblin_sdkgen::{SdkSpec, check_surface_lock, files_of, format_all, render, render_surface_locks, write_files};

const USAGE: &str =
    "usage: cargo xtask gen-sdks [--out DIR] [--check] [--update] [--target rust|std-dyn|python|typescript]";

/// Where the readable copy goes by default, under the ignored `target/`.
const DEFAULT_OUT: &str = "target/sdk";

#[derive(Default)]
struct GenOptions {
    check: bool,
    update: bool,
    out: Option<PathBuf>,
    target: Option<String>,
}

fn parse(args: &[String]) -> Result<GenOptions, String> {
    let mut options = GenOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--check" => options.check = true,
            "--update" => options.update = true,
            "--out" => {
                index += 1;
                options.out = Some(PathBuf::from(args.get(index).ok_or("--out needs a directory")?));
            }
            "--target" => {
                index += 1;
                options.target = Some(args.get(index).ok_or("--target needs a name")?.clone());
            }
            other => return Err(format!("unexpected argument `{other}`")),
        }
        index += 1;
    }
    if options.check && (options.update || options.out.is_some()) {
        return Err("--check writes nothing: it takes no --update and no --out".to_owned());
    }
    Ok(options)
}

/// The `gen-sdks` subcommand.
pub fn run(args: &[String]) -> ExitCode {
    let options = match parse(args) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("[gen-sdks] {error}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let result = if options.check {
        check(Path::new("."), options.target.as_deref())
    } else if options.update {
        update(Path::new("."))
    } else {
        write_copy(Path::new("."), &options)
    };
    match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("[gen-sdks] {error}");
            ExitCode::from(2)
        }
    }
}

/// Writes the readable copy.
fn write_copy(root: &Path, options: &GenOptions) -> Result<ExitCode, String> {
    let spec = SdkSpec::load(root)?;
    let mut rendered = render(&spec, options.target.as_deref())?;
    format_all(&mut rendered, &root.join("rustfmt.toml"))?;
    let out = options.out.clone().unwrap_or_else(|| root.join(DEFAULT_OUT));
    if options.out.is_none() && out.exists() {
        // The default directory is entirely this command's, so a file a renamed class left behind goes with it.
        fs::remove_dir_all(&out).map_err(|error| format!("cannot clear {}: {error}", out.display()))?;
    }
    let files = files_of(&rendered);
    let written = write_files(&out, &files)?;
    for path in &written {
        println!("[gen-sdks] wrote {}", out.join(path).display());
    }
    println!("[gen-sdks] {} files written to {}", written.len(), out.display());
    Ok(ExitCode::SUCCESS)
}

/// Records the catalog's surface in the lock files.
fn update(root: &Path) -> Result<ExitCode, String> {
    let spec = SdkSpec::load(root)?;
    let locks = render_surface_locks(&spec, root)?;
    let written = write_files(root, &locks)?;
    for path in &written {
        println!("[gen-sdks] wrote {path}");
    }
    println!("[gen-sdks] {} surface lock(s) updated", written.len());
    Ok(ExitCode::SUCCESS)
}

/// The proof the generator is sound. Every failure is reported; the exit code says whether there was one.
fn check(root: &Path, target: Option<&str>) -> Result<ExitCode, String> {
    let spec = SdkSpec::load(root)?;
    let mut ok = true;

    let first = render(&spec, target)?;
    let second = render(&spec, target)?;
    let files = files_of(&first);
    if files == files_of(&second) {
        println!(
            "[gen-sdks] {} files render, and a second render is byte-identical",
            files.len()
        );
    } else {
        ok = false;
        eprintln!("[gen-sdks] two renders of the same catalog differ: the generator reads something that changes");
    }

    ok &= python_compiles(root, &files)?;

    if target.is_none() {
        let report = check_surface_lock(&spec, root)?;
        if report.is_clean() {
            println!("[gen-sdks] the public surface matches crates/gnoblin-schema/schema/lock");
        } else {
            ok = false;
            for line in &report.breaking {
                eprintln!("[gen-sdks] breaking: {line}");
            }
            for line in &report.stale {
                eprintln!("[gen-sdks] stale: {line}");
            }
            if report.breaking.is_empty() {
                eprintln!("[gen-sdks] the surface lock is out of date; run `cargo xtask gen-sdks --update` and commit");
            } else {
                eprintln!("[gen-sdks] a released member changed or went: that needs a new major, not an update");
            }
        }
        // The C ABI the generated bindings call, and the golden shells that were built against it.
        ok &= crate::abi_check::check(root, false)?;
    }
    Ok(if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}

/// Compiles the generated Python with the interpreter's own compiler, when there is an interpreter: a syntax error in a
/// generated module would otherwise be found when the first shell imports it.
fn python_compiles(root: &Path, files: &[(String, String)]) -> Result<bool, String> {
    let python: Vec<&(String, String)> = files.iter().filter(|(path, _)| path.ends_with(".py")).collect();
    if python.is_empty() {
        return Ok(true);
    }
    let scratch = root.join("target/gen-sdks-check");
    if scratch.exists() {
        fs::remove_dir_all(&scratch).map_err(|error| format!("cannot clear {}: {error}", scratch.display()))?;
    }
    let owned: Vec<(String, String)> = python.iter().map(|(path, text)| (path.clone(), text.clone())).collect();
    write_files(&scratch, &owned)?;
    let paths: Vec<PathBuf> = owned.iter().map(|(path, _)| scratch.join(path)).collect();
    let status = Command::new("python3")
        .args(["-m", "py_compile"])
        .args(&paths)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output();
    let result = match status {
        Ok(output) if output.status.success() => {
            println!("[gen-sdks] {} generated Python modules compile", paths.len());
            true
        }
        Ok(output) => {
            eprintln!(
                "[gen-sdks] the generated Python does not compile:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            false
        }
        Err(error) => {
            println!("[gen-sdks] python3 is not available ({error}); the generated Python is not compiled");
            true
        }
    };
    let _ = fs::remove_dir_all(&scratch);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|text| (*text).to_owned()).collect()
    }

    #[test]
    fn the_flags_parse_and_a_check_takes_no_output() {
        assert!(parse(&args(&["--check"])).expect("a check").check);
        assert!(parse(&args(&["--check", "--out", "x"])).is_err());
        assert!(parse(&args(&["--check", "--update"])).is_err());
        let options = parse(&args(&["--out", "x", "--target", "rust"])).expect("a copy");
        assert_eq!(options.out.as_deref(), Some(Path::new("x")));
        assert_eq!(options.target.as_deref(), Some("rust"));
        assert!(parse(&args(&["--bogus"])).is_err());
    }
}
