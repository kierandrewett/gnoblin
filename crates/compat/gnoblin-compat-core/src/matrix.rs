//! The support matrix: what one adapter declares about every upstream member.
//!
//! An adapter is only as honest as its declaration, so the declaration is data:
//! one [`SupportMatrix`] per adapter, one [`Interface`] per upstream D-Bus
//! interface, one [`Member`] per method, property and signal. Every member is
//! `supported`, `partial` with a note or `unsupported` with a reason, which is
//! the plan's "declared fidelity" rule ("Compatibility adapters", rule 2).
//!
//! Two renderings share one ordering and one schema, so a matrix can be reviewed
//! in a pull request and read by a script:
//!
//! - [`SupportMatrix::render_text`] is the human report, also printed per
//!   adapter by `cargo xtask compat-report`;
//! - [`SupportMatrix::render_json`] is the checked-in `support-matrix.json` an
//!   adapter loads with [`SupportMatrix::from_json`].
//!
//! A member that is absent is not "supported": [`SupportMatrix::support`]
//! returns `None`, and [`crate::Adapter::serve`] refuses such a call with
//! [`crate::Rule::Undeclared`], because an undeclared member is how a caller
//! ends up depending on behaviour nobody promised.

use std::fmt::{self, Write as _};

use serde_json::{Map, Value};

/// What kind of interface member a matrix entry describes.
///
/// The declaration order is also the report order, because methods, properties
/// and signals are read separately.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub enum Kind {
    /// A D-Bus method. Its `support` covers the call and its immediate reply.
    Method,
    /// A D-Bus property. A read is [`crate::OperationClass::Read`], a write is
    /// [`crate::OperationClass::Write`], and one entry covers both.
    Property,
    /// A D-Bus signal. An adapter emits it; no caller calls it.
    Signal,
}

impl Kind {
    /// The spelling used in the JSON manifest and the text report.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Method => "method",
            Self::Property => "property",
            Self::Signal => "signal",
        }
    }

    /// Parse the manifest spelling, rejecting anything else.
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "method" => Some(Self::Method),
            "property" => Some(Self::Property),
            "signal" => Some(Self::Signal),
            _ => None,
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// How faithfully an adapter serves one member.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Support {
    /// The member behaves as its upstream definition says.
    Supported,
    /// The member answers, but not identically. `note` says how it differs, so
    /// a consumer can decide whether the difference matters to it.
    Partial {
        /// What differs from upstream.
        note: String,
    },
    /// The member is refused with a D-Bus error that carries `reason`. It is
    /// never answered with a plausible-looking default.
    Unsupported {
        /// Why the member is refused, in the terms the caller will read.
        reason: String,
    },
}

impl Support {
    /// A partial member with its note.
    pub fn partial(note: impl Into<String>) -> Self {
        Self::Partial { note: note.into() }
    }

    /// An unsupported member with its reason.
    pub fn unsupported(reason: impl Into<String>) -> Self {
        Self::Unsupported { reason: reason.into() }
    }

    /// The manifest and report spelling.
    pub const fn word(&self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::Partial { .. } => "partial",
            Self::Unsupported { .. } => "unsupported",
        }
    }

    /// The note for a partial member, or the reason for an unsupported one.
    pub fn detail(&self) -> Option<&str> {
        match self {
            Self::Supported => None,
            Self::Partial { note } => Some(note),
            Self::Unsupported { reason } => Some(reason),
        }
    }
}

impl fmt::Display for Support {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.word())
    }
}

/// One method, property or signal of an upstream interface.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Member {
    kind: Kind,
    name: String,
    support: Support,
}

impl Member {
    /// A member declaration. The builder is the usual way to make one.
    pub fn new(kind: Kind, name: impl Into<String>, support: Support) -> Self {
        Self {
            kind,
            name: name.into(),
            support,
        }
    }

    /// What kind of member this is.
    pub const fn kind(&self) -> Kind {
        self.kind
    }

