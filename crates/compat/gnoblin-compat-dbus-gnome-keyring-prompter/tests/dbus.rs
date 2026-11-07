//! The prompter over a private bus: one gcr caller, the real interface, real frames.
//!
//! Every test starts its own `dbus-daemon` (see `support/bus.rs`), so nothing here can
//! reach the developer's session bus, and the caller is a real D-Bus object that exports
//! `org.gnome.keyring.internal.Prompter.Callback` exactly as a `GcrSystemPrompt` does.
//!
//! The one thing that is not real is the provider: `prompt.secret` has no default, and
//! what the services role's connection does is offer a request and hand back the answer.
//! That is the [`Offerer`] seam, and the tests' host records what was offered so the test
//! can answer it.

mod support;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures_lite::future;
use gnoblin_compat_core::{Gate, LockState, Mode};
use gnoblin_compat_gnome_keyring_prompter::{
    Exchange, Names, Offerer, PROMPTER_INTERFACE, PROMPTER_PATH, Prompt, PromptService, SYSTEM_NAME, Unavailable,
    export, run,
};
use gnoblin_model::{RequestId, RequestOutcome, Tick};
use support::bus::Bus;
use zbus::connection::Builder;
use zbus::{Connection, ObjectPath, OwnedVariant};

const CALLBACK_PATH: &str = "/org/example/Prompt1";
const PASSPHRASE: &str = "correct horse battery staple";

/// What the caller's object saw, shared with the test.
#[derive(Debug, Default)]
struct ClientState {
    ready: Mutex<Vec<(String, String)>>,
    done: Mutex<usize>,
}

/// A gcr caller's callback object.
struct FakeClient {
    state: Arc<ClientState>,
}

#[zbus::interface(name = "org.gnome.keyring.internal.Prompter.Callback")]
impl FakeClient {
    fn prompt_ready(&self, response: &str, _properties: HashMap<String, OwnedVariant>, exchange: &str) {
        self.state
            .ready
            .lock()
            .expect("the client lock is not poisoned")
            .push((response.to_owned(), exchange.to_owned()));
    }

    fn prompt_done(&self) {
        *self.state.done.lock().expect("the client lock is not poisoned") += 1;
    }
}

/// The privileged connection, as this crate sees it: it offers and returns an identity.
#[derive(Debug, Default)]
struct Host {
    offers: Arc<Mutex<Vec<(RequestId, Prompt)>>>,
    next: u64,
}

impl Offerer for Host {
    fn offer(&mut self, prompt: &Prompt, _deadline: Tick) -> Result<RequestId, Unavailable> {
        self.next += 1;
        let request = RequestId::new(self.next);
        self.offers
            .lock()
            .expect("the offer lock is not poisoned")
            .push((request, prompt.clone()));
        Ok(request)
    }
}

/// Yield until `condition` holds, or fail the test rather than hang.
async fn until(mut condition: impl FnMut() -> bool) {
    for _ in 0..10_000 {
        if condition() {
            return;
        }
        future::yield_now().await;
    }
    panic!("the condition never became true");
}

/// The caller's object path, as D-Bus wants it typed.
fn callback() -> ObjectPath<'static> {
    ObjectPath::try_from(CALLBACK_PATH).expect("the callback path is a constant")
}

/// A connection to the private bus at `address`.
async fn connect(address: &str) -> Connection {
    Builder::address(address).build().await.expect("the private bus is up")
}

#[test]
fn a_prompt_is_offered_answered_and_encrypted() {
    // The failure mode: the prompt never reaches a provider, or the passphrase travels in
    // the clear on the callback. The whole path is the real one — a caller's object, the
    // served interface, the effect loop's `PromptReady` — and the caller reads the answer
    // with its own half of the secret exchange.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let state = Arc::new(ClientState::default());
        let client = Builder::address(address.as_str())
            .serve_at(
                CALLBACK_PATH,
                FakeClient {
                    state: Arc::clone(&state),
                },
            )
            .build()
            .await
            .expect("the caller connects");
        let service_connection = connect(&address).await;

        let host = Host::default();
        let offers = Arc::clone(&host.offers);
        let service = PromptService::new(host, Gate::default(), Tick::new(120)).expect("the matrix loads");
        export(&service_connection, &service, Names::System)
            .await
            .expect("the name is free and the object is exported");

        let unique = service_connection
            .unique_name()
            .expect("a connection has a unique name")
            .to_string();
        let proxy = zbus::Proxy::new(&client, unique.as_str(), PROMPTER_PATH, PROMPTER_INTERFACE)
            .await
            .expect("the prompter object is there");

        let scenario = async {
            proxy
                .call_method("BeginPrompting", &(callback(),))
                .await
                .expect("the callback is registered");
            until(|| state.ready.lock().is_ok_and(|ready| !ready.is_empty())).await;

            // A gcr caller derives the key from the prompter's first string, then sends
            // its own half with the prompt it wants.
            let prompter_exchange = {
                let ready = state.ready.lock().expect("the client lock is not poisoned");
                ready[0].1.clone()
            };
            let mut caller = Exchange::new().expect("the system has randomness");
            caller.receive(&prompter_exchange).expect("the prompter's first string");
            let caller_exchange = caller.send(None).expect("the caller's half");

            let mut properties: HashMap<String, OwnedVariant> = HashMap::new();
            properties.insert(
                "title".to_owned(),
                OwnedVariant::from(zbus::Str::from("Unlock keyring")),
            );
            properties.insert(
                "message".to_owned(),
                OwnedVariant::from(zbus::Str::from("Enter the keyring password")),
            );
            proxy
                .call_method(
                    "PerformPrompt",
                    &(callback(), "password", properties, caller_exchange.as_str()),
                )
                .await
                .expect("the prompt is accepted");

            until(|| offers.lock().is_ok_and(|offers| offers.len() == 1)).await;
            let (request, prompt) = {
                let offers = offers.lock().expect("the offer lock is not poisoned");
                offers[0].clone()
            };
            assert_eq!(prompt.title, "Unlock keyring");
            assert_eq!(prompt.message, "Enter the keyring password");
            assert_eq!(prompt.callback, CALLBACK_PATH);

            service
                .answer(request, &RequestOutcome::Answered(PASSPHRASE.as_bytes().to_vec()))
                .expect("the request is pending");

            until(|| state.ready.lock().is_ok_and(|ready| ready.len() == 2)).await;
            let (response, answer_exchange) = {
                let ready = state.ready.lock().expect("the client lock is not poisoned");
                ready[1].clone()
            };
            assert_eq!(response, "yes");
            assert!(
                !answer_exchange.contains(PASSPHRASE),
                "the passphrase is not in the clear"
            );
            let secret = caller
                .receive(&answer_exchange)
                .expect("the answer parses")
                .expect("the answer carries a passphrase");
            assert_eq!(secret.as_str(), Some(PASSPHRASE));
        };

        // The service loop is what delivers every callback, so it runs beside the caller.
        future::or(run(service_connection, service.clone()), scenario).await;
    });
}

