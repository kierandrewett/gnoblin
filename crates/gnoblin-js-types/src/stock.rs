//! The stock schema, parsed from the files the build embedded.

use gnoblin_schema::{Catalog, SchemaError, parse_schema};

include!(concat!(env!("OUT_DIR"), "/stock_files.rs"));

/// The checked-in schema, every namespace included (the owner-only `tree` too).
///
/// # Errors
///
/// [`SchemaError`] when an embedded file does not parse or the files disagree. A build the
/// schema's own tests accept cannot hit this; it is returned rather than unwrapped.
pub fn stock_catalog() -> Result<Catalog, SchemaError> {
    let schemas = STOCK_FILES
        .iter()
        .map(|(name, text)| parse_schema(name, text))
        .collect::<Result<Vec<_>, _>>()?;
    Catalog::new(schemas)
}