    /// The upstream member name, without the interface.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The declared fidelity.
    pub const fn support(&self) -> &Support {
        &self.support
    }
}

/// One upstream interface and every member an adapter declares for it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Interface {
    name: String,
    version: u32,
    members: Vec<Member>,
}

impl Interface {
    /// An interface with no members yet.
    pub fn new(name: impl Into<String>, version: u32) -> Self {
        Self {
            name: name.into(),
            version,
            members: Vec::new(),
        }
    }

    /// The upstream interface name, such as `org.gnome.Mutter.DisplayConfig`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The upstream version the adapter was tested against, as upstream reports
    /// it (for `DisplayConfig`, the version its state call returns).
    pub const fn version(&self) -> u32 {
        self.version
    }

    /// Every declared member, in report order.
    pub fn members(&self) -> &[Member] {
        &self.members
    }

    /// The first member with this name, whatever its kind.
    pub fn member(&self, name: &str) -> Option<&Member> {
        self.members.iter().find(|member| member.name == name)
    }
}

/// One adapter's declared fidelity, over every interface it serves.
///
/// Ordered by interface name, and by kind then name within an interface, so the
/// same declaration always renders to the same bytes and a report diff shows
/// only what changed.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct SupportMatrix {
    interfaces: Vec<Interface>,
}

impl SupportMatrix {
    /// A matrix that declares nothing, which serves nothing.
    pub const fn new() -> Self {
        Self { interfaces: Vec::new() }
    }

    /// Start a declaration. Members cannot be added without naming their
    /// interface, so a member can never float free of one.
    pub fn builder() -> Builder {
        Builder::default()
    }

    /// Every interface, in report order.
    pub fn interfaces(&self) -> &[Interface] {
        &self.interfaces
    }

    /// One interface by name.
    pub fn interface(&self, name: &str) -> Option<&Interface> {
        self.interfaces.iter().find(|interface| interface.name == name)
    }

    /// The fidelity one member is declared with, or `None` when the adapter
    /// declares nothing for it. `None` is a refusal, never a silent success.
    pub fn support(&self, interface: &str, member: &str) -> Option<&Support> {
        self.interface(interface)?.member(member).map(Member::support)
    }

    /// Whether nothing is declared, which serves nothing.
    pub fn is_empty(&self) -> bool {
        self.interfaces.is_empty()
    }

    /// The human report: one header line per interface, one line per member.
    ///
    /// No heading and no adapter name, so a caller can nest it under whatever
    /// identifies the adapter; `cargo xtask compat-report` prints exactly these
    /// bytes under its own `[compat-report]` lines.
    pub fn render_text(&self) -> String {
        let mut out = String::new();
        for interface in &self.interfaces {
            writeln!(out, "interface {} version {}", interface.name, interface.version)
                .expect("writing to a String cannot fail");
            for member in &interface.members {
                write!(out, "  {} {} {}", member.kind, member.name, member.support)
                    .expect("writing to a String cannot fail");
                match member.support.detail() {
                    Some(detail) => writeln!(out, ": {detail}").expect("writing to a String cannot fail"),
                    None => writeln!(out).expect("writing to a String cannot fail"),
                }
            }
        }
        out
    }