#[test]
fn a_stolen_name_is_refused() {
    // The failure mode: `gcr-prompter` is running beside this adapter, or another session
    // already owns the name. The adapter must refuse rather than queue behind the winner
    // and silently never be called.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let _other = Builder::address(address.as_str())
            .name(SYSTEM_NAME)
            .build()
            .await
            .expect("the other owner connects");

        let connection = connect(&address).await;
        let service = PromptService::new(Host::default(), Gate::default(), Tick::new(120)).expect("the matrix loads");
        let refusal = export(&connection, &service, Names::System)
            .await
            .expect_err("the name is taken");
        match refusal {
            Unavailable::NameUnavailable { name } => assert_eq!(name, SYSTEM_NAME),
            other => panic!("expected a name refusal, got {other:?}"),
        }
    });
}

#[test]
fn an_adapter_registers_once() {
    // The failure mode: two exports in one process, which would leave two effect loops
    // answering one caller.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let connection = connect(&address).await;
        let service = PromptService::new(Host::default(), Gate::default(), Tick::new(120)).expect("the matrix loads");
        export(&connection, &service, Names::Both)
            .await
            .expect("the first export");
        let refusal = export(&connection, &service, Names::Both)
            .await
            .expect_err("the second export is refused");
        assert!(
            refusal.to_string().contains("already exported"),
            "the refusal says why: {refusal}"
        );
    });
}

#[test]
fn the_mode_gate_refuses_a_disabled_adapter() {
    // The failure mode: an adapter whose mode is `off` still answers, so a distribution
    // that disabled it cannot rely on the switch.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let client = connect(&address).await;
        let service_connection = connect(&address).await;
        let gate = Gate::new(Mode::Off, Default::default(), LockState::default());
        let service = PromptService::new(Host::default(), gate, Tick::new(120)).expect("the matrix loads");
        export(&service_connection, &service, Names::System)
            .await
            .expect("the export does not depend on the mode");

        let unique = service_connection
            .unique_name()
            .expect("a connection has a unique name")
            .to_string();
        let proxy = zbus::Proxy::new(&client, unique.as_str(), PROMPTER_PATH, PROMPTER_INTERFACE)
            .await
            .expect("the prompter object is there");
        let error = proxy
            .call_method("BeginPrompting", &(callback(),))
            .await
            .expect_err("the adapter is off");
        let message = error.to_string();
        assert!(message.contains("off"), "the refusal names the rule: {message}");
    });
}

#[test]
fn an_unknown_prompt_kind_is_refused() {
    // The failure mode: a caller asks for a prompt kind gcr does not have, and the adapter
    // invents one instead of refusing.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let client = connect(&address).await;
        let service_connection = connect(&address).await;
        let service = PromptService::new(Host::default(), Gate::default(), Tick::new(120)).expect("the matrix loads");
        export(&service_connection, &service, Names::System)
            .await
            .expect("the export");

        let unique = service_connection
            .unique_name()
            .expect("a connection has a unique name")
            .to_string();
        let proxy = zbus::Proxy::new(&client, unique.as_str(), PROMPTER_PATH, PROMPTER_INTERFACE)
            .await
            .expect("the prompter object is there");
        proxy
            .call_method("BeginPrompting", &(callback(),))
            .await
            .expect("the callback is registered");
        let caller = Exchange::new().expect("the system has randomness");
        let first = caller.begin();
        let error = proxy
            .call_method(
                "PerformPrompt",
                &(
                    callback(),
                    "banana",
                    HashMap::<String, OwnedVariant>::new(),
                    first.as_str(),
                ),
            )
            .await
            .expect_err("the kind is not gcr's");
        assert!(
            error.to_string().contains("not a prompt kind"),
            "the refusal says which kind: {error}"
        );
    });
}
