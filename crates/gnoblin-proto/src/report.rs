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
//! The descriptor is [`READY_VARIABLE`]. A child that never writes a ready report is
//! unready, which is what the guardian's readiness deadline detects.

/// The variable that carries the descriptor a child reports on.
pub const READY_VARIABLE: &str = "GNOBLIN_READY_FD";

/// The first word of a readiness report.
pub const READY: &str = "ready";

/// The first word of a configuration-accepted report.
pub const CONFIG_ACCEPTED: &str = "config-accepted";

/// The first word of a configuration-rejected report.
pub const CONFIG_REJECTED: &str = "config-rejected";

/// One line of a child's report stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Line {
    /// The readiness report starts here.
    Ready,
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
    let mut report = String::from(READY);
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

/// Read one line of the report stream.
pub fn parse(line: &str) -> Line {
    let line = line.trim_end_matches(['\r', '\n']);
    if line.is_empty() {
        return Line::End;
    }
    if line == READY {
        return Line::Ready;
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

#[cfg(test)]
mod tests {
    use super::{Line, config_accepted, config_rejected, parse, ready};

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
    fn a_line_this_build_does_not_know_is_ignored() {
        assert_eq!(parse("invented 1 2"), Line::Unknown);
        assert_eq!(parse("=no key"), Line::Unknown);
        assert_eq!(parse("ready now"), Line::Unknown);
    }
}
