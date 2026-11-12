//! `gnoblin.runtime.reload_config()` from a script, against a real compositor and the shipped runtime role.
//!
//! The role is the one the guardian starts: [`RoleRunner`] adopts the privileged end of a socket pair the
//! compositor serves, loads the user's `init.lua`, commits it, and then serves. The script asks for the reload
//! itself, from a handler that a real window opening raises, so the request starts inside the runtime and no
//! `gnoblinctl config reload` is involved: the path `gnoblin.runtime.reload_config()` has to complete on its own.

mod support;

use std::os::fd::IntoRawFd;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use gnoblin_client::Client;
use gnoblin_client::unix::UnixTransport;
use gnoblin_config::SearchPaths;
use gnoblin_lua::shell_role::{SHELL_VM_OFF, SHELL_VM_VARIABLE};
use gnoblin_lua::{Limits, RoleError, RoleRunner};
use gnoblin_proto::privilege;
use support::{Session, wait_for};

/// A runtime role running in a thread of this test, and the flag that stops it.
struct Role {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<(), RoleError>>>,
}

impl Role {
    /// Runs the shipped role over `stream`, reading `init`.
    fn start(stream: std::os::unix::net::UnixStream, init: &Path) -> Role {
        let descriptor = stream.into_raw_fd().to_string();
        let runner = RoleRunner {
            env: Box::new(move |name| match name {
                privilege::FD_VARIABLE => Some(descriptor.clone()),
                SHELL_VM_VARIABLE => Some(SHELL_VM_OFF.to_owned()),
                _ => None,
            }),
            paths: SearchPaths {
                home: None,
                config_home: None,
                override_path: Some(init.to_path_buf()),
            },
            limits: Limits::default(),
        };
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let thread = std::thread::spawn(move || runner.run(move || flag.load(Ordering::Relaxed)));
        Role {
            stop,
            thread: Some(thread),
        }
    }

    /// Whether the role has ended, with its error if it ended in one.
    fn ended(&self) -> Option<String> {
        let thread = self.thread.as_ref()?;
        thread
            .is_finished()
            .then(|| "the role ended while the session was running".to_owned())
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

/// A client on the ordinary socket, which the owner's uid makes an owner connection.
fn owner(session: &Session) -> Client<UnixTransport> {
    Client::connect(
        UnixTransport::connect(session.socket()).expect("the session listens"),
        gnoblin_compositor::version(),
    )
    .expect("the handshake")
}

/// `cursor.size` of the accepted document, once the revision in force is `revision`.
fn accepted(session: &Session, role: &Role, revision: u64) -> Option<u32> {
    assert!(role.ended().is_none(), "{:?}", role.ended());
    let (current, document) = owner(session).configuration().expect("the read")?;
    if current != revision {
        return None;
    }
    let text = String::from_utf8(document).expect("the document is UTF-8");
    [24, 48]
        .into_iter()
        .find(|size| text.contains(&format!("\"size\":{size}")))
}

const WITH_HANDLER: &str = r#"
gnoblin.configure({ cursor = { size = 24 } })
gnoblin.on("gnoblin.window.created", function()
    gnoblin.runtime.reload_config()
end)
"#;

#[test]
fn a_script_that_reloads_the_configuration_commits_what_the_file_says_now() {
    // The failure this prevents: `gnoblin.runtime.reload_config()` fails from a script, so a
    // handler cannot ask for the reload an edit needs. The second revision must carry the size
    // the file holds when the window opens, not the one it held at login.
    let (session, mut ends) = Session::start_privileged(&["runtime"]);
    let directory = tempfile::tempdir().expect("a temporary directory");
    let init = directory.path().join("init.lua");
    std::fs::write(&init, WITH_HANDLER).expect("the first configuration");
    let (_, stream) = ends.pop().expect("the runtime's end");
    let role = Role::start(stream, &init);

    wait_for("the login configuration to commit", || {
        (accepted(&session, &role, 1) == Some(24)).then_some(())
    });

    std::fs::write(&init, "gnoblin.configure({ cursor = { size = 48 } })\n").expect("the edit");
    let mut client = support::Client::connect(session.wayland());
    let _window = client.map_window("reload.test", "reload");

    wait_for("the script's reload to commit the edited file", || {
        (accepted(&session, &role, 2) == Some(48)).then_some(())
    });
}

#[test]
fn a_script_reload_of_a_broken_file_keeps_the_accepted_configuration_and_the_role_serving() {
    // The failure this prevents: a refused reload ends the role, or replaces the accepted
    // document. The broken edit must leave revision 1 in force, the handler must survive it,
    // and a fixed file must then commit as revision 2 from the same role.
    let (session, mut ends) = Session::start_privileged(&["runtime"]);
    let directory = tempfile::tempdir().expect("a temporary directory");
    let init = directory.path().join("init.lua");
    std::fs::write(&init, WITH_HANDLER).expect("the first configuration");
    let (_, stream) = ends.pop().expect("the runtime's end");
    let role = Role::start(stream, &init);
    wait_for("the login configuration to commit", || {
        (accepted(&session, &role, 1) == Some(24)).then_some(())
    });

    std::fs::write(&init, "gnoblin.configure({ cursor = { size = \"huge\" } })\n").expect("the broken edit");
    let mut client = support::Client::connect(session.wayland());
    let _first = client.map_window("reload.test", "broken");
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert_eq!(
        accepted(&session, &role, 1),
        Some(24),
        "the refused edit changed nothing"
    );

    std::fs::write(&init, "gnoblin.configure({ cursor = { size = 48 } })\n").expect("the fixed edit");
    let _second = client.map_window("reload.test", "fixed");
    wait_for("the fixed file to commit after the refusal", || {
        (accepted(&session, &role, 2) == Some(48)).then_some(())
    });
}
