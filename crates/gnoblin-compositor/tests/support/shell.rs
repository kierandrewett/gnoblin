//! A shell driven through a `TreeLink`: the same code over the owner socket and over the in-process link.
//!
//! The tests that compare the two transports run one script against a [`Shell`] and read what the compositor and the
//! shell observed. A socket shell runs on the test's thread; an in-process shell runs on the shell thread the compositor
//! spawns, and the test hands it each step as a job and waits for the answer, so the script is the same code either way.

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use gnoblin_compositor::inproc_shell::{InProcShellSpec, ShellEnd, ShellReports};
use gnoblin_secret::SecretText;
use gnoblin_tree_client::tree_link::SocketTreeLink;
use gnoblin_tree_client::tree_mirror::{
    Attached, LinkError, MirrorChangeKind, MirrorValue, SnapshotNode, TreeIncoming, TreeLink, TreeTx, TxOp, TxReply,
};

use super::Session;

/// How long a step waits for the compositor before it fails.
const WAIT: Duration = Duration::from_secs(10);

/// What a shell has read from the compositor, and the serial it has caught up to.
pub struct Driven {
    link: Box<dyn TreeLink>,
    /// The answer to the last attach.
    pub attached: Option<Attached>,
    /// The nodes of the last snapshot.
    pub nodes: Vec<SnapshotNode>,
    snapshot_done: bool,
    /// The serial of the last batch, answer or snapshot the shell read.
    pub serial: u64,
    /// Everything the shell has read since it attached, in arrival order.
    pub seen: Vec<TreeIncoming>,
    replies: VecDeque<TxReply>,
    /// The error the link reported, once it has.
    pub failed: Option<LinkError>,
    /// Set by a step that says the shell is done.
    pub quit: bool,
}

impl Driven {
    pub fn new(link: Box<dyn TreeLink>) -> Self {
        Self {
            link,
            attached: None,
            nodes: Vec::new(),
            snapshot_done: false,
            serial: 0,
            seen: Vec::new(),
            replies: VecDeque::new(),
            failed: None,
            quit: false,
        }
    }

    /// Waits up to `wait` for the compositor to say something, and reads what has arrived.
    pub fn pump(&mut self, wait: Duration) {
        if self.failed.is_some() {
            return;
        }
        match self.link.wait_fd() {
            Some(fd) => {
                let _ = gnoblin_client::wait_readable(&[fd], wait);
            }
            None => std::thread::sleep(wait.min(Duration::from_millis(5))),
        }
        match self.link.poll() {
            Ok(incoming) => {
                for message in incoming {
                    self.note(message);
                }
            }
            Err(error) => self.failed = Some(error),
        }
    }

    fn note(&mut self, message: TreeIncoming) {
        match &message {
            TreeIncoming::Attached(attached) => {
                self.attached = Some(*attached);
                self.serial = attached.serial;
                self.nodes.clear();
                self.snapshot_done = false;
            }
            TreeIncoming::Snapshot { nodes, last, .. } => {
                self.nodes.extend(nodes.iter().cloned());
                self.snapshot_done |= *last;
            }
            TreeIncoming::Changes { serial, .. } => self.serial = self.serial.max(*serial),
            TreeIncoming::Reply(reply) => {
                self.serial = self.serial.max(reply.serial);
                self.replies.push_back(reply.clone());
            }
            _ => {}
        }
        self.seen.push(message);
    }

    /// Reads until `done` holds of what the shell has read, or fails.
    pub fn until(&mut self, what: &str, done: impl Fn(&Driven) -> bool) {
        let deadline = Instant::now() + WAIT;
        while !done(self) {
            assert!(
                self.failed.is_none(),
                "the link failed while waiting for {what}: {:?}",
                self.failed
            );
            assert!(Instant::now() < deadline, "never saw {what}: {:?}", self.seen);
            self.pump(Duration::from_millis(50));
        }
    }

