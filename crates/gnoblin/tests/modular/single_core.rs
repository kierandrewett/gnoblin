//! The duplicate-core check of spec section 10 for the script runtimes: whatever language the shell is written in, the compositor's
//! process maps exactly one copy of the core. A runtime (`libgnoblin-runtime-js.so`, `libgnoblin-runtime-python.so`) and the std library
//! a Python shell reaches are modules of the same process that bind to that one core, and a second copy of the core would be a second
//! compositor with its own state. The native shells (examples 1, 2, 6 and 9) assert the same of themselves in `native_shell.rs`,
//! `native_std.rs` and `abi_upgrade.rs`.
//!
//! Each test starts the shipped binary on a script and reads `/proc/PID/maps`: one core file, the runtime's private copy once, and
//! the std library once or not at all, as the language's design says.

use crate::support::native_shell::{Gnoblin, Start, start_script};
use crate::{copy_example, is_runtime_copy, serving};

fn core_files(session: &Gnoblin) -> Vec<String> {
    session
        .mapped_files()
        .into_iter()
        .filter(|path| {
            let name = path.rsplit('/').next().unwrap_or(path);
            name.starts_with("libgnoblin_core") || name.starts_with("libgnoblin-core")
        })
        .collect()
}

fn std_files(session: &Gnoblin) -> Vec<String> {
    session
        .mapped_files()
        .into_iter()
        .filter(|path| {
            let name = path.rsplit('/').next().unwrap_or(path);
            name.starts_with("libgnoblin_std") || name.starts_with("libgnoblin-std")
        })
        .collect()
}

/// Runs the example's entry file and asserts the mapping facts every language shares; returns the session for the ones that differ.
#[track_caller]
fn one_core(example: &str, entry: &str) -> (Gnoblin, tempfile::TempDir) {
    let dir = copy_example(example);
    let mut session = start_script(Start::Compositor, &dir.path().join(entry), &[]);
    serving(&mut session);
    let files = session.mapped_files();
    assert_eq!(
        core_files(&session).len(),
        1,
        "one mapping of the core under `gnoblin ./{example}/{entry}`: {files:#?}"
    );
    assert_eq!(
        files.iter().filter(|path| is_runtime_copy(path)).count(),
        1,
        "and one of the language runtime: {files:#?}"
    );
    assert!(std_files(&session).len() <= 1, "at most one std library: {files:#?}");
    assert!(session.exited().is_none(), "{}", session.log_text());
    (session, dir)
}

#[test]
fn a_javascript_shell_maps_one_core_and_one_runtime() {
    let (session, _dir) = one_core("04-js-esm", "shell.js");
    assert!(
        session.mapped_files().iter().all(|path| !path.contains("libpython")),
        "no Python in a JavaScript session"
    );
}

#[test]
fn a_typescript_shell_maps_one_core_and_one_runtime() {
    let (session, _dir) = one_core("05-ts-types", "shell.ts");
    assert!(
        session.mapped_files().iter().all(|path| !path.contains("libpython")),
        "no Python in a TypeScript session"
    );
}

#[test]
fn a_python_shell_maps_one_core_one_runtime_and_libpython() {
    let (session, _dir) = one_core("03-python-std", "shell.py");
    assert!(
        session.mapped_files().iter().any(|path| path.contains("libpython3")),
        "libpython is mapped by a Python shell"
    );
}
