//! A fake `polkit-agent-helper-1` for the agent's tests: the same protocol and the same last step, without PAM.
//!
//! The real helper reads the cookie from the first line of its standard input, asks PAM, and only when PAM says yes
//! calls `AuthenticationAgentResponse2` on polkitd (as root) before it prints `SUCCESS`. This one asks for a
//! password with the `PAM_PROMPT_ECHO_OFF` line, accepts `hunter2`, and then makes the same call on the bus named
//! by `DBUS_SYSTEM_BUS_ADDRESS`, which a test points at its private bus and its fake polkitd. A wrong password
//! prints `PAM_ERROR_MSG` and `FAILURE` and tells polkitd nothing. It is a test double: it runs no PAM, and the
//! call it makes is not made as root, so it proves the agent's half of the conversation and not polkitd's.

use std::collections::HashMap;
use std::io::{BufRead as _, Write as _};
use std::os::unix::fs::MetadataExt as _;
use std::process::ExitCode;

use zbus::OwnedVariant;
use zbus::connection::Builder;

const PASSWORD: &str = "hunter2";

fn main() -> ExitCode {
    let mut out = std::io::stdout().lock();
    let mut input = std::io::stdin().lock();
    let mut cookie = String::new();
    if input.read_line(&mut cookie).is_err() {
        let _ = writeln!(out, "FAILURE");
        return ExitCode::FAILURE;
    }
    let cookie = cookie.trim_end().to_owned();
    let _ = writeln!(out, "PAM_PROMPT_ECHO_OFF Password: ");
    let _ = out.flush();
    let mut password = String::new();
    if input.read_line(&mut password).is_err() || password.trim_end() != PASSWORD {
        let _ = writeln!(out, "PAM_ERROR_MSG Authentication failure");
        let _ = writeln!(out, "FAILURE");
        return ExitCode::FAILURE;
    }
    let told = tell_polkitd(&cookie);
    let _ = writeln!(out, "{}", if told { "SUCCESS" } else { "FAILURE" });
    let _ = out.flush();
    if told { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

/// Calls `AuthenticationAgentResponse2(uid, cookie, identity)` on the bus the environment names.
fn tell_polkitd(cookie: &str) -> bool {
    let Ok(address) = std::env::var("DBUS_SYSTEM_BUS_ADDRESS") else {
        return false;
    };
    let Ok(metadata) = std::fs::metadata("/proc/self") else {
        return false;
    };
    let uid = metadata.uid();
    zbus::block_on(async {
        let Ok(connection) = Builder::address(address.as_str()).build().await else {
            return false;
        };
        let mut details: HashMap<String, OwnedVariant> = HashMap::new();
        details.insert("uid".to_owned(), OwnedVariant::from(uid));
        connection
            .call_method(
                Some("org.freedesktop.PolicyKit1"),
                "/org/freedesktop/PolicyKit1/Authority",
                Some("org.freedesktop.PolicyKit1.Authority"),
                "AuthenticationAgentResponse2",
                &(uid, cookie, ("unix-user", details)),
            )
            .await
            .is_ok()
    })
}
