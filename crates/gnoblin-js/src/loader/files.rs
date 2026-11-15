//! Resolution of a file import to a canonical path under the config root: the lexical `..` check, the symlink check
//! and the extensions an import may leave out. See the module documentation of [`super`] for the rules.

use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::error::JsLoadError;

/// Resolves the file import `name` made by module `base` to a canonical path under `root`.
///
/// `root` must be canonical. `base` is the importing module's name: a canonical file path, or empty for
/// an import the host makes with no importing module, in which case `name` may also be an absolute path
/// and relative names count from `root`.
pub(crate) fn resolve_file(root: &Path, base: &str, name: &str) -> std::result::Result<PathBuf, JsLoadError> {
    let target = Path::new(name);
    let joined = if target.is_absolute() {
        if !base.is_empty() {
            return Err(JsLoadError::Unresolvable { name: name.to_owned() });
        }
        target.to_path_buf()
    } else if base.is_empty() {
        root.join(target)
    } else {
        let importer = Path::new(base);
        match importer.parent() {
            Some(dir) if importer.is_absolute() => dir.join(target),
            _ => {
                return Err(JsLoadError::NoFileBase {
                    name: name.to_owned(),
                    base: base.to_owned(),
                });
            }
        }
    };
    let normal = normalise(&joined);
    let escape = |path: PathBuf| JsLoadError::Escape {
        name: name.to_owned(),
        path,
        root: root.to_path_buf(),
    };
    // Lexical check first: a `..` that climbs out is refused without touching the file system.
    if !normal.starts_with(root) {
        return Err(escape(normal));
    }
    let candidates = candidates(&normal);
    for candidate in &candidates {
        // `canonicalize` follows symlinks and fails for a missing file, which moves on to the next
        // candidate. A symlink that points out of the root is an error, not a miss.
        let Ok(canonical) = fs::canonicalize(candidate) else {
            continue;
        };
        if !canonical.starts_with(root) {
            return Err(escape(canonical));
        }
        if !canonical.is_file() {
            return Err(JsLoadError::NotAFile { path: canonical });
        }
        return Ok(canonical);
    }
    Err(JsLoadError::NotFound {
        name: name.to_owned(),
        tried: candidates
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", "),
    })
}

/// The files that can satisfy `path`, best first.
fn candidates(path: &Path) -> Vec<PathBuf> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("js") => vec![path.to_path_buf(), path.with_extension("ts")],
        Some("mjs") => vec![path.to_path_buf(), path.with_extension("mts")],
        Some(_) => vec![path.to_path_buf()],
        None => ["ts", "js", "mjs"]
            .iter()
            .map(|ext| {
                let mut name = OsString::from(path);
                name.push(".");
                name.push(ext);
                PathBuf::from(name)
            })
            .collect(),
    }
}

