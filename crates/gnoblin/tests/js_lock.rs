//! The session lock against a real compositor, with and without the JavaScript shell.
//!
//! The session is [`Compositor::run`] on a headless backend with its sockets in a temporary directory, and its lock
//! check is the scripted `FakeAuthenticator` the other lock tests use in place of PAM (`accept("kieran", "hi")`). The
//! lock is taken through the tree, as the screensaver service takes it (an observer's `lock` call). The keys are injected
//! on the seat path of a real keyboard (`InputTap`): they go through the lock gate, which gives them to the shell's
//! focused node under the compositor's own lock, or to the built-in card, and to nothing else. The shell is a
//! [`JsRole`] in a thread of its own, as the runtime role is a process of its own, and what the tests assert is what the
//! compositor's tree holds, read over a second connection.
//!
//! What is proven here, and what is not:
//!
//! - With no shell, the right password typed at the locked session unlocks it: the compositor's built-in card took the
//!   keys and the PAM test double checked them.
//! - A shell that attaches to the locked session draws its lock screen under the `lock` node and engages the lock (the
//!   clock and the entry are painted in the tree), a wrong password is answered "Wrong password." in that screen and
//!   the session stays locked, and the right one unlocks it, the screen comes down and the lock is disengaged.
//! - A transaction the compositor refuses `stale-base` with no op named (the shell's base is ahead of, or too far behind,
//!   the compositor's serial: the test forges it on the way out, because the harness has no other way to make the real
//!   compositor's base stale between the shell's read and its send) is sent again whole: an `Output.apply` in it still
//!   gets its answer, and a password check in it, whose text is not kept, is answered `failed` on the lock screen, which
//!   takes the next attempt and unlocks. The shell runs on the test's thread for those, which ticks it.
//! - A shell that stays attached and `engaged` but stops ticking (a hang) loses the lock UI to the compositor's card
//!   when its lease lapses.
//! - Not read: the pixels. The compositor draws the shell's lock screen and the card over the cover, but no test here
//!   reads the output's picture; the tree's paint lists are what is read.

use std::cell::RefCell;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use gnoblin_backend::{DeviceInfo, DeviceKind, InputEvent};
use gnoblin_compositor::{Compositor, InputTap, SessionConfig, StopHandle};
use gnoblin_js::{JsConsoleSite, JsRole, JsRoleConfig, JsRoleLinks};
use gnoblin_tree_client::tree_link::SocketTreeLink;
use gnoblin_tree_client::tree_mirror::{LinkError, TreeIncoming, TreeLink, TreeTx, TxOp, TxReply};
use gnoblin_types::DeviceId;

mod support;
use support::snapshot;

const PATIENCE: Duration = Duration::from_secs(60);
const USER: &str = "kieran";

const KEY_H: u32 = 35;
const KEY_I: u32 = 23;
const KEY_N: u32 = 49;
const KEY_O: u32 = 24;
const KEY_ENTER: u32 = 28;

/// The keyboard the tests type on. The seat reads the character a key types from its keyboard's own state, so the
/// keyboard is announced first, as a backend announces a real one.
const KEYBOARD: DeviceId = DeviceId::new(7);

/// A headless compositor on sockets of its own, with the scripted password check.
struct Session {
    directory: tempfile::TempDir,
    stop: StopHandle,
    input: InputTap,
    thread: Option<JoinHandle<()>>,
}

