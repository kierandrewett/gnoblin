//! Resolves the active OpenXR runtime manifest.
//!
//! The rules follow the OpenXR loader's runtime discovery, as specified by the
//! Khronos OpenXR SDK (`specification/loader/runtime.adoc`, CC-BY-4.0;
//! <https://github.com/KhronosGroup/OpenXR-SDK-Source/blob/main/specification/loader/runtime.adoc>,
//! also in the Loader Design and Operation document
//! <https://registry.khronos.org/OpenXR/specs/1.0/loader.html>).
//! Relative XDG paths are ignored, as the XDG Base Directory Specification
//! requires (<https://specifications.freedesktop.org/basedir-spec/basedir-spec-latest.html>).
//!
//! 1. If `XR_RUNTIME_JSON` is set, only that file is read. A missing or invalid
//!    file is an error; the search does not continue.
//! 2. Otherwise each directory is searched in order: `$XDG_CONFIG_HOME` (default
//!    `$HOME/.config`), each absolute entry of `$XDG_CONFIG_DIRS`, then the global
//!    `/etc/xdg`. In each directory the loader reads
//!    `openxr/1/active_runtime.<arch>.json` and then `openxr/1/active_runtime.json`.
//!    A manifest that exists but cannot be used is skipped and recorded.
//!
//! A `library_path` that is relative and contains `/` is resolved against the
//! manifest's directory. A bare name is left for the system library search.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// What discovery reads from the process environment. Built with
/// [`Environment::from_process`]; tests construct it directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Environment {
    /// `XR_RUNTIME_JSON`. Unset or empty means no override.
    pub xr_runtime_json: Option<PathBuf>,
    /// `XDG_CONFIG_HOME`. Unset or empty means the default `$HOME/.config`.
    pub xdg_config_home: Option<PathBuf>,
    /// `HOME`.
    pub home: Option<PathBuf>,
    /// Raw `XDG_CONFIG_DIRS`. Unset means no extra directories: the basedir
    /// default `/etc/xdg` is always searched last as `global_config_dir`.
    pub xdg_config_dirs: Option<OsString>,
    /// The global configuration directory, `/etc/xdg`, searched last.
    pub global_config_dir: PathBuf,
    /// The loader's architecture name: `x86_64`, `aarch64` or `i686`.
    pub arch: String,
}

impl Environment {
    /// Reads the environment of the running process.
    pub fn from_process() -> Self {
        Self {
            xr_runtime_json: non_empty_var("XR_RUNTIME_JSON").map(PathBuf::from),
            xdg_config_home: non_empty_var("XDG_CONFIG_HOME").map(PathBuf::from),
            home: non_empty_var("HOME").map(PathBuf::from),
            xdg_config_dirs: non_empty_var("XDG_CONFIG_DIRS"),
            global_config_dir: PathBuf::from("/etc/xdg"),
            arch: loader_arch(std::env::consts::ARCH).to_owned(),
        }
    }
}

fn non_empty_var(name: &str) -> Option<OsString> {
    std::env::var_os(name).filter(|value| !value.is_empty())
}

