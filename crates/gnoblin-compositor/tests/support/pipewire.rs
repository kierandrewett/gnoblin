//! A private `pipewire` daemon, the recipe in `design/pipewire-capture-spike.md`.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// How long a test waits for the daemon to answer or a node to appear or go.
const TIMEOUT: Duration = Duration::from_secs(20);

/// A private `pipewire` daemon, killed with its runtime directory when dropped.
pub struct Daemon {
    child: Child,
    directory: tempfile::TempDir,
}

impl Daemon {
    /// Starts the daemon named by `GNOBLIN_PIPEWIRE_BIN` (default `pipewire`) in a runtime
    /// directory of its own, and waits for its socket.
    pub fn start() -> Self {
        let directory = tempfile::tempdir().expect("a runtime directory for the daemon");
        let binary = std::env::var_os("GNOBLIN_PIPEWIRE_BIN").unwrap_or_else(|| "pipewire".into());
        let child = Command::new(&binary)
            .env("XDG_RUNTIME_DIR", directory.path())
            .env("PIPEWIRE_RUNTIME_DIR", directory.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap_or_else(|error| panic!("start {binary:?}: {error}"));
        let daemon = Self { child, directory };
        let deadline = Instant::now() + TIMEOUT;
        while !daemon.socket().exists() {
            assert!(Instant::now() < deadline, "the daemon never made its socket");
            std::thread::sleep(Duration::from_millis(20));
        }
        daemon
    }

    /// The daemon's socket.
    pub fn socket(&self) -> PathBuf {
        self.directory.path().join("pipewire-0")
    }

    /// What the daemon says about object `id`, as `pw-dump` prints it.
    pub fn dump(&self, id: u32) -> String {
        let output = Command::new("pw-dump")
            .arg(id.to_string())
            .env("PIPEWIRE_RUNTIME_DIR", self.directory.path())
            .env("XDG_RUNTIME_DIR", self.directory.path())
            .output()
            .expect("pw-dump runs");
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// Every object the daemon holds, as `pw-dump` prints them.
    pub fn dump_all(&self) -> String {
        let output = Command::new("pw-dump")
            .env("PIPEWIRE_RUNTIME_DIR", self.directory.path())
            .env("XDG_RUNTIME_DIR", self.directory.path())
            .output()
            .expect("pw-dump runs");
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// Whether the daemon holds a stream node of the compositor's.
    pub fn holds_stream(&self) -> bool {
        self.dump_all().contains("gnoblin-capture-")
    }

    /// Waits until the stream node is there, or fails.
    #[track_caller]
    pub fn wait_until_present(&self, node: u32) {
        let deadline = Instant::now() + TIMEOUT;
        while !self.dump(node).contains("gnoblin-capture-") {
            assert!(Instant::now() < deadline, "node {node} never appeared in the daemon");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Waits until the stream node is gone, or fails.
    #[track_caller]
    pub fn wait_until_gone(&self, node: u32) {
        let deadline = Instant::now() + TIMEOUT;
        while self.dump(node).contains("gnoblin-capture-") {
            assert!(Instant::now() < deadline, "node {node} is still in the daemon");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
