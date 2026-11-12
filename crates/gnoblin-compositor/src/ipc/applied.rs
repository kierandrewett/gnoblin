//! What the running session has not applied from the documents it accepted.
//!
//! The compositor applies most settings when it commits a document, and a process that reads a
//! setting only when it starts keeps the value it started with. A commit must say which accepted
//! settings are in that second state, or a user who reloads believes they took effect
//! (`design/config-reload.md`). This keeps the three documents the comparison needs.

use gnoblin_client::wire::{ConfigNotice, NoticeKind};
use gnoblin_config::Config;
use gnoblin_config::apply::{self, StartProcess, Unapplied, UnappliedKind};

/// The processes that read a setting at their start, in the order a report lists them.
const PROCESSES: [StartProcess; 3] = [
    StartProcess::Compositor,
    StartProcess::Services,
    StartProcess::CompatHost,
];

/// The documents a session compares each commit with.
pub(crate) struct StartedDocuments {
    /// The document the compositor read when it started. It reads the last accepted file before
    /// the runtime has committed anything, so this is the previous session's document.
    started_with: Config,
    /// The first document the session accepted, which the services role and the compatibility
    /// host read when they start afterwards.
    first: Option<Config>,
    /// The document accepted before the one being committed.
    previous: Option<Config>,
}

impl StartedDocuments {
    /// A session whose compositor started with `started_with`.
    pub(crate) fn new(started_with: Config) -> StartedDocuments {
        StartedDocuments {
            started_with,
            first: None,
            previous: None,
        }
    }

    /// Records `accepted` as the newest document and returns the settings of it that are not in
    /// effect: those a process reads only at its start, and those nothing reads.
    pub(crate) fn notices(&mut self, accepted: &Config) -> Vec<ConfigNotice> {
        let first = self.first.get_or_insert_with(|| accepted.clone());
        let mut pending = Vec::new();
        for process in PROCESSES {
            let baseline = if process.starts_after_first_commit() {
                &*first
            } else {
                &self.started_with
            };
            pending.extend(apply::restart_required(baseline, accepted, process));
        }
        let previous = self.previous.replace(accepted.clone());
        pending.extend(apply::no_effect(
            previous.as_ref().unwrap_or(&self.started_with),
            accepted,
        ));
        pending.iter().map(notice).collect()
    }
}

/// The wire form of a pending setting.
fn notice(pending: &Unapplied) -> ConfigNotice {
    ConfigNotice {
        kind: match pending.kind {
            UnappliedKind::WaitsForRestart(_) => NoticeKind::RestartRequired,
            UnappliedKind::Ignored => NoticeKind::NoEffect,
        },
        text: pending.describe(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(json: &str) -> Config {
        Config::from_json_bytes(json.as_bytes()).expect("the document is valid")
    }

    fn texts(notices: &[ConfigNotice]) -> Vec<&str> {
        notices.iter().map(|notice| notice.text.as_str()).collect()
    }

    #[test]
    fn a_setting_the_compositor_started_without_is_reported_on_the_first_commit() {
        // The compositor reads the last accepted file before the runtime commits, so a setting
        // edited between two logins is not in force when the new session's first commit arrives.
        let mut applied = StartedDocuments::new(config("{}"));
        let notices = applied.notices(&config(r#"{"compositor":{"renderer":"software"}}"#));
        assert_eq!(notices.len(), 1, "{notices:?}");
        assert_eq!(notices[0].kind, NoticeKind::RestartRequired);
        assert!(
            notices[0].text.starts_with("compositor.renderer: restart required"),
            "{notices:?}"
        );
    }

    #[test]
    fn a_setting_the_compositor_started_with_is_not_reported() {
        let started = config(r#"{"compositor":{"renderer":"software"}}"#);
        let mut applied = StartedDocuments::new(started.clone());
        assert!(applied.notices(&started).is_empty());
    }

    #[test]
    fn a_service_setting_is_compared_with_the_first_commit_not_the_compositor_start() {
        // The services role starts after the first commit and reads what it saved: a keyring
        // setting the first commit carries is already in force, and only a later change waits.
        let mut applied = StartedDocuments::new(config("{}"));
        assert!(applied.notices(&config(r#"{"keyring":{"enabled":false}}"#)).is_empty());
        let later = applied.notices(&config(r#"{"keyring":{"enabled":true}}"#));
        assert_eq!(later.len(), 1, "{later:?}");
        assert!(
            later[0].text.starts_with("keyring.enabled: restart required"),
            "{later:?}"
        );
    }

    #[test]
    fn a_setting_nothing_reads_is_reported_once_when_it_changes() {
        let mut applied = StartedDocuments::new(config("{}"));
        let first = applied.notices(&config(r#"{"compositor":{"blur":"on"}}"#));
        assert_eq!(texts(&first), ["compositor.blur: no effect, nothing reads it yet"]);
        assert!(applied.notices(&config(r#"{"compositor":{"blur":"on"}}"#)).is_empty());
    }
}
