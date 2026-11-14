//! The built-in lock card against the real compositor: a locked session whose shell does not hold the lock screen
//! still takes a password and unlocks.
//!
//! Nothing here is a fake but the password check, which is the scripted `FakeAuthenticator` the other lock tests use in
//! place of PAM (`accept("kieran", "hi")`). The session is [`Compositor::run`] on a headless backend; the lock is taken
//! through the tree (an observer's `lock` call, as the screensaver service does); the keys are injected on the seat path
//! of a real keyboard (`InputTap`); the lock's state is read back over a tree observer connection. The shell is a raw
//! tree connection that holds the tree and writes the lock node's `engaged`, as the stock lock screen does, because this
//! crate does not link the JavaScript shell (`crates/gnoblin/tests/js_lock.rs` runs that one).
//!
//! The card's pixels are not read here, and nothing in the tree says it is up: what these tests prove is its effect.
//! Keys typed at a locked session with no shell go nowhere unless the card takes them, so a password that unlocks the
//! session was typed into the card; and a card that is gone takes nothing. Typing is repeated until the state changes,
//! because the card appears a turn or two after the lock is proven and a key typed before that is dropped.
//!
//! 1. No shell: the right password unlocks.
//! 2. A wrong password keeps the session locked, and the card still takes the right one after it.
//! 3. A shell that holds the lock screen keeps the card away, and one that dies while locked brings it up.
//! 4. A shell that attaches while the card is up takes over: the card is gone, and the shell's own `authenticate` unlocks.

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use gnoblin_backend::{DeviceInfo, DeviceKind, InputEvent};
use gnoblin_proto::tree::op;
use gnoblin_proto::tree::tree::{
    AttachArgs, AttachedPayload, ChangesPayload, ReplyPayload, SnapshotPayload, TransactArgs,
};
use gnoblin_proto::{Envelope, Kind};
use gnoblin_types::DeviceId;
use support::{Session, Wire};

const NAMESPACE: &str = "tree";
const USER: &str = "kieran";
const PASSWORD: &str = "hi";

// evdev key codes
const KEY_H: u32 = 35;
const KEY_I: u32 = 23;
const KEY_N: u32 = 49;
const KEY_O: u32 = 24;
const KEY_ENTER: u32 = 28;

/// The keyboard the tests type on. The seat reads the character a key types from its keyboard's own state, so the
/// keyboard is announced first, as a backend announces a real one.
const KEYBOARD: DeviceId = DeviceId::new(7);

/// A session whose lock check accepts `hi` for `kieran`.
fn start() -> Session {
    let (session, _ends) = Session::start_privileged_configured(&[], |config| {
        config.wayland.lock_check = Some(gnoblin_wayland::LockCheck {
            authenticator: Arc::new(gnoblin_lock_auth::FakeAuthenticator::new().accept(USER, PASSWORD)),
            user: Some(USER.to_owned()),
        });
    });
    let announced = Instant::now() + Duration::from_secs(10);
    while !session.input().inject(InputEvent::DeviceAdded {
        device: DeviceInfo::new(KEYBOARD, "Test keyboard", DeviceKind::Keyboard),
    }) {
        assert!(Instant::now() < announced, "the session never took input");
        std::thread::sleep(Duration::from_millis(10));
    }
    session
}

/// A tree connection: an observer or the holder, its serial, and the lock node's id and state as last seen.
struct Peer {
    wire: Wire,
    lock: u64,
    serial: u64,
    state: String,
}

