//! The one-line reports a role child sends its guardian.
//!
//! Design/rust-compositor.md, "Process model": the guardian starts the compositor, the
//! runtime and the services role, and the guardian's policy in `gnoblin-session` needs
//! three events it cannot observe from outside the child — the child is ready, the
//! runtime committed a configuration revision, and the runtime rejected one. A socket
//! is the wrong channel for that (the guardian is not an IPC client of the compositor),
//! so a child reports over an inherited descriptor instead, the way `sd_notify` does.
//!
//! The format is text with one report per line, because the guardian's log and a test's
//! assertion are the two consumers and both read better with words than with bytes:
//!
//! ```text
//! ready
//! WAYLAND_DISPLAY=gnoblin-wayland
//! GNOBLIN_IPC_SOCKET=/run/user/1000/gnoblin/ipc.sock
//!
//! config-accepted 3
//! ```
//!
//! The first word names the report, an `KEY=VALUE` line is one variable of the ready
//! report, and a blank line ends that report: the environment is a block rather than a
//! length-prefixed blob so a truncated report is recognisable instead of being read as
//! a shorter one. A report this build does not know is ignored rather than refused,
//! for the same reason an unknown IPC member is: additions are compatible, and a
//! guardian that dropped a child over one would be the failure.
//!
//! An `autostart` line carries one enabled entry of a configuration revision, and comes
//! before that revision's `config-accepted`. Its name and arguments are percent-encoded,
//! so a space or a newline in one cannot split the line.
//!
//! The descriptor is [`READY_VARIABLE`]. A child that never writes a ready report is
//! unready, which is what the guardian's readiness deadline detects.
//!
//! A `display` report has the same block form. The compositor sends one after its ready
//! report, once its X11 server has a display number, so the guardian can publish that
//! variable to children started afterwards.

/// The variable that carries the descriptor a child reports on.
pub const READY_VARIABLE: &str = "GNOBLIN_READY_FD";

/// The first word of a readiness report.
pub const READY: &str = "ready";

/// The first word of a display report, which the compositor sends after readiness.
pub const DISPLAY: &str = "display";

/// The first word of a configuration-accepted report.
pub const CONFIG_ACCEPTED: &str = "config-accepted";

/// The first word of a configuration-rejected report.
pub const CONFIG_REJECTED: &str = "config-rejected";

/// The first word of an autostart report, one per enabled entry of an accepted revision.
pub const AUTOSTART: &str = "autostart";

/// One line of a child's report stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Line {
    /// The readiness report starts here.
    Ready,
    /// The display block starts here: variables the compositor learned after its
    /// readiness, such as `DISPLAY` once its X server has a display number.
    Display,
    /// One variable the child publishes with its readiness, such as
    /// `WAYLAND_DISPLAY`.
    Environment {
        /// The variable's name.
        key: String,
        /// Its value.
        value: String,
    },
    /// The ready report's environment block ended.
    End,
    /// The runtime committed this configuration revision.
    ConfigAccepted {
        /// The revision the compositor assigned.
        revision: u64,
    },
    /// The runtime refused this configuration revision, and the last accepted one
    /// stays in force.
    ConfigRejected {
        /// The revision that was refused, or zero when the document never got that
        /// far.
        revision: u64,
        /// The diagnostic, with the newline that would break the framing removed.
        diagnostic: String,
    },
    /// An enabled autostart entry of the configuration revision that follows it. The
    /// runtime sends one per entry before that revision's `config-accepted`.
    Autostart {
        /// The revision the entry belongs to.
        revision: u64,
        /// The entry's name, the key of its table in the configuration.
        name: String,
        /// The program and its arguments, run directly without a shell.
        command: Vec<String>,
    },
    /// A line this build does not know, which a later Gnoblin may have added.
    Unknown,
}

/// The readiness report for `environment`.
///
/// A key or value with a newline in it would break the framing, so only the first
/// line of each is sent: an environment variable cannot hold a newline in practice,
/// and a report that silently ended early would be worse than one that dropped a
/// broken entry.
pub fn ready(environment: &[(String, String)]) -> String {
    environment_block(READY, environment)
}

/// The display report for `environment`: the variables the compositor learned after its
/// ready report, such as `DISPLAY` once its X server has a number. It has the ready
/// report's block form, so a truncated one is recognisable too.
pub fn display(environment: &[(String, String)]) -> String {
    environment_block(DISPLAY, environment)
}

