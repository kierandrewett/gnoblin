//! The JavaScript shell's side of the transaction gate: one transaction in flight, a refused transaction's writes sent
//! again.
//!
//! The protocol is the tree client's (`gnoblin_tree_client::gate`, shared with every other shell runtime): which refusals a newer
//! base cures, how many times a write goes again, what cannot go again. This module hands the gate the shell's mirror and
//! builder and says what its effects mean to script: an answer to a call that was given up goes to whoever waits for it, and a
//! write that was given up is handed back to the layout.

use gnoblin_tree_client::gate::GateEffects;
use gnoblin_tree_client::tree_mirror::TxReply;

use super::{JsShell, JsShellError};
use crate::bindings::HostState;

/// Hands an answer to the module that asked: a toolkit measurement, an output `apply` or a lock password check.
pub(super) fn route_answer(state: &mut HostState, token: u64, result: &[u8], error: &str) {
    if !state.ui.accept_answer(token, result, error) && !state.outputs.accept_answer(token, error) {
        state.lock.accept_answer(token, result, error);
    }
}

impl JsShell {
    /// Matches a reply to its transaction and, for a stale refusal, keeps the ops to send again.
    pub(super) fn handle_reply(&mut self, reply: &TxReply, diagnostics: &mut Vec<String>) {
        let mut effects = GateEffects::default();
        {
            let mut guard = self.state.borrow_mut();
            let HostState { mirror, tick, .. } = &mut *guard;
            self.tx.handle_reply(mirror, tick, reply, &mut effects);
        }
        self.apply_effects(effects, diagnostics);
    }

    /// Tells whoever waits for a call that was given up, hands the layout the writes that were lost, and collects the
    /// gate's diagnostics.
    fn apply_effects(&self, effects: GateEffects, diagnostics: &mut Vec<String>) {
        let mut state = self.state.borrow_mut();
        for (token, error) in effects.failed_calls {
            route_answer(&mut state, token, b"", &error);
        }
        state.lost_writes.extend(effects.lost_writes);
        diagnostics.extend(effects.diagnostics);
    }

    /// Sends this frame's writes as one transaction, once the one in flight has landed.
    pub(super) fn send_transaction(&mut self) -> Result<(), JsShellError> {
        let mut effects = GateEffects::default();
        let tx = {
            let mut guard = self.state.borrow_mut();
            let HostState { mirror, tick, .. } = &mut *guard;
            self.tx.next_transaction(mirror, tick, &mut effects)
        };
        self.apply_effects(effects, &mut Vec::new());
        if let Some(tx) = tx {
            self.gate.sent = self.gate.sent.saturating_add(1);
            self.link.transact(&tx).map_err(|e| JsShellError::Link(e.0))?;
        }
        Ok(())
    }
}
