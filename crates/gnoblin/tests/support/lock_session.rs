//! What the tests of the session lock against a real compositor share (`js_lock.rs`, `js_lock_failures.rs`,
//! `js_lock_boundary.rs`).
//!
//! The session is [`Compositor::run`] on a headless backend with its sockets in a temporary directory, and its lock
//! check is the scripted `FakeAuthenticator` the other lock tests use in place of PAM (`accept("kieran", "hi")`). The
//! lock is taken through the tree, as the screensaver service takes it (an observer's `lock` call). The keys are
//! injected on the seat path of a real keyboard (`InputTap`): they go through the lock gate, which gives them to the
//! shell's focused node under the compositor's own lock, or to the built-in card, and to nothing else. The shell is a
//! [`JsRole`], run in a thread of its own ([`Shell`]) or by the test ([`Driven`]), as the runtime role is a process of
//! its own, and what the tests assert is what the compositor's tree holds, read over a second connection.
#![allow(dead_code)]

use std::cell::RefCell;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use gnoblin_backend::{DeviceInfo, DeviceKind, InputEvent};
use gnoblin_compositor::{Compositor, InputTap, SessionConfig, StopHandle};
use gnoblin_js::{JsConsoleSite, JsRole, JsRoleConfig, JsRoleLinks, JsShellError};
use gnoblin_tree_client::tree_link::SocketTreeLink;
use gnoblin_tree_client::tree_mirror::{LinkError, TreeIncoming, TreeLink, TreeTx, TxOp, TxReply};
use gnoblin_types::DeviceId;

use super::snapshot;

pub const PATIENCE: Duration = Duration::from_secs(60);
pub const USER: &str = "kieran";

pub const KEY_A: u32 = 30;
pub const KEY_B: u32 = 48;
pub const KEY_H: u32 = 35;
pub const KEY_I: u32 = 23;
pub const KEY_N: u32 = 49;
pub const KEY_O: u32 = 24;
pub const KEY_ENTER: u32 = 28;

/// The keyboard the tests type on. The seat reads the character a key types from its keyboard's own state, so the
/// keyboard is announced first, as a backend announces a real one.
pub const KEYBOARD: DeviceId = DeviceId::new(7);

/// How long a shell may stop renewing its lease on the lock screen, and then stay away, before the compositor's card
/// takes the password: the lease lapses after `LOCK_UI_LEASE_MS` and the card waits `LOCK_UI_GRACE_MS` more.
pub const CARD_WITHIN: Duration =
    Duration::from_millis(gnoblin_wayland::LOCK_UI_LEASE_MS + gnoblin_wayland::LOCK_UI_GRACE_MS);

/// What a test adds to a bound for the compositor's own latency and for a machine that other tests keep busy. The
/// bounds are wall-clock; this is the one place they are loosened.
pub const SLACK: Duration = Duration::from_secs(5);

/// A headless compositor on sockets of its own, with the scripted password check.
pub struct Session {
    directory: tempfile::TempDir,
    stop: StopHandle,
    pub input: InputTap,
    thread: Option<JoinHandle<()>>,
}

