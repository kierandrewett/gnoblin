//! The polkit agent over a private bus: a fake polkitd registers it and asks it to
//! authenticate.
//!
//! Every test starts its own `dbus-daemon` (see `gnoblin-service-testkit`), so nothing here can
//! reach the developer's system bus — which matters more than usual for this adapter: the
//! real polkitd's agent slot is a per-session resource, and a test that registered on the
//! running session would take it from the desktop.
//!
//! The caller side is a real D-Bus client of the agent's object, so the signatures
//! (`sssa{ss}sa(sa{sv})`, the cookie, the identities) are the ones polkitd uses. The
//! provider side is the [`Offerer`] seam: a host that records the offer and answers it,
//! standing in for the shell that would draw the dialog.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures_lite::future;
use gnoblin_model::{RequestId, RequestOutcome, Tick};
use gnoblin_service_core::{CapabilitySlot, Offerer, Proposition, Provider, Refusal};
use gnoblin_service_polkit::{AGENT_PATH, AUTHORITY_PATH, AUTHORITY_SERVICE, Polkit};
use gnoblin_service_testkit::Bus;
use gnoblin_types::OwnerId;
use zbus::connection::Builder;
use zbus::{Connection, ObjectPath, OwnedVariant, fdo};

const AGENT_INTERFACE: &str = "org.freedesktop.PolicyKit1.AuthenticationAgent";

/// What the fake polkitd was told.
#[derive(Debug, Default)]
struct AuthorityState {
    registrations: Mutex<Vec<(HashMap<String, OwnedVariant>, String)>>,
    unregistrations: Mutex<Vec<String>>,
}

/// A polkitd that only records registrations.
struct FakeAuthority {
    state: Arc<AuthorityState>,
}

#[zbus::interface(name = "org.freedesktop.PolicyKit1.Authority")]
impl FakeAuthority {
    fn register_authentication_agent(
        &self,
        subject: (String, HashMap<String, OwnedVariant>),
        locale: &str,
        object_path: ObjectPath<'_>,
    ) -> fdo::Result<()> {
        let (_, details) = subject;
        self.state
            .registrations
            .lock()
            .expect("the authority lock is not poisoned")
            .push((details, format!("{object_path} {locale}")));
        Ok(())
    }

    fn unregister_authentication_agent(
        &self,
        _subject: (String, HashMap<String, OwnedVariant>),
        object_path: ObjectPath<'_>,
    ) -> fdo::Result<()> {
        self.state
            .unregistrations
            .lock()
            .expect("the authority lock is not poisoned")
            .push(object_path.to_string());
        Ok(())
    }
}

/// The privileged connection: it offers the request and reports what the provider said.
#[derive(Debug, Default)]
struct Host {
    offers: Arc<Mutex<Vec<(RequestId, Proposition)>>>,
    next: u64,
}

impl Offerer for Host {
    fn offer(&mut self, proposition: &Proposition, _deadline: Tick) -> Result<RequestId, Refusal> {
        self.next += 1;
        let request = RequestId::new(self.next);
        self.offers
            .lock()
            .expect("the offer lock is not poisoned")
            .push((request, proposition.clone()));
        Ok(request)
    }
}

async fn until(mut condition: impl FnMut() -> bool) {
    for _ in 0..10_000 {
        if condition() {
            return;
        }
        future::yield_now().await;
    }
    panic!("the condition never became true");
}

async fn connect(address: &str) -> Connection {
    Builder::address(address).build().await.expect("the private bus is up")
}

/// A registered agent and the objects a test drives it with.
struct Fixture {
    polkit: Polkit,
    caller: zbus::Proxy<'static>,
    authority: Arc<AuthorityState>,
    offers: Arc<Mutex<Vec<(RequestId, Proposition)>>>,
    _agent: Connection,
}

/// Build a fake polkitd, an agent registered with it, and a caller connected to the agent.
async fn fixture(address: &str) -> Fixture {
    let authority_state = Arc::new(AuthorityState::default());
    let _polkitd = Builder::address(address)
        .serve_at(
            AUTHORITY_PATH,
            FakeAuthority {
                state: Arc::clone(&authority_state),
            },
        )
        .name(AUTHORITY_SERVICE)
        .build()
        .await
        .expect("the fake polkitd connects");

    let agent_connection = connect(address).await;
    let host = Host::default();
    let offers = Arc::clone(&host.offers);
    let polkit = Polkit::new(host, Tick::new(15));
    agent_connection
        .object_server()
        .at(AGENT_PATH, polkit.clone())
        .await
        .expect("the agent path is a constant");
    polkit
        .register(&agent_connection)
        .await
        .expect("polkitd accepts the registration");

    let unique = agent_connection
        .unique_name()
        .expect("a connection has a unique name")
        .to_string();
    let caller = connect(address).await;
    let caller = zbus::Proxy::new(&caller, unique.to_string(), AGENT_PATH, AGENT_INTERFACE)
        .await
        .expect("the agent object is there");

    // Keep the polkitd connection alive for the test: its name and object go away when the
    // connection drops.
    std::mem::forget(_polkitd);

    Fixture {
        polkit,
        caller,
        authority: authority_state,
        offers,
        _agent: agent_connection,
    }
}

