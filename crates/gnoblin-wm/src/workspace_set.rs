//! Named workspace sets: which workspace each output shows under a name.
//!
//! A set is a list of entries. Each entry names a declared workspace and, optionally,
//! the output that should show it. Activating a set makes its workspaces the live
//! ones: each output named by an entry shows that entry's workspace, and an output
//! the set does not name keeps whatever it shows now. Sets are declared as a whole.
//! A list that names an undeclared workspace, repeats a name, or leaves a set empty
//! is refused and nothing from it is applied.

use std::fmt;

use gnoblin_types::{OutputId, WorkspaceId};

/// One workspace in a set and the output that shows it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WorkspaceSetEntry {
    /// The declared workspace to show.
    pub workspace: WorkspaceId,
    /// The connector name of the output to show it on. `None` means the primary
    /// output, which is the first one connected.
    pub output: Option<String>,
}

/// A named set of workspace-to-output assignments.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WorkspaceSet {
    /// The name the set is activated by. Unique among the declared sets.
    pub name: String,
    /// The assignments, in declaration order.
    pub entries: Vec<WorkspaceSetEntry>,
}

/// Why a set or a set list was refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum WorkspaceSetError {
    /// A set has no name, so it cannot be activated.
    EmptyName,
    /// Two sets share this name.
    DuplicateName(String),
    /// A set has no entries, so activating it would show nothing.
    EmptySet(String),
    /// A set names a workspace that is not declared.
    UndeclaredWorkspace {
        /// The set that names it.
        set: String,
        /// The undeclared workspace.
        workspace: WorkspaceId,
    },
    /// Two entries of one set name the same output.
    DuplicateOutput {
        /// The set with the clash.
        set: String,
        /// The connector name both entries name.
        output: String,
    },
    /// Two entries of one set leave their output unnamed, so both go to the primary
    /// output and one would silently replace the other.
    DuplicateDefaultOutput {
        /// The set with the clash.
        set: String,
    },
    /// Two entries of one set resolve to the same connected output, for example a
    /// name and the primary output that is that same connector.
    OutputClash {
        /// The set with the clash.
        set: String,
        /// The output both entries resolve to.
        output: OutputId,
    },
    /// No set has this name.
    UnknownSet(String),
}

impl fmt::Display for WorkspaceSetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyName => write!(f, "a workspace set has an empty name"),
            Self::DuplicateName(name) => write!(f, "two workspace sets are named {name:?}"),
            Self::EmptySet(name) => write!(f, "workspace set {name:?} has no entries"),
            Self::UndeclaredWorkspace { set, workspace } => {
                write!(
                    f,
                    "workspace set {set:?} names undeclared workspace {}",
                    workspace.get()
                )
            }
            Self::DuplicateOutput { set, output } => {
                write!(f, "workspace set {set:?} names output {output:?} twice")
            }
            Self::DuplicateDefaultOutput { set } => {
                write!(f, "workspace set {set:?} has two entries for the primary output")
            }
            Self::OutputClash { set, output } => {
                write!(
                    f,
                    "workspace set {set:?} resolves two entries to output {}",
                    output.get()
                )
            }
            Self::UnknownSet(name) => write!(f, "no workspace set is named {name:?}"),
        }
    }
}

impl std::error::Error for WorkspaceSetError {}

/// Checks a whole list of sets against the declared workspaces.
///
/// Every set is checked, so the first error reported is the first one in declaration
/// order. Nothing is returned on success that the caller could apply partly.
pub(crate) fn validate(sets: &[WorkspaceSet], declared: impl Fn(WorkspaceId) -> bool) -> Result<(), WorkspaceSetError> {
    for (index, set) in sets.iter().enumerate() {
        if set.name.is_empty() {
            return Err(WorkspaceSetError::EmptyName);
        }
        if sets[..index].iter().any(|earlier| earlier.name == set.name) {
            return Err(WorkspaceSetError::DuplicateName(set.name.clone()));
        }
        if set.entries.is_empty() {
            return Err(WorkspaceSetError::EmptySet(set.name.clone()));
        }
        for entry in &set.entries {
            if !declared(entry.workspace) {
                return Err(WorkspaceSetError::UndeclaredWorkspace {
                    set: set.name.clone(),
                    workspace: entry.workspace,
                });
            }
        }
        for (position, entry) in set.entries.iter().enumerate() {
            let clash = set.entries[..position]
                .iter()
                .any(|earlier| earlier.output == entry.output);
            if clash {
                return Err(match &entry.output {
                    Some(output) => WorkspaceSetError::DuplicateOutput {
                        set: set.name.clone(),
                        output: output.clone(),
                    },
                    None => WorkspaceSetError::DuplicateDefaultOutput { set: set.name.clone() },
                });
            }
        }
    }
    Ok(())
}
