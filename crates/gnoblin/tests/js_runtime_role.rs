//! The JavaScript runtime role against a real compositor: the twin of the Lua role's `runtime_reload.rs` and
//! `starter_config.rs` (`crates/gnoblin-compositor/tests`), and of what `gnoblin --role runtime` is to the guardian.
//!
//! The role is the shipped one, [`JsRoleRunner`]: it adopts the privileged end of a socket pair the compositor serves,
//! locates and evaluates `init.ts`, commits the document, reports to the guardian on a pipe and then serves. Nothing
//! is faked: the session is [`Compositor::run`] on a headless backend, the windows are mapped by a real
//! `wayland-client` connection, and what the compositor holds is read back through a second connection.
//!
//! The test lives in the binary's crate because that crate may name both the compositor and the role; the compositor
//! may not name the shell (`xtask deps`), so the Lua twins cannot be the same files.
//!
//! Every wait is on the state it needs, with a deadline: none of these tests sleeps for a time it guesses.

use std::io::{BufRead as _, BufReader};
use std::os::fd::IntoRawFd as _;
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use gnoblin_client::unix::UnixTransport;
use gnoblin_client::{Client, ClientError};
use gnoblin_compositor::{Compositor, Privileged, SessionConfig, StopHandle};
use gnoblin_config::SearchPaths;
use gnoblin_js::{JsRoleError, JsRoleRunner, STARTER_CONFIG};
use gnoblin_proto::report::{self, Line};
use gnoblin_proto::{Version, privilege};

mod support;
use support::Mapper;

/// How long a test waits for a state before it fails. A passing run never waits this long.
const PATIENCE: Duration = Duration::from_secs(60);

/// A headless compositor whose guardian handed it one privileged connection, for the `runtime` role.
struct Session {
    directory: tempfile::TempDir,
    stop: StopHandle,
    thread: Option<JoinHandle<()>>,
    /// Dropped before the compositor stops: its control thread reads the guardian's end until it closes.
    guardian: Option<std::os::fd::OwnedFd>,
    /// The runtime role's end of its privileged connection, taken once by [`Role::start`].
    role_end: Option<UnixStream>,
}

