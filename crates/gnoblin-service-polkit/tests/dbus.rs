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
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_lite::future;
use gnoblin_model::{RequestId, RequestOutcome, Tick};
use gnoblin_polkit_auth::{AuthRequest, HelperEvent, HelperSession, Identity, SecretText, current_uid, name_of};
use gnoblin_service_core::{CapabilitySlot, Offerer, Proposition, Refusal};
use gnoblin_service_polkit::{AGENT_PATH, AUTHORITY_PATH, AUTHORITY_SERVICE, Polkit};
use gnoblin_service_testkit::Bus;
use zbus::connection::Builder;
use zbus::{Connection, ObjectPath, OwnedVariant, fdo};

const AGENT_INTERFACE: &str = "org.freedesktop.PolicyKit1.AuthenticationAgent";

/// What the fake polkitd was told.
#[derive(Debug, Default)]
struct AuthorityState {
    registrations: Mutex<Vec<(HashMap<String, OwnedVariant>, String)>>,
    unregistrations: Mutex<Vec<String>>,
    /// The `AuthenticationAgentResponse2` calls: the uid, the cookie and the identity's details. Only polkit's helper
    /// makes this call; in these tests, the fake helper does.
    responses: Mutex<Vec<(u32, String, HashMap<String, OwnedVariant>)>>,
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

    /// What polkit's setuid helper calls once PAM said yes. polkitd would refuse any other caller; the fake helper
    /// is not root, so the fake records the call and a test reads what polkitd would have been told.
    fn authentication_agent_response2(
        &self,
        uid: u32,
        cookie: &str,
        identity: (String, HashMap<String, OwnedVariant>),
    ) -> fdo::Result<()> {
        let (_, details) = identity;
        self.state
            .responses
            .lock()
            .expect("the authority lock is not poisoned")
            .push((uid, cookie.to_owned(), details));
        Ok(())
    }
}

/// The privileged connection: it offers the request and reports what the provider said.
#[derive(Debug, Default)]
struct Host {
    offers: Arc<Mutex<Vec<(RequestId, Proposition)>>>,
    /// The requests the agent told the compositor to withdraw.
    withdrawn: Arc<Mutex<Vec<RequestId>>>,
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

    fn withdraw(&mut self, request: RequestId) {
        self.withdrawn
            .lock()
            .expect("the withdrawal lock is not poisoned")
            .push(request);
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
    withdrawn: Arc<Mutex<Vec<RequestId>>>,
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
    let withdrawn = Arc::clone(&host.withdrawn);
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
        withdrawn,
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
            // The offer carries what the compositor's dialog and polkit's helper need, and nothing else: the
            // cookie the helper names the authentication by, the action and who may authenticate.
            let payload = AuthRequest::decode(&proposition.subject).expect("the offer carries the agent's payload");
            assert_eq!(payload.cookie, "cookie-1");
            assert_eq!(payload.action_id, "org.freedesktop.policykit.exec");
            assert_eq!(payload.identities, [Identity::UnixUser { uid: 1000 }]);
            fixture
                .polkit
                .answer(request, &RequestOutcome::answered(b"unix-user:1000".to_vec()))
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
fn a_cancel_tells_the_compositor_to_withdraw_the_request() {
    // The failure mode: polkitd cancels the authentication, the call returns, and the compositor's dialog stays open
    // with its helper running until the request's own deadline.
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
            // A cookie that names nothing is not an error and withdraws nothing.
            fixture
                .caller
                .call_method("CancelAuthentication", &("no-such-cookie",))
                .await
                .expect("polkitd cancels requests that already finished");
        };
        let (reply, ()) = future::zip(begin(&fixture.caller, "cookie-1"), cancel).await;
        reply.expect_err("a cancelled request is not a success");
        let withdrawn = fixture.withdrawn.lock().expect("the withdrawal lock is not poisoned");
        assert_eq!(
            *withdrawn,
            [RequestId::new(1)],
            "exactly the cancelled request is withdrawn"
        );
    });
}

// --- the helper, polkitd and the agent on one private bus -------------------------------------------------------
//
// The compositor runs the helper and draws the dialog; its own tests prove that with a script. These prove the other
// half of the contract with a fake polkitd on a private bus: the agent's payload reaches the helper intact, the helper
// is the only thing that tells polkitd an authentication succeeded, and the reply to `BeginAuthentication` follows it.
// The compositor's role is played by a few lines that run the same `HelperSession` the dialog does.

/// Waits for `condition`, sleeping between looks: the helper is a process, and the bus answers on another thread.
async fn eventually(mut condition: impl FnMut() -> bool) {
    for _ in 0..2_000 {
        if condition() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
        future::yield_now().await;
    }
    panic!("the condition never became true");
}

