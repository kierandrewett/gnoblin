//! The generated command tree: one entry per object the schema declares.
//!
//! `OUT_DIR/cli_tree.rs` is written by `build.rs` from
//! `crates/gnoblin-schema/schema/*.toml`, so this module holds the shape of an
//! entry and the behaviour over it, and the data itself is regenerated on every
//! build. Nothing here reads a file: a schema edit is a rebuild, and a member the
//! schema does not declare cannot reach the command line.
//!
//! Types are kept as their schema short form (`string`, `list<object:window>`)
//! rather than as a generated enum, so the parse and the check are
//! [`gnoblin_schema::Type`]'s own: a generated copy of that grammar could drift
//! from the one the graph validates against.

use gnoblin_model::TypeKey;
use gnoblin_schema::Type;

include!(concat!(env!("OUT_DIR"), "/cli_tree.rs"));

/// One object type, as the command line reaches it.
#[derive(Debug)]
pub struct CliObject {
    /// The subcommand name, derived from the object and its namespace.
    pub command: &'static str,
    /// The schema namespace the object lives in.
    pub namespace: &'static str,
    /// The object's name inside its namespace.
    pub name: &'static str,
    pub properties: &'static [CliProperty],
    pub methods: &'static [CliMethod],
    pub signals: &'static [CliSignal],
}

impl CliObject {
    /// The type key the graph and the wire use for this object.
    pub fn type_key(&self) -> TypeKey {
        TypeKey::new(self.namespace, self.name)
    }

    /// One declared property.
    pub fn property(&self, name: &str) -> Option<&'static CliProperty> {
        self.properties.iter().find(|property| property.name == name)
    }

    /// One declared method.
    pub fn method(&self, name: &str) -> Option<&'static CliMethod> {
        self.methods.iter().find(|method| method.name == name)
    }

    /// The object reached by `command`.
    pub fn find(command: &str) -> Option<&'static CliObject> {
        OBJECTS.iter().find(|object| object.command == command)
    }

    /// Every declared method name.
    pub fn method_names(&self) -> Vec<&'static str> {
        self.methods.iter().map(|method| method.name).collect()
    }
}

/// One typed property of an object.
#[derive(Debug)]
pub struct CliProperty {
    pub name: &'static str,
    /// The schema short form, as the schema wrote it (`string`, `list<uint>`).
    pub ty: &'static str,
    /// The schema marks this property secret, so it is never printed.
    pub secret: bool,
}

/// One method an object declares.
#[derive(Debug)]
pub struct CliMethod {
    pub name: &'static str,
    pub params: &'static [CliParam],
}

/// One typed method parameter.
#[derive(Debug)]
pub struct CliParam {
    pub name: &'static str,
    /// The schema short form, parsed by [`CliParam::ty`].
    pub ty: &'static str,
    /// The schema marks this parameter secret.
    pub secret: bool,
}

impl CliParam {
    /// The schema type, parsed.
    pub fn ty(&self) -> Result<Type, String> {
        Type::parse(self.ty).map_err(|reason| {
            format!(
                "the generated tree types parameter `{}` as `{}`, which the schema crate does not accept: {reason}",
                self.name, self.ty
            )
        })
    }
}

/// One signal an object declares.
///
/// A signal is what `watch` prints. Nothing parses its name into an argument, and
/// the tree still records it so one generated table describes an object
/// completely rather than the parts a command happens to use.
#[derive(Debug)]
pub struct CliSignal {
    pub name: &'static str,
}

/// The IPC version this build speaks.
pub const fn ipc_version() -> gnoblin_schema::Version {
    gnoblin_schema::Version::new(IPC_VERSION.0, IPC_VERSION.1, IPC_VERSION.2)
}
