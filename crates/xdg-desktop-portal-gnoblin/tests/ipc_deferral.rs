//! The IPC thread never waits for the portal's state lock: a handler holds it across a consent offer, only the IPC thread
//! can send the offer, and a graph event the thread has read must wait for the lock to be free.
//!
//! The compositor is a scripted transport. The test plays the handler: it takes the portal's lock, lets the thread
//! read two graph events and a dialog's answer, then makes the offer while still holding the lock. A thread that
//! waited for the lock would never serve the offer, so every wait here is under a deadline. Once the handler lets go
//! the queued work must be applied, in the order it was read.

mod support;

use std::collections::VecDeque;
use std::os::fd::{BorrowedFd, OwnedFd};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use gnoblin_client::wire::{self, member};
use gnoblin_client::{Client, Transport as ClientTransport};
use gnoblin_model::{ObjectId, ObjectView, Observation, Revision, TypeKey, Value};
use gnoblin_proto::{CodecError, Envelope, Handshake, Transport, Version};
use support::bus::Bus;
use xdg_desktop_portal_gnoblin::consent::{ConsentOutcome, Interface, NewSession, Pending, PortalCapability};
use xdg_desktop_portal_gnoblin::{Backends, IpcHost, Portal, Setting};
use zbus::connection::Builder;

const DEADLINE: Duration = Duration::from_secs(10);
const BUS: &str = ":1.42";
const APP: &str = "org.example.Capture";
const SESSION: &str = "/org/freedesktop/portal/desktop/session/1";

/// What the scripted compositor has been sent and has handed out.
#[derive(Default)]
struct Script {
    inbound: VecDeque<Envelope>,
    sent: Vec<Envelope>,
    /// How many frames the thread has read.
    read: usize,
}

/// The compositor, as the portal's IPC thread sees it: the test pushes frames, the thread reads them.
#[derive(Clone, Default)]
struct Compositor(Arc<Mutex<Script>>);

