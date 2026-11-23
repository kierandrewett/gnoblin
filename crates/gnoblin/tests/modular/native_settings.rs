//! A Rust shell and a Python shell write a setting of the compositor, and the next login reads it.
//!
//! The shells call nothing but the setter (`gnoblin.compositor.user_shaders = false`). The core's shell host commits what they wrote, the
//! compositor keeps it as its last accepted document, and the next session of the same home starts with the setting, as it does for a
//! TypeScript shell whose runtime commits the document. What is asserted is what the compositor holds (its revision and document, read over
//! the socket) and what the second login's shell reads back, and not what the first shell believes.
//!
//! A headless session has no input device, so the setting written here is the compositor's own and not a device's: the one a shell can write
//! without one. The settings of the devices take the same path (`gnoblin.input.document` holds both), and the tests of the core hold that
//! path to the document it commits for a device (`gnoblin-core-impl/tests/input.rs`).

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::time::Duration;

use gnoblin_config::{Config, Store, Value};
use serde_json::json;

use crate::support::Mapper;
use crate::support::native_shell::{Gnoblin, Home, Start, shells, start_script_in};
use crate::{save, serving};

use super::native_root::seat_asks_for_reduced_motion;
use super::shader_filter::{accepted_document, home_with_document, start_native};

/// A Python shell that reads `compositor.user_shaders`, switches it off and reads it again.
const PYTHON: &str = "from gnoblin import gnoblin as session
from gnoblin.objects.model import Gnoblin

