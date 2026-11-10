//! The stock shell VM against a real compositor session (design/node-api-refactor.md, section 7.2).
//!
//! The shell connects through the runtime role's own path: [`connect_owner_socket`] reads the
//! session's IPC socket from the environment and boots a [`ShellVm`] over the stock Lua
//! library. The tree is the compositor's: the nodes come from the session, not from a fake.

mod support;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gnoblin_lua::shell_role::{ShellRole, connect_owner_socket};
use support::Session;

/// The stock Lua library the shell VM loads (`node.lua` and the modules beside it).
fn stock_library() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/lua/gnoblin")
}

/// Connects the shell VM to `session` as the runtime role does, with the socket in the environment.
fn connect(session: &Session) -> ShellRole {
    let socket = session.socket().to_str().expect("a UTF-8 socket path").to_owned();
    let env = move |name: &str| (name == gnoblin_client::unix::SOCKET_VARIABLE).then(|| socket.clone());
    connect_owner_socket(&env, &stock_library()).expect("the shell VM attaches to the tree")
}

/// Counts of the mirror's `seat` and `screen` nodes, read from inside the VM.
fn mirror_counts(role: &mut ShellRole) -> (usize, usize) {
    let text = role
        .run(
            "probe",
            "return tostring(#root:all('Seat')) .. ' ' .. tostring(#root:all('Screen'))",
        )
        .expect("the probe runs")
        .expect("the probe returns counts");
    let mut parts = text.split(' ').map(|n| n.parse::<usize>().expect("a count"));
    (parts.next().unwrap(), parts.next().unwrap())
}

/// Ticks the VM until `done` holds, or fails after ten seconds with `what`.
fn tick_until(role: &mut ShellRole, what: &str, mut done: impl FnMut(&mut ShellRole) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut now = 0.0;
    loop {
        role.tick(now).expect("a frame runs");
        now += 0.016;
        if done(role) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{what}: seat/screen counts {:?}",
            mirror_counts(role)
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn the_shell_vm_attaches_and_its_mirror_holds_the_seat_and_a_screen() {
    let session = Session::start();
    let mut role = connect(&session);
    tick_until(&mut role, "the mirror never held one seat and one screen", |role| {
        mirror_counts(role) == (1, 1)
    });
}