/// Maps the Rust architecture name to the loader's file name component.
fn loader_arch(rust_arch: &'static str) -> &'static str {
    match rust_arch {
        "x86" => "i686",
        other => other,
    }
}

/// Why a manifest could not be used.
#[derive(Debug)]
pub enum ManifestError {
    /// The file could not be read.
    Read(io::Error),
    /// The file is not valid JSON.
    Json(serde_json::Error),
    /// A required field is absent or not a string.
    Missing(&'static str),
    /// `file_format_version` is not major version 1.
    UnsupportedVersion(String),
}

/// Why no runtime was found.
#[derive(Debug)]
pub enum LookupError {
    /// `XR_RUNTIME_JSON` names a file that cannot be used.
    Override { path: PathBuf, error: ManifestError },
    /// No usable manifest exists in any searched location.
    NotFound {
        /// Every manifest path that was checked, in search order.
        searched: Vec<PathBuf>,
        /// Manifests that exist but could not be used, with the reason.
        skipped: Vec<(PathBuf, ManifestError)>,
    },
}

/// Where the active runtime was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `XR_RUNTIME_JSON`.
    EnvOverride,
    /// `$XDG_CONFIG_HOME/openxr/1/`, or `$HOME/.config/openxr/1/`.
    UserConfig,
    /// A directory listed in `XDG_CONFIG_DIRS`.
    ConfigDir,
    /// `/etc/xdg/openxr/1/`.
    Global,
}

/// The active OpenXR runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Runtime {
    /// The manifest's `runtime.name`, if it has one.
    pub name: Option<String>,
    /// The runtime library, as written in the manifest but resolved for relative paths.
    pub library_path: PathBuf,
    /// Which search location produced the manifest.
    pub source: Source,
    /// The manifest file that was used.
    pub manifest_path: PathBuf,
}

/// Finds the active runtime for `env`, following the discovery order in the module docs.
pub fn find_active_runtime(env: &Environment) -> Result<Runtime, LookupError> {
    if let Some(path) = &env.xr_runtime_json {
        return read_manifest(path, Source::EnvOverride).map_err(|error| LookupError::Override {
            path: path.clone(),
            error,
        });
    }

    let mut searched = Vec::new();
    let mut skipped = Vec::new();
    for (dir, source) in search_dirs(env) {
        for name in manifest_names(&env.arch) {
            let path = dir.join("openxr/1").join(name);
            searched.push(path.clone());
            match read_manifest(&path, source) {
                Ok(runtime) => return Ok(runtime),
                Err(ManifestError::Read(error)) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => skipped.push((path, error)),
            }
        }
    }
    Err(LookupError::NotFound { searched, skipped })
}

/// The directories to search, in order, with the source each one stands for.
fn search_dirs(env: &Environment) -> Vec<(PathBuf, Source)> {
    let mut dirs = Vec::new();
    let user_dir = match &env.xdg_config_home {
        Some(path) if path.is_absolute() => Some(path.clone()),
        _ => env.home.as_ref().map(|home| home.join(".config")),
    };
    if let Some(dir) = user_dir {
        dirs.push((dir, Source::UserConfig));
    }
    if let Some(list) = &env.xdg_config_dirs {
        dirs.extend(
            std::env::split_paths(list)
                .filter(|dir| dir.is_absolute())
                .map(|dir| (dir, Source::ConfigDir)),
        );
    }
    dirs.push((env.global_config_dir.clone(), Source::Global));
    dirs
}

/// The two manifest file names the loader checks in each directory, arch-specific first.
fn manifest_names(arch: &str) -> [String; 2] {
    [format!("active_runtime.{arch}.json"), "active_runtime.json".to_owned()]
}

/// Reads and validates one manifest. Only fields this crate uses are checked.
fn read_manifest(path: &Path, source: Source) -> Result<Runtime, ManifestError> {
    let text = fs::read_to_string(path).map_err(ManifestError::Read)?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(ManifestError::Json)?;

    let version = value
        .get("file_format_version")
        .and_then(serde_json::Value::as_str)
        .ok_or(ManifestError::Missing("file_format_version"))?;
    let major = version.split('.').next().and_then(|part| part.parse::<u64>().ok());
    if major != Some(1) {
        return Err(ManifestError::UnsupportedVersion(version.to_owned()));
    }

    let runtime = value.get("runtime").ok_or(ManifestError::Missing("runtime"))?;
    let library_path = runtime
        .get("library_path")
        .and_then(serde_json::Value::as_str)
        .filter(|path| !path.is_empty())
        .ok_or(ManifestError::Missing("runtime.library_path"))?;
    let name = runtime
        .get("name")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);

    let manifest_dir = path.parent().unwrap_or_else(|| Path::new(""));
    Ok(Runtime {
        name,
        library_path: resolve_library_path(library_path, manifest_dir),
        source,
        manifest_path: path.to_path_buf(),
    })
}

