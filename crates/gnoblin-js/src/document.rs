//! A user's config, read for its document and nothing else.
//!
//! [`evaluate_document`] answers "what would this config commit?" without a session: a throwaway generation (no
//! tree, no host objects, a short budget) evaluates the entry, and what `defineConfig` was given is merged over the
//! stock config and desugared exactly as the shell does it (`mergeConfig` and `desugarConfig` of the stock library).
//! The services role uses it for the first-boot `keyring.enabled` decision, which has to be made before the runtime
//! role has committed anything.
//!
//! **What it cannot read.** The generation has no compositor: a config that needs a host object while its module
//! loads (a top-level `gnoblin.clients.length`, an import of a stock module that touches the tree) fails here and
//! only here. The caller treats that as "not known yet" and waits for the document the runtime commits, which is
//! authoritative. A config that only calls `defineConfig` with data and functions, which is what the starter and
//! the documented examples do, is read in full.

use std::path::Path;
use std::time::Duration;

use gnoblin_config::Value;

use crate::autostart;
use crate::error::{JsError, JsErrorKind};
use crate::generation::{JsConfig, JsGeneration};
use crate::stock::stock_modules;

/// The wall-clock budget for evaluating the entry. A config's top level only declares things, and the services
/// role must not wait on it, so a loop that never ends is cut off quickly.
pub const DOCUMENT_BUDGET: Duration = Duration::from_secs(1);

/// The heap limit of the throwaway generation, in bytes.
const DOCUMENT_MEMORY: usize = 32 << 20;

/// The module that stands in for the host: its `__register` is where `defineConfig` hands over the config, and it
/// keeps the document the shell would have committed and the autostart commands.
const PROBE: &str = r##"
import { defaultConfig, desugarConfig, mergeConfig } from "gnoblin";
globalThis.__documentPlan = "";
globalThis.gnoblin = {
    __register(config) {
        const plan = desugarConfig(mergeConfig(defaultConfig, config));
        globalThis.__documentPlan = JSON.stringify({ document: plan.document, autostart: plan.autostart });
    },
};
"##;

/// The name the probe module is embedded under.
const PROBE_NAME: &str = "gnoblin-document-probe";

/// The document the config at `entry` commits, as JSON bytes: its `defineConfig` document settings, and its
/// `autostart` commands as the document's `autostart` section. `{}` when the config never calls `defineConfig`.
///
/// The relative imports of `entry` resolve under its own directory, as they do in the shell.
///
/// # Errors
///
/// The entry cannot be read, does not load (a syntax error, an import that does not resolve, a throw, a host
/// object it needs at load time), runs past [`DOCUMENT_BUDGET`], or its `autostart` is not a list of commands. The
/// error says where, in the author's `.ts` coordinates.
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
    let mut config = JsConfig::new(root).embed(PROBE_NAME, PROBE);
    config.stage_budget = DOCUMENT_BUDGET;
    config.memory_limit = DOCUMENT_MEMORY;
    for (name, source) in stock_modules() {
        config = config.embed(name, source);
    }
    let generation = JsGeneration::new(&config, 1)?;
    generation.eval_specifier(PROBE_NAME.to_owned())?;
    generation.eval_module(&entry)?;
    let plan: String = generation.call_budgeted(DOCUMENT_BUDGET, |ctx| ctx.globals().get("__documentPlan"))?;
    document_of(&plan)
}

/// The document bytes for the plan text the probe left, or `{}` when it left none.
fn document_of(plan: &str) -> Result<Vec<u8>, JsError> {
    let invalid = |reason: String| JsError {
        kind: JsErrorKind::Exception,
        message: format!("the config's document could not be read: {reason}"),
        location: None,
        stack: None,
    };
    if plan.is_empty() {
        return Ok(b"{}".to_vec());
    }
    let json: serde_json::Value = serde_json::from_str(plan).map_err(|error| invalid(error.to_string()))?;
    let mut document = json
        .get("document")
        .and_then(Value::from_json)
        .ok_or_else(|| invalid("the plan has no document".to_owned()))?;
    let commands = json
        .get("autostart")
        .map(|commands| autostart::parse(&commands.to_string()))
        .transpose()
        .map_err(|error| invalid(error.to_string()))?
        .unwrap_or_default();
    if let (Some(section), Value::Map(entries)) = (autostart::section(&commands), &mut document) {
        entries.insert("autostart".to_owned(), section);
    }
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
    fn the_keyring_setting_of_a_config_is_in_its_document() {
        let document = document(
            r#"import { defineConfig } from "gnoblin";
            export default defineConfig({ keyring: { enabled: false } });"#,
        )
        .expect("the config evaluates");
        assert_eq!(document["keyring"]["enabled"], serde_json::json!(false));
    }

    #[test]
    fn a_config_that_never_calls_define_config_has_an_empty_document() {
        assert_eq!(
            document("export const x = 1;").expect("it loads"),
            serde_json::json!({})
        );
    }

    #[test]
    fn a_camel_case_key_is_the_documents_spelling() {
        let document = document(
            r#"import { defineConfig } from "gnoblin";
            defineConfig({ windowManagement: { focusMode: "click" } });"#,
        )
        .expect("the config evaluates");
        assert!(document.get("window-management").is_some(), "{document}");
    }

    #[test]
    fn autostart_commands_become_the_documents_autostart_section() {
        let document = document(
            r#"import { defineConfig } from "gnoblin";
            defineConfig({ autostart: ["waybar", ["foot", "--server"]] });"#,
        )
        .expect("the config evaluates");
        assert_eq!(
            document["autostart"]["waybar"]["command"],
            serde_json::json!(["waybar"])
        );
        assert_eq!(
            document["autostart"]["foot"]["command"],
            serde_json::json!(["foot", "--server"])
        );
    }

    #[test]
    fn a_relative_import_resolves_under_the_entrys_directory() {
        let dir = tempfile::tempdir().expect("a temp dir");
        std::fs::write(
            dir.path().join("keys.ts"),
            "export const keyring = { enabled: false };\n",
        )
        .expect("keys.ts");
        std::fs::write(
            dir.path().join("init.ts"),
            r#"import { defineConfig } from "gnoblin";
            import { keyring } from "./keys.ts";
            defineConfig({ keyring });"#,
        )
        .expect("init.ts");
        let bytes = evaluate_document(&dir.path().join("init.ts")).expect("the config evaluates");
        let document: serde_json::Value = serde_json::from_slice(&bytes).expect("JSON");
        assert_eq!(document["keyring"]["enabled"], serde_json::json!(false));
    }

    #[test]
    fn a_config_that_throws_is_an_error_with_its_position() {
        let error = document("throw new Error(\"boom\");\n").expect_err("a throw is an error");
        assert!(error.to_string().contains("boom"), "{error}");
    }

    #[test]
    fn a_config_that_needs_the_host_while_it_loads_is_an_error_not_a_guess() {
        let error = document("export const n = (globalThis as any).gnoblin.clients.length;\n")
            .expect_err("the throwaway generation has no clients");
        assert!(!error.to_string().is_empty());
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
