//! A user's config, read for its document and nothing else.
//!
//! [`evaluate_document`] answers "what would this config commit?" without a session. It boots a throwaway
//! [`JsShell`] over an inert tree (the root node and one seat, which every compositor has: no windows, no outputs, no
//! devices), runs the entry in it with a short budget and reads the document the entry's objects wrote
//! ([`JsShell::declared_document`]). The services role uses it for the first-boot `keyring.enabled` decision,
//! which has to be made before the runtime role has committed anything.
//!
//! **What it reads.** The shell is the real one, so a spec-style `init.ts` (workspaces, bindings, listeners)
//! loads as written and writes the same document it would in a session. Nothing starts: no program is spawned, no
//! claim is made, no transaction is answered. `keyring` and `autostart` have no source in an `init.ts` any more, so a
//! config's document holds only what the device objects wrote, and the keyring keeps its default.
//!
//! **What it cannot read.** The tree is empty, so a config that decides a setting from live state (`gnoblin.outputs`,
//! `gnoblin.clients`) sees none. The caller treats this document as a first-boot hint and waits for the document the
//! runtime commits, which is authoritative. A config that throws or needs more than [`DOCUMENT_BUDGET`] is an error.

use std::path::Path;
use std::time::Duration;

use gnoblin_tree_client::tree_mirror::{
    Attached, LinkError, MirrorValue, SnapshotNode, TreeIncoming, TreeLink, TreeTx,
};

use crate::error::{JsError, JsErrorKind};
use crate::shell::{JsShell, JsShellConfig, JsShellError};

/// The wall-clock budget for evaluating the entry. A config's top level only declares things, and the services
/// role must not wait on it, so a loop that never ends is cut off quickly.
pub const DOCUMENT_BUDGET: Duration = Duration::from_secs(1);

/// The id of the inert tree's root node.
const ROOT: u64 = 1;

/// The id of the inert tree's one seat, so `gnoblin.input.primary` is there for a config that binds keys.
const SEAT: u64 = 2;

/// A compositor with nothing in it: the root node and a seat, which answers every attach and accepts every transaction.
#[derive(Default)]
struct InertTree {
    queue: Vec<TreeIncoming>,
}

impl TreeLink for InertTree {
    fn attach(&mut self, _version: u64) -> Result<(), LinkError> {
        self.queue.push(TreeIncoming::Attached(Attached {
            epoch: 1,
            root: ROOT,
            serial: 0,
        }));
        self.queue.push(TreeIncoming::Snapshot {
            serial: 0,
            chunk: 0,
            last: true,
            nodes: vec![
                SnapshotNode {
                    id: ROOT,
                    parent: 0,
                    index: 0,
                    class: "node".to_owned(),
                    title: String::new(),
                    app_id: String::new(),
                    props: Vec::new(),
                },
                SnapshotNode {
                    id: SEAT,
                    parent: ROOT,
                    index: 0,
                    class: "seat".to_owned(),
                    title: String::new(),
                    app_id: String::new(),
                    props: vec![("name".to_owned(), MirrorValue::Text("seat0".to_owned()))],
                },
            ],
        });
        Ok(())
    }

    fn transact(&mut self, _tx: &TreeTx) -> Result<(), LinkError> {
        Ok(())
    }

    fn poll(&mut self) -> Result<Vec<TreeIncoming>, LinkError> {
        Ok(std::mem::take(&mut self.queue))
    }
}

