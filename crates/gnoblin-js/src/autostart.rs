//! `defineConfig({ autostart })`: commands the shell starts once per session.
//!
//! The config lists commands, and a reload lists them again. What a reload must not do is start them again, so
//! the shell remembers each command it started by its identity, the program and its arguments, for the life of
//! the shell. A reload therefore starts only the commands the new config adds.
//!
//! **Why not the document's `autostart` section.** The session guardian starts those, but it learns them from the
//! Lua host's own commit and stops any it was not told of, so a section the shell wrote would be stopped by the
//! next Lua reload. A command the shell starts is also the shell's: it takes `gnoblin.spawn`'s process group and
//! reaping, and a script can see why one did not start.
//!
//! **The limits.** The memory is the shell's, so it is lost when the shell restarts (the guardian restarts a
//! runtime role that dies): the new shell starts every command again, and a program from the first run that is still
//! alive is then running twice. A command that is removed is not stopped, because the shell does not keep a
//! handle to kill it, and one that is added again is not started again while the shell lives. A command that
//! cannot be started (the program is missing) is reported and tried again at the next reload.

use std::collections::HashSet;

use thiserror::Error;

use crate::bindings::JsSpawnRequest;

/// A config's autostart list that is not a list of argv lists.
#[derive(Debug, Error)]
#[error("the autostart list is not a list of commands: {0}")]
pub(crate) struct JsAutostartError(String);

/// The argv lists a generation's config wants started, from the JSON text its `autostart` entry returns.
///
/// # Errors
///
/// The text is not a JSON list of lists of strings, or a list is empty.
pub(crate) fn parse(text: &str) -> Result<Vec<Vec<String>>, JsAutostartError> {
    let commands: Vec<Vec<String>> = serde_json::from_str(text).map_err(|error| JsAutostartError(error.to_string()))?;
    if commands.iter().any(Vec::is_empty) {
        return Err(JsAutostartError("a command has no program".to_owned()));
    }
    Ok(commands)
}

/// The commands this shell has started.
#[derive(Default)]
pub(crate) struct JsAutostart {
    started: HashSet<Vec<String>>,
}

impl JsAutostart {
    /// Starts each of `commands` that this shell has not started, through `spawn` (which answers the host's refusal,
    /// if any). Returns one diagnostic for each command that could not be started; those are not remembered.
    pub(crate) fn start(
        &mut self,
        commands: Vec<Vec<String>>,
        mut spawn: impl FnMut(&JsSpawnRequest) -> Option<String>,
    ) -> Vec<String> {
        let mut diagnostics = Vec::new();
        for argv in commands {
            if self.started.contains(&argv) {
                continue;
            }
            match spawn(&JsSpawnRequest::Argv(argv.clone())) {
                None => {
                    self.started.insert(argv);
                }
                Some(reason) => diagnostics.push(format!("autostart {}: {reason}", argv.join(" "))),
            }
        }
        diagnostics
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| (*part).to_owned()).collect()
    }

    #[test]
    fn a_command_is_started_once_however_often_it_is_listed() {
        let mut autostart = JsAutostart::default();
        let mut started = Vec::new();
        for _ in 0..3 {
            let diagnostics = autostart.start(vec![argv(&["waybar"])], |request| {
                started.push(request.clone());
                None
            });
            assert!(diagnostics.is_empty());
        }
        assert_eq!(started, vec![JsSpawnRequest::Argv(argv(&["waybar"]))]);
    }

    #[test]
    fn a_command_is_known_by_its_arguments_too() {
        let mut autostart = JsAutostart::default();
        let mut started = 0;
        let mut count = |_: &JsSpawnRequest| {
            started += 1;
            None
        };
        autostart.start(vec![argv(&["foot", "--server"])], &mut count);
        autostart.start(vec![argv(&["foot", "--server"]), argv(&["foot"])], &mut count);
        assert_eq!(started, 2);
    }

    #[test]
    fn a_refused_command_is_reported_and_tried_again() {
        let mut autostart = JsAutostart::default();
        let refused = autostart.start(vec![argv(&["nope", "-x"])], |_| Some("no such program".to_owned()));
        assert_eq!(refused, vec!["autostart nope -x: no such program".to_owned()]);
        let mut tried = 0;
        let again = autostart.start(vec![argv(&["nope", "-x"])], |_| {
            tried += 1;
            None
        });
        assert!(again.is_empty());
        assert_eq!(tried, 1);
    }

    #[test]
    fn the_entry_text_is_a_list_of_argv_lists() {
        assert_eq!(
            parse(r#"[["a"],["b","--c"]]"#).expect("a list of commands"),
            vec![argv(&["a"]), argv(&["b", "--c"])]
        );
        assert!(parse("[]").expect("an empty list").is_empty());
        assert!(parse(r#"["a"]"#).is_err());
        assert!(parse(r#"[[]]"#).is_err());
        assert!(parse("not json").is_err());
    }
}
