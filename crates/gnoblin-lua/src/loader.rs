//! Turning a `gnoblin.load` argument into files, inside the configuration
//! directory and nowhere else.
//!
//! The loader is the only part of the host that reads the filesystem on a script's
//! behalf, so it is where the confinement lives. Three things make an escape
//! impossible rather than unlikely:
//!
//! - The pattern is validated ([`crate::glob::validate`]): no absolute path, no
//!   `..` component, no unclosed class. A refusal names the pattern.
//! - The walk that expands a glob does not follow a directory symbolic link, so
//!   `**` cannot climb out of the tree.
//! - Every file that results, whether from a glob or an exact name, is
//!   canonicalised and must stay under the canonical configuration directory. A
//!   file that is a symbolic link to something outside is refused by name, which
//!   is what catches the one escape a walk cannot see.
//!
//! A missing exact file is an error, because `gnoblin.load("parts/motion.lua")`
//! that silently does nothing is a configuration that half-loaded. A glob with no
//! matches is allowed, because a drop-in directory may legitimately be empty.

use std::fs;
use std::path::{Path, PathBuf};

use crate::glob;

/// What a pattern resolved to.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Resolution {
    /// An exact path that does not exist.
    Missing,
    /// The files to evaluate, in load order.
    Files(Vec<PathBuf>),
}

/// Resolve `pattern`, which is relative to `current`.
///
/// `root` and `current` are canonical paths: the host canonicalises the
/// configuration directory once, and every file it evaluates is checked, so a
/// directory on the stack is inside the root by construction.
pub(super) fn resolve(root: &Path, current: &Path, pattern: &str) -> Result<Resolution, String> {
    glob::validate(pattern)?;
    if glob::is_glob(pattern) {
        let matched =
            glob::expand(current, pattern).map_err(|error| format!("cannot search {}: {error}", current.display()))?;
        let mut files = Vec::with_capacity(matched.len());
        for path in matched {
            let canonical =
                fs::canonicalize(&path).map_err(|error| format!("cannot resolve {}: {error}", path.display()))?;
            if !canonical.starts_with(root) {
                return Err(format!(
                    "{} resolves to {}, which is outside the configuration directory",
                    path.display(),
                    canonical.display()
                ));
            }
            if canonical.is_file() {
                files.push(canonical);
            }
        }
        files.sort();
        files.dedup();
        return Ok(Resolution::Files(files));
    }
    let path = current.join(pattern);
    let canonical = match fs::canonicalize(&path) {
        Ok(canonical) => canonical,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Resolution::Missing),
        Err(error) => return Err(format!("cannot resolve {}: {error}", path.display())),
    };
    if !canonical.starts_with(root) {
        return Err(format!(
            "{} resolves to {}, which is outside the configuration directory",
            path.display(),
            canonical.display()
        ));
    }
    if !canonical.is_file() {
        return Err(format!("{} is not a file", canonical.display()));
    }
    Ok(Resolution::Files(vec![canonical]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A fresh directory under the system temporary directory.
    fn tree(name: &str) -> PathBuf {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let unique = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("gnoblin-lua-{}-{name}-{unique}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("a test can create its directory");
        fs::canonicalize(&path).expect("the directory resolves")
    }

    fn write(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("a test can create a directory");
        }
        fs::write(path, text).expect("a test can write a file");
    }

    #[test]
    fn an_exact_file_resolves_and_a_missing_one_does_not() {
        let root = tree("exact");
        write(&root.join("appearance.lua"), "");
        let current = root.clone();
        assert_eq!(
            resolve(&root, &current, "appearance.lua"),
            Ok(Resolution::Files(vec![root.join("appearance.lua")]))
        );
        assert_eq!(resolve(&root, &current, "missing.lua"), Ok(Resolution::Missing));
    }

    #[test]
    fn a_glob_loads_in_filename_order_and_may_match_nothing() {
        let root = tree("glob");
        write(&root.join("conf.d/20-b.lua"), "");
        write(&root.join("conf.d/10-a.lua"), "");
        write(&root.join("conf.d/sub/30-c.lua"), "");
        let current = root.clone();
        assert_eq!(
            resolve(&root, &current, "conf.d/**/*.lua"),
            Ok(Resolution::Files(vec![
                root.join("conf.d/10-a.lua"),
                root.join("conf.d/20-b.lua"),
                root.join("conf.d/sub/30-c.lua"),
            ]))
        );
        assert_eq!(
            resolve(&root, &current, "nothing/*.lua"),
            Ok(Resolution::Files(Vec::new()))
        );
    }

    #[test]
    fn a_dot_dot_pattern_is_refused_before_the_filesystem_is_read() {
        let root = tree("dotdot");
        assert!(resolve(&root, &root, "../etc/passwd").is_err());
        assert!(resolve(&root, &root, "/etc/passwd").is_err());
    }

    #[test]
    fn a_file_symlink_that_points_outside_is_refused() {
        let root = tree("symlink");
        let outside = tree("symlink-outside");
        write(&outside.join("secret.lua"), "");
        std::os::unix::fs::symlink(outside.join("secret.lua"), root.join("link.lua")).expect("a test can link");
        let error = resolve(&root, &root, "link.lua").expect_err("a link outside the tree is refused");
        assert!(error.contains("outside the configuration directory"), "{error}");
    }

    #[test]
    fn a_directory_symlink_is_not_walked() {
        let root = tree("dir-symlink");
        let outside = tree("dir-symlink-outside");
        write(&outside.join("secret.lua"), "");
        std::os::unix::fs::symlink(&outside, root.join("linked")).expect("a test can link");
        assert_eq!(resolve(&root, &root, "**/*.lua"), Ok(Resolution::Files(Vec::new())));
    }
}
