//! The typed query: conditions, ordering and paging, compiled to bound SQL.
//!
//! A [`Query`] is a set of conditions that all have to hold, joined with `AND`. Each condition
//! is a method, and each method takes typed arguments, so no caller writes SQL and no caller can
//! inject it. [`Query::compile`] turns the conditions into one `WHERE` clause with numbered
//! parameters, and every parameter is bound by value.
//!
//! Matching rules:
//!
//! - A key condition matches whole `/` segments. `prefix("window/1")` matches `window/1` and
//!   `window/1/x`, never `window/10`.
//! - `value_eq` compares the stored encoding, so it matches only values of the same type and
//!   content.
//! - `number_between` compares the number column, which holds every `Int`, `Uint` up to
//!   `i64::MAX`, and `Double`. Other values never match it, and the bounds are `f64`, so an
//!   integer above 2^53 compares with rounding.
//! - `contains` matches text values only, ignoring ASCII case. It uses the FTS5 trigram index
//!   when the phrase is at least three characters, and a `lower()` scan otherwise.

use gnoblin_model::Value;
use rusqlite::types::Value as SqlValue;

use crate::StoreError;
use crate::encode::encode;
use crate::name::validate_prefix;

/// The order results come back in. Every order ends with the key, so equal rows keep a stable
/// position from one page to the next.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Order {
    /// Ascending by key. The default.
    #[default]
    KeyAsc,
    /// Descending by key.
    KeyDesc,
    /// Oldest write first, by the millisecond the key was last written.
    UpdatedAsc,
    /// Newest write first.
    UpdatedDesc,
    /// Ascending by the numeric value. Values that are not numbers do not match a query that
    /// orders this way, because the order only exists for numbers.
    NumberAsc,
    /// Descending by the numeric value.
    NumberDesc,
}

impl Order {
    /// The `ORDER BY` clause for this order. Every string here is a constant of this crate, so
    /// none of it comes from a caller.
    fn sql(self) -> &'static str {
        match self {
            Order::KeyAsc => "key ASC",
            Order::KeyDesc => "key DESC",
            Order::UpdatedAsc => "updated_ms ASC, key ASC",
            Order::UpdatedDesc => "updated_ms DESC, key ASC",
            Order::NumberAsc => "number ASC, key ASC",
            Order::NumberDesc => "number DESC, key ASC",
        }
    }
}

/// One condition of a [`Query`]. Kept private: callers build conditions through the methods.
#[derive(Clone, Debug, PartialEq)]
enum Condition {
    /// The key is exactly this key.
    KeyEq(String),
    /// The key lies under this prefix on whole segments.
    Prefix(String),
    /// The stored value encodes to exactly these bytes.
    ValueEq(Value),
    /// The last write is between these milliseconds, inclusive.
    UpdatedBetween { from_ms: i64, to_ms: i64 },
    /// The number column lies between these bounds, inclusive.
    NumberBetween { low: f64, high: f64 },
    /// The key sorts after this key. Keyset paging: pass the last key of the previous page.
    KeyAfter(String),
    /// The text value contains this phrase, ignoring ASCII case.
    Contains(String),
}

/// A set of conditions, an order, and a page. Build one with [`Query::new`] and the methods.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Query {
    conditions: Vec<Condition>,
    order: Order,
    limit: Option<usize>,
    offset: usize,
}

impl Query {
    /// An empty query: every item, in key order, with no limit.
    pub fn new() -> Self {
        Self::default()
    }

    /// Keeps keys under `prefix` on whole segments. The empty prefix keeps every key.
    pub fn prefix(mut self, prefix: &str) -> Self {
        self.conditions.push(Condition::Prefix(prefix.to_owned()));
        self
    }

    /// Keeps only the key `key`.
    pub fn key_eq(mut self, key: &str) -> Self {
        self.conditions.push(Condition::KeyEq(key.to_owned()));
        self
    }

    /// Keeps values equal to `value`. A value of another type, or a different value of the same
    /// type, does not match.
    pub fn value_eq(mut self, value: Value) -> Self {
        self.conditions.push(Condition::ValueEq(value));
        self
    }

    /// Keeps items last written between `from_ms` and `to_ms` milliseconds since the Unix epoch,
    /// inclusive at both ends.
    pub fn updated_between(mut self, from_ms: i64, to_ms: i64) -> Self {
        self.conditions.push(Condition::UpdatedBetween { from_ms, to_ms });
        self
    }

    /// Keeps numeric values between `low` and `high`, inclusive. A NaN bound is refused when the
    /// query runs.
    pub fn number_between(mut self, low: f64, high: f64) -> Self {
        self.conditions.push(Condition::NumberBetween { low, high });
        self
    }

    /// Keeps text values that contain `phrase`, ignoring ASCII case. An empty phrase is refused
    /// when the query runs.
    pub fn contains(mut self, phrase: &str) -> Self {
        self.conditions.push(Condition::Contains(phrase.to_owned()));
        self
    }

    /// Keeps keys that sort strictly after `key`. Used to page a listing by its last key.
    pub fn after(mut self, key: &str) -> Self {
        self.conditions.push(Condition::KeyAfter(key.to_owned()));
        self
    }

    /// Sets the order. The default is [`Order::KeyAsc`].
    pub fn order(mut self, order: Order) -> Self {
        self.order = order;
        self
    }

    /// Returns at most `limit` items. A search with no limit returns every match.
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Skips the first `offset` matches. Page with `limit` and `offset` together.
    pub fn offset(mut self, offset: usize) -> Self {
        self.offset = offset;
        self
    }