/// The document the config at `entry` commits, as JSON bytes: what its objects wrote to the document, and its
/// autostart commands (only the deprecated `defineConfig` has any) as the document's `autostart` section. `{}` for a
/// config that writes nothing there.
///
/// The relative imports of `entry` resolve under its own directory, as they do in the shell.
///
/// # Errors
///
/// The entry cannot be read, does not load (a syntax error, an import that does not resolve, a throw), or runs past
/// [`DOCUMENT_BUDGET`]. The error says where, in the author's `.ts` coordinates.
pub fn evaluate_document(entry: &Path) -> Result<Vec<u8>, JsError> {
    let unreadable = |reason: String| JsError {
        kind: JsErrorKind::Load,
        message: format!("cannot read {}: {reason}", entry.display()),
        location: None,
        stack: None,
    };
    let entry = std::fs::canonicalize(entry).map_err(|error| unreadable(error.to_string()))?;
    let root = entry
        .parent()
        .ok_or_else(|| unreadable("it has no directory".to_owned()))?;
    let mut config = JsShellConfig::new(root);
    config.js.stage_budget = DOCUMENT_BUDGET;
    config.entry_budget = DOCUMENT_BUDGET;
    config.frame_budget = DOCUMENT_BUDGET;
    let failed = |error: JsShellError| match error {
        JsShellError::Js(error) => error,
        other => JsError {
            kind: JsErrorKind::Load,
            message: other.to_string(),
            location: None,
            stack: None,
        },
    };
    let mut shell = JsShell::new(Box::<InertTree>::default(), config).map_err(failed)?;
    // The shell owns the document, so its autostart commands go into the document instead of being started here.
    shell.own_document();
    shell.load_entry(&entry).map_err(failed)?;
    let document = shell.declared_document().map_err(failed)?;
    Ok(document.to_json().to_string().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The document of a config whose source is `source`, written to `init.ts` in a directory of its own.
    fn document(source: &str) -> Result<serde_json::Value, JsError> {
        let dir = tempfile::tempdir().expect("a temp dir");
        let entry = dir.path().join("init.ts");
        std::fs::write(&entry, source).expect("init.ts is written");
        evaluate_document(&entry).map(|bytes| serde_json::from_slice(&bytes).expect("the document is JSON"))
    }

    #[test]
    fn a_config_that_writes_nothing_to_the_document_has_an_empty_document() {
        assert_eq!(
            document("export const x = 1;").expect("it loads"),
            serde_json::json!({})
        );
    }

    #[test]
    fn a_relative_import_resolves_under_the_entrys_directory() {
        let dir = tempfile::tempdir().expect("a temp dir");
        std::fs::write(dir.path().join("keys.ts"), "export const radius = 8;\n").expect("keys.ts");
        std::fs::write(
            dir.path().join("init.ts"),
            r#"import { radius } from "./keys.ts";
            if (radius !== 8) throw new Error("the import did not run");
            gnoblin.clients.style.cornerRadius = radius;"#,
        )
        .expect("init.ts");
        let bytes = evaluate_document(&dir.path().join("init.ts")).expect("the config evaluates");
        let document: serde_json::Value = serde_json::from_slice(&bytes).expect("JSON");
        assert_eq!(document, serde_json::json!({}));
    }

    #[test]
    fn a_config_that_throws_is_an_error_with_its_position() {
        let error = document("throw new Error(\"boom\");\n").expect_err("a throw is an error");
        assert!(error.to_string().contains("boom"), "{error}");
    }

    #[test]
    fn a_config_that_reads_live_state_sees_an_empty_world() {
        let document = document(
            "const n = (globalThis as any).gnoblin.clients.length + gnoblin.outputs.length;\n\
             if (n !== 0) throw new Error(`the world is not empty: ${n}`);\n",
        )
        .expect("the inert tree has no clients and no outputs");
        assert_eq!(document, serde_json::json!({}));
    }

    #[test]
    fn a_spec_style_config_that_declares_workspaces_and_binds_keys_loads_and_has_an_empty_document() {
        let document = document(
            r#"import { FloatingLayout, Workspace } from "gnoblin";
            gnoblin.workspaces.add(new Workspace({ id: "web", layout: new FloatingLayout() }));
            gnoblin.input.primary.bind("Super+w", () => {});
            gnoblin.clients.style.cornerRadius = 8;"#,
        )
        .expect("a spec-style config loads in the inert shell");
        assert_eq!(document, serde_json::json!({}));
    }

    #[test]
    fn the_starter_loads_and_has_an_empty_document() {
        let document = document(crate::stock::STARTER_CONFIG).expect("the starter loads in the inert shell");
        assert_eq!(document, serde_json::json!({}), "the keyring keeps its default");
    }

    #[test]
    fn a_missing_entry_is_a_load_error() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let error = evaluate_document(&dir.path().join("init.ts")).expect_err("no such file");
        assert!(error.to_string().contains("cannot read"), "{error}");
    }

    #[test]
    fn an_endless_loop_is_cut_off_by_the_budget() {
        let started = std::time::Instant::now();
        let error = document("for (;;) {}\n").expect_err("the loop is interrupted");
        assert_eq!(error.kind, JsErrorKind::Budget, "{error}");
        assert!(started.elapsed() < DOCUMENT_BUDGET * 10, "the budget cut it off");
    }
}
