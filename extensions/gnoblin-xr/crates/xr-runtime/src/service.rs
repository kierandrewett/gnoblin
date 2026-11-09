//! Detection of the OpenXR runtime service binaries on `PATH`.
//!
//! `gnoblin-xr` supervises a runtime service only when one is installed. The two
//! known services are Monado's `monado-service` and WiVRn's `wivrn-server`. Each
//! `PATH` directory is searched in turn, and within one directory Monado wins.

use std::ffi::OsStr;
use std::fs::Metadata;
use std::path::{Path, PathBuf};

/// A runtime service this crate knows how to supervise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceKind {
    /// Monado's service binary.
    MonadoService,
    /// WiVRn's server binary.
    WivrnServer,
}

impl ServiceKind {
    /// The executable file name this service is installed under.
    pub fn binary_name(self) -> &'static str {
        match self {
            Self::MonadoService => "monado-service",
            Self::WivrnServer => "wivrn-server",
        }
    }
}

/// A runtime service binary found on `PATH`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    /// Which service this binary is.
    pub kind: ServiceKind,
    /// The full path to the executable.
    pub path: PathBuf,
}

/// Candidates in preference order. Monado is first so it wins a tie within one directory.
const CANDIDATES: [ServiceKind; 2] = [ServiceKind::MonadoService, ServiceKind::WivrnServer];

/// Finds the first runtime service executable in the directories of `path`
/// (a `PATH`-style value). Returns `None` when `path` is unset or holds no service.
pub fn detect_service(path: Option<&OsStr>) -> Option<Service> {
    for dir in std::env::split_paths(path?) {
        for kind in CANDIDATES {
            let candidate = dir.join(kind.binary_name());
            if is_executable_file(&candidate) {
                return Some(Service { kind, path: candidate });
            }
        }
    }
    None
}

fn is_executable_file(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(metadata) => metadata.is_file() && has_execute_bit(&metadata),
        Err(_) => false,
    }
}

#[cfg(unix)]
fn has_execute_bit(metadata: &Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn has_execute_bit(_metadata: &Metadata) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{TempDir, write_executable, write_file};

    #[test]
    fn finds_the_first_executable_candidate_on_path() {
        let root = TempDir::new("service-found");
        write_executable(&root.join("bin-a/wivrn-server"));
        write_executable(&root.join("bin-b/monado-service"));
        let path = std::env::join_paths([root.join("bin-a"), root.join("bin-b")]).expect("join");

        let found = detect_service(Some(&path)).expect("a service is found");

        assert_eq!(found.kind, ServiceKind::WivrnServer);
        assert_eq!(found.path, root.join("bin-a/wivrn-server"));
    }

    #[test]
    fn monado_is_preferred_when_both_are_present_in_one_directory() {
        let root = TempDir::new("service-both");
        write_executable(&root.join("bin/wivrn-server"));
        write_executable(&root.join("bin/monado-service"));

        let found = detect_service(Some(&root.join("bin").into_os_string())).expect("found");

        assert_eq!(found.kind, ServiceKind::MonadoService);
    }

    #[test]
    fn non_executable_file_is_not_a_service() {
        let root = TempDir::new("service-not-exec");
        write_file(&root.join("bin/monado-service"), "");

        assert_eq!(detect_service(Some(&root.join("bin").into_os_string())), None);
    }

    #[test]
    fn directory_named_like_a_service_is_not_a_service() {
        let root = TempDir::new("service-dir");
        std::fs::create_dir_all(root.join("bin/monado-service")).expect("mkdir");

        assert_eq!(detect_service(Some(&root.join("bin").into_os_string())), None);
    }

    #[test]
    fn absent_binaries_yield_none() {
        let root = TempDir::new("service-absent");
        std::fs::create_dir_all(root.join("bin")).expect("mkdir");

        assert_eq!(detect_service(Some(&root.join("bin").into_os_string())), None);
    }

    #[test]
    fn unset_path_yields_none() {
        assert_eq!(detect_service(None), None);
    }
}
