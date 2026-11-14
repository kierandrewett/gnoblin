//! `cargo xtask native-libs`: every `-sys` crate in a lock file must name the system packages it needs.
//!
//! The hosted CI runners do not ship the libraries the crates link, so `scripts/ci-install-system-libraries.sh`
//! installs them before any Rust job. A new `-sys` crate can add a pkg-config or linker need that the list does
//! not cover, and the failure then shows up only as a build error on the runner. This check makes the link
//! between the two explicit:
//!
//! - Every `-sys` crate in `Cargo.lock` (root and `extensions/gnoblin-xr`) appears in [`NATIVE`] or [`NO_NATIVE`].
//! - Every Debian package [`NATIVE`] names appears in the script's list.
//! - Every entry in [`NATIVE`] and [`NO_NATIVE`] still appears in a lock file, so the table cannot rot.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

const SCRIPT: &str = "scripts/ci-install-system-libraries.sh";
const LOCK_FILES: &[&str] = &["Cargo.lock", "extensions/gnoblin-xr/Cargo.lock"];

/// `-sys` crates that link a system library, with the Debian packages that provide it.
const NATIVE: &[(&str, &[&str])] = &[
    ("clang-sys", &["libclang-dev"]),
    ("drm-sys", &["libdrm-dev"]),
    ("gbm-sys", &["libgbm-dev"]),
    ("input-sys", &["libinput-dev"]),
    ("libseat-sys", &["libseat-dev"]),
    ("libspa-sys", &["libpipewire-0.3-dev", "libclang-dev"]),
    ("libsqlite3-sys", &["libssl-dev"]),
    ("libudev-sys", &["libudev-dev"]),
    ("pipewire-sys", &["libpipewire-0.3-dev"]),
    ("pixman-sys", &["libpixman-1-dev"]),
    ("wayland-sys", &["libwayland-dev"]),
];

/// `-sys` crates that need no system package on Linux: pure bindings, or a vendored C build that needs only the
/// compiler, or a library opened at run time.
const NO_NATIVE: &[&str] = &[
    "inotify-sys",
    "js-sys",
    "kqueue-sys",
    "linux-raw-sys",
    "openxr-sys",
    "rquickjs-sys",
    "web-sys",
    "windows-sys",
];

pub fn run() -> ExitCode {
    match check(Path::new(".")) {
        Ok(problems) if problems.is_empty() => {
            println!("native-libs: every -sys crate is classified and its packages are installed");
            ExitCode::SUCCESS
        }
        Ok(problems) => {
            for problem in &problems {
                eprintln!("native-libs: {problem}");
            }
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("native-libs: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Every problem found under `root`. Empty means the table, the lock files and the script agree.
fn check(root: &Path) -> Result<Vec<String>, String> {
    let mut sys_crates = BTreeSet::new();
    for lock in LOCK_FILES {
        let text = fs::read_to_string(root.join(lock)).map_err(|e| format!("{lock}: {e}"))?;
        sys_crates.extend(lock_names(&text).into_iter().filter(|name| name.ends_with("-sys")));
    }

    let script = fs::read_to_string(root.join(SCRIPT)).map_err(|e| format!("{SCRIPT}: {e}"))?;
    let installed = script_packages(&script);

    let mut problems = Vec::new();
    for crate_name in &sys_crates {
        let classified = NATIVE.iter().any(|(name, _)| name == crate_name) || NO_NATIVE.contains(&crate_name.as_str());
        if !classified {
            problems.push(format!(
                "{crate_name} is in a lock file but not classified; add it to NATIVE with its Debian packages, \
                 or to NO_NATIVE, and install the packages in {SCRIPT}"
            ));
        }
    }
    for (crate_name, packages) in NATIVE {
        if !sys_crates.contains(*crate_name) {
            problems.push(format!(
                "{crate_name} is in NATIVE but no lock file has it; remove the entry"
            ));
        }
        for package in *packages {
            if !installed.contains(*package) {
                problems.push(format!("{crate_name} needs {package}, which {SCRIPT} does not install"));
            }
        }
    }
    for crate_name in NO_NATIVE {
        if !sys_crates.contains(*crate_name) {
            problems.push(format!(
                "{crate_name} is in NO_NATIVE but no lock file has it; remove the entry"
            ));
        }
    }
    Ok(problems)
}

/// The package names of every `name = "..."` line in a Cargo.lock.
fn lock_names(text: &str) -> BTreeSet<String> {
    text.lines()
        .filter_map(|line| line.strip_prefix("name = \""))
        .filter_map(|rest| rest.strip_suffix('"'))
        .map(str::to_owned)
        .collect()
}

/// The Debian package names the script installs: the first field of each `"apt|dnf"` entry.
fn script_packages(script: &str) -> BTreeSet<String> {
    script
        .lines()
        .filter_map(|line| line.trim().strip_prefix('"'))
        .filter_map(|rest| rest.split('|').next())
        .filter(|name| !name.is_empty() && !name.contains('"'))
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_names_reads_package_names_only() {
        let text =
            "[[package]]\nname = \"libudev-sys\"\nversion = \"0.1\"\ndependencies = [\n \"name = \\\"x\\\"\",\n]\n";
        assert_eq!(lock_names(text), BTreeSet::from(["libudev-sys".to_owned()]));
    }

    #[test]
    fn script_packages_takes_the_apt_name() {
        let script = "libraries=(\n    \"libudev-dev|systemd-devel\"  # note\n)\n";
        assert_eq!(script_packages(script), BTreeSet::from(["libudev-dev".to_owned()]));
    }

    #[test]
    fn the_repository_is_classified() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        assert_eq!(check(&root), Ok(Vec::new()));
    }
}
