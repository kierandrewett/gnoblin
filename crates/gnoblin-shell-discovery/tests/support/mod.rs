//! A sandbox: a temporary tree and an [`Environment`] whose every location is inside it.

// Each test file uses the part it needs.
#![allow(dead_code)]

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use gnoblin_shell_discovery::Environment;
use tempfile::TempDir;

/// `<root>/home` is `$HOME`, `<root>/work` the working directory, `<root>/usr-local/share:<root>/usr/share` the data
/// directories, `<root>/prefix/share/gnoblin/default-shell` the packaged default and `<root>/lib/gnoblin/shells`
/// the library directory of launcher modules. Nothing exists until a test writes it.
pub struct Sandbox {
    _directory: TempDir,
    pub root: PathBuf,
    pub environment: Environment,
}

impl Sandbox {
    pub fn new() -> Sandbox {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let root = fs::canonicalize(directory.path()).expect("the temporary directory resolves");
        fs::create_dir_all(root.join("home")).expect("home");
        fs::create_dir_all(root.join("work")).expect("work");
        let environment = Environment {
            home: Some(root.join("home")),
            xdg_config_home: None,
            xdg_data_home: None,
            xdg_data_dirs: Some(OsString::from(format!(
                "{}:{}",
                root.join("usr-local/share").display(),
                root.join("usr/share").display()
            ))),
            gnoblin_shell: None,
            working_dir: Some(root.join("work")),
            packaged_default_dir: Some(root.join("prefix/share/gnoblin/default-shell")),
            module_dir: Some(root.join("lib/gnoblin/shells")),
        };
        Sandbox {
            _directory: directory,
            root,
            environment,
        }
    }

    /// A path under the root.
    pub fn at(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    /// Write a file under the root, with its directories, and return its path.
    pub fn write(&self, relative: &str, contents: &str) -> PathBuf {
        let path = self.at(relative);
        fs::create_dir_all(path.parent().expect("a parent")).expect("the directories");
        fs::write(&path, contents).expect("the file");
        path
    }

    /// An empty file, which is all a script or a module needs to be found.
    pub fn touch(&self, relative: &str) -> PathBuf {
        self.write(relative, "")
    }

    /// Write a file that starts with the ELF magic, whatever it is called.
    pub fn write_elf(&self, relative: &str) -> PathBuf {
        let path = self.at(relative);
        fs::create_dir_all(path.parent().expect("a parent")).expect("the directories");
        fs::write(&path, [0x7f, b'E', b'L', b'F', 2, 1, 1, 0]).expect("the file");
        path
    }

    pub fn dir(&self, relative: &str) -> PathBuf {
        let path = self.at(relative);
        fs::create_dir_all(&path).expect("the directory");
        path
    }

    /// A symlink at `link` (under the root) pointing to `target`, which is written as given.
    pub fn link(&self, link: &str, target: &Path) -> PathBuf {
        let path = self.at(link);
        fs::create_dir_all(path.parent().expect("a parent")).expect("the directories");
        symlink(target, &path).expect("the symlink");
        path
    }

    pub fn config_dir(&self) -> PathBuf {
        self.at("home/.config/gnoblin")
    }

    pub fn user_shells(&self) -> PathBuf {
        self.at("home/.local/share/gnoblin/shells")
    }

    pub fn local_shells(&self) -> PathBuf {
        self.at("usr-local/share/gnoblin/shells")
    }

    pub fn system_shells(&self) -> PathBuf {
        self.at("usr/share/gnoblin/shells")
    }

    pub fn packaged(&self) -> PathBuf {
        self.at("prefix/share/gnoblin/default-shell")
    }

    /// An error or a table with the root replaced by `<root>`, so a golden string does not carry a temporary name.
    pub fn normalized(&self, text: &str) -> String {
        text.replace(self.root.to_str().expect("a UTF-8 root"), "<root>")
    }
}
