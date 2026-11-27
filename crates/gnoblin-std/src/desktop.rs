//! The desktop as a whole: the root collections, and the policies that decide how outputs share workspaces.

use gnoblin_core::{Gnoblin, Window};

use crate::api::Std;
use crate::client::Client;
use crate::collection::Collection;
use crate::error::Result;
use crate::layout::FloatingLayout;
use crate::objects as classes;
use crate::output::Output;
use crate::workspace::Workspace;

/// How the outputs share workspaces.
#[derive(Clone, Debug, Default)]
pub enum OutputPolicy {
    /// Every output shows a workspace of its own (the default).
    #[default]
    Separate,
    /// Every output shows what the lead output shows: the compositor mirrors them. `None` takes the primary output as the lead.
    Duplicate {
        /// The output the others copy.
        lead: Option<Output>,
    },
    /// One workspace is laid out over the union of the outputs, which show a part each. `None` takes the primary output as the lead.
    Extend {
        /// The output the span starts from.
        lead: Option<Output>,
    },
}

/// The options of the per-output workspace policy.
#[derive(Clone, Debug)]
pub struct PerOutput {
    prefix: String,
    gaps: (f64, f64),
    layout: FloatingLayout,
}

impl Default for PerOutput {
    fn default() -> PerOutput {
        PerOutput {
            prefix: "workspace-".to_owned(),
            gaps: (0.0, 0.0),
            layout: FloatingLayout::default(),
        }
    }
}

impl PerOutput {
    /// The workspaces are named `<prefix><n>`, `n` counting from 1.
    #[must_use]
    pub fn prefix(mut self, prefix: impl Into<String>) -> PerOutput {
        self.prefix = prefix.into();
        self
    }

    /// The gaps of every workspace the policy makes.
    #[must_use]
    pub fn gaps(mut self, inner: f64, outer: f64) -> PerOutput {
        self.gaps = (inner, outer);
        self
    }

    /// The floating layout of every workspace the policy makes. (A layout of the shell's has one owner, so the shell sets it on each
    /// workspace itself.)
    #[must_use]
    pub fn layout(mut self, layout: FloatingLayout) -> PerOutput {
        self.layout = layout;
        self
    }
}

/// Where workspaces come from.
#[derive(Clone, Debug, Default)]
pub enum WorkspacePolicy {
    /// The shell adds and removes every workspace itself (the default).
    #[default]
    Manual,
    /// The library adds one workspace for every output that appears and removes it when the output goes, its windows moving to a
    /// workspace that stays.
    PerOutput(PerOutput),
}

impl Std {
    /// Every window of the compositor, in the order they appeared.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn clients(&self) -> Result<Collection<Client>> {
        Collection::all(self)
    }

    /// Every connected output.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn outputs(&self) -> Result<Collection<Output>> {
        Collection::root(self)
    }

    /// The workspaces in the list, in order.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn workspaces(&self) -> Result<Collection<Workspace>> {
        Collection::list(self)
    }

    /// The client of a core window.
    pub fn client(&self, window: &Window) -> Client {
        Client::new(self, window.clone())
    }

    /// Sets how the outputs share workspaces.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn set_output_policy(&self, policy: OutputPolicy) -> Result<()> {
        let (mode, lead) = match &policy {
            OutputPolicy::Separate => (classes::OutputPolicyMode::Separate, None),
            OutputPolicy::Duplicate { lead } => (
                classes::OutputPolicyMode::Duplicate,
                lead.as_ref().map(|lead| lead.id().0),
            ),
            OutputPolicy::Extend { lead } => (classes::OutputPolicyMode::Extend, lead.as_ref().map(|lead| lead.id().0)),
        };
        let desks = classes::Desks::get(&self.objects()?)?;
        Ok(desks.set_outputs(&classes::OutputPolicy { mode, lead })?)
    }

    /// Sets where workspaces come from.
    ///
    /// # Errors
    ///
    /// The generation retired, or a gap is not a finite number of at least 0.
    pub fn set_workspace_policy(&self, policy: WorkspacePolicy) -> Result<()> {
        let (mode, per_output) = match policy {
            WorkspacePolicy::Manual => (classes::WorkspacePolicyMode::Manual, PerOutput::default()),
            WorkspacePolicy::PerOutput(options) => (classes::WorkspacePolicyMode::PerOutput, options),
        };
        let desks = classes::Desks::get(&self.objects()?)?;
        Ok(desks.set_workspaces(&classes::WorkspacePolicy {
            mode,
            name_prefix: per_output.prefix.clone(),
            gaps: Some(classes::Gaps {
                inner: per_output.gaps.0,
                outer: per_output.gaps.1,
            }),
            placement: per_output.layout.place_mode(),
            cascade_step: per_output.layout.cascade_step_value(),
        })?)
    }

    /// One floating workspace for every output: adds one now for each output there is, and one more for each that appears.
    ///
    /// # Errors
    ///
    /// As for [`set_workspace_policy`](Std::set_workspace_policy).
    pub fn workspace_per_output(&self) -> Result<()> {
        self.set_workspace_policy(WorkspacePolicy::PerOutput(PerOutput::default()))
    }
}

/// The std library's conveniences on the core's [`Gnoblin`] handle, which `use gnoblin_std::prelude::*;` brings in: `g.std()`,
/// `g.clients()`, `g.workspaces()`.
pub trait GnoblinStdExt {
    /// The generation's handle on the installed std library; opens it on first use.
    ///
    /// # Errors
    ///
    /// The installed library refuses this SDK's ABI, or cannot attach to the tree.
    fn std(&self) -> Result<Std>;
    /// The typed classes of `gnoblin.std` (see [`Std::objects`]): the handle the classes of the library are made from, which a shell that
    /// also has the core's (`g.objects()?`) can reach as `g.objects()?.std()?` too.
    ///
    /// # Errors
    ///
    /// As for [`std`](GnoblinStdExt::std), or the library does not serve the classes.
    fn std_objects(&self) -> Result<classes::Objects> {
        self.std()?.objects()
    }

    /// Every window of the compositor (see [`Std::clients`]).
    ///
    /// # Errors
    ///
    /// As for [`std`](GnoblinStdExt::std).
    fn clients(&self) -> Result<Collection<Client>> {
        self.std()?.clients()
    }

    /// Every connected output (see [`Std::outputs`]).
    ///
    /// # Errors
    ///
    /// As for [`std`](GnoblinStdExt::std).
    fn outputs(&self) -> Result<Collection<Output>> {
        self.std()?.outputs()
    }

    /// The workspaces in the list (see [`Std::workspaces`]).
    ///
    /// # Errors
    ///
    /// As for [`std`](GnoblinStdExt::std).
    fn workspaces(&self) -> Result<Collection<Workspace>> {
        self.std()?.workspaces()
    }
}

impl GnoblinStdExt for Gnoblin {
    fn std(&self) -> Result<Std> {
        Std::open(self)
    }
}