impl Session {
    pub fn start() -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).expect("the directory mode");
        let stop = StopHandle::new();
        let input = InputTap::new();
        let mut config = SessionConfig {
            ipc_socket: Some(directory.path().join("ipc.sock")),
            stop: Some(stop.clone()),
            input_tap: Some(input.clone()),
            ..SessionConfig::default()
        };
        config.wayland.socket_path = Some(directory.path().join("wayland.sock"));
        // The compositor pings the client of every mapped window and marks one that has not answered within this time as
        // unresponsive: a test thread that a loaded machine does not schedule for that long is not a hung application.
        config.wayland.check_alive_timeout = Duration::ZERO;
        // The screens' memory numbers are published as a change a second into a session: a change the tests do not count.
        config.wayland.gpu_memory_period = Duration::ZERO;
        config.wayland.lock_check = Some(gnoblin_wayland::LockCheck {
            authenticator: Arc::new(gnoblin_lock_auth::FakeAuthenticator::new().accept(USER, "hi")),
            user: Some(USER.to_owned()),
        });
        let thread = std::thread::Builder::new()
            .name("compositor".to_owned())
            .spawn(move || {
                Compositor::run(config).expect("the compositor runs until it is stopped");
            })
            .expect("the session thread");
        let session = Self {
            directory,
            stop,
            input,
            thread: Some(thread),
        };
        let deadline = Instant::now() + PATIENCE;
        while UnixStream::connect(session.ipc()).is_err() {
            assert!(Instant::now() < deadline, "the compositor never listened");
            std::thread::sleep(Duration::from_millis(5));
        }
        while !session.input.inject(InputEvent::DeviceAdded {
            device: DeviceInfo::new(KEYBOARD, "Test keyboard", DeviceKind::Keyboard),
        }) {
            assert!(Instant::now() < deadline, "the session never took input");
            std::thread::sleep(Duration::from_millis(5));
        }
        session
    }

    pub fn ipc(&self) -> PathBuf {
        self.directory.path().join("ipc.sock")
    }

    /// Where the compositor serves Wayland clients.
    pub fn wayland(&self) -> PathBuf {
        self.directory.path().join("wayland.sock")
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.request();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// A [`JsRole`] running its frame loop in a thread. It ends when told to, or when the role fails (the shell
/// crashed: a link or engine error is the end of a role, as it is of the process).
pub struct Shell {
    stop: Arc<AtomicBool>,
    /// When the tick the shell is inside began, in milliseconds after `began` plus one; zero between ticks.
    in_tick: Arc<AtomicU64>,
    began: Instant,
    thread: Option<JoinHandle<Vec<String>>>,
}

impl Shell {
    pub fn start(session: &Session, dir: &Path) -> Self {
        Self::start_with(session, dir, |_| {})
    }

    /// A shell whose role config `configure` has changed: a budget, the watcher.
    pub fn start_with(
        session: &Session,
        dir: &Path,
        configure: impl FnOnce(&mut JsRoleConfig) + Send + 'static,
    ) -> Self {
        let socket = session.ipc().to_str().expect("a UTF-8 socket path").to_owned();
        let dir = dir.to_path_buf();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let in_tick = Arc::new(AtomicU64::new(0));
        let ticking = Arc::clone(&in_tick);
        let began = Instant::now();
        let thread = std::thread::Builder::new()
            .name("js-shell".to_owned())
            .spawn(move || {
                let env = move |name: &str| (name == gnoblin_client::unix::SOCKET_VARIABLE).then(|| socket.clone());
                let mut config = JsRoleConfig::new(&dir, None);
                configure(&mut config);
                let mut role = JsRole::connect(&env, config).expect("the shell attaches");
                let mut problems = Vec::new();
                let mut taken = 0;
                while !stopped.load(Ordering::Relaxed) {
                    ticking.store(
                        u64::try_from(began.elapsed().as_millis()).unwrap_or(u64::MAX - 1) + 1,
                        Ordering::Relaxed,
                    );
                    let ticked = role.tick();
                    ticking.store(0, Ordering::Relaxed);
                    if let Err(error) = ticked {
                        problems.push(error.to_string());
                        break;
                    }
                    problems.extend(role.diagnostics_since(taken).iter().cloned());
                    taken = role.diagnostics_reported();
                    std::thread::sleep(Duration::from_millis(5));
                }
                problems
            })
            .expect("the shell thread");
        Self {
            stop,
            in_tick,
            began,
            thread: Some(thread),
        }
    }

    /// How long the shell has been inside the one tick it is in, which is zero between ticks. A tick that the script
    /// spins in lasts as long as the spin, so this is the proof, read from outside, that the script is spinning.
    pub fn stuck_for(&self) -> Duration {
        match self.in_tick.load(Ordering::Relaxed) {
            0 => Duration::ZERO,
            at => self.began.elapsed().saturating_sub(Duration::from_millis(at - 1)),
        }
    }

    /// Whether the shell's thread has ended: it was told to stop, or its role failed.
    pub fn ended(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// Stops the shell and returns the problems it reported.
    pub fn finish(&mut self) -> Vec<String> {
        self.stop.store(true, Ordering::Relaxed);
        self.thread
            .take()
            .map_or_else(Vec::new, |thread| thread.join().expect("the shell thread ends"))
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        self.finish();
    }
}

// ---- reading the tree ---------------------------------------------------------------------------------------------

/// What the compositor's tree says about the lock and the lock screen.
#[derive(Debug, Default)]
pub struct LockView {
    pub state: String,
    pub engaged: bool,
    /// Every string painted by a canvas under the `lock` node, in tree order.
    pub texts: Vec<String>,
}

pub fn view(ipc: &Path) -> LockView {
    let nodes = snapshot(ipc);
    let mut out = LockView::default();
    let Some(lock) = nodes.iter().find(|node| node.class == "lock") else {
        return out;
    };
    let prop = |node: &gnoblin_proto::tree::snapshot_node::Properties, key: &str| {
        node.props.iter().find(|prop| prop.key == key).cloned()
    };
    out.state = prop(lock, "state").map_or_else(|| "not-locked".to_owned(), |prop| prop.value_text);
    out.engaged = prop(lock, "engaged").is_some_and(|prop| prop.value_bool);
    // The lock screen's canvases: the scrim is a child of the lock node, the rest are beneath it.
    let mut under = vec![lock.id];
    let mut at = 0;
    while at < under.len() {
        let parent = under[at];
        under.extend(nodes.iter().filter(|node| node.parent == parent).map(|node| node.id));
        at += 1;
    }
    for node in nodes
        .iter()
        .filter(|node| node.class == "canvas" && under.contains(&node.id))
    {
        if let Some(content) = prop(node, "content")
            && let Ok(list) = gnoblin_paint::decode(&content.value_bytes)
        {
            out.texts.extend(list.ops.iter().filter_map(|op| match op {
                gnoblin_paint::PaintOp::Text { text, .. } => Some(text.clone()),
                _ => None,
            }));
        }
    }
    out
}

pub fn wait_until(what: &str, session: &Session, mut check: impl FnMut(&LockView) -> bool) -> LockView {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let seen = view(&session.ipc());
        if check(&seen) {
            return seen;
        }
        assert!(Instant::now() < deadline, "never reached: {what}; the lock is {seen:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Takes the lock the way the screensaver service does: an observer's `lock` call.
pub fn lock(session: &Session) {
    super::take_lock(&session.ipc());
}

pub fn key(session: &Session, code: u32) {
    for pressed in [true, false] {
        assert!(session.input.inject(InputEvent::Key {
            device: KEYBOARD,
            keycode: code,
            pressed,
        }));
    }
}

pub fn type_and_enter(session: &Session, codes: &[u32]) {
    for code in codes {
        key(session, *code);
    }
    key(session, KEY_ENTER);
}

/// The right password and Enter, as a user types them.
pub fn type_the_password(session: &Session) {
    type_and_enter(session, &[KEY_H, KEY_I]);
}

/// A wrong password and Enter.
pub fn type_a_wrong_password(session: &Session) {
    type_and_enter(session, &[KEY_N, KEY_O]);
}

/// Enter with nothing typed: the empty password.
pub fn type_nothing(session: &Session) {
    key(session, KEY_ENTER);
}

// ---- a shell over a link the compositor can be made to refuse ------------------------------------------------------

/// What the forging link does to a transaction on its way out.
#[derive(Clone, Copy)]
pub enum Forge {
    /// The next transaction is sent with a base far ahead of the compositor's serial. The real compositor answers
    /// `stale-base` with `failed_op` -1, as it does for a base older than the history it keeps: it cannot check which
    /// node changed, so it names no op.
    NextBaseAhead,
    /// The same for the next transaction that carries a call of this method.
    NextCallBaseAhead(&'static str),
}

/// What the forging link records, for the test to read back.
#[derive(Default)]
pub struct Forged {
    pub forge: Option<Forge>,
    /// The ops of every transaction the shell sent, in order, the refused ones included.
    pub sent: Vec<Vec<TxOp>>,
    /// Every reply the compositor sent, in order.
    pub replies: Vec<TxReply>,
    /// When each transaction in `sent` went out, taken just before the write: the compositor cannot have seen it earlier,
    /// so an age measured from here is never smaller than the real one.
    pub sent_at: Vec<Instant>,
    /// When each `heartbeat` call on the lock node went out, in order: the shell renewing its lease.
    pub beats: Vec<Instant>,
    /// When each write of the seat's `focus` went out, with the node it named.
    pub focus_writes: Vec<(Instant, u64)>,
}

/// The shell's own socket link, with the compositor's answers recorded and one transaction made stale on demand.
pub struct ForgingLink {
    inner: SocketTreeLink,
    forged: Rc<RefCell<Forged>>,
}

impl TreeLink for ForgingLink {
    fn attach(&mut self, version: u64) -> Result<(), LinkError> {
        self.inner.attach(version)
    }

    fn transact(&mut self, tx: &TreeTx) -> Result<(), LinkError> {
        let forge = {
            let mut forged = self.forged.borrow_mut();
            let now = Instant::now();
            forged.sent.push(tx.ops.clone());
            forged.sent_at.push(now);
            for op in &tx.ops {
                match op {
                    TxOp::Call { method, .. } if method == "heartbeat" => forged.beats.push(now),
                    TxOp::Set {
                        key,
                        value: gnoblin_tree_client::tree_mirror::MirrorValue::Node(node),
                        ..
                    } if key == "focus" => forged.focus_writes.push((now, *node)),
                    _ => {}
                }
            }
            let carries = |method: &str| {
                tx.ops
                    .iter()
                    .any(|op| matches!(op, TxOp::Call { method: called, .. } if called == method))
            };
            match forged.forge {
                Some(Forge::NextBaseAhead) => forged.forge.take(),
                Some(Forge::NextCallBaseAhead(method)) if carries(method) => forged.forge.take(),
                _ => None,
            }
        };
        if forge.is_none() {
            return self.inner.transact(tx);
        }
        let mut stale = tx.clone();
        stale.base_serial += 1_000;
        self.inner.transact(&stale)
    }

    fn poll(&mut self) -> Result<Vec<TreeIncoming>, LinkError> {
        let incoming = self.inner.poll()?;
        for message in &incoming {
            if let TreeIncoming::Reply(reply) = message {
                self.forged.borrow_mut().replies.push(reply.clone());
            }
        }
        Ok(incoming)
    }
}

/// A shell on the session, run by the test: it ticks only when the test says so, so the test decides when the shell
/// hangs, how slow its frames are and when it dies. Its budgets are the honest ones of the other real-compositor tests.
pub struct Driven {
    pub role: JsRole,
    pub forged: Rc<RefCell<Forged>>,
    /// Everything the shell reported, in order.
    pub problems: Vec<String>,
}

impl Driven {
    pub fn start(session: &Session, dir: &Path) -> Self {
        let transport = gnoblin_client::unix::UnixTransport::connect(&session.ipc()).expect("the session is listening");
        let version = {
            let (major, minor, patch) = gnoblin_proto::IPC_VERSION;
            gnoblin_types::Version::new(major, minor, patch)
        };
        let stream = gnoblin_client::Client::connect(transport, version)
            .expect("the handshake")
            .into_inner()
            .into_stream()
            .expect("the handshaken socket");
        let forged = Rc::new(RefCell::new(Forged::default()));
        let link = ForgingLink {
            inner: SocketTreeLink::new(stream).expect("the tree link"),
            forged: Rc::clone(&forged),
        };
        let mut config = JsRoleConfig::new(dir, None);
        config.watch = false;
        config.console = JsConsoleSite::Off;
        config.shell.entry_budget = Duration::from_secs(30);
        config.shell.frame_budget = Duration::from_secs(60);
        let role = JsRole::over_link(Box::new(link), JsRoleLinks::default(), config).expect("the shell attaches");
        Self {
            role,
            forged,
            problems: Vec::new(),
        }
    }

    pub fn tick(&mut self) {
        self.role.tick().expect("a frame");
        // The role keeps every diagnostic since it started, so this is the whole list and not the last frame's.
        self.problems = self.role.diagnostics().to_vec();
    }

    /// Ticks until `done`, which reads the shell and the compositor.
    pub fn until(&mut self, what: &str, mut done: impl FnMut(&mut Self) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        loop {
            self.tick();
            if done(self) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "never reached: {what}; the shell reported {:?}",
                self.problems
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    pub fn eval(&mut self, source: &str) -> String {
        self.role
            .shell_mut()
            .eval(source)
            .expect("the script runs")
            .unwrap_or_default()
    }

    /// Ticks for `window`, so the shell reads what the compositor sent and sends what it has to say.
    pub fn run_for(&mut self, window: Duration) {
        let deadline = Instant::now() + window;
        while Instant::now() < deadline {
            self.tick();
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// One more frame, and what it said if it failed: the end of a role, as a crash is.
    pub fn try_tick(&mut self) -> Result<(), JsShellError> {
        self.role.tick()
    }
}