    /// Attaches and reads the whole snapshot.
    pub fn attach(&mut self) -> Attached {
        self.link.attach(1).expect("the attach is sent");
        self.until("the attach answer and the snapshot", |shell| {
            shell.attached.is_some() && shell.snapshot_done
        });
        self.attached.expect("attached")
    }

    /// Applies `ops` on the serial the shell has caught up to, again on the new base while the compositor calls it stale.
    pub fn transact(&mut self, ops: &[TxOp]) -> TxReply {
        for _ in 0..5 {
            let tx = TreeTx {
                base_serial: self.serial,
                serial: self.serial + 1,
                ops: ops.to_vec(),
            };
            self.link.transact(&tx).expect("the transaction is sent");
            self.until("the transaction's answer", |shell| !shell.replies.is_empty());
            let reply = self.replies.pop_front().expect("a reply");
            if reply.code != "stale-base" {
                return reply;
            }
        }
        panic!("the compositor kept calling the shell's base stale");
    }

    /// The first node of `class` the shell has seen: in the snapshot, or created since.
    pub fn node_of(&self, class: &str) -> Option<u64> {
        self.nodes
            .iter()
            .find(|node| node.class == class)
            .map(|node| node.id)
            .or_else(|| {
                self.seen.iter().find_map(|message| match message {
                    TreeIncoming::Changes { changes, .. } => changes
                        .iter()
                        .find(|change| change.kind == MirrorChangeKind::Created && change.class == class)
                        .map(|change| change.node),
                    _ => None,
                })
            })
    }

    /// The nodes of the snapshot by class, sorted: what the shell saw of the tree's shape.
    pub fn snapshot_classes(&self) -> Vec<String> {
        let mut classes: Vec<String> = self.nodes.iter().map(|node| node.class.clone()).collect();
        classes.sort();
        classes
    }

    /// A node id the shell allocates: its epoch in the high half.
    pub fn new_id(&self, n: u64) -> u64 {
        (self.attached.expect("attached").epoch << 32) | n
    }

    /// Every chord claim the shell was told changed state, as the claim and its state.
    pub fn chords(&self) -> Vec<(i64, String)> {
        self.seen
            .iter()
            .filter_map(|message| match message {
                TreeIncoming::Chord(chord) => Some((chord.claim, chord.state.clone())),
                _ => None,
            })
            .collect()
    }
}

pub fn set_double(node: u64, key: &str, value: f64) -> TxOp {
    TxOp::Set {
        node,
        key: key.to_owned(),
        value: MirrorValue::Double(value),
        seconds: None,
        delay: None,
        tween: None,
        samples: None,
    }
}

pub fn set_bool(node: u64, key: &str, value: bool) -> TxOp {
    TxOp::Set {
        node,
        key: key.to_owned(),
        value: MirrorValue::Bool(value),
        seconds: None,
        delay: None,
        tween: None,
        samples: None,
    }
}

pub fn set_node(node: u64, key: &str, target: u64) -> TxOp {
    TxOp::Set {
        node,
        key: key.to_owned(),
        value: MirrorValue::Node(target),
        seconds: None,
        delay: None,
        tween: None,
        samples: None,
    }
}

/// A node of `class` made under `parent`: a `new` and an `insert`.
pub fn make(id: u64, class: &str, parent: u64) -> Vec<TxOp> {
    vec![
        TxOp::New {
            id,
            class: class.to_owned(),
        },
        TxOp::Insert {
            node: id,
            parent,
            index: 0,
        },
    ]
}

/// The seat's claim of `chord` under `claim`.
pub fn claim(seat: u64, claim: i64, chord: &str) -> TxOp {
    TxOp::Call {
        node: seat,
        method: "claim".to_owned(),
        args: claim.to_le_bytes().to_vec(),
        token: 0,
        text: SecretText::new(chord),
    }
}

/// What a shell reported to the guardian, in order, as lines.
#[derive(Clone, Default)]
pub struct Recorder {
    lines: Arc<Mutex<Vec<String>>>,
}

