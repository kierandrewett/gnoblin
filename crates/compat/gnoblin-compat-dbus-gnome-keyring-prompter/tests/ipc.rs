//! The passphrase's way home: a shell's answer over the real client, then the encrypted
//! callback.
//!
//! The state machine's own tests stop at its edge. This one goes through the in-process
//! compositor, so the answer travels the same frames a shell will send: a request is
//! offered, a shell claims it and answers with a passphrase, and the prompter reads that
//! outcome and encrypts it for the caller that asked.

mod support;

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use gnoblin_client::{Event, Server};
use gnoblin_compat_dbus_gnome_keyring_prompter::state::SLOT;
use gnoblin_compat_dbus_gnome_keyring_prompter::{
    Callback, Effect, Exchange, Offerer, Prompt, PromptKind, Prompter, Unavailable,
};
use gnoblin_model::{Graph, RequestId, RequestOutcome, Tick};
use gnoblin_proto::Version;
use gnoblin_schema::{Catalog, Type};

const VERSION: Version = Version::new(0, 2, 0);
const PASSPHRASE: &str = "correct horse battery staple";

/// A prompter that offers through the in-process compositor, sharing it with the shell
/// connection and with the test that reads the outcome.
struct LoopbackHost {
    server: Rc<RefCell<Server>>,
}

impl Offerer for LoopbackHost {
    fn offer(&mut self, prompt: &Prompt, deadline: Tick) -> Result<RequestId, Unavailable> {
        // The offer carries what a provider must show: the slot it belongs to and the
        // title and message the caller wrote, which is exactly what the trusted prompt
        // request is offered with.
        Ok(self
            .server
            .borrow_mut()
            .offer(SLOT, &prompt.title, &prompt.message, deadline))
    }
}

fn password_prompt() -> Prompt {
    Prompt {
        kind: PromptKind::Password,
        callback: "/org/example/Prompt1".to_owned(),
        title: "Unlock keyring".to_owned(),
        message: "Enter the password".to_owned(),
        description: String::new(),
        warning: String::new(),
        choice_label: String::new(),
        choice_chosen: false,
        password_new: false,
        caller_window: String::new(),
        continue_label: String::new(),
        cancel_label: String::new(),
    }
}

fn callback() -> Callback {
    Callback {
        sender: ":1.7".to_owned(),
        path: "/org/example/Prompt1".to_owned(),
    }
}

/// A subscription to the whole graph, so the shell's connection sees the offer.
fn server() -> Server {
    let catalog = Catalog::load_dir(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../gnoblin-schema/schema"
    )))
    .expect("the checked-in schema loads");
    Server::new(Graph::new(catalog), VERSION)
}

#[test]
fn a_shells_answer_reaches_the_prompter_encrypted() {
    // The failure mode this covers is the one no state-machine test can: the passphrase a
    // shell typed is what the caller decrypts. It travels as the request's answer on the
    // wire, is flagged sensitive there, and leaves as the exchange's ciphertext.
    let server = Rc::new(RefCell::new(server()));
    let mut shell = server.borrow_mut().connect(VERSION).expect("versions match");
    let (mut prompter, mut effects) = Prompter::new(LoopbackHost {
        server: Rc::clone(&server),
    });

    prompter.begin(&callback()).expect("the callback is new");
    let Effect::Ready {
        exchange: prompter_exchange,
        ..
    } = effects.try_recv().expect("the began callback is announced")
    else {
        panic!("expected a ready effect");
    };
    let mut caller = Exchange::new().expect("the system has randomness");
    caller.receive(&prompter_exchange).expect("the prompter's first string");
    let caller_exchange = caller.send(None).expect("the caller's half");

    let request = prompter
        .perform(&callback(), password_prompt(), &caller_exchange, Tick::new(120))
        .expect("the prompt is offered");

    let Event::RequestOffered {
        request: offered,
        slot,
        title,
        message,
        ..
    } = shell.next_event().expect("the offer")
    else {
        panic!("expected an offer");
    };
    assert_eq!(offered, request);
    assert_eq!(slot, SLOT, "the offer names the slot the prompt belongs to");
    assert_eq!(title, "Unlock keyring", "the provider is given the caller's title");
    assert_eq!(message, "Enter the password", "and the caller's message");
    shell.claim(request).expect("the shell claims the prompt");
    shell
        .answer(request, PASSPHRASE.as_bytes())
        .expect("the shell answers with the passphrase");

    let outcome = server.borrow().finished(request).expect("the request finished");
    assert_eq!(outcome, RequestOutcome::Answered(PASSPHRASE.as_bytes().to_vec()));
    let RequestOutcome::Answered(bytes) = outcome else {
        panic!("expected an answer");
    };
    prompter
        .answer(request, &RequestOutcome::Answered(bytes))
        .expect("the prompter accepts the answer");

    let Effect::Ready { response, exchange, .. } = effects.try_recv().expect("the caller is told") else {
        panic!("expected a ready effect");
    };
    assert_eq!(response, "yes");
    let secret = caller
        .receive(&exchange)
        .expect("the answer parses")
        .expect("the answer carries the passphrase");
    assert_eq!(secret.as_str(), Some(PASSPHRASE));
}

#[test]
fn the_request_answer_is_the_schemas_secret_field() {
    // Why the frame above is flagged at all: `request.answer` is declared secret in the
    // schema, so the generated table flags every frame that carries one without this crate
    // knowing which prompt asked. A rename in `request.toml` that this adapter missed
    // would be a passphrase in a `gnoblinctl ipc dump`.
    let catalog = Catalog::load_dir(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../gnoblin-schema/schema"
    )))
    .expect("the checked-in schema loads");
    let namespace = gnoblin_schema::Namespace::parse("gnoblin").expect("namespace");
    let request = catalog
        .objects(&namespace)
        .find(|object| object.name == "request")
        .expect("the request object is in the schema");
    let answer = request
        .methods
        .iter()
        .find(|method| method.name == "answer")
        .expect("the answer method is declared")
        .params
        .iter()
        .find(|parameter| parameter.name == "answer")
        .expect("the answer parameter is declared");
    assert!(answer.secret, "`request.answer` must stay secret");
    assert_eq!(answer.ty, Type::Bytes);
}
