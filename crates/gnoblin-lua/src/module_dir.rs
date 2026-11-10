//! Where the installed Lua modules live.
//!
//! The shipped layout modules and the shell's stock library both resolve their install directory from
//! `GNOBLIN_PREFIX`, so the resolution lives here rather than in either consumer.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Where the layout modules of a prefix are installed.
pub fn installed_module_dir(prefix: &Path) -> PathBuf {
    prefix.join("share/gnoblin/lua")
}

/// The installed prefix for `GNOBLIN_PREFIX` as given, defaulting to `/usr`.
pub fn default_prefix(value: Option<&OsStr>) -> PathBuf {
    value.map_or_else(|| PathBuf::from("/usr"), PathBuf::from)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::{Path, PathBuf};

    use super::{default_prefix, installed_module_dir};

    #[test]
    fn installed_modules_live_under_the_prefix_share_dir() {
        assert_eq!(
            installed_module_dir(Path::new("/opt/gnoblin")),
            PathBuf::from("/opt/gnoblin/share/gnoblin/lua")
        );
    }

    #[test]
    fn the_prefix_is_the_given_value_or_usr() {
        assert_eq!(
            default_prefix(Some(OsStr::new("/opt/gnoblin"))),
            PathBuf::from("/opt/gnoblin")
        );
        assert_eq!(default_prefix(None), PathBuf::from("/usr"));
    }
}