/// One block report: its first word, one `KEY=VALUE` line per variable, then a blank line.
fn environment_block(word: &str, environment: &[(String, String)]) -> String {
    let mut report = String::from(word);
    report.push('\n');
    for (key, value) in environment {
        report.push_str(first_line(key));
        report.push('=');
        report.push_str(first_line(value));
        report.push('\n');
    }
    report.push('\n');
    report
}

/// The report that a configuration revision was accepted.
pub fn config_accepted(revision: u64) -> String {
    format!("{CONFIG_ACCEPTED} {revision}\n")
}

/// The report that a configuration revision was rejected.
pub fn config_rejected(revision: u64, diagnostic: &str) -> String {
    format!("{CONFIG_REJECTED} {revision} {}\n", first_line(diagnostic))
}

/// The report of one enabled autostart entry of configuration `revision`.
///
/// The name and each argument are percent-encoded, so a space or a newline in an argument
/// cannot change where the next field starts. `command` must not be empty: a report with
/// no argument would not parse back.
pub fn autostart(revision: u64, name: &str, command: &[String]) -> String {
    let mut report = format!("{AUTOSTART} {revision} {}", encode_token(name));
    for argument in command {
        report.push(' ');
        report.push_str(&encode_token(argument));
    }
    report.push('\n');
    report
}

/// Read one line of the report stream.
pub fn parse(line: &str) -> Line {
    let line = line.trim_end_matches(['\r', '\n']);
    if line.is_empty() {
        return Line::End;
    }
    if line == READY {
        return Line::Ready;
    }
    if line == DISPLAY {
        return Line::Display;
    }
    let mut words = line.splitn(2, ' ');
    let word = words.next().unwrap_or_default();
    let rest = words.next().unwrap_or_default();
    match word {
        CONFIG_ACCEPTED => match rest.trim().parse() {
            Ok(revision) => Line::ConfigAccepted { revision },
            Err(_) => Line::Unknown,
        },
        CONFIG_REJECTED => {
            let mut parts = rest.splitn(2, ' ');
            let revision = parts.next().unwrap_or_default().trim().parse().unwrap_or(0);
            let diagnostic = parts.next().unwrap_or_default().to_owned();
            Line::ConfigRejected { revision, diagnostic }
        }
        AUTOSTART => parse_autostart(rest).unwrap_or(Line::Unknown),
        _ => match line.split_once('=') {
            Some((key, value)) if !key.is_empty() => Line::Environment {
                key: key.to_owned(),
                value: value.to_owned(),
            },
            _ => Line::Unknown,
        },
    }
}

/// The first line of `text`, so a value with a newline cannot break the framing.
fn first_line(text: &str) -> &str {
    text.split('\n').next().unwrap_or_default()
}

/// Percent-encode `text` so it holds no space, percent sign or control character.
fn encode_token(text: &str) -> String {
    let mut encoded = Vec::with_capacity(text.len());
    for &byte in text.as_bytes() {
        if byte == b'%' || byte == b' ' || byte < 0x20 || byte == 0x7f {
            encoded.extend_from_slice(format!("%{byte:02X}").as_bytes());
        } else {
            encoded.push(byte);
        }
    }
    // Only ASCII bytes were replaced, so the result is still the UTF-8 it started as.
    String::from_utf8_lossy(&encoded).into_owned()
}

