//! The compositor→client invocation contract.
//!
//! gnoblin spawns every shell client (whether an `exec` autostart or an
//! `exec_role` on-demand role) with context flags on the command line, so the
//! client is a stateless renderer: it learns its role, which output to bind,
//! where to position, and what it was invoked for entirely from argv. Adding a
//! client to a role is therefore just pointing the config at a binary that
//! understands these flags.
//!
//! Flags (all optional; clients use what they need):
//!   --role <name>      the role being filled (topbar, dock, window-menu, …)
//!   --output <name>    target output/monitor
//!   --x <int> --y <int>        anchor point in layout coords (popups/menus)
//!   --width <int> --height <int>   relevant geometry (output or target rect)
//!   --window <id>      target window id (dev.gnoblin.Shell window id)
//!   --reason <str>     why it was invoked (e.g. titlebar, keybind, button)

/// Parsed invocation context handed to a client by the compositor.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ClientArgs {
    pub role: Option<String>,
    pub output: Option<String>,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub window: Option<u64>,
    pub reason: Option<String>,
}

impl ClientArgs {
    /// Parse the process's own command line (skipping argv[0]).
    pub fn from_env() -> Self {
        Self::parse(std::env::args().skip(1))
    }

    /// Parse `--key value` pairs from an iterator of tokens. Unknown flags are
    /// ignored so a client can layer its own options on top.
    pub fn parse<I: IntoIterator<Item = String>>(tokens: I) -> Self {
        let mut a = ClientArgs::default();
        let mut it = tokens.into_iter().peekable();
        while let Some(tok) = it.next() {
            // Accept both `--key value` and `--key=value`.
            let (key, inline) = match tok.split_once('=') {
                Some((k, v)) => (k.to_string(), Some(v.to_string())),
                None => (tok, None),
            };
            let mut value = || {
                if inline.is_some() {
                    return inline.clone();
                }

                match it.peek() {
                    Some(next) if !next.starts_with("--") => it.next(),
                    _ => None,
                }
            };
            match key.as_str() {
                "--role" => a.role = value(),
                "--output" => a.output = value(),
                "--x" => a.x = value().and_then(|v| v.parse().ok()),
                "--y" => a.y = value().and_then(|v| v.parse().ok()),
                "--width" => a.width = value().and_then(|v| v.parse().ok()),
                "--height" => a.height = value().and_then(|v| v.parse().ok()),
                "--window" => a.window = value().and_then(|v| v.parse().ok()),
                "--reason" => a.reason = value(),
                _ => {
                    let _ = value(); // consume a possible value of an unknown flag
                }
            }
        }
        a
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn parses_role_and_position() {
        let a = ClientArgs::parse(toks(
            "--role window-menu --window 42 --x 100 --y 30 --reason titlebar",
        ));
        assert_eq!(a.role.as_deref(), Some("window-menu"));
        assert_eq!(a.window, Some(42));
        assert_eq!(a.x, Some(100));
        assert_eq!(a.y, Some(30));
        assert_eq!(a.reason.as_deref(), Some("titlebar"));
    }

    #[test]
    fn parses_equals_form_and_ignores_unknown() {
        let a = ClientArgs::parse(toks("--role=dock --frobnicate xyz --output=DP-1"));
        assert_eq!(a.role.as_deref(), Some("dock"));
        assert_eq!(a.output.as_deref(), Some("DP-1"));
    }

    #[test]
    fn unknown_flag_without_value_does_not_consume_next_known_flag() {
        let a = ClientArgs::parse(toks("--grid --output Meta-1 --x 10"));
        assert_eq!(a.output.as_deref(), Some("Meta-1"));
        assert_eq!(a.x, Some(10));
    }

    #[test]
    fn known_flag_without_value_does_not_consume_next_known_flag() {
        let a = ClientArgs::parse(toks("--role --output Meta-1"));
        assert_eq!(a.role, None);
        assert_eq!(a.output.as_deref(), Some("Meta-1"));
    }
}
