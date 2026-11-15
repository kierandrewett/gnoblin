//! The configuration document: what device settings the script wrote, committed to the compositor, and
//! what a commit means for the generation.

use rquickjs::Function;

use super::log;
use super::{JsShell, JsShellError};
use crate::config::{JsApplied, JsConfigLink, parse_patch};
use crate::error::{JsError, JsErrorKind};

impl JsShell {
    /// Gives the shell its connection for the configuration document. A generation's document settings are
    /// validated against, and committed to, the document that connection reads. Without one a config that sets
    /// document keys is validated against an empty document and nothing is committed, which is reported.
    pub fn set_config_link(&mut self, link: Box<dyn JsConfigLink>) {
        self.note_script_ran();
        self.state.borrow_mut().config.set_link(link);
    }

    /// Puts the running generation's document settings back after something else committed the document (the the retired shell
    /// host's own reload does, and its document has none of the shell's settings). The settings are laid over the
    /// document in force and committed; what the other commit set stays as it is.
    ///
    /// # Errors
    ///
    /// The link failed or the generation's entry did.
    pub fn config_committed(&mut self) -> Result<Vec<String>, JsShellError> {
        self.note_script_ran();
        let mut diagnostics = Vec::new();
        self.apply_document(&mut diagnostics, false)?;
        Ok(diagnostics)
    }

    /// The `documentRevision` of the running generation's entry.
    fn document_revision_now(&self) -> Result<u32, JsShellError> {
        self.call_entry(self.entry_budget, |entry| {
            entry.get::<_, Function<'_>>("documentRevision")?.call(())
        })
    }

    /// Commits the document again when a device setting was written since the last commit (`Mouse.sensitivity = 0.5`
    /// in a listener, after the entry has run). A write the entry made while loading is in the first commit.
    pub(super) fn commit_document_changes(&mut self, diagnostics: &mut Vec<String>) -> Result<(), JsShellError> {
        if self.document_revision_now()? != self.document_revision {
            self.apply_document(diagnostics, true)?;
        }
        Ok(())
    }

    /// Commits the running generation's document settings: the patch it declared is laid over the document in
    /// force, after what the last committed generation set is taken back (`retract`), so a key a reload dropped
    /// returns to its default. A refusal is a diagnostic: the document in force stays.
    pub(super) fn apply_document(&mut self, diagnostics: &mut Vec<String>, retract: bool) -> Result<(), JsShellError> {
        self.document_revision = self.document_revision_now()?;
        let text: String = self.call_entry(self.entry_budget, |entry| {
            entry.get::<_, Function<'_>>("document")?.call(())
        })?;
        let patch = match parse_patch(&text) {
            Ok(patch) => patch,
            Err(reason) => {
                diagnostics.push(format!("the configuration settings were not committed: {reason}"));
                return Ok(());
            }
        };
        let outcome = self.state.borrow_mut().config.apply(&patch, retract);
        match outcome {
            Ok(JsApplied::Unchanged) => {}
            Ok(JsApplied::Committed(committed)) => {
                log::info!(
                    area = "config",
                    revision = committed.revision,
                    "the shell committed its configuration settings"
                );
                diagnostics.extend(
                    committed
                        .notices
                        .into_iter()
                        .map(|notice| format!("not applied: {notice}")),
                );
            }
            Err(reason) => diagnostics.push(format!(
                "the configuration settings were not committed, and the document in force stays: {reason}"
            )),
        }
        Ok(())
    }

    /// Makes this shell the owner of the configuration document, as in the JavaScript runtime role: each of its
    /// commits is the generation's settings alone (nothing a retired host wrote, and nothing a previous run left in
    /// the compositor), and it commits on every load so a reload always has an answer.
    pub fn own_document(&mut self) {
        self.state.borrow_mut().config.set_owns_document(true);
    }

    /// The document the running generation declares, as a tree: what the spec objects wrote to the document (the
    /// device settings). Nothing is committed; a throwaway shell over an inert tree reads a config's document this
    /// way ([`evaluate_document`](crate::evaluate_document)).
    ///
    /// # Errors
    ///
    /// The entry's `document` could not be read, or is not what the shell parses.
    pub fn declared_document(&mut self) -> Result<gnoblin_config::Value, JsShellError> {
        let text: String = self.call_entry(self.entry_budget, |entry| {
            entry.get::<_, Function<'_>>("document")?.call(())
        })?;
        parse_patch(&text).map_err(|reason| {
            JsShellError::Js(JsError {
                kind: JsErrorKind::Exception,
                message: format!("the config's document could not be read: {reason}"),
                location: None,
                stack: None,
            })
        })
    }
}
