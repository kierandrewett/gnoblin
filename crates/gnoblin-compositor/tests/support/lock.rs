//! A locked session and the peers that drive it: a tree connection that holds the tree or observes it, the keyboard a
//! test types a password on, and the lock taken the way the screensaver service takes it.
//!
//! `lock_card.rs` proves the built-in card against a shell that holds the tree over the socket; `inproc_shell.rs` proves it
//! against a shell that runs in the compositor process. Both read the lock the same way, so the code is one.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gnoblin_backend::{DeviceInfo, DeviceKind, InputEvent};
use gnoblin_proto::tree::op;
use gnoblin_proto::tree::tree::{
    AttachArgs, AttachedPayload, ChangesPayload, ReplyPayload, SnapshotPayload, TransactArgs,
};
use gnoblin_proto::{Envelope, Kind};
use gnoblin_types::DeviceId;

use super::{Session, Wire};

pub const NAMESPACE: &str = "tree";
pub const USER: &str = "kieran";
pub const PASSWORD: &str = "hi";

// evdev key codes
pub const KEY_H: u32 = 35;
pub const KEY_I: u32 = 23;
pub const KEY_N: u32 = 49;
pub const KEY_O: u32 = 24;
pub const KEY_ENTER: u32 = 28;

/// The keyboard the tests type on. The seat reads the character a key types from its keyboard's own state, so the
/// keyboard is announced first, as a backend announces a real one.
pub const KEYBOARD: DeviceId = DeviceId::new(7);

/// A session whose lock check accepts `hi` for `kieran`.
pub fn start() -> Session {
    start_with(|_| {})
}

