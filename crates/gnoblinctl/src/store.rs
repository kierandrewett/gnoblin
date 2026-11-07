//! `store`: read the key-value namespaces the store holds.
//!
//! The namespaces are the files the compositor writes under the store directory. This
//! command opens them read-only and needs no compositor, so it works while the session is
//! down. It has no write verbs. Writes belong to the process that holds the store, and a
//! second writer would break the one-writer rule, so `put` and `delete` wait for the grant
//! design (design/store.md).

use std::io::Write;
use std::path::{Path, PathBuf};

use gnoblin_cli_style::{ColorChoice, OutputEnvironment, OutputTheme, table};
use gnoblin_store::{Entry, Namespace, Options, Store, default_root, namespace_names};

use crate::value;

/// One `store` invocation, as parsed from the command line.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct StoreRequest {
    /// The store directory to read; `None` for the default one.
    pub(crate) root: Option<PathBuf>,
    pub(crate) action: StoreAction,
    pub(crate) json: bool,
}

/// What `store` does with the directory.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum StoreAction {
    /// Every namespace in the directory, with its state and size.
    List,
    /// The bytes a namespace holds against its quota, or every namespace's when `store` is `None`.
    Usage { store: Option<String> },
    /// The value stored under `key` in `store`, with its revision and write time.
    Get { store: String, key: String },
}

/// Run a `store` request. Text is a table or a value; `--json` prints one JSON document.
pub(crate) fn run(request: &StoreRequest, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    match execute(request, out) {
        Ok(()) => 0,
        Err(message) => {
            let _ = writeln!(err, "gnoblinctl: store: {message}");
            1
        }
    }
}

fn execute(request: &StoreRequest, out: &mut dyn Write) -> Result<(), String> {
    let root = match &request.root {
        Some(path) => path.clone(),
        None => default_root(|name| std::env::var(name).ok())
            .ok_or_else(|| "neither $XDG_STATE_HOME nor $HOME is set, so there is no default store".to_owned())?,
    };
    let theme = OutputTheme::resolve(ColorChoice::Auto, &OutputEnvironment::detect_stdout());
    let json = request.json;
    let text = match &request.action {
        StoreAction::List => list(&root, &theme, json)?,
        StoreAction::Usage { store } => usage(&root, store.as_deref(), &theme, json)?,
        StoreAction::Get { store, key } => get(&root, store, key, json)?,
    };
    writeln!(out, "{text}").map_err(|error| format!("cannot write the output: {error}"))
}

/// Opens one namespace read-only. The file must exist: a mistyped name is reported, not shown
/// as an empty namespace, and opening never creates a file.
fn open_namespace(root: &Path, name: &str) -> Result<Namespace, String> {
    if !root.join(format!("{name}.sqlite3")).exists() {
        return Err(format!("no store named {name} at {}", root.display()));
    }
    let options = Options {
        writable: false,
        ..Options::default()
    };
    let store = Store::with_options(root, options)
        .map_err(|error| format!("cannot open the store at {}: {error}", root.display()))?;
    store
        .namespace(name)
        .map_err(|error| format!("cannot open the store {name}: {error}"))
}

/// The namespaces in `root`. Each row names the store, its state and its bytes in use. The
/// listing does not say whether a namespace is writable: the command is read-only, so that
/// column would always be the same.
fn list(root: &Path, theme: &OutputTheme, json: bool) -> Result<String, String> {
    let names = namespace_names(root).map_err(|error| format!("cannot read {}: {error}", root.display()))?;
    let mut rows = Vec::with_capacity(names.len());
    for name in &names {
        let namespace = open_namespace(root, name)?;
        let state = format!("{:?}", namespace.state()).to_lowercase();
        let bytes = namespace
            .bytes_used()
            .map_err(|error| format!("cannot read the store {name}: {error}"))?;
        rows.push((name.clone(), state, bytes));
    }
    if json {
        let entries: Vec<serde_json::Value> = rows
            .iter()
            .map(|(name, state, bytes)| serde_json::json!({ "name": name, "state": state, "bytes": bytes }))
            .collect();
        return Ok(serde_json::Value::Array(entries).to_string());
    }
    if rows.is_empty() {
        return Ok(format!("no stores at {}", root.display()));
    }
    let cells: Vec<Vec<String>> = rows
        .into_iter()
        .map(|(name, state, bytes)| vec![name, state, bytes.to_string()])
        .collect();
    Ok(table(theme, &["NAME", "STATE", "BYTES"], &cells))
}

/// The bytes in use against the quota, for one namespace or for all of them.
fn usage(root: &Path, store: Option<&str>, theme: &OutputTheme, json: bool) -> Result<String, String> {
    let names = match store {
        Some(name) => vec![name.to_owned()],
        None => namespace_names(root).map_err(|error| format!("cannot read {}: {error}", root.display()))?,
    };
    let quota = Options::default().quota;
    let mut rows = Vec::with_capacity(names.len());
    for name in &names {
        let bytes = open_namespace(root, name)?
            .bytes_used()
            .map_err(|error| format!("cannot read the store {name}: {error}"))?;
        rows.push((name.clone(), bytes));
    }
    if json {
        let entries: Vec<serde_json::Value> = rows
            .iter()
            .map(|(name, bytes)| serde_json::json!({ "name": name, "bytes": bytes, "quota": quota }))
            .collect();
        return Ok(serde_json::Value::Array(entries).to_string());
    }
    if rows.is_empty() {
        return Ok(format!("no stores at {}", root.display()));
    }
    let cells: Vec<Vec<String>> = rows
        .into_iter()
        .map(|(name, bytes)| vec![name, bytes.to_string(), quota.to_string()])
        .collect();
    Ok(table(theme, &["NAME", "BYTES", "QUOTA"], &cells))
}

/// The value under `key`. A key with nothing stored under it is an error, so a script can tell
/// it from a stored empty value.
fn get(root: &Path, store: &str, key: &str, json: bool) -> Result<String, String> {
    let namespace = open_namespace(root, store)?;
    let entry = namespace
        .get(key)
        .map_err(|error| format!("cannot read {key} from {store}: {error}"))?
        .ok_or_else(|| format!("{key} is not set in {store}"))?;
    Ok(if json {
        entry_json(&entry).to_string()
    } else {
        value::display(&entry.value)
    })
}

/// One entry as a JSON object: the value, its revision, and when it was written.
fn entry_json(entry: &Entry) -> serde_json::Value {
    serde_json::json!({
        "key": entry.key,
        "value": value::to_json(&entry.value),
        "revision": entry.revision,
        "updated_ms": entry.updated_ms,
    })
}