gnoblin = Gnoblin.get()
session.log(f\"settings: before = {str(gnoblin.compositor.user_shaders).lower()}\")
gnoblin.compositor.user_shaders = False
session.log(f\"settings: after = {str(gnoblin.compositor.user_shaders).lower()}\")
";

/// The same shell in TypeScript, whose runtime commits the document it builds.
const TYPESCRIPT: &str = "import { gnoblin } from \"gnoblin-core/generated/objects\";
console.log(`settings: before = ${gnoblin.compositor.userShaders}`);
gnoblin.compositor.userShaders = false;
console.log(`settings: after = ${gnoblin.compositor.userShaders}`);
";

/// The revision the compositor holds and its document, once it holds one.
fn accepted(ipc: &Path) -> Option<(u64, serde_json::Value)> {
    let (major, minor, patch) = gnoblin_proto::IPC_VERSION;
    let transport = gnoblin_client::unix::UnixTransport::connect(ipc).ok()?;
    let mut client =
        gnoblin_client::Client::connect(transport, gnoblin_proto::Version::new(major, minor, patch)).ok()?;
    let (revision, bytes) = client.configuration().ok()??;
    Some((revision, serde_json::from_slice(&bytes).ok()?))
}

/// Whether the compositor holds `compositor.user-shaders = false`.
fn switched_off(ipc: &Path) -> bool {
    accepted_document(ipc).is_some_and(|document| document["compositor"]["user-shaders"] == false)
}

/// A home whose last accepted document is `document`, kept for the `shell.ts` of its configuration directory, which does not exist: the file
/// is nobody's, as on a first login, so a shell run by path may write it.
fn home_with_stored(document: &serde_json::Value) -> Home {
    let home = Home::new();
    let state = home.root().join("state");
    fs::create_dir(&state).expect("the state directory");
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).expect("the state mode");
    let config = Config::from_value(Value::from_json(document).expect("a document")).expect("a valid document");
    Store::new(&state)
        .save(&home.root().join("config/gnoblin/shell.ts"), &config)
        .expect("the stored document");
    home
}

/// Whether the stored document of a home with no configuration shell has `compositor.user-shaders = false`, read from the store and not from
/// the session, so it holds whether or not the session is still up.
fn stored_switched_off(home: &Home) -> bool {
    Store::new(&home.root().join("state"))
        .load(&home.root().join("config/gnoblin/shell.ts"))
        .is_ok_and(|stored| stored.config.document_json().contains("\"user-shaders\":false"))
}

/// A session over the script at `shell`, in `home`, once the shell serves.
fn script_login(home: Home, shell: &Path) -> Gnoblin {
    let mut session = start_script_in(home, None, Start::Compositor, shell, &[]);
    serving(&mut session);
    session
}

/// The lines every login's shell logs: it read the default (or what the last login left) and then what it wrote.
#[track_caller]
fn reads(session: &mut Gnoblin, before: &str) {
    session.wait_log("the shell read the setting", |line| {
        line.contains(&format!("settings: before = {before}"))
    });
    session.wait_log("the shell read what it wrote", |line| {
        line.contains("settings: after = false")
    });
}

#[test]
fn a_rust_shell_writes_compositor_user_shaders_and_the_next_login_reads_it() {
    let shell = &shells().settings;
    let mut first = start_native(Home::new(), shell);
    reads(&mut first, "true");
    let ipc = first.ipc();
    first.wait_for("the compositor accepted compositor.user-shaders = false", |_| {
        switched_off(&ipc)
    });

    let mut second = start_native(first.into_home(), shell);
    reads(&mut second, "false");
}

#[test]
fn a_python_shell_writes_compositor_user_shaders_and_the_next_login_reads_it() {
    let dir = tempfile::tempdir().expect("a directory for the shell");
    let shell = dir.path().join("shell.py");
    fs::write(&shell, PYTHON).expect("the shell");
    let mut first = script_login(Home::new(), &shell);
    reads(&mut first, "true");
    let ipc = first.ipc();
    first.wait_for("the compositor accepted compositor.user-shaders = false", |_| {
        switched_off(&ipc)
    });
    assert!(!first.log_text().contains("Traceback"), "{}", first.log_text());

    let mut second = script_login(first.into_home(), &shell);
    reads(&mut second, "false");
    assert!(!second.log_text().contains("Traceback"), "{}", second.log_text());
}

/// `~/.config/gnoblin/shell.py` is how a Python shell runs at every start. The last accepted document is kept for the file that
/// configures the session, so the shell that is the configuration file has to be a file the document can be kept for.
#[test]
fn a_python_shell_that_is_the_configuration_file_has_its_write_kept_for_the_next_login() {
    let home = Home::new();
    let shell = home.write("config/gnoblin/shell.py", PYTHON);
    let mut first = script_login(home, &shell);
    reads(&mut first, "true");
    let ipc = first.ipc();
    first.wait_for("the compositor accepted compositor.user-shaders = false", |_| {
        switched_off(&ipc)
    });

    let mut second = script_login(first.into_home(), &shell);
    reads(&mut second, "false");
}

/// The commit lays what the shell wrote over the document the session started with: a section no setting of the shell names is carried
/// over, and the login after it still has all of them.
#[test]
fn the_commit_of_a_rust_shell_keeps_the_sections_the_last_login_accepted() {
    let shell = &shells().settings;
    let home = home_with_stored(&json!({"appearance": {"reduced-motion": true}, "cursor": {"size": 32}}));
    let mut first = start_native(home, shell);
    reads(&mut first, "true");
    let ipc = first.ipc();
    first.wait_for("the compositor accepted compositor.user-shaders = false", |_| {
        switched_off(&ipc)
    });
    assert_eq!(
        accepted_document(&ipc),
        Some(json!({
            "appearance": {"reduced-motion": true},
            "cursor": {"size": 32},
            "compositor": {"user-shaders": false},
        })),
        "the sections of the last login are still in the document"
    );

    let mut second = start_native(first.into_home(), shell);
    reads(&mut second, "false");
    let ipc = second.ipc();
    second.wait_for("the seat still asks for reduced motion", |_| {
        seat_asks_for_reduced_motion(&ipc)
    });
}

/// A write that the document already says is not a commit, and the writes of one start are one: the second login of a Rust shell that
/// writes the same value makes no revision, and the first makes exactly one.
#[test]
fn a_rust_shell_makes_one_revision_for_its_write_and_none_for_the_same_write_at_the_next_login() {
    let shell = &shells().settings;
    let mut first = start_native(Home::new(), shell);
    reads(&mut first, "true");
    let ipc = first.ipc();
    first.wait_for("the compositor accepted compositor.user-shaders = false", |_| {
        switched_off(&ipc)
    });
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(accepted(&ipc).map(|(revision, _)| revision), Some(1));

    let mut second = start_native(first.into_home(), shell);
    reads(&mut second, "false");
    let ipc = second.ipc();
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        accepted(&ipc).is_none(),
        "the document of the last login already says it: {:?}",
        accepted(&ipc)
    );
}

