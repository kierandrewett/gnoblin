//! Extensions named in host configuration (plan "IPC extensions", D21 and D22).
//!
//! `gnoblin.configure { extensions = { ... } }` lists each extension the host loads.
//! Each entry is a table with four keys:
//!
//! - `namespace`: the namespace, such as `example.ext`. It must be unique, must not
//!   be the core namespace, and must not be a namespace the stock catalog owns.
//! - `schema`: the path to the schema file the extension's package ships. A relative
//!   path is read from the directory of the configuration file.
//! - `provider`: the privileged connection role allowed to provide the namespace.
//! - `grants`: the restricted socket roles granted the namespace.
//!
//! Each entry is checked on its own. A bad entry is refused with a named reason and
//! the other entries still load, so one broken extension never stops the compositor
//! from starting without the rest. An accepted schema never alters the stock catalog:
//! its objects must not reuse a stock or already accepted object name, and the schema
//! parser already refuses duplicate member names within an object.
//!
//! This module reads schema files, so the caller passes the directory of the
//! configuration file and the stock schemas. It holds no extension name of its own.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use gnoblin_schema::{Catalog, Schema, parse_schema};

use crate::value::Value;

/// The namespace the compositor's own members live under. An extension cannot claim it.
pub const CORE_NAMESPACE: &str = "gnoblin";

/// The keys an entry may hold. Any other key refuses the entry.
const ENTRY_KEYS: [&str; 4] = ["namespace", "schema", "provider", "grants"];

/// One extension that passed every check, with its parsed schema.
#[derive(Clone, Debug)]
pub struct ExtensionEntry {
    /// The namespace the extension provides, such as `example.ext`.
    pub namespace: String,
    /// The schema path, resolved against the configuration directory.
    pub schema_path: PathBuf,
    /// The privileged connection role that provides the namespace.
    pub provider: String,
    /// The restricted socket roles granted the namespace, in configuration order.
    pub grants: Vec<String>,
    /// The parsed schema, whose namespace equals `namespace`.
    pub schema: Schema,
}

/// One entry that was refused, with the namespace it named and the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtensionRefusal {
    /// The namespace the entry named, or `<extensions[N]>` when it named none.
    pub namespace: String,
    /// The reason, naming the check that failed.
    pub reason: String,
}

impl fmt::Display for ExtensionRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "extension `{}` refused: {}", self.namespace, self.reason)
    }
}

/// The result of reading the `extensions` section: what loaded, and what did not.
#[derive(Debug, Default)]
pub struct LoadedExtensions {
    /// Extensions that passed every check, in configuration order.
    pub accepted: Vec<ExtensionEntry>,
    /// Entries that failed, in configuration order.
    pub refused: Vec<ExtensionRefusal>,
}

/// Read the `extensions` section of an accepted configuration document.
///
/// `base` is the directory relative schema paths resolve against. `stock` is the
/// stock catalog's schemas; an extension may not reuse their namespaces or object
/// names. A document with no `extensions` section loads nothing and refuses nothing.
pub fn load_extensions(document: &Value, base: &Path, stock: &[Schema]) -> LoadedExtensions {
    let mut out = LoadedExtensions::default();
    let Some(section) = document.as_table().and_then(|table| table.get("extensions")) else {
        return out;
    };
    let Some(entries) = section.as_list() else {
        out.refused.push(ExtensionRefusal {
            namespace: "<extensions>".to_owned(),
            reason: format!("expected a list of extensions, found {}", section.kind()),
        });
        return out;
    };
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for (index, entry) in entries.iter().enumerate() {
        match load_one(entry, index, base, stock, &out.accepted, &mut seen) {
            Ok(accepted) => out.accepted.push(accepted),
            Err(refusal) => out.refused.push(refusal),
        }
    }
    out
}

