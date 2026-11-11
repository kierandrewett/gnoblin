//! A stand-in for the shell a session serves: it holds the tree through the hooks the session was started with, reads
//! the batches the compositor publishes, and writes the answers a real shell writes to window nodes.
//!
//! It is what makes a toplevel request visible: with a shell holding the tree the window manager does not act on a
//! client's request, the request becomes an event on the window's node, and the shell's writes become the client's
//! configure.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_render::transaction::{
    ClassInfo, Code, PropertyInfo, TreeChange, TreeOp, TreeTransaction, TreeValue, ValueType,
};
use gnoblin_secret::SecretText;
use gnoblin_types::NodeId;
use gnoblin_wayland::{TreeBatch, TreeFrameInput, TreeHooks, TreeQueue};

use super::TIMEOUT;

/// The schema the compositor serves for a window node, as far as a request answer needs it: the three state flags and
/// the size are the shell's to write, and `close` is a method.
struct WindowTable;

impl ClassInfo for WindowTable {
    fn has_class(&self, class: &str) -> bool {
        matches!(class, "node" | "window")
    }

    fn property(&self, class: &str, key: &str) -> Option<PropertyInfo> {
        let kind = match (class, key) {
            ("window", "maximised" | "fullscreen" | "minimised") | ("node" | "window", "visible") => ValueType::Bool,
            ("node" | "window", "x" | "y" | "width" | "height") => ValueType::Double,
            _ => return None,
        };
        Some(PropertyInfo {
            kind,
            readonly: false,
            animated: false,
        })
    }

    fn has_method(&self, class: &str, method: &str) -> bool {
        class == "window" && method == "close"
    }
}

/// The shell side of a session: its queue and everything the compositor published to it.
#[derive(Clone)]
pub struct Shell {
    queue: TreeQueue,
    batches: Arc<Mutex<Vec<TreeBatch>>>,
    serial: Arc<AtomicU64>,
}

impl Shell {
    /// A shell and the hooks that connect it to a session.
    pub fn new() -> (Self, TreeHooks) {
        let queue = TreeQueue::default();
        let batches: Arc<Mutex<Vec<TreeBatch>>> = Arc::default();
        let serial = Arc::new(AtomicU64::new(0));
        let hooks = TreeHooks {
            queue: queue.clone(),
            classes: Arc::new(WindowTable),
            publish: {
                let batches = Arc::clone(&batches);
                let serial = Arc::clone(&serial);
                Arc::new(move |batch| {
                    serial.store(batch.serial, Ordering::SeqCst);
                    batches.lock().expect("the batch lock is not poisoned").push(batch);
                })
            },
            ..TreeHooks::default()
        };
        (Self { queue, batches, serial }, hooks)
    }

    /// Takes the tree, as a connected shell does: from now on window requests are the shell's to decide.
    pub fn attach(&self) {
        self.queue.read(|scene, _| scene.set_shell_places(true));
    }

    /// Lets go of the tree: window requests go back to the window manager.
    pub fn detach(&self) {
        self.queue.read(|scene, _| scene.set_shell_places(false));
    }

    /// The first window node the compositor announced, once it has.
    pub fn window_node(&self) -> Option<NodeId> {
        self.batches
            .lock()
            .expect("the batch lock is not poisoned")
            .iter()
            .flat_map(|batch| &batch.changes)
            .find_map(|change| match change {
                TreeChange::Created { node, class, .. } if class == "window" => Some(*node),
                _ => None,
            })
    }

    /// Waits for the first window node.
    #[track_caller]
    pub fn wait_window_node(&self) -> NodeId {
        wait("a window node", || self.window_node())
    }

    /// The names of the requests the compositor delivered for `node`, in the order they arrived.
    pub fn requests(&self, node: NodeId) -> Vec<&'static str> {
        self.batches
            .lock()
            .expect("the batch lock is not poisoned")
            .iter()
            .flat_map(|batch| &batch.events)
            .filter(|event| event.node == node)
            .filter_map(|event| match &event.event {
                TreeFrameInput::Request(request) => Some(request.name()),
                _ => None,
            })
            .collect()
    }

    /// Waits until `node` has been sent `count` requests named `name`.
    #[track_caller]
    pub fn wait_request(&self, node: NodeId, name: &str, count: usize) {
        wait(&format!("{count} `{name}` request(s)"), || {
            (self.requests(node).iter().filter(|seen| **seen == name).count() >= count).then_some(())
        });
    }

    /// Writes `values` to `node` in one transaction, as the shell answers a request, and waits for it to apply. A
    /// transaction refused as stale because the compositor published meanwhile is built again against the new serial.
    #[track_caller]
    pub fn write(&self, node: NodeId, values: &[(&str, TreeValue)]) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let ops = values
                .iter()
                .map(|(key, value)| TreeOp::Set {
                    node,
                    key: (*key).to_owned(),
                    value: value.clone(),
                    timing: None,
                })
                .collect();
            match self.transact(TreeTransaction::new(ops)) {
                Ok(()) => return,
                Err(code) => assert!(
                    matches!(code, Code::StaleBase) && Instant::now() < deadline,
                    "the shell's answer was refused: {code:?}"
                ),
            }
        }
    }

    /// Calls `method` on `node`, as the shell's `window:close()` does.
    #[track_caller]
    pub fn call(&self, node: NodeId, method: &str) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let call = TreeTransaction::new(vec![TreeOp::TreeCall {
                node,
                method: method.to_owned(),
                args: Vec::new(),
                text: SecretText::new(""),
                token: 0,
            }]);
            match self.transact(call) {
                Ok(()) => return,
                Err(code) => assert!(
                    matches!(code, Code::StaleBase) && Instant::now() < deadline,
                    "the shell's call was refused: {code:?}"
                ),
            }
        }
    }

    /// Submits one transaction against the last published serial and waits for its answer.
    fn transact(&self, transaction: TreeTransaction) -> Result<(), Code> {
        let answer: Arc<Mutex<Option<Result<(), Code>>>> = Arc::default();
        let reply = Arc::clone(&answer);
        self.queue.submit(
            self.serial.load(Ordering::SeqCst),
            transaction,
            || true,
            move |_, result| {
                *reply.lock().expect("the answer lock is not poisoned") =
                    Some(result.map(|_| ()).map_err(|refusal| refusal.code));
            },
        );
        wait("the transaction's answer", || {
            answer.lock().expect("the answer lock is not poisoned").take()
        })
    }
}

/// Waits for `check` to produce a value, or fails after the session timeout.
#[track_caller]
fn wait<T>(what: &str, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Some(value) = check() {
            return value;
        }
        assert!(Instant::now() < deadline, "never saw {what}");
        thread::sleep(Duration::from_millis(5));
    }
}