/// The TypeScript runtime builds and commits its own document, ledger included, so the core must not commit a second one: that would be a
/// second revision, and a ledger alone would take the script's other sections out of the document.
#[test]
fn a_typescript_shell_commits_once_and_the_core_commits_nothing_more() {
    let dir = tempfile::tempdir().expect("a directory for the shell");
    let shell = dir.path().join("shell.ts");
    fs::write(&shell, TYPESCRIPT).expect("the shell");
    let mut session = script_login(Home::new(), &shell);
    reads(&mut session, "true");
    let ipc = session.ipc();
    session.wait_for("the compositor accepted compositor.user-shaders = false", |_| {
        switched_off(&ipc)
    });
    // A window and a few turns of the shell: nothing the core does after the runtime's commit is a revision.
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("foot", "Terminal");
    mapper.roundtrip();
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(
        accepted(&ipc).map(|(revision, _)| revision),
        Some(1),
        "{}",
        session.log_text()
    );
    assert!(
        !session.log_text().contains("the shell committed the settings it wrote"),
        "the core committed for a generation whose runtime owns the document:\n{}",
        session.log_text()
    );

    let mut second = script_login(session.into_home(), &shell);
    reads(&mut second, "false");
}

/// A reload that no longer writes the setting takes it back, as it does for a TypeScript shell: the compositor's document loses the key,
/// and the next login has the default.
#[test]
fn a_python_reload_whose_script_no_longer_writes_the_setting_takes_it_back() {
    let dir = tempfile::tempdir().expect("a directory for the shell");
    let shell = dir.path().join("shell.py");
    fs::write(&shell, PYTHON).expect("the shell");
    let mut session = script_login(Home::new(), &shell);
    reads(&mut session, "true");
    let ipc = session.ipc();
    session.wait_for("the compositor accepted compositor.user-shaders = false", |_| {
        switched_off(&ipc)
    });

    save(
        &shell,
        "from gnoblin import gnoblin as session\nsession.log(\"settings: reloaded and wrote nothing\")\n",
    );
    session.wait_log("the new generation ran", |line| {
        line.contains("settings: reloaded and wrote nothing")
    });
    session.wait_for("the compositor holds a document without the setting", |_| {
        accepted_document(&ipc).is_some_and(|document| document.get("compositor").is_none())
    });
    assert!(!session.log_text().contains("Traceback"), "{}", session.log_text());
}

/// A Python shell that writes the setting in the turn in which it asks to exit.
const PYTHON_EXITS: &str = "from gnoblin import gnoblin as session
from gnoblin.objects.model import Gnoblin

gnoblin = Gnoblin.get()


def leave():
    gnoblin.compositor.user_shaders = False
    session.request_exit()


gnoblin.timers.after(100, leave)
";

/// The host commits before it looks at the exit request, so a write made in the turn that asks to exit is not lost.
#[test]
fn a_write_made_in_the_turn_that_asks_to_exit_is_committed() {
    let dir = tempfile::tempdir().expect("a directory for the shell");
    let shell = dir.path().join("shell.py");
    fs::write(&shell, PYTHON_EXITS).expect("the shell");
    let mut session = script_login(Home::new(), &shell);
    session.wait_for(
        "the stored document has the write of the turn that asked to exit",
        |session| stored_switched_off(session.home()),
    );
}

/// `gnoblin --shell ./dev.so` while the configuration directory has a shell: the stored document is that shell's, and a shell run by another
/// path must not write over it. The commit is skipped, the log says so, and the stored document is byte for byte what it was.
#[test]
fn a_shell_run_by_path_does_not_write_over_the_document_of_the_shell_the_configuration_belongs_to() {
    let (home, _) = home_with_document(json!({"cursor": {"size": 32}}), "");
    let stored = Store::new(&home.root().join("state")).path();
    let before = fs::read(&stored).expect("the stored document");
    let mut session = start_native(home, &shells().settings);
    reads(&mut session, "true");
    session.wait_log("the log says the write is not kept", |line| {
        line.contains("will not be kept for this run")
    });
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(
        fs::read(&stored).expect("the stored document"),
        before,
        "the document of the configuration shell was written over:\n{}",
        session.log_text()
    );
    assert!(accepted(&session.ipc()).is_none(), "the compositor was sent a commit");
}
