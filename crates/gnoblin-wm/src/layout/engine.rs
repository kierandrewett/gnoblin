//! The contract between the wm and a layout backend, and the slot that holds one.
//!
//! A backend never runs layout code on the compositor's frame thread.
//! [`LayoutEngine::submit`] starts a run and returns at once. The backend runs
//! it on its own worker. [`LayoutEngine::collect`] returns whatever is ready and
//! never waits past the deadline it is given.
//!
//! Fuel and instruction limits bound the total work of a run, but they do not
//! stop a synchronous call from stalling. A native call that never returns is
//! not interrupted by any hook, so a backend that runs on the calling thread can
//! block the frame for as long as that call takes. For that reason every backend
//! is a worker, and the wm never calls one synchronously.
//!
//! [`Slot`] is the only thing the wm holds. It keeps one run in flight per
//! workspace, and it discards any result whose generation is no longer the one
//! it asked for.

use std::collections::BTreeMap;
use std::time::Instant;

use super::error::LayoutFailure;
use super::kind::{LayoutExtent, LayoutKind};
use super::placement::{Params, Reply, Request};
use super::settle::{Layout, settle};

/// Identifies one submitted run. Generations only ever increase within a slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Generation(u64);

impl Generation {
    /// The generation of the first run a slot submits.
    pub const FIRST: Self = Self(1);

    /// The generation after this one.
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

/// The handle [`LayoutEngine::submit`] returns for a started run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub struct Ticket(Generation);

impl Ticket {
    /// The ticket a backend returns from [`LayoutEngine::submit`] for `generation`.
    pub const fn new(generation: Generation) -> Self {
        Self(generation)
    }

    /// The generation of the run this ticket names.
    pub const fn generation(self) -> Generation {
        self.0
    }
}

/// Work for a layout engine.
#[derive(Clone, Debug, PartialEq)]
pub enum Job {
    /// Arrange the whole tiled set of a workspace.
    Arrange(Params),
    /// One geometry request, which must apply atomically with its reply.
    Request(Request),
    /// A named command the layout answers for the whole tiled set. A layout that
    /// does not know `command.name` declines.
    Command(Params, LayoutCommand),
}

impl Job {
    /// The parameters the run is validated against.
    pub const fn params(&self) -> &Params {
        match self {
            Self::Arrange(params) | Self::Command(params, _) => params,
            Self::Request(request) => &request.params,
        }
    }
}

/// A command a client sends to the layout of a workspace, with its arguments.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutCommand {
    /// The command's name in the layout's command table.
    pub name: String,
    /// The arguments, by name.
    pub args: BTreeMap<String, Arg>,
}

/// One argument of a [`LayoutCommand`].
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    /// A number.
    Number(f64),
    /// A string.
    Text(String),
    /// A boolean.
    Bool(bool),
}

/// What [`LayoutEngine::collect`] found.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum LayoutOutcome {
    /// Nothing was ready before the deadline. The caller keeps its last good
    /// placements for this frame.
    Pending,
    /// A run finished. `generation` may name an older run than the one asked
    /// for, in which case the caller discards it.
    Ready {
        /// The run this result belongs to.
        generation: Generation,
        /// The layout's reply, not yet validated, or the reason it failed.
        result: Result<Reply, LayoutFailure>,
    },
}

/// A layout backend. Implementations must not block the caller.
pub trait LayoutEngine {
    /// Starts `job` as run `generation` and returns at once.
    fn submit(&mut self, job: Job, generation: Generation) -> Ticket;

    /// Returns a finished run if one is ready before `deadline`, and
    /// [`LayoutOutcome::Pending`] otherwise. Must not wait past `deadline`.
    fn collect(&mut self, generation: Generation, deadline: Instant) -> LayoutOutcome;

    /// The kind the layout declares. Required: the host derives placement policy from it.
    fn kind(&self) -> LayoutKind;

    /// The extent the layout arranges in. Only [`LayoutExtent::Output`] is supported.
    fn extent(&self) -> LayoutExtent;

    /// Whether the layout's manifest declares support for the named standard command.
    fn declares(&self, command: &str) -> bool;
}

/// What a slot reports for a frame.
#[derive(Clone, Debug, PartialEq)]
pub enum Collected {
    /// No run is in flight.
    Idle,
    /// A run is in flight and has not finished. Keep the last good placements.
    Pending,
    /// The latest run finished and its reply passed validation.
    Settled(Layout),
    /// The latest run failed, or its reply was refused.
    Failed(LayoutFailure),
    /// The latest run was a geometry request and the layout declined it.
    Declined,
}

struct InFlight {
    ticket: Ticket,
    params: Params,
    declinable: bool,
}

/// One workspace's engine, with the bookkeeping that keeps stale results out.
pub struct Slot {
    engine: Box<dyn LayoutEngine>,
    latest: Option<Generation>,
    in_flight: Option<InFlight>,
}

impl Slot {
    /// Wraps `engine`. Nothing is in flight until the first [`Slot::submit`].
    pub fn new(engine: Box<dyn LayoutEngine>) -> Self {
        Self {
            engine,
            latest: None,
            in_flight: None,
        }
    }

    /// Starts `job`, superseding any run still in flight.
    pub fn submit(&mut self, job: Job) {
        let declinable = matches!(job, Job::Request(_) | Job::Command(..));
        let params = job.params().clone();
        let generation = self.next_generation();
        let ticket = self.engine.submit(job, generation);
        self.in_flight = Some(InFlight {
            ticket,
            params,
            declinable,
        });
    }

    /// Starts a geometry request, superseding any run still in flight. A
    /// [`Reply::Declined`] to a request is reported as [`Collected::Declined`]
    /// rather than replaced by the fallback, so the caller can apply its own logic.
    pub fn submit_request(&mut self, request: Request) {
        self.submit(Job::Request(request));
    }

    /// Starts a named command, superseding any run still in flight. A
    /// [`Reply::Declined`] is reported as [`Collected::Declined`]: the layout does
    /// not know the command.
    pub fn submit_command(&mut self, params: Params, command: LayoutCommand) {
        self.submit(Job::Command(params, command));
    }

    /// Drops the run in flight, so that its result is never reported.
    pub fn cancel(&mut self) {
        self.in_flight = None;
    }

    fn next_generation(&mut self) -> Generation {
        let generation = self.latest.map_or(Generation::FIRST, Generation::next);
        self.latest = Some(generation);
        generation
    }

    /// Collects the in-flight run if it is ready before `deadline`.
    ///
    /// A result for any other generation is discarded and reported as
    /// [`Collected::Pending`], so a late reply never reaches the wm.
    pub fn collect(&mut self, deadline: Instant) -> Collected {
        let generation = match &self.in_flight {
            Some(in_flight) => in_flight.ticket.generation(),
            None => return Collected::Idle,
        };
        match self.engine.collect(generation, deadline) {
            LayoutOutcome::Pending => Collected::Pending,
            LayoutOutcome::Ready { generation: found, .. } if found != generation => Collected::Pending,
            LayoutOutcome::Ready { result, .. } => {
                let Some(in_flight) = self.in_flight.take() else {
                    return Collected::Idle;
                };
                match result {
                    Ok(Reply::Declined) if in_flight.declinable => Collected::Declined,
                    result => match result.and_then(|reply| settle(&in_flight.params, reply)) {
                        Ok(layout) => Collected::Settled(layout),
                        Err(failure) => Collected::Failed(failure),
                    },
                }
            }
        }
    }
}
