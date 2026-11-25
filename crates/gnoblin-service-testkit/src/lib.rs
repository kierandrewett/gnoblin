//! A private message bus for one test.
//!
//! `dbus-daemon --session --nofork --print-address` writes the address of a bus with a
//! fresh socket, and the child is killed when the guard is dropped. Nothing here reads
//! `DBUS_SESSION_BUS_ADDRESS` or `DBUS_SYSTEM_BUS_ADDRESS`, so a test can never reach the
//! developer's session bus, and two tests can never share one: the address is the only
//! way a connection is told which bus to use.

use std::io::{BufRead as _, BufReader};
use std::process::{Child, Command, Stdio};

/// One private bus, killed on drop.
pub struct Bus {
    child: Child,
    address: String,
}

impl Bus {
    /// Start a bus and read its address.
    pub fn start() -> Bus {
        let mut child = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("dbus-daemon is installed");
        let stdout = child.stdout.take().expect("stdout is piped");
        let mut lines = BufReader::new(stdout).lines();
        let address = lines
            .by_ref()
            .map_while(Result::ok)
            .find(|line| line.starts_with("unix:"))
            .expect("dbus-daemon prints its address");
        Bus { child, address }
    }

    /// The address to connect to, which is the only bus a test may use.
    pub fn address(&self) -> &str {
        &self.address
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Runs `step` on a thread of its own and fails the test, naming `what`, if it has not returned within `limit`.
///
/// Two service tests never returned in CI over a private bus (the screensaver's GNOME SetActive test in workspace job
/// 139021815236, the session idle's `changes_to_other_objects_are_ignored` in the Arch container job 139077575145); the
/// screensaver's passes in about 0.3 s on the a37 box. A bus step bounded with this fails the test and says which step
/// waited, instead of holding the job until it is cancelled.
///
/// # Panics
///
/// When `step` has not returned within `limit`, or panicked.
pub fn within<T: Send + 'static>(
    what: &str,
    limit: std::time::Duration,
    step: impl FnOnce() -> T + Send + 'static,
) -> T {
    let (sender, answer) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(step());
    });
    answer
        .recv_timeout(limit)
        .unwrap_or_else(|_| panic!("{what} did not return within {limit:?}"))
}
