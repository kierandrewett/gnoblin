//! Layer names: a provider namespace, a dot, then a local part.
//!
//! Every layer has one full name, `<provider>.<local>`. The registry builds it
//! from the provider that registers the layer, so a provider supplies only the
//! local part and cannot choose the prefix. Gnoblin's built-in providers share
//! the single namespace `gnoblin`, so `gnoblin.portals` is the portal layer. An
//! extension with id `<id>` gets the namespace `ext.<id>`, so its layers are
//! `ext.<id>.<local>`.
//!
//! The uniform prefix is deliberate. Two providers can never collide, because
//! their namespaces differ by construction, and no reserved-name list has to be
//! kept in step with the built-ins. The origin of a layer is also visible in
//! `layers.list`, in config keys (`layers.<full name>.secure`) and in logs.
//!
//! The grammar is kept unambiguous so a full name can be parsed back from text.
//! A namespace is one segment for `gnoblin`, or `ext.` and a single segment
//! `<id>`. A local part has no dots, so the first dot after the namespace is the
//! boundary. A local part may carry one `:<key>` suffix for per-object records,
//! such as `surface_toplevel:42`; the key may contain dots because it is the
//! rest of the string, as in `surface_layer_shell:org.gnoblin.TestBar`.

use std::fmt;

/// The namespace every Gnoblin built-in provider shares.
pub const GNOBLIN_NAMESPACE: &str = "gnoblin";

/// The longest local part, counting any `:<key>` suffix. The limit keeps a full
/// name short enough for a log line and a config key.
pub const MAX_LOCAL_LEN: usize = 48;

/// The longest extension id. The namespace is `ext.<id>`, so this bounds it.
pub const MAX_EXTENSION_ID_LEN: usize = 32;

/// Why a name or identifier was refused. Each variant is one grammar rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerNameError {
    /// The text is empty. Every name and identifier needs at least one character.
    Empty,
    /// The text is longer than its limit.
    TooLong,
    /// The text has a character outside its alphabet.
    BadChar(char),
    /// A `:<key>` suffix is empty or malformed.
    BadKey,
}

impl fmt::Display for LayerNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LayerNameError::Empty => write!(f, "name is empty"),
            LayerNameError::TooLong => write!(f, "name is too long"),
            LayerNameError::BadChar(c) => write!(f, "name has an invalid character {c:?}"),
            LayerNameError::BadKey => write!(f, "name has a malformed `:<key>` suffix"),
        }
    }
}

impl std::error::Error for LayerNameError {}

/// Whether `c` may appear in a local part's base segment.
///
/// Lowercase only, so a name is the same string on every platform and in every
/// config file. Dots and colons are excluded: dots separate the namespace from
/// the local part, and the colon is reserved for the `:<key>` suffix.
fn is_base_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-'
}

/// Whether `c` may appear in a `:<key>` suffix. The key is a per-object id, such as
/// a window id or a client namespace, so it may use the case and dots of its source.
/// `#` is the separator a second live surface with the same namespace gets, as in
/// `surface_layer_shell:bar#2`.
fn is_key_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '#'
}

/// The part of a full name after the namespace: a base name, optionally with a
/// `:<key>` suffix.
///
/// Built only through [`LocalName::new`] or [`LocalName::keyed`], so every value
/// obeys the grammar. The base is at most [`MAX_LOCAL_LEN`] characters with the
/// suffix included.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalName(String);

impl LocalName {
    /// Validates a local part such as `portals` or `surface_toplevel:42`.
    ///
    /// # Errors
    ///
    /// Returns [`LayerNameError::Empty`] or [`LayerNameError::TooLong`] for a length
    /// violation, [`LayerNameError::BadChar`] for a character outside the alphabet, and
    /// [`LayerNameError::BadKey`] when the `:<key>` suffix is empty or malformed.
    pub fn new(raw: &str) -> Result<Self, LayerNameError> {
        if raw.is_empty() {
            return Err(LayerNameError::Empty);
        }
        if raw.len() > MAX_LOCAL_LEN {
            return Err(LayerNameError::TooLong);
        }
        let (base, key) = match raw.split_once(':') {
            Some((base, key)) => (base, Some(key)),
            None => (raw, None),
        };
        if base.is_empty() {
            return Err(LayerNameError::Empty);
        }
        if let Some(c) = base.chars().find(|&c| !is_base_char(c)) {
            return Err(LayerNameError::BadChar(c));
        }
        if let Some(key) = key {
            // A key is the per-object id; an empty key or a second colon is
            // refused so `name:` and `a:b:c` have one meaning each.
            if key.is_empty() || key.contains(':') {
                return Err(LayerNameError::BadKey);
            }
            if let Some(c) = key.chars().find(|&c| !is_key_char(c)) {
                return Err(LayerNameError::BadChar(c));
            }
        }
        Ok(Self(raw.to_owned()))
    }