fn attach(session: &Session, mode: &str) -> Peer {
    let mut wire = Wire::connect(session.socket());
    wire.send(&Envelope::request_message(
        NAMESPACE,
        "attach",
        1,
        &AttachArgs {
            version: 1,
            mode: mode.to_owned(),
        },
    ));
    let envelope = wire.receive().expect("an attach answer");
    assert!(envelope.error_code().is_none(), "the {mode} attaches: {envelope:?}");
    let attached: AttachedPayload = envelope.payload_as().expect("the payload decodes");
    let (mut lock, mut state) = (None, "not-locked".to_owned());
    loop {
        let envelope = wire.receive().expect("a snapshot chunk");
        let chunk: SnapshotPayload = envelope.payload_as().expect("the payload decodes");
        for node in chunk.nodes.iter().filter(|node| node.class == "lock") {
            lock = Some(node.id);
            if let Some(prop) = node.props.iter().find(|prop| prop.key == "state") {
                state.clone_from(&prop.value_text);
            }
        }
        if chunk.last {
            break;
        }
    }
    Peer {
        wire,
        lock: lock.expect("the tree has a lock node"),
        serial: attached.serial,
        state,
    }
}

impl Peer {
    fn note(&mut self, envelope: &Envelope) {
        if envelope.member != "changes" {
            return;
        }
        let batch: ChangesPayload = envelope.payload_as().expect("the payload decodes");
        self.serial = self.serial.max(batch.serial);
        for change in &batch.changes {
            if change.kind == "set" && change.node == self.lock && change.key == "state" {
                self.state.clone_from(&change.value_text);
            }
        }
    }

    /// Sends one transaction and reads to its reply.
    fn transact(&mut self, request_id: u64, ops: Vec<op::Properties>) -> ReplyPayload {
        self.wire.send(&Envelope::request_message(
            NAMESPACE,
            "transact",
            request_id,
            &TransactArgs {
                base_serial: self.serial,
                serial: self.serial + 1,
                ops,
            },
        ));
        loop {
            let envelope = self.wire.receive().expect("a frame");
            match envelope.kind {
                Some(Kind::Event(_)) => self.note(&envelope),
                Some(Kind::Reply(_)) => {
                    let reply: ReplyPayload = envelope.payload_as().expect("the payload decodes");
                    self.serial = self.serial.max(reply.serial);
                    return reply;
                }
                _ => {}
            }
        }
    }

    /// Reads for up to `window`, noting the lock's state.
    fn watch(&mut self, window: Duration) {
        self.wire.set_read_timeout(Duration::from_millis(50));
        let deadline = Instant::now() + window;
        while Instant::now() < deadline {
            if let Some(envelope) = self.wire.receive_within() {
                self.note(&envelope);
            }
        }
    }

    fn wait_for_state(&mut self, target: &str) {
        let deadline = Instant::now() + Duration::from_secs(20);
        self.wire.set_read_timeout(Duration::from_millis(100));
        while self.state != target {
            assert!(
                Instant::now() < deadline,
                "the lock never reached `{target}`; it is `{}`",
                self.state
            );
            if let Some(envelope) = self.wire.receive_within() {
                self.note(&envelope);
            }
        }
    }

    fn call(&mut self, method: &str, text: &str) -> ReplyPayload {
        let lock = self.lock;
        self.transact(
            10,
            vec![op::Properties {
                kind: "call".to_owned(),
                node: lock,
                method: method.to_owned(),
                text: text.to_owned(),
                ..Default::default()
            }],
        )
    }

    /// The shell records that its lock screen is up (or down).
    fn engage(&mut self, engaged: bool) {
        let lock = self.lock;
        let reply = self.transact(
            11,
            vec![op::Properties {
                kind: "set".to_owned(),
                node: lock,
                key: "engaged".to_owned(),
                value_kind: "bool".to_owned(),
                value_bool: engaged,
                ..Default::default()
            }],
        );
        assert!(reply.applied, "the holder writes engaged: {reply:?}");
    }
}

fn key(session: &Session, code: u32) {
    for pressed in [true, false] {
        assert!(
            session.input().inject(InputEvent::Key {
                device: KEYBOARD,
                keycode: code,
                pressed,
            }),
            "the session takes input"
        );
    }
}

/// Types `codes` and Enter, as a user does.
fn type_and_enter(session: &Session, codes: &[u32]) {
    for code in codes {
        key(session, *code);
    }
    key(session, KEY_ENTER);
}

