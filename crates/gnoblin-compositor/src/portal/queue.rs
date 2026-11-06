//! Admission for built-in UI requests: queue caps, a per-app rate limit and the
//! lock embargo (D19 "Rate and queue caps" and "Lock embargo").

use std::collections::{BTreeMap, VecDeque};

use gnoblin_types::{RequestId, Tick};

use super::outcome::PortalError;

/// At most this many UI requests are queued in total.
pub const MAX_QUEUED: usize = 8;
/// At most this many queued UI requests per app id.
pub const MAX_QUEUED_PER_APP: usize = 2;
/// At most this many UI requests per app id in any window of [`RATE_WINDOW_MS`].
pub const MAX_PER_APP_PER_WINDOW: usize = 5;
/// The rate window, in milliseconds (one minute).
pub const RATE_WINDOW_MS: u64 = 60_000;

/// Why a request was refused admission (mapped to a [`PortalError`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The session is locked (`error { locked }`).
    Locked,
    /// A cap was reached (`error { busy }`).
    Busy,
}

impl Refusal {
    /// The `error { code }` this refusal produces.
    #[must_use]
    pub const fn error(self) -> PortalError {
        match self {
            Self::Locked => PortalError::Locked,
            Self::Busy => PortalError::Busy,
        }
    }
}

/// The result of a successful admission check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    /// The request may open its dialog now.
    Admitted,
}

/// The queue of UI requests, and the per-app rate history.
#[derive(Clone, Debug, Default)]
pub struct Queue {
    queued: Vec<(RequestId, String)>,
    recent: BTreeMap<String, VecDeque<Tick>>,
}

impl Queue {
    /// An empty queue.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of requests currently queued.
    #[must_use]
    pub fn len(&self) -> usize {
        self.queued.len()
    }

    /// `true` when nothing is queued.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.queued.is_empty()
    }

    /// Admits `request` from `app_id` at `now`, or refuses it.
    ///
    /// The lock embargo is checked first: while locked, nothing is admitted.
    /// Refused requests are not recorded, so a refusal costs no rate budget.
    ///
    /// # Errors
    ///
    /// [`Refusal::Locked`] while the session is locked; [`Refusal::Busy`] when the
    /// total queue, the per-app queue or the per-app rate is full.
    pub fn admit(&mut self, request: RequestId, app_id: &str, now: Tick, locked: bool) -> Result<Admission, Refusal> {
        if locked {
            return Err(Refusal::Locked);
        }
        if self.queued.len() >= MAX_QUEUED {
            return Err(Refusal::Busy);
        }
        if self.queued.iter().filter(|(_, app)| app == app_id).count() >= MAX_QUEUED_PER_APP {
            return Err(Refusal::Busy);
        }
        let history = self.recent.entry(app_id.to_owned()).or_default();
        while history.front().is_some_and(|at| now.since(*at) >= RATE_WINDOW_MS) {
            history.pop_front();
        }
        if history.len() >= MAX_PER_APP_PER_WINDOW {
            return Err(Refusal::Busy);
        }
        history.push_back(now);
        self.queued.push((request, app_id.to_owned()));
        Ok(Admission::Admitted)
    }

    /// Removes a request once its dialog has closed, whatever the outcome.
    /// Returns `false` when the request was not queued.
    pub fn complete(&mut self, request: RequestId) -> bool {
        let before = self.queued.len();
        self.queued.retain(|(queued, _)| *queued != request);
        self.queued.len() != before
    }

    /// Drops every queued request while the session locks. The caller closes
    /// each dialog with `error { locked }`; this returns the requests dropped.
    pub fn lock(&mut self) -> Vec<RequestId> {
        self.queued.drain(..).map(|(request, _)| request).collect()
    }
}