    /// The manifest form: checked in beside an adapter's `Cargo.toml` and loaded
    /// with [`SupportMatrix::from_json`] at start-up.
    pub fn render_json(&self) -> String {
        let mut out = String::new();
        if self.interfaces.is_empty() {
            out.push_str("{\n  \"interfaces\": []\n}\n");
            return out;
        }
        out.push_str("{\n  \"interfaces\": [\n");
        for (index, interface) in self.interfaces.iter().enumerate() {
            out.push_str("    {\n");
            writeln!(out, "      \"name\": {},", json_string(&interface.name))
                .expect("writing to a String cannot fail");
            writeln!(out, "      \"version\": {},", interface.version).expect("writing to a String cannot fail");
            if interface.members.is_empty() {
                out.push_str("      \"members\": []\n");
            } else {
                out.push_str("      \"members\": [\n");
                for (position, member) in interface.members.iter().enumerate() {
                    // One field per line rather than a compact object: a note
                    // or a reason is prose of any length, so a compact form
                    // would have to be wrapped, and the wrap would depend on the
                    // prose. This form is the same for every declaration, which
                    // is what makes the checked-in manifest diff cleanly and
                    // what `.prettierrc.json` leaves alone.
                    out.push_str("        {\n");
                    write!(out, "          \"kind\": {},", json_string(member.kind.as_str()))
                        .expect("writing to a String cannot fail");
                    out.push('\n');
                    write!(out, "          \"name\": {},", json_string(&member.name))
                        .expect("writing to a String cannot fail");
                    out.push('\n');
                    match member.support {
                        Support::Supported => {
                            out.push_str("          \"support\": \"supported\"\n");
                        }
                        Support::Partial { ref note } => {
                            out.push_str("          \"support\": \"partial\",\n");
                            write!(out, "          \"note\": {}", json_string(note))
                                .expect("writing to a String cannot fail");
                            out.push('\n');
                        }
                        Support::Unsupported { ref reason } => {
                            out.push_str("          \"support\": \"unsupported\",\n");
                            write!(out, "          \"reason\": {}", json_string(reason))
                                .expect("writing to a String cannot fail");
                            out.push('\n');
                        }
                    }
                    if position + 1 == interface.members.len() {
                        out.push_str("        }\n");
                    } else {
                        out.push_str("        },\n");
                    }
                }
                out.push_str("      ]\n");
            }
            if index + 1 == self.interfaces.len() {
                out.push_str("    }\n");
            } else {
                out.push_str("    },\n");
            }
        }
        out.push_str("  ]\n}\n");
        out
    }

    /// Parse a manifest, rejecting anything the frame could not honour.
    ///
    /// Unknown keys are errors rather than ignored, so a mistyped `support` word
    /// cannot turn into a silent default, and a `partial` or `unsupported`
    /// member without its note or reason is refused as well: a declaration with
    /// nothing to say is not a declaration.
    pub fn from_json(text: &str) -> Result<Self, MatrixError> {
        let document: Value =
            serde_json::from_str(text).map_err(|error| MatrixError::new("<document>", error.to_string()))?;
        let object = expect_object(&document, "<document>")?;
        expect_keys(object, &["interfaces"], "<document>")?;
        let entries = expect_array(expect_field(object, "interfaces", "<document>")?, "interfaces")?;
        let mut interfaces = Vec::with_capacity(entries.len());
        for (index, entry) in entries.iter().enumerate() {
            interfaces.push(parse_interface(entry, index)?);
        }
        for (index, interface) in interfaces.iter().enumerate() {
            if let Some(previous) = interfaces[..index]
                .iter()
                .position(|other| other.name == interface.name)
            {
                return Err(MatrixError::new(
                    format!("interfaces[{index}]"),
                    format!("`{}` is already declared at interfaces[{previous}]", interface.name),
                ));
            }
            for (position, member) in interface.members.iter().enumerate() {
                if let Some(previous) = interface.members[..position]
                    .iter()
                    .find(|other| other.kind == member.kind && other.name == member.name)
                {
                    return Err(MatrixError::new(
                        format!("interfaces[{index}].members[{position}]"),
                        format!("`{} {}` is already declared", previous.kind, previous.name),
                    ));
                }
            }
        }
        let mut matrix = Self { interfaces };
        matrix.normalise();
        Ok(matrix)
    }

    /// Sort into report order, which every renderer relies on.
    fn normalise(&mut self) {
        self.interfaces.sort_by(|left, right| left.name.cmp(&right.name));
        for interface in &mut self.interfaces {
            interface
                .members
                .sort_by(|left, right| (left.kind, &left.name).cmp(&(right.kind, &right.name)));
        }
    }
}