/// `path` with `.` and `..` folded away without touching the file system. A `..` at the root is dropped,
/// which cannot make a path leave the root: the caller compares against it afterwards.
fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::path::{Path, PathBuf};

    use tempfile::TempDir;

    use super::{candidates, normalise, resolve_file};
    use crate::error::JsLoadError;

    /// A canonical temp directory with a `root` inside it and a sibling `outside` directory.
    struct Tree {
        _dir: TempDir,
        root: PathBuf,
        outside: PathBuf,
    }

    fn tree() -> Tree {
        let dir = tempfile::tempdir().expect("a temp dir");
        let base = fs::canonicalize(dir.path()).expect("the temp dir exists");
        let root = base.join("root");
        let outside = base.join("outside");
        fs::create_dir_all(root.join("sub")).expect("root/sub");
        fs::create_dir_all(&outside).expect("outside");
        Tree {
            _dir: dir,
            root,
            outside,
        }
    }

    fn write(path: &Path, text: &str) {
        fs::write(path, text).expect("the file is written");
    }

    fn base_of(root: &Path, file: &str) -> String {
        root.join(file).to_str().expect("utf-8 temp path").to_owned()
    }

    #[test]
    fn relative_ts_resolves_to_its_canonical_path() {
        let t = tree();
        write(&t.root.join("a.ts"), "");
        write(&t.root.join("x.ts"), "");
        let found = resolve_file(&t.root, &base_of(&t.root, "a.ts"), "./x.ts").expect("x.ts is under the root");
        assert_eq!(found, t.root.join("x.ts"));
    }

    #[test]
    fn js_specifier_falls_back_to_ts_but_prefers_an_existing_js() {
        let t = tree();
        write(&t.root.join("a.ts"), "");
        write(&t.root.join("only_ts.ts"), "");
        write(&t.root.join("both.ts"), "");
        write(&t.root.join("both.js"), "");
        let base = base_of(&t.root, "a.ts");
        assert_eq!(
            resolve_file(&t.root, &base, "./only_ts.js").expect("falls back to .ts"),
            t.root.join("only_ts.ts")
        );
        assert_eq!(
            resolve_file(&t.root, &base, "./both.js").expect("the .js exists"),
            t.root.join("both.js")
        );
    }

    #[test]
    fn extensionless_specifier_tries_ts_then_js() {
        let t = tree();
        write(&t.root.join("a.ts"), "");
        write(&t.root.join("m.js"), "");
        let base = base_of(&t.root, "a.ts");
        assert_eq!(resolve_file(&t.root, &base, "./m").expect("m.js"), t.root.join("m.js"));
        write(&t.root.join("m.ts"), "");
        assert_eq!(resolve_file(&t.root, &base, "./m").expect("m.ts"), t.root.join("m.ts"));
    }

    #[test]
    fn parent_that_stays_inside_the_root_is_allowed() {
        let t = tree();
        write(&t.root.join("shared.ts"), "");
        write(&t.root.join("sub/a.ts"), "");
        let found = resolve_file(&t.root, &base_of(&t.root, "sub/a.ts"), "../shared.ts").expect("inside the root");
        assert_eq!(found, t.root.join("shared.ts"));
    }

    #[test]
    fn parent_that_climbs_out_of_the_root_is_rejected() {
        let t = tree();
        write(&t.root.join("a.ts"), "");
        write(&t.outside.join("secret.ts"), "");
        let error = resolve_file(&t.root, &base_of(&t.root, "a.ts"), "../outside/secret.ts")
            .expect_err("the path leaves the root");
        assert!(matches!(error, JsLoadError::Escape { .. }), "{error}");
    }

    #[test]
    fn symlinked_file_pointing_out_of_the_root_is_rejected() {
        let t = tree();
        write(&t.root.join("a.ts"), "");
        write(&t.outside.join("secret.ts"), "");
        symlink(t.outside.join("secret.ts"), t.root.join("link.ts")).expect("a symlink");
        let error = resolve_file(&t.root, &base_of(&t.root, "a.ts"), "./link.ts").expect_err("the link points out");
        assert!(matches!(error, JsLoadError::Escape { .. }), "{error}");
    }

    #[test]
    fn symlinked_directory_pointing_out_of_the_root_is_rejected() {
        let t = tree();
        write(&t.root.join("a.ts"), "");
        write(&t.outside.join("secret.ts"), "");
        symlink(&t.outside, t.root.join("dir")).expect("a symlink");
        let error = resolve_file(&t.root, &base_of(&t.root, "a.ts"), "./dir/secret.ts")
            .expect_err("the directory link points out");
        assert!(matches!(error, JsLoadError::Escape { .. }), "{error}");
    }

    #[test]
    fn symlink_that_stays_inside_the_root_is_allowed() {
        let t = tree();
        write(&t.root.join("a.ts"), "");
        write(&t.root.join("real.ts"), "");
        symlink(t.root.join("real.ts"), t.root.join("link.ts")).expect("a symlink");
        let found = resolve_file(&t.root, &base_of(&t.root, "a.ts"), "./link.ts").expect("inside the root");
        assert_eq!(found, t.root.join("real.ts"));
    }

    #[test]
    fn absolute_import_is_rejected_from_a_module_but_allowed_for_the_host_entry() {
        let t = tree();
        write(&t.root.join("a.ts"), "");
        write(&t.root.join("x.ts"), "");
        let absolute = t.root.join("x.ts");
        let name = absolute.to_str().expect("utf-8 temp path");
        let error = resolve_file(&t.root, &base_of(&t.root, "a.ts"), name)
            .expect_err("a module may not import by absolute path");
        assert!(matches!(error, JsLoadError::Unresolvable { .. }), "{error}");
        assert_eq!(resolve_file(&t.root, "", name).expect("the host entry"), absolute);
        let outside = t.outside.join("x.ts");
        write(&outside, "");
        let error = resolve_file(&t.root, "", outside.to_str().expect("utf-8")).expect_err("outside the root");
        assert!(matches!(error, JsLoadError::Escape { .. }), "{error}");
    }

    #[test]
    fn missing_file_names_what_was_tried() {
        let t = tree();
        write(&t.root.join("a.ts"), "");
        let error = resolve_file(&t.root, &base_of(&t.root, "a.ts"), "./nope.js").expect_err("no such file");
        let JsLoadError::NotFound { tried, .. } = &error else {
            panic!("expected NotFound, got {error}");
        };
        assert!(tried.contains("nope.js") && tried.contains("nope.ts"), "{tried}");
    }

    #[test]
    fn relative_import_from_an_embedded_module_is_rejected() {
        let t = tree();
        let error = resolve_file(&t.root, "gnoblin", "./x.ts").expect_err("gnoblin is not a file");
        assert!(matches!(error, JsLoadError::NoFileBase { .. }), "{error}");
    }

    #[test]
    fn normalise_folds_dots_and_never_climbs_past_the_root_directory() {
        assert_eq!(normalise(Path::new("/a/./b/../c")), Path::new("/a/c"));
        assert_eq!(normalise(Path::new("/../..")), Path::new("/"));
    }

    #[test]
    fn candidates_follow_the_extension() {
        assert_eq!(
            candidates(Path::new("/r/x.js")),
            [Path::new("/r/x.js"), Path::new("/r/x.ts")]
        );
        assert_eq!(
            candidates(Path::new("/r/x")),
            [Path::new("/r/x.ts"), Path::new("/r/x.js"), Path::new("/r/x.mjs")]
        );
        assert_eq!(candidates(Path::new("/r/x.ts")), [Path::new("/r/x.ts")]);
    }
}
