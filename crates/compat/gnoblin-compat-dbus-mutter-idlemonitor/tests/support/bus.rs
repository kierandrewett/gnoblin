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
