//! The compositor's link, as the core relays it.
//!
//! The engine drives a [`TreeLink`]: attach, transactions out, messages in, and a descriptor to sleep on. Here the link is the one
//! the core holds for this shell (the in-process queue of the compositor, or the fake of a test), reached through the
//! table's `tree_link_*` functions with the byte form of `gnoblin_tree_client::relay`. The core keeps no mirror for it: the engine
//! keeps its own, as it always has.
//!
//! The link outlives a generation. Each call is made with the context of the generation that serves, which the runtime moves to
//! the new one when a reload commits ([`Serving`]); a context that has retired is refused.

use std::cell::Cell;
use std::os::fd::{BorrowedFd, OwnedFd};
use std::rc::Rc;

use gnoblin_core_abi::GnoblinCtx;
use gnoblin_tree_client::relay;
use gnoblin_tree_client::tree_mirror::{LinkError, TreeIncoming, TreeLink, TreeTx};

use crate::table::CoreTable;

/// The context of the generation that serves, shared between the runtime and the link it made.
pub(crate) type Serving = Rc<Cell<GnoblinCtx>>;

/// The engine's [`TreeLink`] over the core's relayed link.
pub(crate) struct RelayedLink {
    table: CoreTable,
    serving: Serving,
    fd: Option<OwnedFd>,
}

impl RelayedLink {
    /// Opens the link as the shell. `serving` names the context the calls are made with.
    pub(crate) fn open(table: CoreTable, serving: Serving) -> Result<Self, String> {
        table.link_open(serving.get())?;
        let fd = table.link_fd(serving.get());
        Ok(Self { table, serving, fd })
    }
}

impl TreeLink for RelayedLink {
    fn attach(&mut self, version: u64) -> Result<(), LinkError> {
        self.table.link_attach(self.serving.get(), version).map_err(LinkError)
    }

    fn transact(&mut self, tx: &TreeTx) -> Result<(), LinkError> {
        self.table
            .link_send(self.serving.get(), &relay::encode_tx(tx))
            .map_err(LinkError)
    }

    fn poll(&mut self) -> Result<Vec<TreeIncoming>, LinkError> {
        let bytes = self.table.link_recv(self.serving.get()).map_err(LinkError)?;
        relay::decode_incoming(&bytes).map_err(|error| LinkError(error.to_string()))
    }

    fn wait_fd(&self) -> Option<BorrowedFd<'_>> {
        use std::os::fd::AsFd as _;
        self.fd.as_ref().map(OwnedFd::as_fd)
    }
}
