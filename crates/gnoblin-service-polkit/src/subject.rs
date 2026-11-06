//! The process identity polkitd regs the agent under.
//!
//! `RegisterAuthenticationAgent` names the caller as a `(sa{sv})` subject of kind
//! `unix-process`, whose details are the pid and the process start time. The start
//! time is what makes the identity survive pid reuse: polkitd compares both, which
//! is why a subject built from the pid alone is a different, weaker thing.
//!
//! `/proc/self/stat`'s second field is the command name in parentheses and may
//! itself contain spaces, so it is stripped by its closing parenthesis before the
//! remaining fields are counted.

use std::collections::HashMap;

use zbus::OwnedVariant;

/// The subject kind for a process's own identity.
pub const KIND: &str = "unix-process";

/// This process as polkitd names a caller: `("unix-process", {"pid", "start-time"})`.
pub fn unix_process() -> (String, HashMap<String, OwnedVariant>) {
    let mut details = HashMap::new();
    details.insert("pid".to_owned(), OwnedVariant::from(std::process::id()));
    details.insert("start-time".to_owned(), OwnedVariant::from(start_time().unwrap_or(0)));
    (KIND.to_owned(), details)
}

/// The process start time from `/proc/self/stat`, in clock ticks since boot.
///
/// `None` when `/proc` is not mounted or the file has fewer fields than Linux
/// declares, which only happens in a container that hid it: the caller then sends
/// the subject polkitd treats as "a pid with no start time" rather than making up a
/// value.
pub fn start_time() -> Option<u64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    parse_start_time(&stat)
}

/// The 22nd field of one `stat` line.
///
/// Field 1 is the pid and field 2 is the command in parentheses, so the start time
/// is the 20th whitespace-separated field after the closing parenthesis.
fn parse_start_time(stat: &str) -> Option<u64> {
    let rest = stat.split_once(')')?.1;
    rest.split_whitespace().nth(19)?.parse().ok()
}
