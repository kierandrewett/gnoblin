//! The golden shells of example 12, as `cargo xtask abi-check` sees them.
//!
//! `tests/modular/golden/bin/` holds three shell binaries built against ABI 1.0 (two core-only, one of which writes an effect list with
//! the `GnoblinEffect` of before the shader fields, and a std one), and
//! `tests/modular/golden/golden.lock` records what they are: the ABI each was built for, the commit and the schema catalog hash of
//! the SDK sources, the toolchain, and a sha256 and size for each file. The modular tests run them in real sessions against newer libraries; this
//! module is the guard that makes that proof mean something. On every `abi-check` it fails when
//!
//! - a file's bytes are not what `golden.lock` records (someone rebuilt or replaced a golden without saying so), or a file in
//!   `bin/` is not listed, or a listed one is missing;
//! - a golden does not have the dynamic facts of a shell: NEEDED the core (and std for the std one, never std for the core-only
//!   one), no `gnoblin_*` symbol defined, `gnoshell_entry_v1` the only export;
//! - a golden claims an ABI the workspace has not got (another major, or a minor above the current one), which would mean the
//!   lock was edited to say something the build never saw.
//!
//! Rebuilding is deliberately not part of the check: the binaries are the test subject, and an owner replaces them with
//! `tests/modular/golden/regenerate.sh`.

use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::abi_check::Version;
use crate::dist_check::{check_core_only_shell, check_std_shell};
use crate::elf;

/// The golden directory, relative to the workspace root.
pub const DIR: &str = "tests/modular/golden";

/// The record file in [`DIR`].
const LOCK: &str = "golden.lock";

/// Which tier a golden shell links.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GoldenKind {
    CoreOnly,
    Std,
}

/// One golden shell the lock describes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoldenShell {
    pub kind: GoldenKind,
    /// Relative to [`DIR`].
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

/// What `golden.lock` says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Golden {
    pub core_abi: Version,
    pub std_abi: Version,
    pub shells: Vec<GoldenShell>,
}

fn parse_version(text: &str) -> Result<Version, String> {
    let (major, minor) = text
        .split_once('.')
        .ok_or_else(|| format!("`{text}` is not a version like 1.0"))?;
    let number = |part: &str| {
        part.parse::<u32>()
            .map_err(|_| format!("`{text}` is not a version like 1.0"))
    };
    Ok((number(major)?, number(minor)?))
}

/// Parses `golden.lock`. Comments and blank lines are skipped; `core-abi`, `std-abi`, `base`, `catalog` and `toolchain` are required and
/// every `shell` line has a kind, a path, a sha256 and a size.
pub fn parse(text: &str) -> Result<Golden, String> {
    let mut core_abi = None;
    let mut std_abi = None;
    let mut base = false;
    let mut catalog = false;
    let mut toolchain = false;
    let mut shells = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let at = |message: String| format!("{LOCK}:{}: {message}", number + 1);
        let (key, rest) = line.split_once(' ').unwrap_or((line, ""));
        match key {
            "core-abi" => core_abi = Some(parse_version(rest.trim()).map_err(at)?),
            "std-abi" => std_abi = Some(parse_version(rest.trim()).map_err(at)?),
            "base" => {
                if rest.trim().len() != 40 || !rest.trim().bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return Err(at(format!("`{}` is not a commit id", rest.trim())));
                }
                base = true;
            }
            "catalog" => {
                let digits = rest.trim().strip_prefix("0x").unwrap_or("");
                if digits.len() != 16 || !digits.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')) {
                    return Err(at(format!(
                        "`{}` is not a catalog hash like 0x0123456789abcdef",
                        rest.trim()
                    )));
                }
                catalog = true;
            }
            "toolchain" => {
                if rest.trim().is_empty() {
                    return Err(at("the toolchain line is empty".to_owned()));
                }
                toolchain = true;
            }
            "shell" => {
                let fields: Vec<&str> = rest.split_whitespace().collect();
                let [kind, path, sha256, bytes] = fields[..] else {
                    return Err(at("a shell line is `shell <kind> <path> <sha256> <bytes>`".to_owned()));
                };
                let kind = match kind {
                    "core-only" => GoldenKind::CoreOnly,
                    "std" => GoldenKind::Std,
                    other => return Err(at(format!("shell kind `{other}` is neither `core-only` nor `std`"))),
                };
                if sha256.len() != 64 || !sha256.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')) {
                    return Err(at(format!("`{sha256}` is not a lowercase sha256")));
                }
                let bytes = bytes.parse().map_err(|_| at(format!("`{bytes}` is not a size")))?;
                shells.push(GoldenShell {
                    kind,
                    path: path.to_owned(),
                    sha256: sha256.to_owned(),
                    bytes,
                });
            }
            other => return Err(at(format!("unknown key `{other}`"))),
        }
    }
    let missing = |what: &str| format!("{LOCK} has no `{what}` line");
    if !base {
        return Err(missing("base"));
    }
    if !catalog {
        return Err(missing("catalog"));
    }
    if !toolchain {
        return Err(missing("toolchain"));
    }
    if shells.is_empty() {
        return Err(missing("shell"));
    }
    Ok(Golden {
        core_abi: core_abi.ok_or_else(|| missing("core-abi"))?,
        std_abi: std_abi.ok_or_else(|| missing("std-abi"))?,
        shells,
    })
}