    /// Compiles the conditions to a `WHERE` clause and its parameters. `select` is the column
    /// list, or `COUNT(*)`. When `paged` is true the statement gets this query's order, limit
    /// and offset.
    ///
    /// # Errors
    ///
    /// `invalid-key` for a bad prefix or key, `invalid-query` for an empty phrase or a NaN bound,
    /// `value-not-storable` for a value the store refuses to match.
    pub(crate) fn compile(&self, select: &str, paged: bool) -> Result<(String, Vec<SqlValue>), StoreError> {
        let mut clauses = Vec::new();
        let mut params = Vec::new();

        for condition in &self.conditions {
            match condition {
                Condition::KeyEq(key) => {
                    crate::name::validate_key(key)?;
                    params.push(SqlValue::Text(key.clone()));
                    clauses.push(format!("key = ?{}", params.len()));
                }
                Condition::KeyAfter(key) => {
                    crate::name::validate_key(key)?;
                    params.push(SqlValue::Text(key.clone()));
                    clauses.push(format!("key > ?{}", params.len()));
                }
                Condition::Prefix(prefix) => {
                    validate_prefix(prefix)?;
                    if !prefix.is_empty() {
                        // Keys under `prefix/` sort after `prefix/` and before `prefix0`, because
                        // `0` is the byte after `/`. The two bounds select exactly that run, so
                        // the key index serves the match.
                        params.push(SqlValue::Text(prefix.clone()));
                        let exact = params.len();
                        params.push(SqlValue::Text(format!("{prefix}/")));
                        let low = params.len();
                        params.push(SqlValue::Text(format!("{prefix}0")));
                        let high = params.len();
                        clauses.push(format!("(key = ?{exact} OR (key >= ?{low} AND key < ?{high}))"));
                    }
                }
                Condition::ValueEq(value) => {
                    let bytes = encode(value, usize::MAX)?;
                    params.push(SqlValue::Blob(bytes));
                    clauses.push(format!("value = ?{}", params.len()));
                }
                Condition::UpdatedBetween { from_ms, to_ms } => {
                    params.push(SqlValue::Integer(*from_ms));
                    let low = params.len();
                    params.push(SqlValue::Integer(*to_ms));
                    clauses.push(format!("updated_ms BETWEEN ?{low} AND ?{}", params.len()));
                }
                Condition::NumberBetween { low, high } => {
                    if low.is_nan() || high.is_nan() {
                        return Err(StoreError::InvalidQuery("a number bound is NaN".to_owned()));
                    }
                    params.push(SqlValue::Real(*low));
                    let low_index = params.len();
                    params.push(SqlValue::Real(*high));
                    clauses.push(format!("number BETWEEN ?{low_index} AND ?{}", params.len()));
                }
                Condition::Contains(phrase) => {
                    if phrase.is_empty() {
                        return Err(StoreError::InvalidQuery("contains needs a non-empty phrase".to_owned()));
                    }
                    if phrase.chars().count() >= 3 {
                        // The trigram tokenizer matches the phrase as a quoted string, with any
                        // inner quote doubled so it cannot end the phrase early.
                        let quoted = format!("\"{}\"", phrase.replace('"', "\"\""));
                        params.push(SqlValue::Text(quoted));
                        clauses.push(format!(
                            "rowid IN (SELECT rowid FROM items_fts WHERE items_fts MATCH ?{})",
                            params.len()
                        ));
                    } else {
                        // Shorter phrases have no trigram, so they scan. `lower` folds ASCII only,
                        // the same folding the index uses.
                        params.push(SqlValue::Text(phrase.to_ascii_lowercase()));
                        clauses.push(format!("instr(lower(text), ?{}) > 0", params.len()));
                    }
                }
            }
        }

        let mut sql = format!("SELECT {select} FROM items");
        if !clauses.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&clauses.join(" AND "));
        }
        if paged {
            sql.push_str(" ORDER BY ");
            sql.push_str(self.order.sql());
            // SQLite reads a negative LIMIT as no limit, which is how an unlimited search runs.
            params.push(SqlValue::Integer(self.limit.map_or(-1, |limit| limit as i64)));
            sql.push_str(&format!(" LIMIT ?{}", params.len()));
            params.push(SqlValue::Integer(self.offset as i64));
            sql.push_str(&format!(" OFFSET ?{}", params.len()));
        }
        Ok((sql, params))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prefix_compiles_to_a_key_range_and_never_to_like() {
        let (sql, params) = Query::new().prefix("window/1").compile("key", false).expect("compiles");
        assert!(sql.contains("key >= ?2 AND key < ?3"), "{sql}");
        assert!(!sql.to_ascii_uppercase().contains("LIKE"));
        assert_eq!(params.len(), 3);
    }

    #[test]
    fn a_short_contains_scans_and_a_long_one_uses_the_index() {
        let (short, _) = Query::new().contains("ab").compile("key", false).expect("compiles");
        assert!(short.contains("instr(lower(text)"), "{short}");
        let (long, _) = Query::new().contains("abc").compile("key", false).expect("compiles");
        assert!(long.contains("items_fts MATCH"), "{long}");
    }

    #[test]
    fn an_empty_phrase_and_a_nan_bound_are_refused() {
        assert!(matches!(
            Query::new().contains("").compile("key", false),
            Err(StoreError::InvalidQuery(_))
        ));
        assert!(matches!(
            Query::new().number_between(f64::NAN, 1.0).compile("key", false),
            Err(StoreError::InvalidQuery(_))
        ));
    }

    #[test]
    fn paging_binds_the_limit_and_offset_and_an_unlimited_search_is_minus_one() {
        let (sql, params) = Query::new()
            .order(Order::UpdatedDesc)
            .compile("key", true)
            .expect("compiles");
        assert!(sql.ends_with("LIMIT ?1 OFFSET ?2"), "{sql}");
        assert_eq!(params, vec![SqlValue::Integer(-1), SqlValue::Integer(0)]);
    }
}