/// Fluent declaration of a [`SupportMatrix`].
///
/// ```ignore
/// let matrix = SupportMatrix::builder()
///     .interface("org.gnome.Mutter.IdleMonitor", 1, |members| {
///         members.method("GetIdletime", Support::Supported);
///         members.method("AddIdleWatch", Support::partial("idle time is clamped to the monitor's resolution"));
///     })
///     .build();
/// ```
#[derive(Clone, Debug, Default)]
pub struct Builder {
    interfaces: Vec<Interface>,
}

impl Builder {
    /// Declare one interface; `members` adds every member served on it.
    pub fn interface(
        mut self,
        name: impl Into<String>,
        version: u32,
        members: impl FnOnce(&mut InterfaceBuilder),
    ) -> Self {
        let mut declaration = InterfaceBuilder::default();
        members(&mut declaration);
        self.interfaces.push(Interface {
            name: name.into(),
            version,
            members: declaration.members,
        });
        self
    }

    /// Finish the declaration in report order.
    pub fn build(self) -> SupportMatrix {
        let mut matrix = SupportMatrix {
            interfaces: self.interfaces,
        };
        matrix.normalise();
        matrix
    }
}

/// The member list of one interface inside [`Builder::interface`].
#[derive(Clone, Debug, Default)]
pub struct InterfaceBuilder {
    members: Vec<Member>,
}

impl InterfaceBuilder {
    /// Declare a method.
    pub fn method(&mut self, name: impl Into<String>, support: Support) {
        self.members.push(Member::new(Kind::Method, name, support));
    }

    /// Declare a property, covering its read and its write.
    pub fn property(&mut self, name: impl Into<String>, support: Support) {
        self.members.push(Member::new(Kind::Property, name, support));
    }

    /// Declare a signal.
    pub fn signal(&mut self, name: impl Into<String>, support: Support) {
        self.members.push(Member::new(Kind::Signal, name, support));
    }
}

/// Why a manifest could not be read, with the path of the offending member.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MatrixError {
    path: String,
    message: String,
}

impl MatrixError {
    fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }

    /// Where in the document the problem is, such as `interfaces[0].members[2]`.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// What is wrong, in the terms the manifest author needs.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for MatrixError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.path, self.message)
    }
}

impl std::error::Error for MatrixError {}

fn parse_interface(entry: &Value, index: usize) -> Result<Interface, MatrixError> {
    let path = format!("interfaces[{index}]");
    let object = expect_object(entry, &path)?;
    expect_keys(object, &["name", "version", "members"], &path)?;
    let name = expect_string(object, "name", &path)?.to_owned();
    if name.is_empty() {
        return Err(MatrixError::new(
            format!("{path}.name"),
            "an interface name cannot be empty",
        ));
    }
    let version = expect_u32(object, "version", &path)?;
    let entries = expect_array(expect_field(object, "members", &path)?, &format!("{path}.members"))?;
    let mut members = Vec::with_capacity(entries.len());
    for (position, entry) in entries.iter().enumerate() {
        members.push(parse_member(entry, &format!("{path}.members[{position}]"))?);
    }
    Ok(Interface { name, version, members })
}

