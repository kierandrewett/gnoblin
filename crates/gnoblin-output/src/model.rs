//! The hotplug state machine: the connected set, the declared layout and its transaction.

use crate::effect::{Effect, Rejection, RollbackReason};
use crate::engine::compute;
use crate::ids::OutputId;
use crate::layout::{Layout, LayoutConfig};
use crate::output::OutputDescriptor;
use crate::transaction::{Phase, Transaction, TransactionToken, Verdict};

/// How long a committed layout waits for confirmation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ModelOptions {
    /// Ticks the transaction waits after a commit before restoring the previous
    /// layout. A tick is whatever the caller's clock counts, so the default of
    /// `5_000` is five seconds when a tick is a millisecond, which is the usual
    /// confirmation interval for a display change.
    pub confirm_timeout_ticks: u64,
}

impl Default for ModelOptions {
    fn default() -> Self {
        Self {
            confirm_timeout_ticks: 5_000,
        }
    }
}

/// Something that happened to the outputs or to their configuration.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Event {
    /// A connector became usable, or was reconnected with a new descriptor.
    Connected(OutputDescriptor),
    /// A connector went away.
    Disconnected(OutputId),
    /// The declarative configuration was replaced.
    Configured(LayoutConfig),
    /// A configuration was offered for a transactional apply.
    Propose(LayoutConfig),
    /// The caller's test of the current proposal finished.
    Verdict {
        /// The transaction being answered.
        token: TransactionToken,
        /// What the test reported.
        verdict: Verdict,
    },
    /// The caller's clock advanced.
    Tick {
        /// Ticks since the last tick.
        ticks: u64,
    },
    /// The committed layout was accepted.
    Confirm {
        /// The transaction being confirmed.
        token: TransactionToken,
    },
    /// The caller withdrew the proposal.
    Rollback {
        /// The transaction being withdrawn.
        token: TransactionToken,
    },
}

/// A broken invariant reported by [`OutputModel::check_invariants`].
///
/// Every variant names the monitor or output that broke it, because the only
/// consumer is a test and the message has to be enough to find the event that
/// broke it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Broken {
    /// A monitor's rectangle is not the size its mode, scale and transform imply.
    LogicalSize(OutputId),
    /// Two monitors share a pixel.
    Overlap {
        /// One monitor.
        first: OutputId,
        /// The other monitor.
        second: OutputId,
    },
    /// A monitor is not edge-adjacent to the rest of the layout.
    Disconnected(OutputId),
    /// A monitor names an output that is not connected.
    MemberNotConnected(OutputId),
    /// An enabled entry names a connected output that no monitor shows.
    OutputMissing(OutputId),
    /// A connected output is shown by more than one monitor.
    OutputDuplicated(OutputId),
    /// There are monitors but no primary output.
    NoPrimary,
    /// The primary output is not shown by any monitor.
    PrimaryMissing(OutputId),
    /// The monitors are not in the documented order.
    Unsorted,
}

/// The session's outputs and their configuration, as a state machine.
///
/// The model owns the three things a display configuration has to keep consistent:
/// which outputs are connected, what the configuration declares, and which layout
/// the session is presenting. Hotplug is the hard part, so it is handled in one
/// place: a connect or disconnect during a pending test abandons the test, and a
/// declared layout that no longer resolves is replaced by a row of whatever is
/// left, rather than leaving a monitor that names a connector which is gone.
#[derive(Debug)]
pub struct OutputModel {
    outputs: Vec<OutputDescriptor>,
    config: LayoutConfig,
    proposed: Option<LayoutConfig>,
    applied: Layout,
    transaction: Transaction,
}

impl OutputModel {
    /// A session with nothing connected and nothing configured.
    #[must_use]
    pub fn new(options: ModelOptions) -> Self {
        Self {
            outputs: Vec::new(),
            config: LayoutConfig::default(),
            proposed: None,
            applied: Layout::default(),
            transaction: Transaction::new(options.confirm_timeout_ticks),
        }
    }

    /// The layout the session is presenting.
    #[must_use]
    pub fn layout(&self) -> &Layout {
        &self.applied
    }

    /// Where the transaction is.
    #[must_use]
    pub const fn phase(&self) -> Phase {
        self.transaction.phase()
    }