/// The arguments polkitd sends for one authentication.
async fn begin(host: &zbus::Proxy<'_>, cookie: &str) -> zbus::Result<()> {
    let mut details = HashMap::new();
    details.insert("polkit.message".to_owned(), "Authentication is required".to_owned());
    let mut identity_details: HashMap<String, OwnedVariant> = HashMap::new();
    identity_details.insert("uid".to_owned(), OwnedVariant::from(1000u32));
    let identities = vec![("unix-user".to_owned(), identity_details)];
    host.call_method(
        "BeginAuthentication",
        &(
            "org.freedesktop.policykit.exec",
            "Authentication is required to run /usr/bin/true",
            "dialog-password",
            details,
            cookie,
            identities,
        ),
    )
    .await?;
    Ok(())
}

#[test]
fn the_agent_registers_with_polkitd_and_answers_an_authentication() {
    // The failure mode: the agent never registers, so polkitd has nobody to ask, or the
    // offer never reaches a provider and the reply is a timeout instead of the user's
    // decision. The reply to polkitd is success, and only success.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let fixture = fixture(&address).await;
        {
            let registrations = fixture
                .authority
                .registrations
                .lock()
                .expect("the authority lock is not poisoned");
            assert_eq!(registrations.len(), 1, "one registration per session");
            let (details, where_) = &registrations[0];
            assert_eq!(
                details.get("pid"),
                Some(&OwnedVariant::from(std::process::id())),
                "polkitd is told which process registered"
            );
            assert!(details.contains_key("start-time"), "and when it started");
            assert!(where_.starts_with(AGENT_PATH), "and where to call it: {where_}");
        }

        let provider = async {
            until(|| fixture.offers.lock().is_ok_and(|offers| offers.len() == 1)).await;
            let (request, proposition) = {
                let offers = fixture.offers.lock().expect("the offer lock is not poisoned");
                offers[0].clone()
            };
            assert_eq!(proposition.slot, CapabilitySlot::AuthPolkit);
            assert_eq!(proposition.request.cookie, "cookie-1");
            assert_eq!(proposition.request.identities.len(), 1);
            assert_eq!(proposition.request.identities[0].kind, "unix-user");
            fixture
                .polkit
                .answer(request, &RequestOutcome::Answered(b"unix-user:1000".to_vec()))
                .expect("the request is pending");
        };
        let (reply, ()) = future::zip(begin(&fixture.caller, "cookie-1"), provider).await;
        reply.expect("the user authenticated, so polkitd sees success");
        assert!(
            fixture.polkit.pending().is_empty(),
            "the request finished with its reply"
        );
    });
}

#[test]
fn a_begin_authentication_without_a_provider_names_the_slot() {
    // The failure mode: the slot has no provider, so nothing shows a dialog and polkitd
    // waits for its own timeout. The D-Bus error must name the slot and nothing may be
    // offered.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let fixture = fixture(&address).await;
        fixture
            .polkit
            .claim(CapabilitySlot::AuthPolkit, Provider::None, None)
            .expect("no shell holds the slot");

        let error = begin(&fixture.caller, "cookie-1")
            .await
            .expect_err("nobody provides the slot");
        assert!(
            error.to_string().contains("auth.polkit"),
            "the error names the slot: {error}"
        );
        assert!(
            fixture.offers.lock().is_ok_and(|offers| offers.is_empty()),
            "nothing is offered when nobody can answer"
        );
    });
}

#[test]
fn a_cancel_ends_the_waiting_authentication() {
    // The failure mode: polkitd cancels an authentication (the calling application exited)
    // and the handler waits for an answer that will never come.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let fixture = fixture(&address).await;
        let cancel = async {
            until(|| fixture.offers.lock().is_ok_and(|offers| offers.len() == 1)).await;
            fixture
                .caller
                .call_method("CancelAuthentication", &("cookie-1",))
                .await
                .expect("polkitd can cancel");
        };
        let (reply, ()) = future::zip(begin(&fixture.caller, "cookie-1"), cancel).await;
        let error = reply.expect_err("a cancelled request is not a success");
        assert!(
            error.to_string().contains("cancelled"),
            "the error says the request was cancelled: {error}"
        );
        assert!(fixture.polkit.pending().is_empty());
    });
}

#[test]
fn an_agent_registers_once() {
    // The failure mode: two registrations from one process, which polkitd would have to
    // resolve by guessing which agent to call.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let fixture = fixture(&address).await;
        let refusal = fixture
            .polkit
            .register(&fixture._agent)
            .await
            .expect_err("the session already has an agent");
        assert_eq!(refusal, Refusal::AlreadyRegistered);
    });
}

#[test]
fn a_departed_provider_ends_the_waiting_authentication() {
    // The failure mode: the shell that claimed the slot goes away mid-authentication, and
    // the polkitd call waits for a dialog nobody is drawing.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let fixture = fixture(&address).await;
        let owner = OwnerId::new(1);
        fixture
            .polkit
            .claim(CapabilitySlot::AuthPolkit, Provider::Shell, Some(owner))
            .expect("a shell claims the slot");

        let provider = async {
            until(|| fixture.offers.lock().is_ok_and(|offers| offers.len() == 1)).await;
            let finished = fixture
                .polkit
                .release(CapabilitySlot::AuthPolkit, owner)
                .expect("the holder releases");
            assert_eq!(finished.len(), 1, "the pending request ended");
        };
        let (reply, ()) = future::zip(begin(&fixture.caller, "cookie-1"), provider).await;
        let error = reply.expect_err("the provider went away");
        assert!(
            error.to_string().contains("cancelled"),
            "the error says the request was cancelled: {error}"
        );
    });
}
