//! Which process a window's client is, and whether it is still that process.
//!
//! A pid alone names a process only until the process is reaped: the kernel hands the
//! number out again. The start time read from `/proc` tells the two apart, since no two
//! processes share one.

use std::fs;

/// A process as a connection saw it: its pid, and the start time read with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessIdentity {
    /// The process id.
    pub pid: u32,
    /// The start time in clock ticks since boot, field 22 of `/proc/<pid>/stat`.
    pub start_time: u64,
}

impl ProcessIdentity {
    /// The identity of a live process. `None` when the pid has no readable stat.
    #[must_use]
    pub fn of(pid: u32) -> Option<Self> {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        Some(Self {
            pid,
            start_time: parse_start_time(&stat)?,
        })
    }

    /// Opens the process for a kill, when it is still the process that connected and is one a kill may name.
    ///
    /// The pidfd is opened before the identity is checked. Once it is open it names the process that holds the pid, and
    /// the check shows whether that process is the one that connected: a pid reused since then fails the check, and
    /// the signal later goes through the pidfd, so it cannot land on a process that took the pid after the check.
    ///
    /// # Errors
    ///
    /// [`KillRefusal::Unkillable`] for the compositor's own process and init, which are never a client, and
    /// [`KillRefusal::Changed`] when the process has exited or its pid now names another one.
    pub fn kill_target(self) -> Result<KillTarget, KillRefusal> {
        if self.pid == std::process::id() || self.pid == 1 {
            return Err(KillRefusal::Unkillable);
        }
        let pidfd = i32::try_from(self.pid)
            .ok()
            .and_then(rustix::process::Pid::from_raw)
            .and_then(|pid| rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty()).ok());
        match pidfd {
            Some(pidfd) if Self::of(self.pid) == Some(self) => Ok(KillTarget(pidfd)),
            _ => Err(KillRefusal::Changed),
        }
    }
}

/// Why a kill was refused before any signal was sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KillRefusal {
    /// The process is the compositor's own or init's.
    Unkillable,
    /// The process has exited, or its pid now names a different process than the one that connected.
    Changed,
}

/// A process held open by its pidfd after its identity was checked: the one signal a kill sends goes to it.
#[derive(Debug)]
pub struct KillTarget(std::os::fd::OwnedFd);

impl KillTarget {
    /// Sends SIGKILL. `false` when the process had already exited.
    #[must_use]
    pub fn kill(&self) -> bool {
        rustix::process::pidfd_send_signal(&self.0, rustix::process::Signal::KILL).is_ok()
    }
}

/// What the compositor knows about the process behind a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowProcess {
    /// No process the compositor can name: an X11 window, or a connection whose
    /// credentials or start time could not be read.
    Unknown,
    /// The window's client is connected, and this is its process.
    Connected(ProcessIdentity),
    /// The window's client has disconnected since it mapped the window.
    Disconnected,
}

impl WindowProcess {
    /// The pid of the window's process, or `None` when the compositor cannot name it.
    ///
    /// Only a connected client has a pid the compositor read from the socket. An unknown
    /// or disconnected client gives `None` rather than the last pid it had, because a
    /// reused pid would name a different program.
    #[must_use]
    pub fn pid(self) -> Option<u32> {
        match self {
            Self::Connected(identity) => Some(identity.pid),
            Self::Unknown | Self::Disconnected => None,
        }
    }
}

/// The start time from one `/proc/<pid>/stat` line.
///
/// The second field is the executable name in parentheses. It may contain spaces and
/// parentheses of its own, so the fields are counted from the last `)`. The field after
/// it is field 3 (`state`), so the start time, field 22, is the twentieth one after it.
fn parse_start_time(stat: &str) -> Option<u64> {
    let (_, after_name) = stat.rsplit_once(')')?;
    after_name.split_whitespace().nth(19)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_start_time_of_this_process() {
        let stat = fs::read_to_string("/proc/self/stat").expect("procfs is mounted");
        let start = parse_start_time(&stat).expect("the stat line parses");
        assert!(start > 0, "a running process started after boot");
        let identity = ProcessIdentity::of(std::process::id()).expect("this process has a stat line");
        assert_eq!(identity.start_time, start);
    }

    #[test]
    fn parses_a_name_with_spaces_and_parentheses() {
        let stat = "42 (weird (name) x) S 1 42 42 0 -1 4194304 100 0 0 0 0 0 0 0 20 0 1 0 98765 1000 200";
        assert_eq!(parse_start_time(stat), Some(98765));
    }

    #[test]
    fn only_a_connected_client_names_a_pid() {
        let identity = ProcessIdentity {
            pid: 4242,
            start_time: 7,
        };
        assert_eq!(WindowProcess::Connected(identity).pid(), Some(4242));
        assert_eq!(WindowProcess::Unknown.pid(), None, "no credentials, no guess");
        assert_eq!(
            WindowProcess::Disconnected.pid(),
            None,
            "a gone client has no pid to report"
        );
    }

    #[test]
    fn refuses_a_line_without_a_start_time() {
        assert_eq!(parse_start_time("42 (short) S 1"), None);
        assert_eq!(parse_start_time("no parenthesis at all"), None);
    }
}
