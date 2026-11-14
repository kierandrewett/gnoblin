//! Names for the uids polkit offers, and for the process that asks.
//!
//! `polkit-agent-helper-1` takes a user *name* and polkitd names a *uid*, so the compositor has to map one to the
//! other. The workspace forbids unsafe code, so `getpwuid` is out of reach: `/etc/passwd` answers for local
//! accounts, and `getent passwd` for the ones a directory service (NSS) provides.

use std::os::unix::fs::MetadataExt as _;
use std::process::{Command, Stdio};

/// The login name of `uid`, or `None` when no account has it.
#[must_use]
pub fn name_of(uid: u32) -> Option<String> {
    if let Ok(text) = std::fs::read_to_string("/etc/passwd")
        && let Some(name) = parse_passwd(&text, uid)
    {
        return Some(name);
    }
    let output = Command::new("getent")
        .args(["passwd", &uid.to_string()])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_passwd(&String::from_utf8_lossy(&output.stdout), uid)
}

/// The name of `uid` in `passwd` text (`name:password:uid:gid:...` lines).
#[must_use]
pub fn parse_passwd(text: &str, uid: u32) -> Option<String> {
    text.lines().find_map(|line| {
        let mut fields = line.split(':');
        let name = fields.next()?;
        let _password = fields.next()?;
        let found: u32 = fields.next()?.parse().ok()?;
        (found == uid && !name.is_empty()).then(|| name.to_owned())
    })
}

/// The uid this process runs as, read from the owner of `/proc/self`.
#[must_use]
pub fn current_uid() -> Option<u32> {
    std::fs::metadata("/proc/self").ok().map(|metadata| metadata.uid())
}

/// The name of process `pid` (`/proc/<pid>/comm`), or `None` when it is gone or cannot be read.
pub(crate) fn process_name(pid: u32) -> Option<String> {
    let name = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWD: &str = "root:x:0:0:root:/root:/bin/bash\nalice:x:1000:1000:Alice:/home/alice:/bin/fish\nbroken\n";

    #[test]
    fn a_uid_maps_to_its_login_name() {
        assert_eq!(parse_passwd(PASSWD, 0).as_deref(), Some("root"));
        assert_eq!(parse_passwd(PASSWD, 1000).as_deref(), Some("alice"));
    }

    #[test]
    fn an_unknown_uid_has_no_name() {
        assert_eq!(parse_passwd(PASSWD, 4242), None);
        assert_eq!(parse_passwd("", 0), None);
    }

    #[test]
    fn this_process_has_a_uid() {
        assert!(current_uid().is_some());
    }
}