fn resolve_library_path(raw: &str, manifest_dir: &Path) -> PathBuf {
    let path = Path::new(raw);
    if path.is_absolute() || !raw.contains('/') {
        path.to_path_buf()
    } else {
        manifest_dir.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{TempDir, write_manifest};

    fn environment(root: &TempDir) -> Environment {
        Environment {
            xr_runtime_json: None,
            xdg_config_home: Some(root.join("xdg-home")),
            home: Some(root.join("home")),
            xdg_config_dirs: None,
            global_config_dir: root.join("global"),
            arch: "x86_64".to_owned(),
        }
    }

    fn user_manifest(root: &TempDir, file: &str) -> PathBuf {
        root.join(&format!("xdg-home/openxr/1/{file}"))
    }

    #[test]
    fn env_override_wins_over_every_config_dir() {
        let root = TempDir::new("override-wins");
        let override_path = root.join("override.json");
        write_manifest(&override_path, "/opt/override/libopenxr.so");
        write_manifest(&user_manifest(&root, "active_runtime.json"), "/opt/user/libopenxr.so");
        let env = Environment {
            xr_runtime_json: Some(override_path.clone()),
            ..environment(&root)
        };

        let runtime = find_active_runtime(&env).expect("override resolves");

        assert_eq!(runtime.library_path, PathBuf::from("/opt/override/libopenxr.so"));
        assert_eq!(runtime.source, Source::EnvOverride);
        assert_eq!(runtime.manifest_path, override_path);
    }

    #[test]
    fn env_override_missing_file_is_an_error_without_fallthrough() {
        let root = TempDir::new("override-missing");
        write_manifest(&user_manifest(&root, "active_runtime.json"), "/opt/user/libopenxr.so");
        let env = Environment {
            xr_runtime_json: Some(root.join("absent.json")),
            ..environment(&root)
        };

        match find_active_runtime(&env) {
            Err(LookupError::Override {
                error: ManifestError::Read(_),
                ..
            }) => {}
            other => panic!("expected override read error, got {other:?}"),
        }
    }

    #[test]
    fn env_override_malformed_json_is_an_error_without_fallthrough() {
        let root = TempDir::new("override-malformed");
        let override_path = root.join("override.json");
        crate::testing::write_file(&override_path, "{not json");
        write_manifest(&user_manifest(&root, "active_runtime.json"), "/opt/user/libopenxr.so");
        let env = Environment {
            xr_runtime_json: Some(override_path),
            ..environment(&root)
        };

        match find_active_runtime(&env) {
            Err(LookupError::Override {
                error: ManifestError::Json(_),
                ..
            }) => {}
            other => panic!("expected override JSON error, got {other:?}"),
        }
    }

    #[test]
    fn user_config_home_is_searched_before_config_dirs() {
        let root = TempDir::new("home-before-dirs");
        write_manifest(&user_manifest(&root, "active_runtime.json"), "/opt/user/libopenxr.so");
        write_manifest(&root.join("dir/openxr/1/active_runtime.json"), "/opt/dir/libopenxr.so");
        let env = Environment {
            xdg_config_dirs: Some(root.join("dir").into_os_string()),
            ..environment(&root)
        };

        let runtime = find_active_runtime(&env).expect("resolves");

        assert_eq!(runtime.source, Source::UserConfig);
        assert_eq!(runtime.library_path, PathBuf::from("/opt/user/libopenxr.so"));
    }

    #[test]
    fn config_dirs_are_searched_in_order() {
        let root = TempDir::new("dirs-order");
        write_manifest(
            &root.join("first/openxr/1/active_runtime.json"),
            "/opt/first/libopenxr.so",
        );
        write_manifest(
            &root.join("second/openxr/1/active_runtime.json"),
            "/opt/second/libopenxr.so",
        );
        let dirs = std::env::join_paths([root.join("first"), root.join("second")]).expect("join");
        let env = Environment {
            xdg_config_dirs: Some(dirs),
            ..environment(&root)
        };

        let runtime = find_active_runtime(&env).expect("resolves");

        assert_eq!(runtime.source, Source::ConfigDir);
        assert_eq!(runtime.manifest_path, root.join("first/openxr/1/active_runtime.json"));
    }

    #[test]
    fn global_dir_is_the_last_resort() {
        let root = TempDir::new("global-last");
        write_manifest(
            &root.join("global/openxr/1/active_runtime.json"),
            "/opt/global/libopenxr.so",
        );
        let env = Environment {
            global_config_dir: root.join("global"),
            ..environment(&root)
        };

        let runtime = find_active_runtime(&env).expect("resolves");
        assert_eq!(runtime.source, Source::Global);

        write_manifest(&root.join("dir/openxr/1/active_runtime.json"), "/opt/dir/libopenxr.so");
        let env = Environment {
            xdg_config_dirs: Some(root.join("dir").into_os_string()),
            ..environment(&root)
        };
        let runtime = find_active_runtime(&env).expect("resolves");
        assert_eq!(runtime.source, Source::ConfigDir, "a config dir beats the global dir");
    }

    #[test]
    fn arch_specific_file_wins_over_generic_in_the_same_directory() {
        let root = TempDir::new("arch-first");
        write_manifest(&user_manifest(&root, "active_runtime.json"), "generic.so");
        write_manifest(&user_manifest(&root, "active_runtime.x86_64.json"), "arch.so");
        let env = environment(&root);

        let runtime = find_active_runtime(&env).expect("resolves");

        assert_eq!(runtime.library_path, PathBuf::from("arch.so"));
        assert_eq!(
            runtime.manifest_path,
            user_manifest(&root, "active_runtime.x86_64.json")
        );
    }

    #[test]
    fn manifest_for_another_arch_is_not_used() {
        let root = TempDir::new("other-arch");
        write_manifest(&user_manifest(&root, "active_runtime.aarch64.json"), "arm.so");
        let env = environment(&root);

        match find_active_runtime(&env) {
            Err(LookupError::NotFound { .. }) => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn unset_config_home_defaults_to_home_dot_config() {
        let root = TempDir::new("home-default");
        write_manifest(
            &root.join("home/.config/openxr/1/active_runtime.json"),
            "/opt/home/libopenxr.so",
        );
        let env = Environment {
            xdg_config_home: None,
            ..environment(&root)
        };

        let runtime = find_active_runtime(&env).expect("resolves");

        assert_eq!(runtime.source, Source::UserConfig);
        assert_eq!(runtime.library_path, PathBuf::from("/opt/home/libopenxr.so"));
    }

    #[test]
    fn relative_config_home_is_ignored_and_home_dot_config_is_used() {
        let root = TempDir::new("relative-home");
        write_manifest(
            &root.join("home/.config/openxr/1/active_runtime.json"),
            "/opt/home/libopenxr.so",
        );
        let env = Environment {
            xdg_config_home: Some(PathBuf::from("relative-home")),
            ..environment(&root)
        };

        let runtime = find_active_runtime(&env).expect("resolves");

        assert_eq!(runtime.library_path, PathBuf::from("/opt/home/libopenxr.so"));
    }

    #[test]
    fn relative_config_dirs_entry_is_ignored() {
        let root = TempDir::new("relative-dirs");
        let env = Environment {
            xdg_config_dirs: Some("relative-dir".into()),
            ..environment(&root)
        };

        match find_active_runtime(&env) {
            Err(LookupError::NotFound { searched, .. }) => {
                assert!(
                    searched.iter().all(|path| !path.starts_with("relative-dir")),
                    "relative entry must not be searched: {searched:?}"
                );
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn no_manifest_anywhere_is_not_found_and_lists_the_searched_paths() {
        let root = TempDir::new("missing");
        let env = environment(&root);

        match find_active_runtime(&env) {
            Err(LookupError::NotFound { searched, skipped }) => {
                assert!(searched.contains(&user_manifest(&root, "active_runtime.json")));
                assert!(searched.contains(&root.join("global/openxr/1/active_runtime.json")));
                assert!(skipped.is_empty());
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn malformed_manifest_is_skipped_and_a_later_location_is_used() {
        let root = TempDir::new("malformed-skipped");
        crate::testing::write_file(&user_manifest(&root, "active_runtime.json"), "{");
        write_manifest(
            &root.join("global/openxr/1/active_runtime.json"),
            "/opt/global/libopenxr.so",
        );
        let env = environment(&root);

        let runtime = find_active_runtime(&env).expect("resolves");

        assert_eq!(runtime.source, Source::Global);
    }

    #[test]
    fn unsupported_major_version_is_skipped() {
        let root = TempDir::new("version");
        crate::testing::write_file(
            &user_manifest(&root, "active_runtime.json"),
            r#"{"file_format_version":"2.0.0","runtime":{"library_path":"/opt/v2.so"}}"#,
        );
        write_manifest(
            &root.join("global/openxr/1/active_runtime.json"),
            "/opt/global/libopenxr.so",
        );
        let env = environment(&root);

        let runtime = find_active_runtime(&env).expect("resolves");

        assert_eq!(runtime.source, Source::Global);
    }

    #[test]
    fn manifest_without_library_path_is_reported_as_skipped() {
        let root = TempDir::new("no-library");
        crate::testing::write_file(
            &user_manifest(&root, "active_runtime.json"),
            r#"{"file_format_version":"1.0.0","runtime":{"name":"x"}}"#,
        );
        let env = environment(&root);

        match find_active_runtime(&env) {
            Err(LookupError::NotFound { skipped, .. }) => {
                assert_eq!(skipped.len(), 1);
                assert!(matches!(skipped[0].1, ManifestError::Missing("runtime.library_path")));
            }
            other => panic!("expected NotFound with a skipped entry, got {other:?}"),
        }
    }

    #[test]
    fn runtime_name_is_optional() {
        let root = TempDir::new("no-name");
        crate::testing::write_file(
            &user_manifest(&root, "active_runtime.json"),
            r#"{"file_format_version":"1.0.0","runtime":{"library_path":"/opt/x.so"}}"#,
        );

        let runtime = find_active_runtime(&environment(&root)).expect("resolves");

        assert_eq!(runtime.name, None);
    }

    #[test]
    fn relative_library_path_is_resolved_against_the_manifest_directory() {
        let root = TempDir::new("relative-lib");
        write_manifest(&user_manifest(&root, "active_runtime.json"), "lib/libopenxr_monado.so");

        let runtime = find_active_runtime(&environment(&root)).expect("resolves");

        assert_eq!(
            runtime.library_path,
            root.join("xdg-home/openxr/1/lib/libopenxr_monado.so")
        );
    }

    #[test]
    fn bare_library_name_is_left_for_the_system_search() {
        let root = TempDir::new("bare-lib");
        write_manifest(&user_manifest(&root, "active_runtime.json"), "libopenxr_monado.so");

        let runtime = find_active_runtime(&environment(&root)).expect("resolves");

        assert_eq!(runtime.library_path, PathBuf::from("libopenxr_monado.so"));
    }

    #[test]
    fn absolute_library_path_is_kept() {
        let root = TempDir::new("absolute-lib");
        write_manifest(&user_manifest(&root, "active_runtime.json"), "/usr/lib/libopenxr.so");

        let runtime = find_active_runtime(&environment(&root)).expect("resolves");

        assert_eq!(runtime.library_path, PathBuf::from("/usr/lib/libopenxr.so"));
    }
}