    /// Builds `<base>:<key>`, the name of a per-object record such as a window's
    /// `surface_toplevel` layer. The base must be a plain local name, not already
    /// keyed.
    ///
    /// # Errors
    ///
    /// Refuses everything [`LocalName::new`] refuses, and a base that already
    /// carries a key.
    pub fn keyed(base: &str, key: &str) -> Result<Self, LayerNameError> {
        if base.contains(':') {
            return Err(LayerNameError::BadKey);
        }
        Self::new(&format!("{base}:{key}"))
    }

    /// The text of the local part, with any `:<key>` suffix.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for LocalName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The namespace a provider registers under: `gnoblin` for every built-in, or
/// `ext.<id>` for an extension.
///
/// The registry derives it from the [`crate::LayerProvider`], so a provider cannot
/// register under another namespace.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProviderId(String);

impl ProviderId {
    /// The namespace of every Gnoblin built-in provider.
    #[must_use]
    pub fn gnoblin() -> Self {
        Self(GNOBLIN_NAMESPACE.to_owned())
    }

    /// The namespace `ext.<id>` for an extension with this id.
    ///
    /// The id is a single segment of lowercase letters, digits, `_` and `-`. A
    /// dot would make `ext.a.b` ambiguous with a namespace `ext.a` and a local
    /// part `b`, so it is refused. The id `gnoblin` is refused by the caller
    /// with `RegisterError::ProviderIdReserved`, not here, so this function only
    /// checks the grammar.
    ///
    /// # Errors
    ///
    /// Returns [`LayerNameError::Empty`], [`LayerNameError::TooLong`] or
    /// [`LayerNameError::BadChar`] for an id outside the grammar.
    pub fn extension(id: &str) -> Result<Self, LayerNameError> {
        if id.is_empty() {
            return Err(LayerNameError::Empty);
        }
        if id.len() > MAX_EXTENSION_ID_LEN {
            return Err(LayerNameError::TooLong);
        }
        if let Some(c) = id.chars().find(|&c| !is_base_char(c)) {
            return Err(LayerNameError::BadChar(c));
        }
        Ok(Self(format!("ext.{id}")))
    }

    /// The namespace text, such as `gnoblin` or `ext.example`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A full layer name: `<provider>.<local>`.
///
/// This is the one identity a layer has. Config keys, `Order` targets, secure
/// overrides and logs all use it, so a name means the same layer everywhere.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LayerName {
    provider: ProviderId,
    local: LocalName,
}

impl LayerName {
    /// Joins a namespace and a local part. Both are already validated, so the
    /// result is always a well-formed name.
    #[must_use]
    pub fn new(provider: ProviderId, local: LocalName) -> Self {
        Self { provider, local }
    }

    /// The namespace that registered the layer.
    #[must_use]
    pub fn provider(&self) -> &ProviderId {
        &self.provider
    }

    /// The part after the namespace, which the provider chose.
    #[must_use]
    pub fn local(&self) -> &LocalName {
        &self.local
    }