fn load_one(
    entry: &Value,
    index: usize,
    base: &Path,
    stock: &[Schema],
    accepted: &[ExtensionEntry],
    seen: &mut BTreeSet<String>,
) -> Result<ExtensionEntry, ExtensionRefusal> {
    let Some(table) = entry.as_table() else {
        return Err(refuse(
            format!("<extensions[{index}]>"),
            format!("expected a table, found {}", entry.kind()),
        ));
    };
    let namespace =
        text_field(table, "namespace").map_err(|reason| refuse(format!("<extensions[{index}]>"), reason))?;
    let here = |reason: String| refuse(namespace.clone(), reason);

    if let Some(unknown) = table.keys().find(|key| !ENTRY_KEYS.contains(&key.as_str())) {
        return Err(here(format!("`{unknown}` is not an extension key")));
    }
    check_namespace(&namespace, stock).map_err(here)?;
    if !seen.insert(namespace.clone()) {
        return Err(here("the namespace is listed more than once".to_owned()));
    }
    let provider = text_field(table, "provider").map_err(here)?;
    if provider.is_empty() {
        return Err(here("the provider role is empty".to_owned()));
    }
    let grants = grants_field(table).map_err(here)?;
    let schema_name = text_field(table, "schema").map_err(here)?;

    let schema_path = base.join(&schema_name);
    let text = std::fs::read_to_string(&schema_path)
        .map_err(|source| here(format!("cannot read the schema at {}: {source}", schema_path.display())))?;
    let schema = parse_schema(&schema_path.display().to_string(), &text)
        .map_err(|error| here(format!("the schema does not parse: {error}")))?;
    if schema.namespace.as_str() != namespace {
        return Err(here(format!(
            "the schema declares namespace `{}`, not `{namespace}`",
            schema.namespace
        )));
    }
    check_members(&schema, stock, accepted).map_err(here)?;

    let candidate: Vec<Schema> = stock
        .iter()
        .cloned()
        .chain(accepted.iter().map(|extension| extension.schema.clone()))
        .chain(std::iter::once(schema.clone()))
        .collect();
    Catalog::new(candidate).map_err(|error| here(format!("the schema does not fit the catalog: {error}")))?;

    Ok(ExtensionEntry {
        namespace,
        schema_path,
        provider,
        grants,
        schema,
    })
}

fn refuse(namespace: String, reason: String) -> ExtensionRefusal {
    ExtensionRefusal { namespace, reason }
}

fn check_namespace(namespace: &str, stock: &[Schema]) -> Result<(), String> {
    if namespace == CORE_NAMESPACE {
        return Err(format!("`{CORE_NAMESPACE}` is the core namespace"));
    }
    if namespace.is_empty() || namespace.split('.').any(str::is_empty) {
        return Err("the namespace is empty or has an empty segment".to_owned());
    }
    if stock.iter().any(|known| known.namespace.as_str() == namespace) {
        return Err("the namespace belongs to the stock catalog".to_owned());
    }
    Ok(())
}

/// Refuse a schema whose object names collide with a stock or accepted object.
///
/// An object name may not repeat one that the stock catalog or an already accepted
/// extension declares, because Lua exposes each object as a class by name. Member
/// names are unique within an object by the schema parser's own rule.
fn check_members(schema: &Schema, stock: &[Schema], accepted: &[ExtensionEntry]) -> Result<(), String> {
    let mut declared: BTreeMap<&str, &str> = BTreeMap::new();
    for known in stock.iter().chain(accepted.iter().map(|extension| &extension.schema)) {
        for object in &known.objects {
            declared.insert(object.name.as_str(), known.namespace.as_str());
        }
    }
    for object in &schema.objects {
        if let Some(owner) = declared.get(object.name.as_str()) {
            return Err(format!(
                "object `{}` is already declared by namespace `{owner}`",
                object.name
            ));
        }
    }
    Ok(())
}

fn text_field(table: &BTreeMap<String, Value>, key: &str) -> Result<String, String> {
    match table.get(key) {
        Some(Value::String(text)) if !text.is_empty() => Ok(text.clone()),
        Some(Value::String(_)) => Err(format!("`{key}` is empty")),
        Some(other) => Err(format!("`{key}` expected text, found {}", other.kind())),
        None => Err(format!("`{key}` is missing")),
    }
}

fn grants_field(table: &BTreeMap<String, Value>) -> Result<Vec<String>, String> {
    let Some(value) = table.get("grants") else {
        return Err("`grants` is missing".to_owned());
    };
    let Some(items) = value.as_list() else {
        return Err(format!("`grants` expected a list of roles, found {}", value.kind()));
    };
    let mut roles: Vec<String> = Vec::with_capacity(items.len());
    for item in items {
        let Some(role) = item.as_str() else {
            return Err(format!("`grants` expected role names, found {}", item.kind()));
        };
        if role.is_empty() {
            return Err("`grants` names an empty role".to_owned());
        }
        if roles.iter().any(|known| known == role) {
            return Err(format!("`grants` names `{role}` twice"));
        }
        roles.push(role.to_owned());
    }
    Ok(roles)
}