impl Session {
    fn start() -> Self {
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

    fn ipc(&self) -> PathBuf {
        self.directory.path().join("ipc.sock")
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

/// A [`JsRole`] running its frame loop in a thread.
struct Shell {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<Vec<String>>>,
}

impl Shell {
    fn start(session: &Session, dir: &Path) -> Self {
        let socket = session.ipc().to_str().expect("a UTF-8 socket path").to_owned();
        let dir = dir.to_path_buf();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("js-shell".to_owned())
            .spawn(move || {
                let env = move |name: &str| (name == gnoblin_client::unix::SOCKET_VARIABLE).then(|| socket.clone());
                let mut role = JsRole::connect(&env, JsRoleConfig::new(&dir, None)).expect("the shell attaches");
                let mut problems = Vec::new();
                while !stopped.load(Ordering::Relaxed) {
                    if let Err(error) = role.tick() {
                        problems.push(error.to_string());
                        break;
                    }
                    problems.extend(role.diagnostics().iter().cloned());
                    std::thread::sleep(Duration::from_millis(5));
                }
                problems
            })
            .expect("the shell thread");
        Self {
            stop,
            thread: Some(thread),
        }
    }

    /// Stops the shell and returns the problems it reported.
    fn finish(&mut self) -> Vec<String> {
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
struct LockView {
    state: String,
    engaged: bool,
    /// Every string painted by a canvas under the `lock` node, in tree order.
    texts: Vec<String>,
}

fn view(ipc: &Path) -> LockView {
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

fn wait_until(what: &str, session: &Session, mut check: impl FnMut(&LockView) -> bool) -> LockView {
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
fn lock(session: &Session) {
    support::take_lock(&session.ipc());
}

fn key(session: &Session, code: u32) {
    for pressed in [true, false] {
        assert!(session.input.inject(InputEvent::Key {
            device: KEYBOARD,
            keycode: code,
            pressed,
        }));
    }
}

fn type_and_enter(session: &Session, codes: &[u32]) {
    for code in codes {
        key(session, *code);
    }
    key(session, KEY_ENTER);
}

// ---- the tests ----------------------------------------------------------------------------------------------------

#[test]
fn with_no_shell_the_right_password_typed_at_the_locked_session_unlocks_it() {
    let session = Session::start();
    lock(&session);
    wait_until("the session locked", &session, |seen| seen.state == "locked");
    // A key typed before the built-in card is up is dropped by the lock gate, so the password is typed until it works.
    let deadline = Instant::now() + PATIENCE;
    loop {
        type_and_enter(&session, &[KEY_H, KEY_I]);
        std::thread::sleep(Duration::from_millis(300));
        let seen = view(&session.ipc());
        if seen.state == "not-locked" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "a locked session with no shell never unlocked: {seen:?}"
        );
    }
}

#[test]
fn a_js_shell_that_attaches_to_a_locked_session_takes_over_from_the_card_and_unlocks_it() {
    let session = Session::start();
    lock(&session);
    wait_until("the session locked", &session, |seen| seen.state == "locked");
    // The card has been up since the cover was proven: nobody holds the lock screen.
    std::thread::sleep(Duration::from_millis(1500));

    let config = tempfile::tempdir().expect("a config directory");
    fs::write(config.path().join("init.ts"), "export {};\n").expect("init.ts");
    let mut shell = Shell::start(&session, config.path());

    let screen = wait_until("the shell's lock screen engaged and painted", &session, |seen| {
        seen.engaged
            && seen.texts.iter().any(|text| text == "Enter your password.")
            && seen
                .texts
                .iter()
                .any(|text| text.len() == 5 && text.as_bytes()[2] == b':')
    });
    assert_eq!(
        screen.state, "locked",
        "the shell's screen replaced the card without unlocking"
    );

    // A wrong password, typed at the keyboard the shell's entry holds, is answered in the shell's screen.
    type_and_enter(&session, &[KEY_N, KEY_O]);
    wait_until("a wrong password answered on the lock screen", &session, |seen| {
        seen.texts.iter().any(|text| text == "Wrong password.")
    });
    assert_eq!(view(&session.ipc()).state, "locked", "a wrong password never unlocks");

    // The right one unlocks; the screen comes down and the lock is disengaged.
    let deadline = Instant::now() + PATIENCE;
    loop {
        type_and_enter(&session, &[KEY_H, KEY_I]);
        std::thread::sleep(Duration::from_millis(400));
        let seen = view(&session.ipc());
        if seen.state == "not-locked" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the shell's screen never unlocked the session: {seen:?}"
        );
    }
    let after = wait_until("the lock screen came down", &session, |seen| {
        !seen.engaged && seen.texts.is_empty()
    });
    assert_eq!(after.state, "not-locked");
    let problems = shell.finish();
    assert!(problems.is_empty(), "the shell reported: {problems:?}");
}

// ---- a shell over a link the compositor can be made to refuse ----------------------------------------------------------

/// What the forging link does to a transaction on its way out.
#[derive(Clone, Copy)]
enum Forge {
    /// The next transaction is sent with a base far ahead of the compositor's serial. The real compositor answers
    /// `stale-base` with `failed_op` -1, as it does for a base older than the history it keeps: it cannot check which
    /// node changed, so it names no op.
    NextBaseAhead,
    /// The same for the next transaction that carries a call of this method.
    NextCallBaseAhead(&'static str),
}

/// What the forging link records, for the test to read back.
#[derive(Default)]
struct Forged {
    forge: Option<Forge>,
    /// The ops of every transaction the shell sent, in order, the refused ones included.
    sent: Vec<Vec<TxOp>>,
    /// Every reply the compositor sent, in order.
    replies: Vec<TxReply>,
}

/// The shell's own socket link, with the compositor's answers recorded and one transaction made stale on demand.
struct ForgingLink {
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
            forged.sent.push(tx.ops.clone());
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

/// A shell on the session, run by the test: it ticks only when the test says so. Its budgets are the honest ones of the
/// other real-compositor tests.
struct Driven {
    role: JsRole,
    forged: Rc<RefCell<Forged>>,
    /// Everything the shell reported, in order.
    problems: Vec<String>,
}

impl Driven {
    fn start(session: &Session, dir: &Path) -> Self {
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

    fn tick(&mut self) {
        self.role.tick().expect("a frame");
        // The role keeps every diagnostic since it started, so this is the whole list and not the last frame's.
        self.problems = self.role.diagnostics().to_vec();
    }

    /// Ticks until `done`, which reads the shell and the compositor.
    fn until(&mut self, what: &str, mut done: impl FnMut(&mut Self) -> bool) {
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

    fn eval(&mut self, source: &str) -> String {
        self.role
            .shell_mut()
            .eval(source)
            .expect("the script runs")
            .unwrap_or_default()
    }

    /// Ticks for `window`, so the shell reads what the compositor sent and sends what it has to say.
    fn run_for(&mut self, window: Duration) {
        let deadline = Instant::now() + window;
        while Instant::now() < deadline {
            self.tick();
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[test]
fn an_output_apply_refused_stale_base_with_no_op_named_goes_again_and_is_answered() {
    let session = Session::start();
    let config = tempfile::tempdir().expect("a config directory");
    fs::write(config.path().join("init.ts"), "export {};\n").expect("init.ts");
    let mut shell = Driven::start(&session, config.path());
    shell.until("the shell sees the compositor's screen", |shell| {
        shell.eval("String(gnoblin.outputs.length)") != "0"
    });

    shell.forged.borrow_mut().forge = Some(Forge::NextBaseAhead);
    shell.eval("globalThis.app = gnoblin.outputs.at(0).apply({ scale: 1 }); 0");
    shell.until("the apply is answered", |shell| {
        shell.eval("String(app.done)") == "true"
    });

    let forged = shell.forged.borrow();
    let refused: Vec<&TxReply> = forged.replies.iter().filter(|reply| !reply.applied).collect();
    assert_eq!(refused.len(), 1, "the compositor refused one transaction: {refused:?}");
    assert_eq!(
        (refused[0].code.as_str(), refused[0].failed_op),
        ("stale-base", -1),
        "the real compositor's word for a base it cannot check"
    );
    let tokens: Vec<u64> = forged
        .sent
        .iter()
        .flatten()
        .filter_map(|op| match op {
            TxOp::Call { method, token, .. } if method == "apply" => Some(*token),
            _ => None,
        })
        .collect();
    assert_eq!(tokens.len(), 2, "the apply went out, was refused and went out again");
    assert_eq!(tokens[0], tokens[1], "under the same token");
}

#[test]
fn a_password_check_the_compositor_refused_stale_is_answered_failed_on_the_lock_screen_which_takes_the_next_attempt() {
    let session = Session::start();
    lock(&session);
    wait_until("the session locked", &session, |seen| seen.state == "locked");
    let config = tempfile::tempdir().expect("a config directory");
    fs::write(config.path().join("init.ts"), "export {};\n").expect("init.ts");
    let mut shell = Driven::start(&session, config.path());
    shell.until("the shell's lock screen is up and engaged", |_| {
        let seen = view(&session.ipc());
        seen.engaged && seen.texts.iter().any(|text| text == "Enter your password.")
    });

    // The check goes out with a base the compositor cannot take: it refuses the transaction `stale-base` and the
    // password, which is wiped once sent, cannot go again. The screen is told, and does not wait on "Checking...".
    shell.forged.borrow_mut().forge = Some(Forge::NextCallBaseAhead("authenticate"));
    type_and_enter(&session, &[KEY_H, KEY_I]);
    shell.until("the lost check is answered on the lock screen", |_| {
        view(&session.ipc())
            .texts
            .iter()
            .any(|text| text == "The password could not be checked.")
    });
    assert_eq!(view(&session.ipc()).state, "locked", "a lost check never unlocks");
    let refused: Vec<(String, i64)> = shell
        .forged
        .borrow()
        .replies
        .iter()
        .filter(|reply| !reply.applied)
        .map(|reply| (reply.code.clone(), reply.failed_op))
        .collect();
    assert_eq!(refused, [("stale-base".to_owned(), -1)]);

    // The screen takes another attempt, and the right password unlocks.
    let deadline = Instant::now() + PATIENCE;
    loop {
        type_and_enter(&session, &[KEY_H, KEY_I]);
        shell.run_for(Duration::from_millis(400));
        if view(&session.ipc()).state == "not-locked" {
            break;
        }
        assert!(Instant::now() < deadline, "the screen took no second attempt");
    }
    let secrets: Vec<&String> = shell
        .problems
        .iter()
        .filter(|line| line.contains("carries a secret"))
        .collect();
    assert_eq!(
        secrets.len(),
        1,
        "the lost check was reported once: {:?}",
        shell.problems
    );
}

#[test]
fn a_js_shell_that_stays_engaged_but_stops_ticking_loses_the_lock_ui_to_the_card() {
    let session = Session::start();
    lock(&session);
    wait_until("the session locked", &session, |seen| seen.state == "locked");
    let config = tempfile::tempdir().expect("a config directory");
    fs::write(config.path().join("init.ts"), "export {};\n").expect("init.ts");
    let mut shell = Driven::start(&session, config.path());
    shell.until("the shell's lock screen is up and engaged", |_| {
        let seen = view(&session.ipc());
        seen.engaged && seen.texts.iter().any(|text| text == "Enter your password.")
    });
    // While it ticks, it renews its lease: its screen is the lock UI and the card stays away.
    shell.run_for(Duration::from_millis(1500));
    assert_eq!(view(&session.ipc()).state, "locked");

    // The shell hangs: it stays attached and engaged and sends nothing. Its lease lapses and the card takes over.
    let deadline = Instant::now() + PATIENCE;
    loop {
        type_and_enter(&session, &[KEY_H, KEY_I]);
        std::thread::sleep(Duration::from_millis(400));
        let seen = view(&session.ipc());
        if seen.state == "not-locked" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "a shell that hung while holding the lock screen left the session with nothing to type into: {seen:?}"
        );
    }
}