/// A `polkit-agent-helper-1` for these tests: the `fake_polkit_helper` example behind a script that points it at the
/// private bus, as the real helper finds polkitd on the system bus. `cargo test` builds the example with this test.
fn fake_helper(address: &str, directory: &Path) -> PathBuf {
    let binary = std::env::current_exe()
        .expect("the test knows its own path")
        .parent()
        .and_then(Path::parent)
        .expect("the test lives in the target directory")
        .join("examples")
        .join("fake_polkit_helper");
    assert!(
        binary.is_file(),
        "the fake helper example is not built; run `cargo test -p gnoblin-service-polkit` ({})",
        binary.display()
    );
    let script = directory.join("polkit-agent-helper-1");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nexec env DBUS_SYSTEM_BUS_ADDRESS='{address}' '{}' \"$@\"\n",
            binary.display()
        ),
    )
    .expect("the wrapper is written");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("it is executable");
    script
}

/// Runs the helper for the request the agent offered, and types `password` at its prompt. Returns what the helper
/// said before it ended.
async fn run_helper(helper: &Path, payload: &AuthRequest, password: &str) -> Vec<HelperEvent> {
    let uid = current_uid().expect("this process has a uid");
    let user = name_of(uid).expect("and an account");
    // The wrapper was just written: a fork elsewhere in the test process can briefly hold it busy.
    let mut session = None;
    for _ in 0..50 {
        match HelperSession::spawn(helper, &user, &payload.cookie) {
            Ok(started) => {
                session = Some(started);
                break;
            }
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    let mut session = session.expect("the fake helper starts");
    let mut said = Vec::new();
    let mut answered = false;
    eventually(|| {
        for event in session.poll() {
            if matches!(event, HelperEvent::Prompt { .. }) && !answered {
                answered = true;
                session
                    .respond(&SecretText::new(password))
                    .expect("the helper reads the password");
            }
            said.push(event);
        }
        session.is_done()
    })
    .await;
    said
}

#[test]
fn polkitd_is_told_by_the_helper_and_the_reply_follows_its_success() {
    let bus = Bus::start();
    let address = bus.address().to_owned();
    let scratch = std::env::temp_dir().join(format!("gnoblin-polkit-chain-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).expect("a scratch directory");
    let helper = fake_helper(&address, &scratch);
    zbus::block_on(async move {
        let fixture = fixture(&address).await;
        let compositor = async {
            until(|| fixture.offers.lock().is_ok_and(|offers| offers.len() == 1)).await;
            let (request, proposition) = fixture.offers.lock().expect("the offer lock is not poisoned")[0].clone();
            let payload = AuthRequest::decode(&proposition.subject).expect("the agent's payload");
            let said = run_helper(&helper, &payload, "hunter2").await;
            assert_eq!(said.last(), Some(&HelperEvent::Success), "{said:?}");
            // The helper's call came first: it is what polkitd keeps, and the agent's reply follows it.
            assert_eq!(
                fixture
                    .authority
                    .responses
                    .lock()
                    .expect("the authority lock is not poisoned")
                    .len(),
                1,
                "polkitd was told by the helper before the dialog ended"
            );
            fixture
                .polkit
                .answer(request, &RequestOutcome::answered(b"unix-user:me".to_vec()))
                .expect("the request is pending");
        };
        let (reply, ()) = future::zip(begin(&fixture.caller, "cookie-1"), compositor).await;
        reply.expect("the helper authenticated, so the agent's reply is success");

        let responses = fixture
            .authority
            .responses
            .lock()
            .expect("the authority lock is not poisoned");
        let (uid, cookie, details) = &responses[0];
        assert_eq!(*uid, current_uid().expect("a uid"), "the helper reports its real uid");
        assert_eq!(cookie, "cookie-1", "and the cookie the agent offered");
        assert_eq!(
            details.get("uid"),
            Some(&OwnedVariant::from(*uid)),
            "for the identity it authenticated"
        );
    });
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn a_wrong_password_tells_polkitd_nothing_and_the_request_ends_unauthenticated() {
    let bus = Bus::start();
    let address = bus.address().to_owned();
    let scratch = std::env::temp_dir().join(format!("gnoblin-polkit-chain-wrong-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).expect("a scratch directory");
    let helper = fake_helper(&address, &scratch);
    zbus::block_on(async move {
        let fixture = fixture(&address).await;
        let compositor = async {
            until(|| fixture.offers.lock().is_ok_and(|offers| offers.len() == 1)).await;
            let (request, proposition) = fixture.offers.lock().expect("the offer lock is not poisoned")[0].clone();
            let payload = AuthRequest::decode(&proposition.subject).expect("the agent's payload");
            let said = run_helper(&helper, &payload, "not the password").await;
            assert_eq!(said.last(), Some(&HelperEvent::Failure), "{said:?}");
            // The dialog gives up (the user cancelled): the agent's reply is an error and polkitd holds nothing.
            fixture.polkit.cancel(request).expect("the request is pending");
        };
        let (reply, ()) = future::zip(begin(&fixture.caller, "cookie-1"), compositor).await;
        reply.expect_err("nobody authenticated");
        assert!(
            fixture
                .authority
                .responses
                .lock()
                .expect("the authority lock is not poisoned")
                .is_empty(),
            "polkitd was never told an authentication succeeded"
        );
    });
    let _ = std::fs::remove_dir_all(&scratch);
}
