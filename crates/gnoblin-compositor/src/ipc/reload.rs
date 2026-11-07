//! The `config.reload` request: a privileged caller asks the runtime to re-read its document.
//!
//! The compositor does not parse the document, so it cannot answer the request itself. It
//! raises `session.reload_requested` and holds each caller's request until the runtime
//! answers: `config.commit` accepts the reload with the revision it committed, and
//! `config.reject` refuses it with the diagnostic. A request made while one is outstanding
//! shares its answer, so a reload pressed twice runs the runtime once. A runtime that never
//! answers ends the wait at a deadline, so a caller is never held by a process that is gone.

use gnoblin_client::wire;
use gnoblin_model::Tick;
use gnoblin_proto::Envelope;
use gnoblin_types::ClientId;

/// How long a reload waits for the runtime, in the milliseconds the model measures ticks in.
const DEADLINE_MS: u64 = 10_000;

/// What became of an outstanding reload, as the runtime or the deadline decided it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ReloadVerdict {
    /// The runtime committed the document, which is now in force under this revision.
    Accepted { revision: u64 },
    /// The runtime refused the document, and the last accepted one stays in force.
    Rejected { message: String },
    /// The runtime did not answer before the deadline.
    TimedOut,
}

/// The reload the compositor is waiting on, and the requests that share its answer.
#[derive(Default)]
pub(crate) struct Reload {
    /// How many reloads the compositor has raised. The signal carries it, so a runtime
    /// that missed one still sees that it must act.
    generation: u64,
    /// The reload in flight, if any.
    outstanding: Option<Outstanding>,
}

struct Outstanding {
    deadline: Tick,
    /// Each caller and the request id its answer must carry.
    waiters: Vec<(ClientId, u64)>,
}

impl Reload {
    /// Records one reload request. Returns the generation to raise when this request starts
    /// a reload, and `None` when it joins the one already outstanding.
    pub(crate) fn request(&mut self, client: ClientId, request_id: u64, now: Tick) -> Option<u64> {
        if let Some(outstanding) = &mut self.outstanding {
            outstanding.waiters.push((client, request_id));
            return None;
        }
        self.generation = self.generation.saturating_add(1);
        self.outstanding = Some(Outstanding {
            deadline: now.saturating_add(DEADLINE_MS),
            waiters: vec![(client, request_id)],
        });
        Some(self.generation)
    }

    /// Ends the outstanding reload with `outcome`, returning the answer for every caller
    /// waiting on it. Nothing is returned when no reload is outstanding.
    pub(crate) fn finish(&mut self, outcome: &ReloadVerdict) -> Vec<(ClientId, Envelope)> {
        let Some(outstanding) = self.outstanding.take() else {
            return Vec::new();
        };
        outstanding
            .waiters
            .into_iter()
            .map(|(client, request_id)| (client, answer(request_id, outcome)))
            .collect()
    }

    /// Ends the outstanding reload as timed out when its deadline has passed.
    pub(crate) fn expire(&mut self, now: Tick) -> Vec<(ClientId, Envelope)> {
        match &self.outstanding {
            Some(outstanding) if now.reached(outstanding.deadline) => self.finish(&ReloadVerdict::TimedOut),
            _ => Vec::new(),
        }
    }
}

/// The envelope that answers one caller with the outcome of its reload.
fn answer(request_id: u64, outcome: &ReloadVerdict) -> Envelope {
    match outcome {
        ReloadVerdict::Accepted { revision } => Envelope::reply(
            wire::NAMESPACE,
            wire::member::CONFIG_RELOAD,
            request_id,
            revision.to_be_bytes().to_vec(),
        ),
        ReloadVerdict::Rejected { message } => Envelope::error(
            wire::NAMESPACE,
            wire::member::CONFIG_RELOAD,
            request_id,
            wire::code::CONFIG_REJECTED,
            message.as_str(),
        ),
        ReloadVerdict::TimedOut => Envelope::error(
            wire::NAMESPACE,
            wire::member::CONFIG_RELOAD,
            request_id,
            wire::code::RELOAD_TIMEOUT,
            "the runtime did not answer the reload before its deadline; the last accepted \
             configuration stays in force",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIRST: ClientId = ClientId::new(7);
    const SECOND: ClientId = ClientId::new(8);

    #[test]
    fn a_reload_raises_the_signal_once_and_later_requests_share_it() {
        let mut reload = Reload::default();
        assert_eq!(reload.request(FIRST, 1, Tick::ZERO), Some(1));
        assert_eq!(reload.request(SECOND, 2, Tick::ZERO), None);
        let answers = reload.finish(&ReloadVerdict::Accepted { revision: 4 });
        let callers: Vec<(ClientId, u64)> = answers
            .iter()
            .map(|(client, envelope)| (*client, envelope.request_id))
            .collect();
        assert_eq!(callers, vec![(FIRST, 1), (SECOND, 2)]);
        for (_, envelope) in &answers {
            assert_eq!(envelope.payload(), Some(4_u64.to_be_bytes().as_slice()));
        }
    }

    #[test]
    fn the_next_reload_after_an_answer_raises_a_new_generation() {
        let mut reload = Reload::default();
        assert_eq!(reload.request(FIRST, 1, Tick::ZERO), Some(1));
        let _ = reload.finish(&ReloadVerdict::Rejected {
            message: "line 3".to_owned(),
        });
        assert_eq!(reload.request(FIRST, 2, Tick::ZERO), Some(2));
    }

    #[test]
    fn a_rejection_answers_every_caller_with_the_diagnostic() {
        let mut reload = Reload::default();
        let _ = reload.request(FIRST, 1, Tick::ZERO);
        let _ = reload.request(SECOND, 2, Tick::ZERO);
        let answers = reload.finish(&ReloadVerdict::Rejected {
            message: "init.lua:3: unexpected symbol".to_owned(),
        });
        assert_eq!(answers.len(), 2);
        for (_, envelope) in &answers {
            assert_eq!(envelope.error_code(), Some(wire::code::CONFIG_REJECTED));
        }
    }

    #[test]
    fn a_silent_runtime_times_the_reload_out_at_its_deadline() {
        let mut reload = Reload::default();
        let _ = reload.request(FIRST, 1, Tick::ZERO);
        assert!(reload.expire(Tick::new(DEADLINE_MS - 1)).is_empty());
        let answers = reload.expire(Tick::new(DEADLINE_MS));
        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0].1.error_code(), Some(wire::code::RELOAD_TIMEOUT));
        assert!(reload.expire(Tick::new(DEADLINE_MS * 2)).is_empty());
    }

    #[test]
    fn an_answer_with_no_reload_outstanding_is_nothing() {
        let mut reload = Reload::default();
        assert!(reload.finish(&ReloadVerdict::Accepted { revision: 1 }).is_empty());
    }
}
