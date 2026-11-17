//! The one holder of the compositor's link, and the two things that read it.
//!
//! A shell has one connection to the compositor (the compositor admits one holder), and the core holds it: the [`Hub`] owns the
//! link, and the core's own tree session reads and writes through [`HubLink`]. A language runtime that keeps a mirror of its own (the
//! JavaScript engine) is a second reader of the same link, a *view*: its attach is answered from the core's mirror (a snapshot of
//! it, then every message that arrives afterwards), what it sends goes out through the one link, and the replies are handed to
//! whoever sent the transaction. The core's mirror is therefore the one source of truth for the tree, the library of `gnoblin-std`
//! (which works on that mirror through the node functions) and a runtime's engine see the same tree, and the compositor sees one
//! holder.
//!
//! Rules:
//!
//! - **One transaction on the wire.** The core's session and the view each keep a gate that sends one transaction at a time. Between
//!   the two the hub does the same: a transaction waits while another is unanswered, so a base is never made stale by the other side's
//!   transaction in flight. A refusal still reaches the side that sent it, whose gate sends it again as it always did.
//! - **Replies by position.** The compositor answers in order, so the hub keeps who sent each transaction and gives the reply to
//!   that side only. Every other message (change batches, answers, events, chords) goes to both.
//! - **Disjoint ids.** Both sides make nodes, call tokens and claims; the view counts from one, and the core's session is moved to
//!   ranges of its own when a view attaches ([`crate::session::Session::open_relayed`]).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::rc::Rc;

use gnoblin_tree_client::tree_mirror::{Attached, LinkError, TreeIncoming, TreeLink, TreeMirror, TreeTx};

/// Which side of the hub a transaction came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Core,
    View,
}

/// The link and what is queued for each reader. See the module docs.
pub(crate) struct Hub {
    link: Box<dyn TreeLink>,
    /// Messages for the core's session, in arrival order.
    core: VecDeque<TreeIncoming>,
    /// Messages for the view, once it has attached.
    view: Option<VecDeque<TreeIncoming>>,
    /// Who sent each transaction that has not been answered, oldest first.
    flying: VecDeque<Side>,
    /// Transactions that wait for the one on the wire to be answered.
    parked: VecDeque<(Side, TreeTx)>,
}

/// The hub, shared by the session and the two links.
pub(crate) type SharedHub = Rc<RefCell<Hub>>;

impl Hub {
    /// A hub over `link`.
    pub(crate) fn new(link: Box<dyn TreeLink>) -> SharedHub {
        Rc::new(RefCell::new(Self {
            link,
            core: VecDeque::new(),
            view: None,
            flying: VecDeque::new(),
            parked: VecDeque::new(),
        }))
    }

    /// Whether a view is attached.
    pub(crate) fn has_view(&self) -> bool {
        self.view.is_some()
    }

    /// Reads the compositor's link and hands each message to whom it is for.
    fn pump(&mut self) -> Result<(), LinkError> {
        for message in self.link.poll()? {
            match message {
                TreeIncoming::Reply(reply) => {
                    let side = self.flying.pop_front().unwrap_or(Side::Core);
                    // A transaction that changed no node (one that only called) advances the serial and publishes no batch, so the
                    // reader that did not send it learns the serial from an empty batch at it: a mirror that sees the next batch with
                    // that serial missing would take it for a missed one.
                    let advance = reply.applied.then(|| TreeIncoming::Changes {
                        serial: reply.serial,
                        changes: Vec::new(),
                    });
                    match (side, self.view.as_mut()) {
                        (Side::View, Some(view)) => {
                            view.push_back(TreeIncoming::Reply(reply));
                            self.core.extend(advance);
                        }
                        (_, view) => {
                            self.core.push_back(TreeIncoming::Reply(reply));
                            if let Some(view) = view {
                                view.extend(advance);
                            }
                        }
                    }
                    if let Some((side, tx)) = self.parked.pop_front() {
                        self.link.transact(&tx)?;
                        self.flying.push_back(side);
                    }
                }
                other => {
                    if let Some(view) = self.view.as_mut() {
                        view.push_back(other.clone());
                    }
                    self.core.push_back(other);
                }
            }
        }
        Ok(())
    }

    fn send(&mut self, side: Side, tx: &TreeTx) -> Result<(), LinkError> {
        if self.flying.is_empty() {
            self.link.transact(tx)?;
            self.flying.push_back(side);
        } else {
            self.parked.push_back((side, tx.clone()));
        }
        Ok(())
    }

    /// The view's attach: the core's mirror as a snapshot, then the messages the core has not applied yet and every one that
    /// comes after. A second attach (a reload's generation never makes one, the link outlives it) starts the view again.
    pub(crate) fn attach_view(&mut self, mirror: &TreeMirror) {
        self.pump().ok();
        let mut queue = VecDeque::new();
        queue.push_back(TreeIncoming::Attached(Attached {
            epoch: mirror.epoch(),
            root: mirror.root(),
            serial: mirror.serial(),
        }));
        queue.push_back(TreeIncoming::Snapshot {
            serial: mirror.serial(),
            chunk: 0,
            last: true,
            nodes: mirror.to_snapshot(),
        });
        // What the core has not applied to its mirror yet is not in the snapshot: the view reads it as the core will.
        queue.extend(
            self.core
                .iter()
                .filter(|message| !matches!(message, TreeIncoming::Reply(_)))
                .cloned(),
        );
        self.view = Some(queue);
    }

    /// Everything that has arrived for the view.
    pub(crate) fn take_view(&mut self) -> Result<Vec<TreeIncoming>, LinkError> {
        self.pump()?;
        Ok(self
            .view
            .as_mut()
            .map(|queue| queue.drain(..).collect())
            .unwrap_or_default())
    }

    /// Sends the view's transaction.
    pub(crate) fn send_view(&mut self, tx: &TreeTx) -> Result<(), LinkError> {
        self.send(Side::View, tx)
    }

    /// The compositor's wake descriptor, for the view to sleep on.
    pub(crate) fn wait_fd(&self) -> Option<BorrowedFd<'_>> {
        self.link.wait_fd()
    }
}

/// The core session's [`TreeLink`]: the hub, as the core's side of it.
pub(crate) struct HubLink {
    hub: SharedHub,
    /// A duplicate of the link's wake descriptor, for a loop that sleeps on the core's side.
    fd: Option<OwnedFd>,
}

impl HubLink {
    /// The core's side of `hub`.
    pub(crate) fn new(hub: SharedHub) -> Self {
        let fd = hub.borrow().wait_fd().and_then(|fd| fd.try_clone_to_owned().ok());
        Self { hub, fd }
    }
}

impl TreeLink for HubLink {
    fn attach(&mut self, version: u64) -> Result<(), LinkError> {
        self.hub.borrow_mut().link.attach(version)
    }

    fn transact(&mut self, tx: &TreeTx) -> Result<(), LinkError> {
        self.hub.borrow_mut().send(Side::Core, tx)
    }

    fn poll(&mut self) -> Result<Vec<TreeIncoming>, LinkError> {
        let mut hub = self.hub.borrow_mut();
        hub.pump()?;
        Ok(hub.core.drain(..).collect())
    }

    fn wait_fd(&self) -> Option<BorrowedFd<'_>> {
        self.fd.as_ref().map(AsFd::as_fd)
    }
}
