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

use std::time::Instant;

use super::error::LayoutFailure;
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
}

impl Job {
    /// The parameters the run is validated against.
    pub const fn params(&self) -> &Params {
        match self {
            Self::Arrange(params) => params,
            Self::Request(request) => &request.params,
        }
    }
}

/// What [`LayoutEngine::collect`] found.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
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
    /// [`Outcome::Pending`] otherwise. Must not wait past `deadline`.
    fn collect(&mut self, generation: Generation, deadline: Instant) -> Outcome;
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
}

struct InFlight {
    ticket: Ticket,
    params: Params,
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
        let generation = self.latest.map_or(Generation::FIRST, Generation::next);
        self.latest = Some(generation);
        let params = job.params().clone();
        let ticket = self.engine.submit(job, generation);
        self.in_flight = Some(InFlight { ticket, params });
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
            Outcome::Pending => Collected::Pending,
            Outcome::Ready { generation: found, .. } if found != generation => Collected::Pending,
            Outcome::Ready { result, .. } => {
                let Some(in_flight) = self.in_flight.take() else {
                    return Collected::Idle;
                };
                match result.and_then(|reply| settle(&in_flight.params, reply)) {
                    Ok(layout) => Collected::Settled(layout),
                    Err(failure) => Collected::Failed(failure),
                }
            }
        }
    }
}