impl Recorder {
    pub fn lines(&self) -> Vec<String> {
        self.lines.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn push(&self, line: String) {
        self.lines.lock().unwrap_or_else(PoisonError::into_inner).push(line);
    }

    /// Waits until a line starting with `prefix` was reported, and returns it.
    pub fn wait_for(&self, prefix: &str) -> String {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(line) = self.lines().into_iter().find(|line| line.starts_with(prefix)) {
                return line;
            }
            assert!(
                Instant::now() < deadline,
                "no `{prefix}` report; got {:?}",
                self.lines()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl ShellReports for Recorder {
    fn autostart(&self, revision: u64, name: &str, command: &[String]) {
        self.push(format!("autostart {revision} {name} {}", command.join(" ")));
    }

    fn config_accepted(&self, revision: u64) {
        self.push(format!("config-accepted {revision}"));
    }

    fn config_rejected(&self, revision: u64, diagnostic: &str) {
        self.push(format!("config-rejected {revision} {diagnostic}"));
    }

    fn ready(&self, revision: u64) {
        self.push(format!("ready {revision}"));
    }

    fn ended(&self, end: &ShellEnd) {
        self.push(format!("ended {end}"));
    }
}

type Job = Box<dyn FnOnce(&mut Driven) + Send>;

/// A shell the tests drive, over either transport.
pub enum Shell {
    /// Connected to the owner socket, driven on the test's thread.
    Socket(Option<Driven>),
    /// Running on the compositor's shell thread, driven by jobs sent to it.
    InProc { jobs: Sender<Job> },
}

impl Shell {
    /// A shell over the owner socket of `session`.
    pub fn socket(session: &Session) -> Self {
        let transport = gnoblin_client::unix::UnixTransport::connect(session.socket()).expect("the session listens");
        let (major, minor, patch) = gnoblin_proto::IPC_VERSION;
        let stream = gnoblin_client::Client::connect(transport, gnoblin_types::Version::new(major, minor, patch))
            .expect("the handshake")
            .into_inner()
            .into_stream()
            .expect("the handshaken socket");
        let link = SocketTreeLink::new(stream).expect("the tree link");
        Self::Socket(Some(Driven::new(Box::new(link))))
    }

    /// A shell for the compositor to run in process: the spec to put in the session's configuration, and the handle that
    /// drives it. `reports` is where the shell reports.
    pub fn in_process(reports: Recorder) -> (InProcShellSpec, Self) {
        let (jobs, received) = channel::<Job>();
        let spec = InProcShellSpec::new("test", move |link, context| {
            let mut driven = Driven::new(link);
            while !driven.quit && !context.stop.requested() {
                match received.recv_timeout(Duration::from_millis(50)) {
                    Ok(job) => job(&mut driven),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
            Ok(())
        })
        .with_reports(Arc::new(reports));
        (spec, Self::InProc { jobs })
    }

    /// Runs `step` on the shell, wherever the shell runs, and returns what it returns.
    pub fn run<R: Send + 'static>(&mut self, step: impl FnOnce(&mut Driven) -> R + Send + 'static) -> R {
        match self {
            Self::Socket(driven) => step(driven.as_mut().expect("the shell is still connected")),
            Self::InProc { jobs } => {
                let (answer, answered): (Sender<R>, Receiver<R>) = channel();
                jobs.send(Box::new(move |driven| {
                    let _ = answer.send(step(driven));
                }))
                .expect("the shell is running");
                answered
                    .recv_timeout(Duration::from_secs(30))
                    .expect("the shell answered the step")
            }
        }
    }

    /// Ends the shell the way each transport ends: a socket shell closes its connection, an in-process shell returns.
    pub fn finish(mut self) {
        match &mut self {
            Self::Socket(driven) => drop(driven.take()),
            Self::InProc { .. } => self.run(|driven| driven.quit = true),
        }
    }
}