/// Undo [`encode_token`]. `None` when an escape is malformed or the bytes are not UTF-8.
fn decode_token(token: &str) -> Option<String> {
    let mut decoded = Vec::with_capacity(token.len());
    let mut bytes = token.bytes();
    while let Some(byte) = bytes.next() {
        if byte != b'%' {
            decoded.push(byte);
            continue;
        }
        let high = bytes.next().and_then(hex_digit)?;
        let low = bytes.next().and_then(hex_digit)?;
        decoded.push((high << 4) | low);
    }
    String::from_utf8(decoded).ok()
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

/// The fields of an autostart report after its first word: the revision, the entry's
/// name, then one argument per remaining field. `None` when a field is missing or
/// malformed, so a truncated report is unknown rather than a shorter command.
fn parse_autostart(rest: &str) -> Option<Line> {
    let mut fields = rest.split(' ');
    let revision = fields.next()?.parse().ok()?;
    let name = decode_token(fields.next()?)?;
    let command = fields.map(decode_token).collect::<Option<Vec<String>>>()?;
    if name.is_empty() || command.is_empty() {
        return None;
    }
    Some(Line::Autostart {
        revision,
        name,
        command,
    })
}

#[cfg(test)]
mod tests {
    use super::{Line, autostart, config_accepted, config_rejected, display, parse, ready};

    #[test]
    fn a_ready_report_round_trips_its_environment() {
        let environment = [
            ("WAYLAND_DISPLAY".to_owned(), "gnoblin-wayland".to_owned()),
            ("DISPLAY".to_owned(), ":17".to_owned()),
        ];
        let report = ready(&environment);
        let lines: Vec<Line> = report.lines().map(parse).collect();
        assert_eq!(
            lines,
            vec![
                Line::Ready,
                Line::Environment {
                    key: "WAYLAND_DISPLAY".to_owned(),
                    value: "gnoblin-wayland".to_owned(),
                },
                Line::Environment {
                    key: "DISPLAY".to_owned(),
                    value: ":17".to_owned(),
                },
                Line::End,
            ]
        );
        assert!(report.ends_with("\n\n"), "{report:?}");
        assert_eq!(parse(""), Line::End);
    }

    #[test]
    fn a_display_report_round_trips_its_environment() {
        // The failure this prevents: a display report read as an unknown line, so the
        // guardian never learns the DISPLAY the compositor creates after its readiness.
        let report = display(&[("DISPLAY".to_owned(), ":1".to_owned())]);
        assert_eq!(report, "display\nDISPLAY=:1\n\n");
        let lines: Vec<Line> = report.lines().map(parse).collect();
        assert_eq!(
            lines,
            vec![
                Line::Display,
                Line::Environment {
                    key: "DISPLAY".to_owned(),
                    value: ":1".to_owned(),
                },
                Line::End,
            ]
        );
    }

    #[test]
    fn a_value_with_a_newline_cannot_break_the_framing() {
        // The failure this prevents: a diagnostic or a display name with a newline in
        // it swallows the rest of the report, so the guardian reads the next report as
        // a variable and loses the configuration revision.
        let report = ready(&[("K".to_owned(), "one\ntwo".to_owned())]);
        assert_eq!(report, "ready\nK=one\n\n");
        let rejected = config_rejected(4, "line one\nready");
        assert_eq!(
            parse(&rejected),
            Line::ConfigRejected {
                revision: 4,
                diagnostic: "line one".to_owned(),
            }
        );
    }

    #[test]
    fn a_configuration_report_round_trips() {
        assert_eq!(parse(&config_accepted(7)), Line::ConfigAccepted { revision: 7 });
        assert_eq!(parse("config-accepted"), Line::Unknown, "a report needs its revision");
        assert_eq!(
            parse("config-rejected 2 bad setting"),
            Line::ConfigRejected {
                revision: 2,
                diagnostic: "bad setting".to_owned(),
            }
        );
    }

    #[test]
    fn an_autostart_entry_keeps_its_arguments_exactly() {
        // The failure this prevents: an argument with a space splits into two, or one with
        // a newline ends the report early, so the guardian runs a different command.
        let command = vec![
            "/bin/sh".to_owned(),
            "-c".to_owned(),
            "echo 100% done\nnow".to_owned(),
            String::new(),
            "héllo".to_owned(),
        ];
        let report = autostart(3, "panel", &command);
        assert_eq!(report.matches('\n').count(), 1, "one line per entry: {report:?}");
        assert_eq!(
            parse(&report),
            Line::Autostart {
                revision: 3,
                name: "panel".to_owned(),
                command,
            }
        );
    }

    #[test]
    fn a_malformed_autostart_line_is_unknown() {
        assert_eq!(parse("autostart 3 panel"), Line::Unknown, "an entry needs a command");
        assert_eq!(
            parse("autostart x panel waybar"),
            Line::Unknown,
            "the revision must parse"
        );
        assert_eq!(
            parse("autostart 3 panel bad%zz"),
            Line::Unknown,
            "an escape must be hex"
        );
        assert_eq!(parse("autostart 3 panel %FF"), Line::Unknown, "arguments must be UTF-8");
    }

    #[test]
    fn a_line_this_build_does_not_know_is_ignored() {
        assert_eq!(parse("invented 1 2"), Line::Unknown);
        assert_eq!(parse("=no key"), Line::Unknown);
        assert_eq!(parse("ready now"), Line::Unknown);
    }
}