    /// Parses a full name from text, as config and Lua references give it.
    ///
    /// A `gnoblin.` prefix names a built-in. An `ext.` prefix takes the next
    /// segment as the extension id and the rest as the local part. Anything else
    /// is refused.
    ///
    /// # Errors
    ///
    /// Returns the [`LayerNameError`] for the first part that breaks the grammar. A
    /// missing namespace or local part is [`LayerNameError::Empty`].
    pub fn parse(text: &str) -> Result<Self, LayerNameError> {
        if let Some(local) = text.strip_prefix("gnoblin.") {
            return Ok(Self::new(ProviderId::gnoblin(), LocalName::new(local)?));
        }
        if let Some(rest) = text.strip_prefix("ext.") {
            let (id, local) = rest.split_once('.').ok_or(LayerNameError::Empty)?;
            return Ok(Self::new(ProviderId::extension(id)?, LocalName::new(local)?));
        }
        let first = text.chars().next().ok_or(LayerNameError::Empty)?;
        Err(LayerNameError::BadChar(first))
    }
}

impl fmt::Display for LayerName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.provider, self.local)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_accepts_plain_and_keyed_names() {
        assert!(LocalName::new("portals").is_ok());
        assert!(LocalName::new("window_menu").is_ok());
        assert!(LocalName::new("surface_toplevel:42").is_ok());
        assert!(LocalName::new("surface_layer_shell:org.gnoblin.TestBar").is_ok());
    }

    #[test]
    fn local_refuses_empty_and_too_long() {
        assert_eq!(LocalName::new(""), Err(LayerNameError::Empty));
        assert_eq!(LocalName::new(":key"), Err(LayerNameError::Empty));
        let long = "a".repeat(MAX_LOCAL_LEN + 1);
        assert_eq!(LocalName::new(&long), Err(LayerNameError::TooLong));
        assert!(LocalName::new(&"a".repeat(MAX_LOCAL_LEN)).is_ok());
    }

    #[test]
    fn local_refuses_uppercase_and_dots_in_the_base() {
        // Dots separate the namespace; an uppercase base would differ by platform.
        assert_eq!(LocalName::new("Portals"), Err(LayerNameError::BadChar('P')));
        assert_eq!(LocalName::new("a.b"), Err(LayerNameError::BadChar('.')));
    }

    #[test]
    fn local_refuses_malformed_key() {
        assert_eq!(LocalName::new("surface:"), Err(LayerNameError::BadKey));
        assert_eq!(LocalName::new("surface:a:b"), Err(LayerNameError::BadKey));
        assert_eq!(LocalName::new("surface:a/b"), Err(LayerNameError::BadChar('/')));
    }

    #[test]
    fn keyed_builds_one_suffix_and_refuses_a_keyed_base() {
        assert_eq!(
            LocalName::keyed("surface_toplevel", "42").unwrap().as_str(),
            "surface_toplevel:42"
        );
        // A second live surface with the same client namespace takes a `#n` suffix.
        assert_eq!(
            LocalName::keyed("surface_layer_shell", "bar#2").unwrap().as_str(),
            "surface_layer_shell:bar#2"
        );
        assert_eq!(LocalName::keyed("surface:x", "42"), Err(LayerNameError::BadKey));
    }

    #[test]
    fn extension_id_is_one_segment() {
        assert_eq!(ProviderId::extension("example").unwrap().as_str(), "ext.example");
        assert_eq!(ProviderId::extension("a.b"), Err(LayerNameError::BadChar('.')));
        assert_eq!(ProviderId::extension(""), Err(LayerNameError::Empty));
    }

    #[test]
    fn full_name_displays_with_its_namespace() {
        let name = LayerName::new(ProviderId::gnoblin(), LocalName::new("portals").unwrap());
        assert_eq!(name.to_string(), "gnoblin.portals");
        let ext = LayerName::new(ProviderId::extension("ex").unwrap(), LocalName::new("bar").unwrap());
        assert_eq!(ext.to_string(), "ext.ex.bar");
    }

    #[test]
    fn parse_round_trips_built_in_and_extension_names() {
        let built = LayerName::parse("gnoblin.surface_layer_shell:org.gnoblin.TestBar").unwrap();
        assert_eq!(built.provider(), &ProviderId::gnoblin());
        assert_eq!(built.to_string(), "gnoblin.surface_layer_shell:org.gnoblin.TestBar");

        let ext = LayerName::parse("ext.ex.bar").unwrap();
        assert_eq!(ext.provider().as_str(), "ext.ex");
        assert_eq!(ext.local().as_str(), "bar");
        assert_eq!(ext.to_string(), "ext.ex.bar");
    }

    #[test]
    fn parse_refuses_unknown_namespaces_and_missing_parts() {
        assert_eq!(LayerName::parse("portals"), Err(LayerNameError::BadChar('p')));
        assert_eq!(LayerName::parse("ext.ex"), Err(LayerNameError::Empty));
        assert_eq!(LayerName::parse("gnoblin."), Err(LayerNameError::Empty));
        assert_eq!(LayerName::parse(""), Err(LayerNameError::Empty));
    }
}
