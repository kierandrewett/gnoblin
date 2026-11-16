//! A shell that runs in the compositor process, on a thread of its own (`inproc_shell`, design M2.8), against the real
//! compositor: the session is [`gnoblin_compositor::Compositor::run`] on a headless backend, the shell is a fake that holds the
//! tree through the in-process link, and everything it is compared with is the socket's.
//!
//! 1. An in-process shell is observed identically to a socket shell: the same script (attach, a snapshot, a second holder
//!    refused, a window placed and configured, a node made, a chord claimed and delivered, the hold released and the shell's
//!    nodes reaped when it ends) gives the same observations over both transports.
//! 2. A shell thread that hangs holds up no frame: the session locks, the shell never renews its lease, and the built-in
//!    card takes the password; the hung shell is then detached because its mailbox is not drained.
//! 3. A shell that panics is contained: the tree is detached, the standard window manager takes over, the compositor
//!    continues, and the error is logged and reported.
//! 4. A shell reports to the guardian through the hook it is given.
//! 5. The shell stops with the compositor: its thread ends and is joined, and one that will not end does not hold the
//!    compositor's exit hostage.

mod support;

use std::sync::mpsc::channel;
use std::sync::{Arc, Condvar, LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

use gnoblin_backend::InputEvent;
use gnoblin_compositor::inproc_shell::{InProcShellSpec, MailboxLimits, ShellEnd};
use gnoblin_log::__tracing::span::{Attributes, Id, Record};
use gnoblin_log::__tracing::{Event as LogEvent, Metadata, Subscriber};
use gnoblin_log::event::Logged;
use gnoblin_proto::tree::tree::{AttachArgs, ChangesPayload, SnapshotPayload};
use gnoblin_proto::{Envelope, Kind};
use gnoblin_secret::SecretText;
use gnoblin_tree_client::tree_mirror::{MirrorChangeKind, TreeIncoming, TxOp};
use support::lock::{KEYBOARD, lock, start, start_with, type_right_until_unlocked};
use support::shell::{Driven, Recorder, Shell, claim as seat_claim, make, set_bool, set_double, set_node};
use support::{Client, Session, Wire};

/// A latch a test holds a shell thread on, and opens to let it go.
#[derive(Clone, Default)]
struct Gate {
    open: Arc<(Mutex<bool>, Condvar)>,
}

impl Gate {
    fn wait(&self) {
        let (open, changed) = &*self.open;
        let mut open = open.lock().unwrap_or_else(PoisonError::into_inner);
        while !*open {
            open = changed.wait(open).unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn release(&self) {
        let (open, changed) = &*self.open;
        *open.lock().unwrap_or_else(PoisonError::into_inner) = true;
        changed.notify_all();
    }
}

// --- the log -----------------------------------------------------------------------------------------------------

/// Every event any thread of this test process logs.
#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<Logged>>>);

impl Subscriber for Logs {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _: &Id, _: &Record<'_>) {}

    fn record_follows_from(&self, _: &Id, _: &Id) {}

    fn event(&self, event: &LogEvent<'_>) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Logged::from_event(event));
    }

    fn enter(&self, _: &Id) {}

    fn exit(&self, _: &Id) {}
}

/// The log every compositor thread of this process writes to, installed once.
static LOGS: LazyLock<Logs> = LazyLock::new(|| {
    let logs = Logs::default();
    let _ = gnoblin_log::__tracing::subscriber::set_global_default(logs.clone());
    logs
});

fn logs() -> &'static Logs {
    &LOGS
}

// --- the world around the shell ----------------------------------------------------------------------------------

/// The answer to an attach by a second connection that wants the tree: the code it is refused with, or `attached`.
fn second_holder_answer(session: &Session) -> String {
    let mut wire = Wire::connect(session.socket());
    wire.send(&Envelope::request_message(
        "tree",
        "attach",
        1,
        &AttachArgs {
            version: 1,
            mode: String::new(),
        },
    ));
    let envelope = wire.receive().expect("an attach answer");
    envelope.error_code().unwrap_or("attached").to_owned()
}

/// Attaches as the holder once the previous holder's hold has been released and the nodes it made (`gone`) are reaped. The
/// hold is free a frame before the reap runs, so a new holder's first snapshot may still list them: it attaches again.
fn hold_once_reaped(session: &Session, gone: u64) -> Wire {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let (hold, nodes) = hold_once_released(session, Duration::from_secs(10));
        if !nodes.contains(&gone) {
            return hold;
        }
        assert!(Instant::now() < deadline, "node {gone} outlived the shell that made it");
        drop(hold);
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Attaches as the holder until the previous holder's hold has been released, and returns the connection with the node ids
/// of the snapshot.
fn hold_once_released(session: &Session, patience: Duration) -> (Wire, Vec<u64>) {
    let deadline = Instant::now() + patience;
    loop {
        let mut wire = Wire::connect(session.socket());
        wire.send(&Envelope::request_message(
            "tree",
            "attach",
            1,
            &AttachArgs {
                version: 1,
                mode: String::new(),
            },
        ));
        let envelope = wire.receive().expect("an attach answer");
        if envelope.error_code().is_none() {
            let mut nodes = Vec::new();
            loop {
                let chunk: SnapshotPayload = wire
                    .receive()
                    .expect("a chunk")
                    .payload_as()
                    .expect("the payload decodes");
                nodes.extend(chunk.nodes.iter().map(|node| node.id));
                if chunk.last {
                    return (wire, nodes);
                }
            }
        }
        assert!(Instant::now() < deadline, "the hold was never released");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Waits until the client's last configure has `size`, and returns it.
fn wait_for_size(client: &mut Client, window: &support::Window, size: (i32, i32)) -> (i32, i32) {
    support::wait_for(&format!("a configure of {size:?}"), || {
        client.roundtrip();
        (client.configured_size(window) == Some(size)).then_some(size)
    })
}

/// The keys of the chord `<Control><Alt>p`, on the keyboard the session was told about.
fn press_chord(session: &Session) {
    const CTRL: u32 = 29;
    const ALT: u32 = 56;
    const P: u32 = 25;
    for (keycode, pressed) in [
        (CTRL, true),
        (ALT, true),
        (P, true),
        (P, false),
        (ALT, false),
        (CTRL, false),
    ] {
        assert!(
            session.input().inject(InputEvent::Key {
                device: KEYBOARD,
                keycode,
                pressed,
            }),
            "the session takes input"
        );
    }
}

/// Maximises `window` and waits for the standard window manager to act on it: with a shell holding the tree the request is
/// the shell's event and nobody acts.
fn wm_maximises(client: &mut Client, window: &support::Window) {
    // Asked again each turn: a request that arrives while the dropped holder is still being let go of is its event.
    support::wait_for("the window manager maximising the window", || {
        client.request_maximize(window);
        client.roundtrip();
        client
            .configured_size(window)
            .is_some_and(|(width, _)| width > 200)
            .then_some(())
    });
}

/// Reads an observer's frames until a window node is created, which says the frame ran and published it.
fn hears_a_window_created(observer: &mut support::lock::Peer) {
    observer.wire.set_read_timeout(Duration::from_millis(100));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(Instant::now() < deadline, "the frame never published the window");
        let Some(envelope) = observer.wire.receive_within() else {
            continue;
        };
        if envelope.member != "changes" || !matches!(envelope.kind, Some(Kind::Event(_))) {
            continue;
        }
        let batch: ChangesPayload = envelope.payload_as().expect("the payload decodes");
        if batch
            .changes
            .iter()
            .any(|change| change.kind == "created" && change.class == "window")
        {
            return;
        }
    }
}

// --- 1. observed identically --------------------------------------------------------------------------------------

/// What one run of the script saw, from the shell and from the world around it.
#[derive(Debug, PartialEq)]
struct Observed {
    epoch: u64,
    snapshot_classes: Vec<String>,
    second_holder: String,
    window_parent: u64,
    placed: (bool, i64, String),
    configured: (i32, i32),
    heard_its_own_write: bool,
    made: (bool, String),
    claimed: (bool, String),
    first_chord: Option<(i64, String)>,
}

/// The parent of the window node as it was created, as the shell read it.
fn created_parent(shell: &Driven, node: u64) -> u64 {
    shell
        .seen
        .iter()
        .find_map(|message| match message {
            TreeIncoming::Changes { changes, .. } => changes
                .iter()
                .find(|change| change.kind == MirrorChangeKind::Created && change.node == node)
                .map(|change| change.parent),
            _ => None,
        })
        .unwrap_or_default()
}

/// Whether the shell was told `key` of `node` was set.
fn was_told_set(shell: &Driven, node: u64, key: &str) -> bool {
    shell.seen.iter().any(|message| match message {
        TreeIncoming::Changes { changes, .. } => changes
            .iter()
            .any(|change| change.kind == MirrorChangeKind::Set && change.node == node && change.key == key),
        _ => false,
    })
}

/// The script both transports run.
fn script(session: &Session, mut shell: Shell) -> Observed {
    let attached = shell.run(Driven::attach);
    let snapshot_classes = shell.run(|shell| shell.snapshot_classes());
    let second_holder = second_holder_answer(session);

    let mut client = Client::connect(session.wayland());
    let window = client.map_window("org.example.window", "Placed");
    shell.run(|shell| shell.until("the window node", |shell| shell.node_of("window").is_some()));
    let node = shell.run(|shell| shell.node_of("window").expect("the window"));
    let window_parent = shell.run(move |shell| created_parent(shell, node));

    let placed = shell.run(move |shell| {
        let reply = shell.transact(&[
            set_double(node, "x", 333.0),
            set_double(node, "y", 120.0),
            set_double(node, "width", 400.0),
            set_double(node, "height", 300.0),
        ]);
        (reply.applied, reply.failed_op, reply.code)
    });
    let configured = wait_for_size(&mut client, &window, (400, 300));
    let heard_its_own_write = shell.run(move |shell| {
        shell.until("its own write", |shell| was_told_set(shell, node, "width"));
        true
    });

    let (id, made, claimed) = shell.run(|shell| {
        let root = shell.attached.expect("attached").root;
        let id = shell.new_id(1);
        let made = shell.transact(&make(id, "node", root));
        let seat = shell.node_of("seat").expect("the seat");
        let claimed = shell.transact(&[seat_claim(seat, 7, "<Control><Alt>p")]);
        (id, (made.applied, made.code), (claimed.applied, claimed.code))
    });
    assert!(
        made.0 && claimed.0,
        "the shell's writes apply: {made:?} {claimed:?} {id}"
    );
    // The claim is applied by the compositor a turn after its answer, so the chord is pressed until it is delivered.
    let deadline = Instant::now() + Duration::from_secs(10);
    let first_chord = loop {
        press_chord(session);
        let chords = shell.run(|shell| {
            shell.pump(Duration::from_millis(200));
            shell.chords()
        });
        if let Some(first) = chords.first() {
            break Some(first.clone());
        }
        assert!(Instant::now() < deadline, "the claimed chord never reached the shell");
    };

    shell.finish();
    // The shell's nodes are reaped, and the hold is free for the next holder.
    let _hold = hold_once_reaped(session, id);
    Observed {
        epoch: attached.epoch,
        snapshot_classes,
        second_holder,
        window_parent,
        placed,
        configured,
        heard_its_own_write,
        made,
        claimed,
        first_chord,
    }
}

#[test]
fn an_in_process_shell_is_observed_identically_to_a_socket_shell() {
    let over_the_socket = {
        let session = start();
        let shell = Shell::socket(&session);
        let observed = script(&session, shell);
        session.stop().expect("the session stops cleanly");
        observed
    };
    let in_process = {
        let reports = Recorder::default();
        let (spec, shell) = Shell::in_process(reports.clone());
        let session = start_with(|config| config.shell = Some(spec));
        let observed = script(&session, shell);
        session.stop().expect("the session stops cleanly");
        reports.wait_for("ended the shell returned");
        observed
    };
    assert_eq!(in_process, over_the_socket, "the two transports saw different things");
    // The script is not vacuous: the shell was the holder, placed a window the client was told of, and had its chord.
    assert_eq!(in_process.epoch, 1, "the first attach is epoch one");
    assert_eq!(in_process.second_holder, "shell-already-attached");
    assert_eq!(in_process.window_parent, 0, "a window node is unparented");
    assert!(
        in_process.placed.0 && in_process.made.0 && in_process.claimed.0,
        "{in_process:?}"
    );
    assert_eq!(in_process.configured, (400, 300));
    assert!(in_process.first_chord.is_some(), "{in_process:?}");
}

// --- 2. a hung shell ---------------------------------------------------------------------------------------------

/// The shell thread builds a lock screen, engages the lock, renews its lease once and then hangs: it never reads its link,
/// and never writes to it again.
fn hanging_shell(hang: &Gate, engaged: std::sync::mpsc::Sender<()>) -> InProcShellSpec {
    let hang = hang.clone();
    InProcShellSpec::new("hung", move |link, _context| {
        let mut shell = Driven::new(link);
        shell.attach();
        let lock = shell.node_of("lock").expect("the lock node");
        let seat = shell.node_of("seat").expect("the seat");
        let screen = shell.new_id(1);
        shell.transact(&make(screen, "canvas", lock));
        shell.transact(&[set_bool(screen, "input", true), set_node(seat, "focus", screen)]);
        shell.transact(&[set_bool(lock, "engaged", true)]);
        shell.transact(&[TxOp::Call {
            node: lock,
            method: "heartbeat".to_owned(),
            args: Vec::new(),
            token: 0,
            text: SecretText::new(""),
        }]);
        engaged.send(()).expect("the test is waiting");
        hang.wait();
        Ok(())
    })
    .with_limits(MailboxLimits {
        stall: Duration::from_secs(2),
        ..MailboxLimits::default()
    })
    .with_stop_grace(Duration::from_millis(200))
}

#[test]
fn a_hung_shell_thread_blocks_no_frame_and_the_lock_card_takes_the_locked_session() {
    let hang = Gate::default();
    let (engaged, engaged_rx) = channel();
    let spec = hanging_shell(&hang, engaged);
    let session = start_with(|config| config.shell = Some(spec));
    engaged_rx
        .recv_timeout(Duration::from_secs(20))
        .expect("the shell built its lock screen and hung");

    // The session locks with the shell holding the tree and hung. Its lease lapses, and the card takes the password: the
    // frames that decide it ran on the compositor's thread all along.
    let mut observer = lock(&session);
    assert!(
        type_right_until_unlocked(&session, &mut observer, Duration::from_secs(30)),
        "a hung in-process shell kept the card from the locked session: nobody can unlock"
    );

    // Frames go on: a window a client maps is published to the observer.
    let mut client = Client::connect(session.wayland());
    let _window = client.map_window("org.example.window", "While hung");
    hears_a_window_created(&mut observer);

    // The shell's mailbox has had nothing taken from it for the stall limit, so the shell is detached and the hold is free.
    let (_hold, nodes) = hold_once_released(&session, Duration::from_secs(30));
    assert!(!nodes.is_empty());

    hang.release();
    session.stop().expect("the session stops cleanly");
}

// --- 3. a panic --------------------------------------------------------------------------------------------------

#[test]
fn a_panicking_shell_is_contained_logged_and_reported_and_the_standard_window_manager_takes_over() {
    let logs = logs();
    let reports = Recorder::default();
    let (made, made_rx) = channel();
    let spec = InProcShellSpec::new("panicking", move |link, _context| {
        let mut shell = Driven::new(link);
        shell.attach();
        let root = shell.attached.expect("attached").root;
        let id = shell.new_id(1);
        assert!(shell.transact(&make(id, "node", root)).applied);
        made.send(id).expect("the test is waiting");
        panic!("the shell blew up");
    })
    .with_reports(Arc::new(reports.clone()));
    let session = start_with(|config| config.shell = Some(spec));

    let id = made_rx
        .recv_timeout(Duration::from_secs(20))
        .expect("the shell made its node");
    assert_eq!(
        reports.wait_for("ended"),
        "ended the shell panicked: the shell blew up",
        "the panic is reported"
    );
    assert!(
        logs.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .any(|event| event.message.contains("ended with an error")
                && event
                    .fields
                    .iter()
                    .any(|(key, value)| key == "why" && value.contains("the shell blew up"))),
        "the panic is logged as an error"
    );

    // The tree is detached: the hold is free and the shell's node is reaped.
    let hold = hold_once_reaped(&session, id);
    // The compositor continues, and with no shell the standard window manager acts on a window's request.
    let mut client = Client::connect(session.wayland());
    let window = client.map_window("org.example.window", "Unshelled");
    drop(hold);
    wm_maximises(&mut client, &window);
    session.stop().expect("the session stops cleanly");
}

// --- 4. the guardian hook ----------------------------------------------------------------------------------------

#[test]
fn what_the_shell_reports_reaches_the_guardian_hook_and_its_end_is_reported_once() {
    let reports = Recorder::default();
    let spec = InProcShellSpec::new("reporting", |link, context| {
        let mut shell = Driven::new(link);
        shell.attach();
        context.reports.autostart(
            4,
            "terminal",
            &["foot".to_owned(), "--title".to_owned(), "a b".to_owned()],
        );
        context.reports.config_accepted(4);
        context.reports.ready(4);
        while !context.stop.requested() {
            shell.pump(Duration::from_millis(20));
        }
        Ok(())
    })
    .with_reports(Arc::new(reports.clone()));
    let session = start_with(|config| config.shell = Some(spec));
    reports.wait_for("ready 4");
    session.stop().expect("the session stops cleanly");
    assert_eq!(
        reports.lines(),
        [
            "autostart 4 terminal foot --title a b",
            "config-accepted 4",
            "ready 4",
            "ended the shell returned"
        ]
    );
}

// --- 5. stopping with the compositor -----------------------------------------------------------------------------

/// The thread id of the caller, as the kernel names it under `/proc/self/task`.
fn thread_id() -> String {
    let link = std::fs::read_link("/proc/thread-self").expect("/proc/thread-self");
    link.file_name().expect("a thread id").to_string_lossy().into_owned()
}

fn thread_is_alive(id: &str) -> bool {
    std::path::Path::new("/proc/self/task").join(id).exists()
}

#[test]
fn the_shell_thread_is_stopped_and_joined_when_the_compositor_exits() {
    let reports = Recorder::default();
    let (id_tx, id_rx) = channel();
    let spec = InProcShellSpec::new("orderly", move |link, context| {
        let mut shell = Driven::new(link);
        shell.attach();
        id_tx.send(thread_id()).expect("the test is waiting");
        // A shell that sleeps on its link, as a runtime with its own loop does: the end of the compositor wakes it.
        while !context.stop.requested() && shell.failed.is_none() {
            shell.pump(Duration::from_secs(30));
        }
        Ok(())
    })
    .with_reports(Arc::new(reports.clone()));
    let session = start_with(|config| config.shell = Some(spec));
    let id = id_rx.recv_timeout(Duration::from_secs(20)).expect("the shell attached");
    assert!(thread_is_alive(&id), "the shell thread is running");

    let started = Instant::now();
    session.stop().expect("the session stops cleanly");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the stop waited for a shell that was asleep on its link: {:?}",
        started.elapsed()
    );
    assert!(!thread_is_alive(&id), "the shell thread outlived the compositor");
    assert_eq!(reports.lines(), ["ended the shell returned"]);
}

#[test]
fn a_shell_that_will_not_stop_does_not_hold_the_compositors_exit_hostage() {
    let hang = Gate::default();
    let reports = Recorder::default();
    let (id_tx, id_rx) = channel();
    let spec = InProcShellSpec::new("stuck", {
        let hang = hang.clone();
        move |_link, _context| {
            id_tx.send(thread_id()).expect("the test is waiting");
            hang.wait();
            Ok(())
        }
    })
    .with_reports(Arc::new(reports.clone()))
    .with_stop_grace(Duration::from_millis(300));
    let session = start_with(|config| config.shell = Some(spec));
    let id = id_rx
        .recv_timeout(Duration::from_secs(20))
        .expect("the shell is running");

    let started = Instant::now();
    session.stop().expect("the session stops cleanly");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the compositor waited for a shell that will not end: {:?}",
        started.elapsed()
    );
    assert_eq!(
        reports.lines(),
        [format!("ended {}", ShellEnd::Abandoned)],
        "the abandoned shell is reported"
    );
    assert!(
        thread_is_alive(&id),
        "the abandoned thread cannot be killed; it ends with the process"
    );

    // Let it go: it ends, and does not report a second time.
    hang.release();
    support::wait_for("the abandoned thread ending", || (!thread_is_alive(&id)).then_some(()));
    assert_eq!(reports.lines().len(), 1, "{:?}", reports.lines());
}