/// Whether a golden built for `built` is one the workspace's `current` ABI may be asked to keep running: the same major and a
/// minor that exists.
fn covered(built: Version, current: Version) -> bool {
    built.0 == current.0 && built.1 <= current.1
}

/// The problems of the golden set under `root`, empty when it is sound. `core` and `std` are the ABI versions of this build.
pub fn problems(root: &Path, core: Version, std: Version) -> Result<(Golden, Vec<String>), String> {
    let dir = root.join(DIR);
    let text = fs::read_to_string(dir.join(LOCK)).map_err(|error| format!("{DIR}/{LOCK}: {error}"))?;
    let golden = parse(&text)?;
    let mut problems = Vec::new();
    if !covered(golden.core_abi, core) {
        problems.push(format!(
            "{LOCK} says the shells were built for core ABI {}.{}, which the workspace's core ABI {}.{} does not cover",
            golden.core_abi.0, golden.core_abi.1, core.0, core.1
        ));
    }
    if !covered(golden.std_abi, std) {
        problems.push(format!(
            "{LOCK} says the shells were built for std ABI {}.{}, which the workspace's std ABI {}.{} does not cover",
            golden.std_abi.0, golden.std_abi.1, std.0, std.1
        ));
    }
    for shell in &golden.shells {
        let path = dir.join(&shell.path);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                problems.push(format!("{}: {error}", shell.path));
                continue;
            }
        };
        let sha256: String = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        if sha256 != shell.sha256 || bytes.len() as u64 != shell.bytes {
            problems.push(format!(
                "{}: sha256 {sha256} ({} bytes) is not what {LOCK} records ({}, {} bytes); the golden shells are replaced by an owner with {DIR}/regenerate.sh, never edited",
                shell.path,
                bytes.len(),
                shell.sha256,
                shell.bytes
            ));
            continue;
        }
        match elf::parse(&bytes) {
            Ok(facts) => {
                let facts_problems = match shell.kind {
                    GoldenKind::CoreOnly => check_core_only_shell(&facts),
                    GoldenKind::Std => check_std_shell(&facts),
                };
                problems.extend(
                    facts_problems
                        .into_iter()
                        .map(|problem| format!("{}: {problem}", shell.path)),
                );
            }
            Err(error) => problems.push(format!("{}: {error}", shell.path)),
        }
    }
    if let Ok(entries) = fs::read_dir(dir.join("bin")) {
        let mut listed: Vec<&str> = golden
            .shells
            .iter()
            .filter_map(|shell| shell.path.strip_prefix("bin/"))
            .collect();
        listed.sort_unstable();
        for entry in entries.filter_map(Result::ok) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if listed.binary_search(&name.as_str()).is_err() {
                problems.push(format!("bin/{name} is not listed in {LOCK}"));
            }
        }
    }
    Ok((golden, problems))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::scratch;

    const GOOD: &str = "# comment\n\
        core-abi 1.0\n\
        std-abi 1.0\n\
        base 55128b286b1e1822e030db25725478d799a310a2\n\
        catalog 0x8d9a5c8fd0c2c922\n\
        toolchain rustc 1.99.1\n\
        shell core-only bin/a.so 850ecdf2ba192d79c52473c04fc914369d07fa3b030ed6c60de3047d83dc227d 12\n\
        shell std bin/b.so 0fe44b86fd700723536098dc32b8bad685db3976122ea1bcd64e25a0ef518fd9 7\n";

    #[test]
    fn a_lock_parses_into_its_versions_and_shells() {
        let golden = parse(GOOD).unwrap();
        assert_eq!(golden.core_abi, (1, 0));
        assert_eq!(golden.std_abi, (1, 0));
        assert_eq!(golden.shells.len(), 2);
        assert_eq!(golden.shells[0].kind, GoldenKind::CoreOnly);
        assert_eq!(golden.shells[1].path, "bin/b.so");
        assert_eq!(golden.shells[1].bytes, 7);
    }

    #[test]
    fn a_lock_with_a_missing_or_malformed_line_is_refused_with_its_line_number() {
        for (text, wanted) in [
            (GOOD.replace("core-abi 1.0\n", ""), "no `core-abi` line"),
            (GOOD.replace("toolchain rustc 1.99.1\n", ""), "no `toolchain` line"),
            (GOOD.replace("catalog 0x8d9a5c8fd0c2c922\n", ""), "no `catalog` line"),
            (GOOD.replace("0x8d9a5c8fd0c2c922", "0xZZ"), "is not a catalog hash"),
            (GOOD.replace("base 55128b", "base zz128b"), "is not a commit id"),
            (
                GOOD.replace("shell std", "shell weird"),
                "neither `core-only` nor `std`",
            ),
            (GOOD.replace("core-abi 1.0", "core-abi one"), "is not a version"),
            (GOOD.replace("12\n", "twelve\n"), "is not a size"),
            (format!("{GOOD}surprise 1\n"), "unknown key `surprise`"),
            (GOOD.replace("850ecdf2ba", "850ECDF2BA"), "lowercase sha256"),
        ] {
            let error = parse(&text).unwrap_err();
            assert!(error.contains(wanted), "{error}");
        }
    }

    #[test]
    fn a_golden_is_covered_by_the_same_major_and_a_minor_that_exists() {
        assert!(covered((1, 0), (1, 0)));
        assert!(covered((1, 0), (1, 3)));
        assert!(!covered((1, 2), (1, 1)));
        assert!(!covered((1, 0), (2, 0)));
    }

    #[test]
    fn a_file_that_is_not_what_the_lock_records_is_named_with_both_hashes() {
        let root = scratch("golden-hash");
        let dir = root.join(DIR);
        fs::create_dir_all(dir.join("bin")).unwrap();
        fs::write(dir.join(LOCK), GOOD).unwrap();
        fs::write(dir.join("bin/a.so"), b"not the golden").unwrap();
        fs::write(dir.join("bin/stray.so"), b"x").unwrap();
        let (_, problems) = problems(&root, (1, 0), (1, 0)).unwrap();
        let text = problems.join("\n");
        assert!(text.contains("bin/a.so: sha256"), "{text}");
        assert!(text.contains("850ecdf2ba"), "{text}");
        assert!(text.contains("bin/b.so: "), "the missing file is named: {text}");
        assert!(text.contains("bin/stray.so is not listed"), "{text}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_lock_for_an_abi_the_workspace_lacks_is_a_problem() {
        let root = scratch("golden-abi");
        let dir = root.join(DIR);
        fs::create_dir_all(dir.join("bin")).unwrap();
        fs::write(dir.join(LOCK), GOOD.replace("core-abi 1.0", "core-abi 1.4")).unwrap();
        let (_, problems) = problems(&root, (1, 0), (1, 0)).unwrap();
        assert!(
            problems.iter().any(|problem| problem.contains("core ABI 1.4")),
            "{problems:?}"
        );
        let _ = fs::remove_dir_all(root);
    }
}