fn parse_member(entry: &Value, path: &str) -> Result<Member, MatrixError> {
    let object = expect_object(entry, path)?;
    expect_keys(object, &["kind", "name", "support", "note", "reason"], path)?;
    let word = expect_string(object, "kind", path)?;
    let kind = Kind::parse(word)
        .ok_or_else(|| MatrixError::new(format!("{path}.kind"), format!("unknown member kind `{word}`")))?;
    let name = expect_string(object, "name", path)?.to_owned();
    if name.is_empty() {
        return Err(MatrixError::new(
            format!("{path}.name"),
            "a member name cannot be empty",
        ));
    }
    let word = expect_string(object, "support", path)?;
    let note = optional_string(object, "note", path)?;
    let reason = optional_string(object, "reason", path)?;
    let support = match word {
        "supported" => {
            if note.is_some() || reason.is_some() {
                return Err(MatrixError::new(
                    path,
                    format!("`{name}` is supported, so it takes neither a note nor a reason"),
                ));
            }
            Support::Supported
        }
        "partial" => {
            let note = note.ok_or_else(|| {
                MatrixError::new(
                    format!("{path}.note"),
                    format!("`{name}` is partial, so it needs a note"),
                )
            })?;
            if reason.is_some() {
                return Err(MatrixError::new(
                    format!("{path}.reason"),
                    format!("`{name}` is partial, so it takes a note and not a reason"),
                ));
            }
            Support::partial(note)
        }
        "unsupported" => {
            let reason = reason.ok_or_else(|| {
                MatrixError::new(
                    format!("{path}.reason"),
                    format!("`{name}` is unsupported, so it needs a reason"),
                )
            })?;
            if note.is_some() {
                return Err(MatrixError::new(
                    format!("{path}.note"),
                    format!("`{name}` is unsupported, so it takes a reason and not a note"),
                ));
            }
            Support::unsupported(reason)
        }
        other => {
            return Err(MatrixError::new(
                format!("{path}.support"),
                format!("unknown support `{other}`; expected `supported`, `partial` or `unsupported`"),
            ));
        }
    };
    Ok(Member::new(kind, name, support))
}

fn expect_object<'a>(value: &'a Value, path: &str) -> Result<&'a Map<String, Value>, MatrixError> {
    value
        .as_object()
        .ok_or_else(|| MatrixError::new(path, "expected an object"))
}

fn expect_array<'a>(value: &'a Value, path: &str) -> Result<&'a Vec<Value>, MatrixError> {
    value
        .as_array()
        .ok_or_else(|| MatrixError::new(path, "expected an array"))
}

fn expect_field<'a>(object: &'a Map<String, Value>, key: &str, path: &str) -> Result<&'a Value, MatrixError> {
    object
        .get(key)
        .ok_or_else(|| MatrixError::new(path, format!("missing `{key}`")))
}

fn expect_string<'a>(object: &'a Map<String, Value>, key: &str, path: &str) -> Result<&'a str, MatrixError> {
    expect_field(object, key, path)?
        .as_str()
        .ok_or_else(|| MatrixError::new(format!("{path}.{key}"), "expected a string"))
}

fn optional_string(object: &Map<String, Value>, key: &str, path: &str) -> Result<Option<String>, MatrixError> {
    match object.get(key) {
        None => Ok(None),
        Some(value) => {
            let text = value
                .as_str()
                .ok_or_else(|| MatrixError::new(format!("{path}.{key}"), "expected a string"))?;
            if text.trim().is_empty() {
                return Err(MatrixError::new(
                    format!("{path}.{key}"),
                    "an empty explanation is not a declaration",
                ));
            }
            Ok(Some(text.to_owned()))
        }
    }
}

fn expect_u32(object: &Map<String, Value>, key: &str, path: &str) -> Result<u32, MatrixError> {
    let value = expect_field(object, key, path)?;
    let number = value
        .as_u64()
        .ok_or_else(|| MatrixError::new(format!("{path}.{key}"), "expected a non-negative integer"))?;
    u32::try_from(number).map_err(|_| MatrixError::new(format!("{path}.{key}"), "version does not fit in a u32"))
}

fn expect_keys(object: &Map<String, Value>, allowed: &[&str], path: &str) -> Result<(), MatrixError> {
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(MatrixError::new(
                path,
                format!("unknown key `{key}`; expected one of {allowed:?}"),
            ));
        }
    }
    Ok(())
}

/// Render one string as JSON, escaping what a note or a reason may contain.
fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if u32::from(control) < 0x20 => {
                write!(out, "\\u{:04x}", u32::from(control)).expect("writing to a String cannot fail");
            }
            plain => out.push(plain),
        }
    }
    out.push('"');
    out
}