/// Types the right password until the session unlocks or the time is up. A key typed before the card is up is dropped
/// by the lock gate, so the attempt is repeated; it is a no-op once the session is unlocked.
fn type_right_until_unlocked(session: &Session, peer: &mut Peer, patience: Duration) -> bool {
    let deadline = Instant::now() + patience;
    while Instant::now() < deadline {
        type_and_enter(session, &[KEY_H, KEY_I]);
        peer.watch(Duration::from_millis(250));
        if peer.state == "not-locked" {
            return true;
        }
    }
    false
}

fn lock(session: &Session) -> Peer {
    let mut observer = attach(session, "observer");
    let reply = observer.call("lock", "");
    assert!(reply.applied, "an observer takes the lock: {reply:?}");
    observer.wait_for_state("locked");
    observer
}

#[test]
fn with_no_shell_the_card_takes_the_right_password_and_unlocks_the_session() {
    let session = start();
    let mut observer = lock(&session);
    assert!(
        type_right_until_unlocked(&session, &mut observer, Duration::from_secs(15)),
        "a locked session with no shell never unlocked: the lock cover has nothing to type into"
    );
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_wrong_password_keeps_the_session_locked_and_the_card_still_takes_the_right_one() {
    let session = start();
    let mut observer = lock(&session);
    // Wrong passwords for a few seconds. The check's limiter answers `rate-limited` after three, which the card shows.
    let until = Instant::now() + Duration::from_secs(4);
    while Instant::now() < until {
        type_and_enter(&session, &[KEY_N, KEY_O]);
        observer.watch(Duration::from_millis(300));
        assert_eq!(observer.state, "locked", "a wrong password never unlocks");
    }
    // After the limiter's wait the right one gets through.
    assert!(
        type_right_until_unlocked(&session, &mut observer, Duration::from_secs(20)),
        "the card took no password after a wrong one"
    );
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_shell_that_holds_the_lock_screen_keeps_the_card_away_and_one_that_dies_brings_it_up() {
    let session = start();
    let mut holder = attach(&session, "holder");
    holder.engage(true);
    let mut observer = lock(&session);
    // The shell's screen is up: the card is not, so a typed password goes to the shell's (absent) entry and nowhere else.
    let until = Instant::now() + Duration::from_secs(3);
    while Instant::now() < until {
        type_and_enter(&session, &[KEY_H, KEY_I]);
        observer.watch(Duration::from_millis(300));
        assert_eq!(
            observer.state, "locked",
            "the card must not draw over a shell that holds the lock screen"
        );
    }
    // The shell dies while the session is locked: the card is up at once and takes the password.
    drop(holder);
    assert!(
        type_right_until_unlocked(&session, &mut observer, Duration::from_secs(15)),
        "the shell died and nothing took its place: the locked session has nothing to type into"
    );
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_shell_that_attaches_while_the_card_is_up_takes_over_and_unlocks_through_its_own_call() {
    let session = start();
    let mut observer = lock(&session);
    // The card has been up since the cover was proven; give the session a few turns to show it.
    observer.watch(Duration::from_millis(1500));
    let mut holder = attach(&session, "holder");
    holder.engage(true);
    observer.watch(Duration::from_millis(500));
    // The card is gone: the right password typed at the keyboard now goes to the shell's (absent) entry, not the card.
    let until = Instant::now() + Duration::from_secs(3);
    while Instant::now() < until {
        type_and_enter(&session, &[KEY_H, KEY_I]);
        observer.watch(Duration::from_millis(300));
        assert_eq!(
            observer.state, "locked",
            "the card must not outlive the shell's takeover"
        );
    }
    // The shell unlocks through the call the stock lock screen makes.
    let reply = holder.call("authenticate", PASSWORD);
    assert!(reply.applied, "the holder asks for the check: {reply:?}");
    observer.wait_for_state("not-locked");
    session.stop().expect("the session stops cleanly");
}