    /// Applies one event and returns everything the backend must do, in order.
    pub fn apply(&mut self, event: Event) -> Vec<Effect> {
        match event {
            Event::Connected(descriptor) => {
                // A reconnect replaces the descriptor and keeps connection order, so a
                // monitor that comes back stays where it was in the row.
                match self.outputs.iter().position(|connected| connected.id == descriptor.id) {
                    Some(index) => self.outputs[index] = descriptor,
                    None => self.outputs.push(descriptor),
                }
                self.after_hotplug()
            }
            Event::Disconnected(id) => {
                if !self.outputs.iter().any(|descriptor| descriptor.id == id) {
                    // A duplicate unplug is what a retrying udev monitor produces;
                    // ignoring it keeps the model idempotent.
                    return Vec::new();
                }
                self.outputs.retain(|descriptor| descriptor.id != id);
                self.config.outputs.retain(|entry| entry.id != id);
                if self.config.primary.as_ref() == Some(&id) {
                    self.config.primary = None;
                }
                self.after_hotplug()
            }
            Event::Configured(config) => self.configure(config),
            Event::Propose(config) => self.propose(config),
            Event::Verdict { token, verdict } => {
                let result = self.transaction.verdict(token, verdict);
                self.observe(result)
            }
            Event::Tick { ticks } => {
                let effects = self.transaction.tick(ticks);
                self.observe(Ok(effects))
            }
            Event::Confirm { token } => {
                let result = self.transaction.confirm(token);
                self.observe(result)
            }
            Event::Rollback { token } => {
                let result = self.transaction.rollback(token);
                self.observe(result)
            }
        }
    }

    /// Re-derives the layout after the connected set changed.
    ///
    /// A connect during a pending test abandons it as well: the tested layout was
    /// validated against the old set, and keeping it would make the confirmation a
    /// decision about hardware that was not tested.
    fn after_hotplug(&mut self) -> Vec<Effect> {
        let aborted = self.transaction.abort();
        let layout = self.derive_layout();
        self.proposed = None;
        match aborted {
            Some(token) => {
                self.applied = layout.clone();
                vec![Effect::RolledBack {
                    token,
                    layout,
                    reason: RollbackReason::Hotplug,
                }]
            }
            None => {
                if layout == self.applied {
                    return Vec::new();
                }
                self.applied = layout.clone();
                vec![Effect::Applied { layout }]
            }
        }
    }

    /// The layout the declared configuration produces against what is connected.
    ///
    /// The declared configuration can only stop resolving because an output went
    /// away, so the fallback is the connection order: a row is connected, gapless
    /// and free of overlap by construction, which is the most useful thing a
    /// desktop can show while the configuration is repaired.
    fn derive_layout(&self) -> Layout {
        if let Ok(layout) = compute(&self.outputs, &self.config) {
            return layout;
        }
        let order: Vec<OutputId> = self
            .outputs
            .iter()
            .filter(|descriptor| is_presentable(descriptor))
            .map(|descriptor| descriptor.id.clone())
            .collect();
        if order.is_empty() {
            return Layout::default();
        }
        // A row over the presentable outputs cannot fail: each one takes its
        // preferred mode and its default scale, both of which are supported by
        // construction.
        compute(&self.outputs, &LayoutConfig::row_over(&order)).unwrap_or_default()
    }

    /// Replaces the declared configuration, if it resolves today.
    fn configure(&mut self, config: LayoutConfig) -> Vec<Effect> {
        if !self.transaction.is_idle() {
            return vec![Effect::Rejected(Rejection::Busy)];
        }
        if self.outputs.is_empty() {
            // The configuration usually arrives before the first connector does, so
            // an empty session stores it instead of reporting that it enables
            // nothing.
            self.config = config;
            return Vec::new();
        }
        let layout = match compute(&self.outputs, &config) {
            Ok(layout) => layout,
            Err(error) => return vec![Effect::Rejected(Rejection::Invalid(error))],
        };
        self.config = config;
        if layout == self.applied {
            return Vec::new();
        }
        self.applied = layout.clone();
        vec![Effect::Applied { layout }]
    }

    /// Offers a configuration for a transactional apply.
    fn propose(&mut self, config: LayoutConfig) -> Vec<Effect> {
        if !self.transaction.is_idle() {
            return vec![Effect::Rejected(Rejection::Busy)];
        }
        let layout = match compute(&self.outputs, &config) {
            Ok(layout) => layout,
            Err(error) => return vec![Effect::Rejected(Rejection::Invalid(error))],
        };
        match self.transaction.propose(layout.clone(), self.applied.clone()) {
            Ok(effects) => {
                self.applied = layout;
                self.proposed = Some(config);
                effects
            }
            Err(rejection) => vec![Effect::Rejected(rejection)],
        }
    }

    /// Records the state the transaction's effects imply.
    ///
    /// A confirmed proposal becomes the declared configuration, because a layout a
    /// person accepted is what the session should come back to; a rolled-back one
    /// leaves the declared configuration alone.
    fn observe(&mut self, result: Result<Vec<Effect>, Rejection>) -> Vec<Effect> {
        let effects = match result {
            Ok(effects) => effects,
            Err(rejection) => return vec![Effect::Rejected(rejection)],
        };
        for effect in &effects {
            match effect {
                Effect::Confirmed { layout, .. } => {
                    self.applied = layout.clone();
                    if let Some(config) = self.proposed.take() {
                        self.config = config;
                    }
                }
                Effect::RolledBack { layout, .. } => {
                    self.applied = layout.clone();
                    self.proposed = None;
                }
                Effect::TestRequested { .. }
                | Effect::Committed { .. }
                | Effect::Applied { .. }
                | Effect::Rejected(_) => {}
            }
        }
        effects
    }