/// Like [`start`], and `configure` sets the rest of the session's configuration before the compositor runs.
pub fn start_with(configure: impl FnOnce(&mut gnoblin_compositor::SessionConfig)) -> Session {
    let (session, _ends) = Session::start_privileged_configured(&[], |config| {
        config.wayland.lock_check = Some(gnoblin_wayland::LockCheck {
            authenticator: Arc::new(gnoblin_lock_auth::FakeAuthenticator::new().accept(USER, PASSWORD)),
            user: Some(USER.to_owned()),
        });
        configure(config);
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
pub struct Peer {
    pub wire: Wire,
    pub lock: u64,
    pub serial: u64,
    pub state: String,
    /// The tree's root, the seat, and this connection's epoch: a node the holder makes has the epoch in its high half.
    pub root: u64,
    pub seat: u64,
    pub epoch: u64,
    /// How many nodes the holder made, and the request id of its next transaction.
    pub made: u64,
    pub request: u64,
    /// The serial of every batch of changes this connection was told, in the order it was told them.
    pub batches: Vec<u64>,
}

pub fn attach(session: &Session, mode: &str) -> Peer {
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
    let (mut lock, mut seat, mut state) = (None, None, "not-locked".to_owned());
    loop {
        let envelope = wire.receive().expect("a snapshot chunk");
        let chunk: SnapshotPayload = envelope.payload_as().expect("the payload decodes");
        for node in chunk.nodes.iter().filter(|node| node.class == "lock") {
            lock = Some(node.id);
            if let Some(prop) = node.props.iter().find(|prop| prop.key == "state") {
                state.clone_from(&prop.value_text);
            }
        }
        if let Some(node) = chunk.nodes.iter().find(|node| node.class == "seat") {
            seat = Some(node.id);
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
        root: attached.root,
        seat: seat.expect("the tree has a seat"),
        epoch: attached.epoch,
        made: 0,
        request: 20,
        batches: Vec::new(),
    }
}

impl Peer {
    pub fn note(&mut self, envelope: &Envelope) {
        if envelope.member != "changes" {
            return;
        }
        let batch: ChangesPayload = envelope.payload_as().expect("the payload decodes");
        self.batches.push(batch.serial);
        self.serial = self.serial.max(batch.serial);
        for change in &batch.changes {
            if change.kind == "set" && change.node == self.lock && change.key == "state" {
                self.state.clone_from(&change.value_text);
            }
        }
    }

    /// Sends one transaction and reads to its reply.
    pub fn transact(&mut self, request_id: u64, ops: Vec<op::Properties>) -> ReplyPayload {
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
    pub fn watch(&mut self, window: Duration) {
        self.wire.set_read_timeout(Duration::from_millis(50));
        let deadline = Instant::now() + window;
        while Instant::now() < deadline {
            if let Some(envelope) = self.wire.receive_within() {
                self.note(&envelope);
            }
        }
    }

    pub fn wait_for_state(&mut self, target: &str) {
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

    pub fn call(&mut self, method: &str, text: &str) -> ReplyPayload {
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

    /// The shell renews its lease on the lock screen, as the stock lock screen does twice a second. A write the
    /// compositor made since the shell's last read (the lock's own `state`) makes the call stale, and the stock shell
    /// sends it again on the new base, so this does too.
    pub fn heartbeat(&mut self) {
        for _ in 0..5 {
            let reply = self.call("heartbeat", "");
            if reply.applied {
                return;
            }
            assert_eq!(reply.code, "stale-base", "{reply:?}");
        }
        panic!("the compositor kept calling the shell's base stale");
    }

    /// The shell records that its lock screen is up (or down).
    pub fn engage(&mut self, engaged: bool) {
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

    /// Applies `ops`, again on the new base while the compositor calls the base stale (it writes the lock's `state`
    /// itself, and the stock shell sends a refused transaction again).
    pub fn apply(&mut self, ops: &[op::Properties]) {
        for _ in 0..5 {
            self.request += 1;
            let request = self.request;
            let reply = self.transact(request, ops.to_vec());
            if reply.applied {
                return;
            }
            assert_eq!(reply.code, "stale-base", "{reply:?}");
        }
        panic!("the compositor kept calling the shell's base stale");
    }

    /// A canvas node of this connection, under `parent`, that takes input. Returns its id.
    pub fn canvas_under(&mut self, parent: u64) -> u64 {
        self.made += 1;
        let id = (self.epoch << 32) | self.made;
        self.apply(&[
            op::Properties {
                kind: "new".to_owned(),
                class: "canvas".to_owned(),
                id,
                ..Default::default()
            },
            op::Properties {
                kind: "insert".to_owned(),
                node: id,
                parent,
                index: 0,
                ..Default::default()
            },
            op::Properties {
                kind: "set".to_owned(),
                node: id,
                key: "input".to_owned(),
                value_kind: "bool".to_owned(),
                value_bool: true,
                ..Default::default()
            },
        ]);
        id
    }

    /// The shell gives the seat's keyboard focus to `node`, as the stock shell does with a canvas of its own.
    pub fn focus(&mut self, node: u64) {
        let seat = self.seat;
        self.apply(&[op::Properties {
            kind: "set".to_owned(),
            node: seat,
            key: "focus".to_owned(),
            value_kind: "node".to_owned(),
            value_node: node,
            ..Default::default()
        }]);
    }

    /// The shell's lock screen as the stock one makes it: a canvas under the `lock` node that takes input, with the
    /// keyboard on it. Returns the entry's id. The screen is up (`engage`) separately, as it is in the stock shell.
    pub fn hold_the_keyboard_on_a_screen(&mut self) -> u64 {
        let lock = self.lock;
        let entry = self.canvas_under(lock);
        self.focus(entry);
        entry
    }
}

pub fn key(session: &Session, code: u32) {
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
pub fn type_and_enter(session: &Session, codes: &[u32]) {
    for code in codes {
        key(session, *code);
    }
    key(session, KEY_ENTER);
}

/// Types the right password until the session unlocks or the time is up. A key typed before the card is up is dropped
/// by the lock gate, so the attempt is repeated; it is a no-op once the session is unlocked.
pub fn type_right_until_unlocked(session: &Session, peer: &mut Peer, patience: Duration) -> bool {
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

/// An observer that takes the lock, as the screensaver service does, and waits until the session is `locked`.
pub fn lock(session: &Session) -> Peer {
    let mut observer = attach(session, "observer");
    let reply = observer.call("lock", "");
    assert!(reply.applied, "an observer takes the lock: {reply:?}");
    observer.wait_for_state("locked");
    observer
}
