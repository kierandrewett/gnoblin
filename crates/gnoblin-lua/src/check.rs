//! Validate a configuration with the runtime's own loader, without serving it.
//!
//! `gnoblinctl config validate init.lua` must give the answer the runtime would give for
//! the same file. This module runs the same steps the runtime role runs before it commits:
//! the stock schemas, a host, `run_config`, the configured extensions and the autostart
//! entries. It stops before the commit, so nothing reaches a compositor.
//!
//! # Limitation
//!
//! The host runs against an in-process server with an empty object graph. A configuration
//! that only evaluates against live objects, such as a `gnoblin.*` read of a window that
//! exists in the session, can validate here and still fail at runtime, or the reverse. The
//! document's shape, its types, its extensions and its autostart entries are checked the way
//! the runtime checks them.

use gnoblin_client::Server;
use gnoblin_config::SearchPaths;
use gnoblin_lua_sandbox::Limits;
use gnoblin_model::Graph;
use gnoblin_schema::{Catalog, Schema, SchemaError, parse_schema};

use crate::host::Host;
use crate::role::{configured_extensions, enabled_autostart, version};

include!(concat!(env!("OUT_DIR"), "/stock_files.rs"));

/// What one validation found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Check {
    /// Deprecated spellings the document still uses. They do not fail validation.
    pub warnings: Vec<String>,
    /// Every reason the runtime would refuse the document. Empty when it would accept it.
    pub problems: Vec<String>,
}

/// The stock schemas, parsed from the files the build embedded.
///
/// # Errors
///
/// [`SchemaError`] when an embedded file does not parse. A build the build script accepted
/// cannot hit this; it is reported rather than unwrapped.
pub fn stock_schemas() -> Result<Vec<Schema>, SchemaError> {
    STOCK_FILES
        .iter()
        .map(|(name, text)| parse_schema(name, text))
        .collect()
}

/// Validate the configuration `paths` locate, the way the runtime would.
///
/// # Errors
///
/// A `String` when the check cannot run at all: the stock schemas do not load, the
/// configuration cannot be located or read, or the host cannot start. A configuration that
/// is read and refused is not an error: it is a [`Check`] with problems.
pub fn check(paths: &SearchPaths) -> Result<Check, String> {
    let stock = stock_schemas().map_err(|error| format!("the stock schemas do not load: {error}"))?;
    let catalog = Catalog::new(stock.clone()).map_err(|error| format!("the stock schemas do not load: {error}"))?;
    let mut server = Server::new(Graph::new(catalog), version());
    let client = server
        .connect(version())
        .map_err(|error| format!("cannot connect the validator: {error}"))?;
    let mut host = Host::new(client, Limits::default()).map_err(|error| error.to_string())?;
    let report = host.run_config(paths).map_err(|error| error.to_string())?;

    let mut check = Check::default();
    for error in &report.errors {
        check.problems.push(error.to_string());
    }
    match &report.config {
        Ok(config) => {
            check.warnings = config.deprecations().iter().map(ToString::to_string).collect();
            if let Err(problem) = enabled_autostart(config) {
                check.problems.push(problem);
            }
            let (_registry, refusals) = configured_extensions(&report, &stock);
            check.problems.extend(refusals);
        }
        Err(problems) => check.problems.push(problems.to_string()),
    }
    Ok(check)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stock_schemas_parse_and_form_a_catalog() {
        let schemas = stock_schemas().expect("the embedded schemas parse");
        assert!(!schemas.is_empty());
        Catalog::new(schemas).expect("the embedded schemas make one catalog");
    }
}
