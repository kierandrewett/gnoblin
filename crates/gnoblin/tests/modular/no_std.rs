//! Example 17, the script halves: a prefix with no std library. A core-only script shell runs, and a script that imports std is
//! refused with the text every runtime gives, which says where it looked.

use std::fs;

use crate::support::Mapper;
use crate::support::native_shell::{Prefix, Start, start_script, start_script_with, wait_for_window};
use crate::{near, repo_root, screen_of, serving};

/// A stock library of an installed prefix that has no std in it: `gnoblin-core` alone.
pub(crate) fn core_only_library() -> tempfile::TempDir {
    let library = tempfile::tempdir().expect("a directory for the library");
    let core = library.path().join("gnoblin-core");
    copy_tree(&repo_root().join("data/js/gnoblin-core"), &core);
    library
}

fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    fs::create_dir_all(to).expect("a directory");
    for entry in fs::read_dir(from).expect("a directory to copy") {
        let path = entry.expect("an entry").path();
        let target = to.join(path.file_name().expect("a name"));
        if path.is_dir() {
            copy_tree(&path, &target);
        } else {
            fs::copy(&path, &target).expect("a file");
        }
    }
}

const CORE_ONLY: &str = r#"import { CollectionListener, gnoblin } from "gnoblin-core";

class Tiler extends CollectionListener {
    onAdded() {
        tile();
    }
    onRemoved() {
        tile();
    }
}

function tile() {
    const output = gnoblin.outputs.at(0);
    const windows = [...gnoblin.clients];
    const width = output.width / windows.length;
    windows.forEach((client, at) => {
        client.bounds = { x: output.x + at * width, y: output.y, width, height: output.height };
    });
}
gnoblin.clients.addListener(new Tiler());
"#;

#[test]
fn example_17_a_js_shell_that_imports_only_gnoblin_core_runs_when_no_std_is_installed() {
    let library = core_only_library();
    let shell = tempfile::tempdir().expect("a directory for the shell");
    fs::write(shell.path().join("shell.js"), CORE_ONLY).expect("the shell");
    let stock = library.path().display().to_string();
    let mut session = start_script(
        Start::Compositor,
        &shell.path().join("shell.js"),
        &[("GNOBLIN_JS_STOCK_DIR", &stock)],
    );
    serving(&mut session);
    let (width, height) = screen_of(&session);
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("a", "A");
    mapper.map_window("b", "B");
    wait_for_window(
        &mut session,
        "the core-only shell tiles the first window",
        "a",
        |window| near(window.x, 0.0) && near(window.width, width / 2.0) && near(window.height, height),
    );
    wait_for_window(&mut session, "and the second", "b", |window| {
        near(window.x, width / 2.0) && near(window.width, width / 2.0)
    });
    let files = session.mapped_files();
    assert!(
        !files
            .iter()
            .any(|path| path.contains("gnoblin-std") || path.contains("gnoblin_std")),
        "no std library is mapped: {files:#?}"
    );
}

#[test]
fn example_17_a_js_shell_that_imports_gnoblin_std_is_refused_with_the_loader_text_and_the_session_goes_on() {
    let library = core_only_library();
    let shell = tempfile::tempdir().expect("a directory for the shell");
    fs::write(
        shell.path().join("shell.ts"),
        "import { gnoblin } from \"gnoblin-core\";\nimport { Workspace } from \"gnoblin-std\";\nvoid Workspace; void gnoblin;\n",
    )
    .expect("the shell");
    let stock = library.path().display().to_string();
    let mut session = start_script(
        Start::Compositor,
        &shell.path().join("shell.ts"),
        &[("GNOBLIN_JS_STOCK_DIR", &stock)],
    );
    session.wait_serving();
    // The loader's text: the package, that it is not installed, and where it looked.
    let reason = session.wait_log("the shell is refused with the loader's text", |line| {
        line.contains("gnoblin-std is not installed (looked in: ")
    });
    assert!(reason.text.contains(&stock), "it says where it looked: {}", reason.text);
    // The session does not end for it: the compositor's own window management has the windows.
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("a", "A");
    wait_for_window(&mut session, "a window maps without a shell", "a", |_| true);
    assert!(session.exited().is_none(), "{}", session.log_text());
}

#[test]
fn example_17_a_python_shell_that_imports_gnoblin_std_is_told_why_and_the_rest_of_the_shell_runs() {
    // The module looks in the system's library directories and beside the core, which the prefix below has none of: a machine that
    // has a std library installed is not the prefix this example is about.
    if ["/usr/lib64", "/usr/lib", "/usr/local/lib", "/lib64", "/lib"]
        .iter()
        .any(|dir| std::path::Path::new(dir).join("libgnoblin-std.so.1").exists())
    {
        eprintln!("skipped: a libgnoblin-std.so.1 is installed on this machine");
        return;
    }
    let shell = tempfile::tempdir().expect("a directory for the shell");
    fs::write(
        shell.path().join("shell.py"),
        r#"from gnoblin import gnoblin

try:
    from gnoblin.std import Workspace
except ImportError as error:
    # Written beside the note and renamed into place: the test reads the note as soon as it exists, and `>` creates it empty first.
    gnoblin.spawn(["sh", "-c", "echo '%s' > \"$GNOBLIN_FIXTURE_DIR/std-error.part\" && mv \"$GNOBLIN_FIXTURE_DIR/std-error.part\" \"$GNOBLIN_FIXTURE_DIR/std-error\"" % error])


@gnoblin.on_client_created
def place(window):
    window.width = 321
"#,
    )
    .expect("the shell");
    // The build tree stages `libgnoblin-std.so.1` beside the core, and the std library is looked for there: an installed prefix with
    // the launcher and the core and no std beside them is the machine this example is about.
    let prefix = Prefix::new();
    let mut session = start_script_with(
        Some(&prefix.binary()),
        Start::Compositor,
        &shell.path().join("shell.py"),
        &[],
    );
    serving(&mut session);
    let text = session.wait_noted("std-error");
    assert!(
        text.starts_with("gnoblin-std is not installed (looked for libgnoblin-std.so.1 in "),
        "{text}"
    );
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("a", "A");
    wait_for_window(&mut session, "the core-only part of the shell ran", "a", |window| {
        near(window.width, 321.0)
    });
}