impl Compositor {
    fn script(&self) -> std::sync::MutexGuard<'_, Script> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn push(&self, envelope: Envelope) {
        self.script().inbound.push_back(envelope);
    }

    fn wait_until(&self, what: &str, ready: impl Fn(&Script) -> bool) {
        let deadline = Instant::now() + DEADLINE;
        while !ready(&self.script()) {
            assert!(Instant::now() < deadline, "the scripted compositor never saw {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Transport for Compositor {
    fn send(&mut self, envelope: &Envelope, _fds: &[BorrowedFd<'_>]) -> Result<(), CodecError> {
        let mut script = self.script();
        // The loop subscribes first; the compositor refuses it, which the loop logs and carries on from.
        if envelope.member == member::SUBSCRIBE {
            script.inbound.push_back(Envelope::error(
                wire::NAMESPACE,
                member::SUBSCRIBE,
                envelope.request_id,
                "unknown-slot",
                "no settings object in this script",
            ));
        }
        script.sent.push(envelope.clone());
        Ok(())
    }

    fn receive(&mut self, _fds: &mut Vec<OwnedFd>) -> Result<Option<Envelope>, CodecError> {
        {
            let mut script = self.script();
            if let Some(envelope) = script.inbound.pop_front() {
                script.read += 1;
                return Ok(Some(envelope));
            }
        }
        std::thread::sleep(Duration::from_millis(5));
        Err(CodecError::Io(std::io::Error::from(std::io::ErrorKind::TimedOut)))
    }
}

impl ClientTransport for Compositor {
    fn send_handshake(&mut self, _frame: &Handshake) -> Result<(), gnoblin_client::HandshakeError> {
        Ok(())
    }

    fn receive_handshake(&mut self) -> Result<Handshake, gnoblin_client::HandshakeError> {
        Ok(Handshake::accept(version()))
    }

    fn set_read_timeout(&mut self, _timeout: Option<Duration>) -> Result<(), gnoblin_client::ClientError> {
        Ok(())
    }
}

fn version() -> Version {
    Version::new(0, 2, 0)
}

/// The settings object with its `icon_theme` at `theme`.
fn icon_theme(theme: &str) -> Envelope {
    wire::observation(&Observation::Opened {
        view: ObjectView::new(
            ObjectId::new(1),
            TypeKey::new("gnoblin", "settings"),
            std::iter::once(("icon_theme".to_owned(), Value::Text(theme.to_owned()))).collect(),
        ),
        revision: Revision::ZERO,
    })
}

#[test]
fn a_graph_event_read_while_a_handler_holds_the_lock_waits_and_is_applied_in_order_after_it_lets_go() {
    let bus = Bus::start();
    let compositor = Compositor::default();
    let client = Client::connect(compositor.clone(), version()).expect("the scripted compositor accepts");
    let (host, ipc_loop) = IpcHost::pair(client);
    let connection = futures_lite::future::block_on(Builder::address(bus.address()).build()).expect("the private bus");
    let portal = Portal::new(connection, Box::new(host), Backends::default(), 60_000, true);
    {
        let mut shared = portal.lock();
        shared
            .guard
            .create(NewSession {
                bus_name: BUS.to_owned(),
                app_id: APP.to_owned(),
                sandboxed: Some(true),
                interface: Interface::ScreenCast,
                path: SESSION.to_owned(),
            })
            .expect("a session");
        shared.guard.start(BUS, SESSION).expect("it starts");
    }
    let looped = portal.clone();
    std::thread::spawn(move || ipc_loop.run(looped));

    // The handler: it takes the lock, waits for the thread to have read the work, then offers a prompt with the lock held.
    let (locked_tx, locked_rx) = mpsc::channel();
    let (go_tx, go_rx) = mpsc::channel::<()>();
    let (done_tx, done_rx) = mpsc::channel();
    let handler_portal = portal.clone();
    std::thread::spawn(move || {
        let mut shared = handler_portal.lock();
        locked_tx.send(()).expect("the test is listening");
        go_rx.recv().expect("the test says go");
        let now = shared.now();
        let asked = shared.guard.ask(now, BUS, SESSION, PortalCapability::Capture, "");
        // Still holding the lock: nothing the thread read has been applied.
        let applied_while_held = shared
            .settings
            .read("org.gnome.desktop.interface", "icon-theme")
            .cloned();
        drop(shared);
        let _ = done_tx.send((asked, applied_while_held));
    });
    locked_rx.recv_timeout(DEADLINE).expect("the handler took the lock");

    // The thread reads two graph events while the handler holds the lock, then the handler offers.
    compositor.push(icon_theme("first"));
    compositor.push(icon_theme("second"));
    compositor.wait_until("both events read", |script| script.read >= 3);
    std::thread::sleep(Duration::from_millis(100));
    go_tx.send(()).expect("the handler waits");

    // A thread that waits for the lock never serves this offer.
    let (asked, applied_while_held) = done_rx
        .recv_timeout(DEADLINE)
        .expect("the IPC thread served the offer while the handler held the lock: it did not deadlock");
    assert!(
        applied_while_held.is_none(),
        "an event is not applied under the handler's lock"
    );
    let pending = asked.unwrap_or_else(|refusal| panic!("the offer stands: {refusal:?}"));
    assert!(matches!(pending, Pending::Asked { .. }), "a prompt was offered");

    // The dialog is answered, and the answer queues behind the two events.
    let ticket = {
        compositor.wait_until("the offer", |script| {
            script.sent.iter().any(|e| e.member == member::PORTAL_UI)
        });
        compositor
            .script()
            .sent
            .iter()
            .find(|e| e.member == member::PORTAL_UI)
            .expect("the offer was sent")
            .request_id
    };
    compositor.push(wire::portal_outcome(ticket, "handled", ""));

    let outcome = futures_lite::future::block_on(futures_lite::future::or(
        async { Some(xdg_desktop_portal_gnoblin::iface::wait(pending).await) },
        async {
            async_io::Timer::after(DEADLINE).await;
            None
        },
    ));
    assert_eq!(
        outcome,
        Some(ConsentOutcome::Granted),
        "the deferred answer was applied once the handler let go"
    );

    // Both events were applied, in the order they were read: the last one stands.
    let deadline = Instant::now() + DEADLINE;
    loop {
        let value = portal
            .lock()
            .settings
            .read("org.gnome.desktop.interface", "icon-theme")
            .cloned();
        if value == Some(Setting::Text("second".to_owned())) {
            break;
        }
        assert!(Instant::now() < deadline, "the settings never held `second`: {value:?}");
        std::thread::sleep(Duration::from_millis(5));
    }
}