impl Session {
    fn start() -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        // The compositor refuses an IPC directory that is not private, as `$XDG_RUNTIME_DIR` is.
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).expect("the directory mode");
        let (guardian, compositor_control) = rustix::net::socketpair(
            rustix::net::AddressFamily::UNIX,
            rustix::net::SocketType::SEQPACKET,
            rustix::net::SocketFlags::CLOEXEC,
            None,
        )
        .expect("a control socket pair");
        let (compositor_end, role_end) = UnixStream::pair().expect("a socket pair");
        privilege::send_handover(&guardian, "runtime", &compositor_end).expect("the handover");
        let mut privileged = Privileged::declared();
        privileged.retain(|entry| entry.role == "runtime");
        let stop = StopHandle::new();
        let mut config = SessionConfig {
            ipc_socket: Some(directory.path().join("ipc.sock")),
            stop: Some(stop.clone()),
            privileged,
            guardian: Some(UnixStream::from(compositor_control)),
            ..SessionConfig::default()
        };
        config.wayland.socket_path = Some(directory.path().join("wayland.sock"));
        // The pings are off: a test thread a loaded machine does not schedule is not a hung application.
        config.wayland.check_alive_timeout = Duration::ZERO;
        let thread = std::thread::Builder::new()
            .name("compositor".to_owned())
            .spawn(move || {
                Compositor::run(config).expect("the compositor runs until it is stopped");
            })
            .expect("the session thread");
        let session = Self {
            directory,
            stop,
            thread: Some(thread),
            guardian: Some(guardian),
            role_end: Some(role_end),
        };
        let deadline = Instant::now() + PATIENCE;
        while UnixStream::connect(session.ipc()).is_err() {
            assert!(Instant::now() < deadline, "the compositor never listened");
            std::thread::sleep(Duration::from_millis(5));
        }
        session
    }

    fn ipc(&self) -> PathBuf {
        self.directory.path().join("ipc.sock")
    }

    fn wayland(&self) -> PathBuf {
        self.directory.path().join("wayland.sock")
    }

    /// A client on the ordinary socket, which the owner's uid makes an owner connection.
    fn owner(&self) -> Client<UnixTransport> {
        let (major, minor, patch) = gnoblin_proto::IPC_VERSION;
        Client::connect(
            UnixTransport::connect(&self.ipc()).expect("the session listens"),
            Version::new(major, minor, patch),
        )
        .expect("the handshake")
    }

    /// The revision the compositor holds and its document, or `None` while it holds none.
    fn accepted(&self) -> Option<(u64, serde_json::Value)> {
        let (revision, bytes) = self.owner().configuration().expect("the read")?;
        Some((revision, serde_json::from_slice(&bytes).expect("the document is JSON")))
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // The guardian's end goes first: the compositor's control thread reads until EOF.
        drop(self.guardian.take());
        self.stop.request();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// A runtime role running in a thread of this test, the lines it reported to the guardian, and the flag that stops it.
struct Role {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<(), JsRoleError>>>,
    reports: Arc<Mutex<Vec<Line>>>,
}

impl Role {
    /// Runs the shipped role over the session's privileged connection, reading `init` (`GNOBLIN_CONFIG`), with the
    /// config watcher off so only a request reloads.
    fn start(session: &mut Session, init: &Path) -> Role {
        let descriptor = session
            .role_end
            .take()
            .expect("the role's end is taken once")
            .into_raw_fd()
            .to_string();
        let (report_write, report_read) = UnixStream::pair().expect("a report pipe");
        let reported = report_write.into_raw_fd().to_string();
        let ipc = session.ipc();
        let runner = JsRoleRunner {
            env: Box::new(move |name| match name {
                privilege::FD_VARIABLE => Some(descriptor.clone()),
                report::READY_VARIABLE => Some(reported.clone()),
                gnoblin_client::SOCKET_VARIABLE => Some(ipc.display().to_string()),
                gnoblin_js::WATCH_VARIABLE => Some("0".to_owned()),
                _ => None,
            }),
            paths: SearchPaths {
                home: None,
                config_home: None,
                override_path: Some(init.to_path_buf()),
            },
        };
        let reports: Arc<Mutex<Vec<Line>>> = Arc::default();
        let sink = Arc::clone(&reports);
        std::thread::spawn(move || {
            for line in BufReader::new(report_read).lines().map_while(Result::ok) {
                sink.lock().expect("the report list").push(report::parse(&line));
            }
        });
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("runtime-role".to_owned())
            .spawn(move || runner.run(move || flag.load(Ordering::Relaxed)))
            .expect("the role thread");
        Role {
            stop,
            thread: Some(thread),
            reports,
        }
    }

    /// Whether the role has ended, while the session is still running.
    fn ended(&self) -> bool {
        self.thread.as_ref().is_some_and(JoinHandle::is_finished)
    }

    fn lines(&self) -> Vec<Line> {
        self.reports.lock().expect("the report list").clone()
    }

    /// Stops the role and returns how it ended.
    fn finish(mut self) -> Result<(), JsRoleError> {
        self.stop.store(true, Ordering::Relaxed);
        self.thread
            .take()
            .expect("the role thread")
            .join()
            .expect("the role does not panic")
    }
}

impl Drop for Role {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Waits until `check` returns a value, or fails with `what`.
#[track_caller]
fn wait_for<T>(what: &str, role: &Role, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + PATIENCE;
    loop {
        assert!(!role.ended(), "the role ended while waiting for: {what}");
        if let Some(found) = check() {
            return found;
        }
        assert!(Instant::now() < deadline, "never reached: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// `cursor.size` of the accepted document, once the revision in force is `revision`.
fn cursor_at(session: &Session, role: &Role, revision: u64) -> Option<i64> {
    assert!(!role.ended(), "the role ended while the session was running");
    let (current, document) = session.accepted()?;
    if current != revision {
        return None;
    }
    document["cursor"]["size"].as_i64()
}

fn write(path: &Path, source: &str) {
    std::fs::write(path, source).expect("a config file");
}

fn config_with_cursor(size: i64) -> String {
    format!(
        "import {{ defineConfig }} from \"gnoblin\";\nexport default defineConfig({{ cursor: {{ size: {size} }} }});\n"
    )
}

/// A config that asks for a reload itself, from a listener a real window opening raises, so the request starts inside
/// the runtime and no `gnoblinctl config reload` is involved.
const WITH_HANDLER: &str = r#"import { defineConfig, gnoblin } from "gnoblin";

defineConfig({ cursor: { size: 24 } });
gnoblin.clients.addListener({
    onAdded() {
        gnoblin.runtime.reloadConfig();
    },
});
"#;

/// A started session and role over a config directory holding `init` as `init.ts`.
fn start(init: &str) -> (Session, Role, tempfile::TempDir, PathBuf) {
    let mut session = Session::start();
    let directory = tempfile::tempdir().expect("a config directory");
    let init_path = directory.path().join("init.ts");
    write(&init_path, init);
    let role = Role::start(&mut session, &init_path);
    (session, role, directory, init_path)
}

// ---- what the role tells the guardian ---------------------------------------------------------------------------

#[test]
fn the_role_commits_the_document_then_reports_it_accepted_and_ready_with_the_revision() {
    // The guardian waits for `ready` before it starts anything that reads the document, so the commit comes first.
    let (session, role, _directory, _init) = start(&config_with_cursor(32));
    wait_for("the login configuration to commit", &role, || {
        (cursor_at(&session, &role, 1) == Some(32)).then_some(())
    });
    let lines = wait_for("the ready report", &role, || {
        let lines = role.lines();
        lines.contains(&Line::End).then_some(lines)
    });
    let accepted = lines
        .iter()
        .position(|line| *line == Line::ConfigAccepted { revision: 1 })
        .expect("the revision is reported accepted");
    let ready = lines
        .iter()
        .position(|line| *line == Line::Ready)
        .expect("ready is reported");
    assert!(accepted < ready, "{lines:?}");
    assert!(
        lines.contains(&Line::Environment {
            key: "GNOBLIN_CONFIG_REVISION".to_owned(),
            value: "1".to_owned()
        }),
        "{lines:?}"
    );
    assert!(
        !lines.iter().any(|line| matches!(line, Line::ConfigRejected { .. })),
        "{lines:?}"
    );
}

#[test]
fn autostart_entries_go_to_the_guardian_before_the_revision_that_enables_them() {
    let (session, role, _directory, _init) = start(
        "import { defineConfig } from \"gnoblin\";\n\
         export default defineConfig({ autostart: [\"waybar\", [\"foot\", \"--server\"]] });\n",
    );
    let lines = wait_for("the revision to be reported", &role, || {
        let lines = role.lines();
        lines.contains(&Line::ConfigAccepted { revision: 1 }).then_some(lines)
    });
    let entry = |name: &str, command: &[&str]| Line::Autostart {
        revision: 1,
        name: name.to_owned(),
        command: command.iter().map(|part| (*part).to_owned()).collect(),
    };
    let accepted = lines
        .iter()
        .position(|line| *line == Line::ConfigAccepted { revision: 1 })
        .expect("accepted");
    // The entries are the document's, in name order.
    assert_eq!(
        &lines[accepted - 2..accepted],
        [entry("foot", &["foot", "--server"]), entry("waybar", &["waybar"])],
        "{lines:?}"
    );
    let (_, document) = session.accepted().expect("the document is held");
    assert_eq!(
        document["autostart"]["foot"]["command"],
        serde_json::json!(["foot", "--server"])
    );
}

#[test]
fn a_config_the_validator_refuses_runs_the_stock_config_and_the_role_still_commits_and_reports_ready() {
    // An autostart entry that would start a second keyring is refused while the config loads, so the shell has no
    // config of the user's to run. It must not be left bare: the stock config runs and its document is committed.
    let (session, role, _directory, _init) = start(
        "import { defineConfig } from \"gnoblin\";\n\
         export default defineConfig({ autostart: [[\"gnome-keyring-daemon\", \"--start\"]] });\n",
    );
    let lines = wait_for("ready", &role, || {
        let lines = role.lines();
        lines.contains(&Line::End).then_some(lines)
    });
    assert!(lines.contains(&Line::ConfigAccepted { revision: 1 }), "{lines:?}");
    let (_, document) = session.accepted().expect("the document is held");
    assert!(document.get("autostart").is_none(), "{document}");
}

// ---- the twins of runtime_reload.rs ------------------------------------------------------------------------------

#[test]
fn a_script_that_reloads_the_configuration_commits_what_the_file_says_now() {
    // The failure this prevents: `gnoblin.runtime.reloadConfig()` fails from a script, so a handler cannot ask for the
    // reload an edit needs. The second revision must carry the size the file holds when the window opens, not the one
    // it held at login.
    let (session, role, _directory, init) = start(WITH_HANDLER);
    wait_for("the login configuration to commit", &role, || {
        (cursor_at(&session, &role, 1) == Some(24)).then_some(())
    });

    write(&init, &config_with_cursor(48));
    let mut client = Mapper::connect(&session.wayland());
    client.map_window("reload.test", "reload");
    wait_for("the script's reload to commit the edited file", &role, || {
        (cursor_at(&session, &role, 2) == Some(48)).then_some(())
    });
}

#[test]
fn a_script_reload_of_a_broken_file_keeps_the_accepted_configuration_and_the_role_serving() {
    // The failure this prevents: a refused reload ends the role, or replaces the accepted document. The broken edit
    // must leave revision 1 in force, the handler must survive it, and a fixed file must then commit as revision 2
    // from the same role.
    let (session, role, _directory, init) = start(WITH_HANDLER);
    wait_for("the login configuration to commit", &role, || {
        (cursor_at(&session, &role, 1) == Some(24)).then_some(())
    });

    write(
        &init,
        "import { defineConfig, gnoblin } from \"gnoblin\";\n\
         defineConfig({ cursor: { size: \"huge\" } });\n\
         gnoblin.clients.addListener({ onAdded() { gnoblin.runtime.reloadConfig(); } });\n",
    );
    let mut client = Mapper::connect(&session.wayland());
    client.map_window("reload.test", "broken");
    // The refusal is the role's answer to the script's request; the request being answered is the evidence that the
    // reload ran, so the wait is on the role reporting it.
    wait_for("the refusal to be reported to the guardian", &role, || {
        role.lines()
            .iter()
            .any(|line| matches!(line, Line::ConfigRejected { .. }))
            .then_some(())
    });
    assert_eq!(
        cursor_at(&session, &role, 1),
        Some(24),
        "the refused edit changed nothing"
    );
    assert!(!role.ended(), "the refused reload ended the role");

    write(&init, &config_with_cursor(48));
    client.map_window("reload.test", "fixed");
    wait_for("the fixed file to commit after the refusal", &role, || {
        (cursor_at(&session, &role, 2) == Some(48)).then_some(())
    });
}

#[test]
fn a_reload_the_autostart_check_refuses_keeps_the_role_serving_and_the_old_handlers() {
    // The failure this prevents: a reload that evaluates but is refused before the commit (an autostart entry that
    // would start a second keyring) ended the runtime role, so one bad edit took down the process that holds the last
    // good configuration. The refusal must leave revision 1 in force, the role serving and the handler of the accepted
    // file still answering, so the next reload, with the edit fixed, commits.
    let (session, role, _directory, init) = start(WITH_HANDLER);
    wait_for("the login configuration to commit", &role, || {
        (cursor_at(&session, &role, 1) == Some(24)).then_some(())
    });

    write(
        &init,
        "import { defineConfig } from \"gnoblin\";\n\
         defineConfig({ autostart: [[\"gnome-keyring-daemon\", \"--start\"]] });\n",
    );
    let mut client = Mapper::connect(&session.wayland());
    client.map_window("reload.test", "refused");
    wait_for("the refusal to be reported to the guardian", &role, || {
        role.lines()
            .iter()
            .any(|line| matches!(line, Line::ConfigRejected { .. }))
            .then_some(())
    });
    assert!(!role.ended(), "the refused reload ended the runtime role");
    assert_eq!(
        cursor_at(&session, &role, 1),
        Some(24),
        "the refused edit changed nothing"
    );

    write(&init, &config_with_cursor(48));
    client.map_window("reload.test", "fixed");
    wait_for("the fixed file to commit after the refusal", &role, || {
        (cursor_at(&session, &role, 2) == Some(48)).then_some(())
    });
}

// ---- `config.reload`, the owner verb `gnoblinctl config reload` uses ---------------------------------------------

#[test]
fn config_reload_commits_the_file_and_answers_with_the_revision() {
    let (session, role, _directory, init) = start(&config_with_cursor(32));
    wait_for("the login configuration to commit", &role, || {
        (cursor_at(&session, &role, 1) == Some(32)).then_some(())
    });
    write(&init, &config_with_cursor(40));
    let committed = session
        .owner()
        .reload_config()
        .expect("the reload is answered with its revision");
    assert_eq!(committed.revision, 2);
    assert_eq!(cursor_at(&session, &role, 2), Some(40));
    assert!(role.lines().contains(&Line::ConfigAccepted { revision: 2 }));
}

#[test]
fn config_reload_of_a_removed_file_is_refused_with_why_and_the_accepted_document_stays() {
    let (session, role, _directory, init) = start(&config_with_cursor(32));
    wait_for("the login configuration to commit", &role, || {
        (cursor_at(&session, &role, 1) == Some(32)).then_some(())
    });
    std::fs::remove_file(&init).expect("the file is removed");
    let error = session
        .owner()
        .reload_config()
        .expect_err("there is nothing to reload, and the caller is told so");
    let ClientError::Refused { code, message, .. } = error else {
        panic!("expected a refusal, got {error:?}");
    };
    assert_eq!(code, "config-rejected");
    assert!(message.contains("stays in force"), "{message}");
    assert_eq!(cursor_at(&session, &role, 1), Some(32), "revision 1 is still in force");
    assert!(!role.ended());
}

#[test]
fn the_role_ends_cleanly_when_the_compositor_goes_away() {
    let (mut session, role, _directory, _init) = start(&config_with_cursor(32));
    wait_for("the login configuration to commit", &role, || {
        (cursor_at(&session, &role, 1) == Some(32)).then_some(())
    });
    drop(session.guardian.take());
    session.stop.request();
    if let Some(thread) = session.thread.take() {
        thread.join().expect("the compositor stops");
    }
    let deadline = Instant::now() + PATIENCE;
    while !role.ended() {
        assert!(Instant::now() < deadline, "the role outlived its compositor");
        std::thread::sleep(Duration::from_millis(20));
    }
    role.finish()
        .expect("a lost compositor is the end of the session and not a failure");
}

// ---- the twin of starter_config.rs ------------------------------------------------------------------------------

#[test]
fn the_starter_config_loads_as_a_user_gets_it_and_commits_with_no_refusal() {
    // `gnoblinctl config init` writes this file. It must evaluate without a script error and commit without a refused
    // key: a starter that fails tells every new user the session is broken before they have changed anything.
    let (session, role, _directory, _init) = start(STARTER_CONFIG);
    let lines = wait_for("ready", &role, || {
        let lines = role.lines();
        lines.contains(&Line::End).then_some(lines)
    });
    assert!(lines.contains(&Line::ConfigAccepted { revision: 1 }), "{lines:?}");
    assert!(
        !lines.iter().any(|line| matches!(line, Line::ConfigRejected { .. })),
        "{lines:?}"
    );
    assert!(session.accepted().is_some(), "the starter's document is held");
    assert!(!role.ended());
}
