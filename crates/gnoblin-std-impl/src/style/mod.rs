//! Styles: borders, shadows, backdrop filters, opacity and corner radius, as the cascade resolves them for each client and as the
//! tree's effects express them.
//!
//! `data/js/gnoblin-std/style/` and `host/style_scopes.ts`, `host/styles.ts` are the reference.

pub(crate) mod cascade;
pub(crate) mod edit;
pub(crate) mod objects;
pub(crate) mod table;
pub(crate) mod values;
