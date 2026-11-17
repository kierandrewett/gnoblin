//! Queries: the data-only conditions of `Collection::where_`.

/// One value a property is compared with.
#[derive(Clone, Debug, PartialEq)]
pub enum QueryValue {
    /// Compared with a text property (`app_id`, `title`).
    Text(String),
    /// Compared with a number property (`x`, `width`).
    Number(f64),
    /// Compared with a boolean property (`focused`).
    Bool(bool),
}

impl From<&str> for QueryValue {
    fn from(value: &str) -> QueryValue {
        QueryValue::Text(value.to_owned())
    }
}

impl From<String> for QueryValue {
    fn from(value: String) -> QueryValue {
        QueryValue::Text(value)
    }
}

impl From<f64> for QueryValue {
    fn from(value: f64) -> QueryValue {
        QueryValue::Number(value)
    }
}

impl From<i32> for QueryValue {
    fn from(value: i32) -> QueryValue {
        QueryValue::Number(f64::from(value))
    }
}

impl From<u32> for QueryValue {
    fn from(value: u32) -> QueryValue {
        QueryValue::Number(f64::from(value))
    }
}

impl From<bool> for QueryValue {
    fn from(value: bool) -> QueryValue {
        QueryValue::Bool(value)
    }
}

/// The conditions of a selection: each property has to equal the value, or be one of the values when a key has several. The
/// properties a collection can be queried on are documented on its member type (`Client`, `Output`, `Workspace`).
///
/// ```ignore
/// let terminals = g.std()?.clients().where_(&Query::new().is("app_id", "foot"))?;
/// ```
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Query {
    terms: Vec<(String, QueryValue)>,
}

impl Query {
    /// A query with no conditions: it holds every member.
    pub fn new() -> Query {
        Query::default()
    }

    /// Requires `key` to equal `value`.
    pub fn is(mut self, key: &str, value: impl Into<QueryValue>) -> Query {
        self.terms.push((key.to_owned(), value.into()));
        self
    }

    /// Requires `key` to equal one of `values`.
    pub fn one_of<V: Into<QueryValue>>(mut self, key: &str, values: impl IntoIterator<Item = V>) -> Query {
        self.terms
            .extend(values.into_iter().map(|value| (key.to_owned(), value.into())));
        self
    }

    pub(crate) fn terms(&self) -> &[(String, QueryValue)] {
        &self.terms
    }
}