    /// The configuration whose layout is on screen.
    ///
    /// While a proposal is in flight its configuration is what the backend is
    /// showing, so the invariants have to be checked against it rather than
    /// against the declared configuration it temporarily replaced.
    fn effective_config(&self) -> &LayoutConfig {
        self.proposed.as_ref().unwrap_or(&self.config)
    }

    /// Checks every invariant the model promises, returning how many were checked.
    ///
    /// The count is what makes a test meaningful: a suite that silently stops
    /// exercising the state machine would otherwise still pass.
    pub fn check_invariants(&self) -> Result<usize, Broken> {
        let mut checks = 0_usize;

        for monitor in &self.applied.monitors {
            checks += 1;
            if monitor.rect.size() != monitor.derived_size() {
                return Err(Broken::LogicalSize(monitor.id.clone()));
            }
            for member in &monitor.members {
                checks += 1;
                if !self.outputs.iter().any(|descriptor| &descriptor.id == member) {
                    return Err(Broken::MemberNotConnected(member.clone()));
                }
                checks += 1;
                let shown = self
                    .applied
                    .monitors
                    .iter()
                    .filter(|monitor| monitor.contains(member))
                    .count();
                if shown != 1 {
                    return Err(Broken::OutputDuplicated(member.clone()));
                }
            }
        }

        for (index, monitor) in self.applied.monitors.iter().enumerate() {
            for other in &self.applied.monitors[index + 1..] {
                checks += 1;
                if monitor.rect.overlaps(other.rect) {
                    return Err(Broken::Overlap {
                        first: monitor.id.clone(),
                        second: other.id.clone(),
                    });
                }
            }
        }

        let config = self.effective_config();
        if !config.allow_gaps && self.applied.monitors.len() > 1 {
            checks += 1;
            if let Some(index) = unreachable_monitor(&self.applied) {
                return Err(Broken::Disconnected(self.applied.monitors[index].id.clone()));
            }
        }

        for (position, monitor) in self.applied.monitors.iter().enumerate() {
            checks += 1;
            let sorted = position == 0
                || (
                    self.applied.monitors[position - 1].rect.x,
                    self.applied.monitors[position - 1].rect.y,
                ) <= (monitor.rect.x, monitor.rect.y);
            if !sorted {
                return Err(Broken::Unsorted);
            }
        }

        for entry in &config.outputs {
            if !entry.enabled {
                continue;
            }
            if !self
                .outputs
                .iter()
                .any(|descriptor| descriptor.id == entry.id && is_presentable(descriptor))
            {
                continue;
            }
            checks += 1;
            if !self.applied.monitors.iter().any(|monitor| monitor.contains(&entry.id)) {
                return Err(Broken::OutputMissing(entry.id.clone()));
            }
        }

        match (&self.applied.primary, self.applied.monitors.is_empty()) {
            (Some(primary), false) => {
                checks += 1;
                if !self.applied.monitors.iter().any(|monitor| monitor.contains(primary)) {
                    return Err(Broken::PrimaryMissing(primary.clone()));
                }
            }
            (None, false) => return Err(Broken::NoPrimary),
            (Some(primary), true) => return Err(Broken::PrimaryMissing(primary.clone())),
            (None, true) => {}
        }

        Ok(checks)
    }
}

/// Whether anything can be shown on this connector.
///
/// A connector that reports no mode, or no scale the session is willing to drive it
/// at, cannot present a frame. It stays in the connected set so a later report can
/// replace the descriptor, but the layout ignores it: one such connector must not be
/// able to leave the desktop with no monitor at all.
fn is_presentable(descriptor: &OutputDescriptor) -> bool {
    descriptor.preferred_mode().is_some() && !descriptor.scales.is_empty()
}

/// The first monitor that cannot be reached from the first one by shared edges.
fn unreachable_monitor(layout: &Layout) -> Option<usize> {
    let mut reached = vec![false; layout.monitors.len()];
    reached[0] = true;
    let mut frontier = vec![0_usize];
    while let Some(index) = frontier.pop() {
        for (other, monitor) in layout.monitors.iter().enumerate() {
            if reached[other] || !layout.monitors[index].rect.touches(monitor.rect) {
                continue;
            }
            reached[other] = true;
            frontier.push(other);
        }
    }
    reached.iter().position(|reached| !reached)
}
