//! The guardian's report descriptor, written by the role child.
//!
//! [`gnoblin_proto::report`] is the format; this is the writer. It exists because a
//! descriptor number can be adopted exactly once — two `OwnedFd`s for one number close it
//! twice — so the child takes the descriptor at start-up and keeps it, rather than
//! adopting it again for every line.
//!
//! A child the guardian did not start has no report descriptor, and that is not an
//! error: every write becomes a no-op, so a role runs unchanged under a test that only
//! wanted the connection.

use std::fs::File;
use std::io::Write as _;

use gnoblin_proto::report;

use crate::inherit;

/// The descriptor a role child reports on.
#[derive(Debug)]
pub struct Reporter {
    file: Option<File>,
}

impl Reporter {
    /// Adopt `env`'s report descriptor, if it names one.
    ///
    /// A value this build cannot read is treated as no descriptor rather than as a
    /// failure: the report is a log line for the guardian, and a role that refused to
    /// start over it would turn a diagnostic into an outage.
    pub fn new(env: impl Fn(&str) -> Option<String>) -> Reporter {
        let file = inherit::file(report::READY_VARIABLE, env).ok().flatten();
        Reporter { file }
    }

    /// Whether this child has anyone to report to.
    pub fn is_present(&self) -> bool {
        self.file.is_some()
    }

    /// Write one report, ignoring a pipe that has been closed.
    pub fn write(&mut self, text: &str) {
        let Some(file) = self.file.as_mut() else {
            return;
        };
        if file.write_all(text.as_bytes()).and_then(|()| file.flush()).is_err() {
            // The guardian is gone. Dropping the descriptor is what stops this from
            // becoming a write error on every later report.
            self.file = None;
        }
    }

    /// Report that this child is ready, with the environment it publishes.
    pub fn ready(&mut self, environment: &[(String, String)]) {
        self.write(&report::ready(environment));
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;

    use super::Reporter;

    fn environment(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        move |name| {
            pairs
                .iter()
                .find(|(known, _)| known == name)
                .map(|(_, value)| value.clone())
        }
    }

    #[test]
    fn a_child_without_a_report_descriptor_writes_nothing_and_does_not_fail() {
        let mut reporter = Reporter::new(environment(&[]));
        assert!(!reporter.is_present());
        reporter.ready(&[("WAYLAND_DISPLAY".to_owned(), "wayland-0".to_owned())]);
        reporter.write("config-accepted 1\n");
    }

    #[test]
    fn a_report_reaches_the_pipe_once() {
        // The failure this prevents: the child adopts the descriptor number twice and the
        // second owner closes it under the first, so the guardian reads a truncated report
        // and a later write fails.
        let (mut read, write) = std::io::pipe().expect("a pipe");
        let number = {
            use std::os::fd::IntoRawFd as _;
            write.into_raw_fd()
        };
        let mut reporter = Reporter::new(environment(&[("GNOBLIN_READY_FD", &number.to_string())]));
        assert!(reporter.is_present());
        reporter.ready(&[("WAYLAND_DISPLAY".to_owned(), "wayland-0".to_owned())]);
        reporter.write("config-accepted 2\n");
        drop(reporter);

        let mut report = String::new();
        read.read_to_string(&mut report).expect("the report");
        assert_eq!(report, "ready\nWAYLAND_DISPLAY=wayland-0\n\nconfig-accepted 2\n");
    }
}
