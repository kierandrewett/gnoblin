//! The gcr passphrase prompter, linked into the services role.
//!
//! `gnoblin-services` owns the privileged connection and the loop that pumps it, and names
//! no compat adapter. This module is the composition the `gnoblin` binary makes: it serves
//! `org.gnome.keyring.SystemPrompter` and `org.gnome.keyring.Prompter` on the session bus,
//! turning each gcr prompt into a `prompt.secret` request on the services role's [`Host`].
//!
//! The session bus or a taken name makes the prompter unavailable, which is logged and
//! skipped: a session that cannot prompt still has a compositor.

use futures_lite::future::block_on;
use gnoblin_compat_core::Gate;
use gnoblin_compat_gnome_keyring_prompter::{
    Names, Offerer as PromptOfferer, Prompt, PromptService, Unavailable, export, run,
};
use gnoblin_log::{debug, info, warn};
use gnoblin_services::link::{Host, LinkedAgent, OfferFailure, RequestId, RequestOutcome, Tick};
use zbus::Connection;

/// The slot the prompter adapter exists for, as the plan's table names it.
pub const PROMPT_SLOT: &str = "prompt.secret";

/// The services role's [`Host`], seen as the offerer the prompt adapter draws through.
#[derive(Clone)]
struct PromptHost(Host);

impl PromptOfferer for PromptHost {
    fn offer(&mut self, prompt: &Prompt, deadline: Tick) -> Result<RequestId, Unavailable> {
        match Host::offer(&self.0, PROMPT_SLOT, &prompt.title, &prompt.message, deadline) {
            Ok(request) => Ok(request),
            // The adapter's own refusals name the slot, because `prompt.secret` has no
            // default provider and a prompt nobody draws must fail closed.
            Err(OfferFailure::NoProvider { .. }) => Err(Unavailable::NoProvider { slot: PROMPT_SLOT }),
            Err(OfferFailure::Refused { reason }) => Err(Unavailable::Bus { reason }),
        }
    }
}

/// The gcr prompter as a linked agent of the services role.
#[derive(Default)]
pub struct GcrPrompter {
    served: Option<(PromptService, std::thread::JoinHandle<()>)>,
}

impl GcrPrompter {
    /// A prompter that has not started: it serves nothing until the role starts it.
    pub fn new() -> GcrPrompter {
        GcrPrompter::default()
    }
}

impl LinkedAgent for GcrPrompter {
    fn start(&mut self, host: &Host, deadline: Tick) -> bool {
        let connection = match block_on(Connection::session()) {
            Ok(connection) => connection,
            Err(error) => {
                info!(%error, "[services] no session bus, so no prompt adapter");
                return false;
            }
        };
        let service = match PromptService::new(PromptHost(host.clone()), Gate::default(), deadline) {
            Ok(service) => service,
            Err(error) => {
                warn!(%error, "[services] the prompt adapter refused its support matrix");
                return false;
            }
        };
        if let Err(error) = block_on(export(&connection, &service, Names::Both)) {
            warn!(%error, "[services] the prompt names are unavailable");
            return false;
        }
        let serving = service.clone();
        let handle = std::thread::Builder::new()
            .name("gnoblin-prompter".to_owned())
            .spawn(move || block_on(run(connection, serving)))
            .map_err(|error| warn!(%error, "[services] the prompt loop could not be started"));
        match handle {
            Ok(handle) => {
                self.served = Some((service, handle));
                true
            }
            Err(()) => false,
        }
    }

    fn answer(&self, request: RequestId, outcome: &RequestOutcome) {
        if let Some((prompter, _)) = &self.served
            && let Err(error) = prompter.answer(request, outcome)
        {
            debug!(%error, request = request.get(), "[services] the prompter did not own the answer");
        }
    }
}
