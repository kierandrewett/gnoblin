//! The claims a context holds on its seat and the registrations that stand on them. Bindings of one trigger and one device share a claim;
//! the last registration to go gives it up. The candidates for an input come back in the order of precedence: the tier, then the priority,
//! then the older registration.

use std::sync::Arc;

use gnoblin_core_abi as abi;
use gnoblin_object::server::Cx;
use gnoblin_object::{Closure, Fail, Obj, Res};
use gnoblin_shell_host::generation::CallbackOutcome;

use super::trigger::{self, SeatClaim};
use super::{BindingRegistration, BindingTier, ClaimEntry, ContextRec, InputTable, Life, matchers};
use crate::events::ClaimNotice;
use crate::listen::{ClaimHandler, ClaimSpec, claim_with};

impl InputTable {
    /// The context `obj`, which the caller knows is in the table.
    pub(super) fn context(&mut self, obj: Obj) -> Res<&mut ContextRec> {
        self.contexts
            .get_mut(&obj)
            .ok_or_else(|| Fail::stale("the input context is gone"))
    }

    /// Which of two registrations of one trigger runs first: the higher tier, then the higher priority, then the older.
    pub(super) fn precedence(&self, a: &BindingRegistration, b: &BindingRegistration) -> std::cmp::Ordering {
        let priority = |registration: &BindingRegistration| {
            self.bindings
                .get(&registration.binding)
                .map_or(0.0, |binding| binding.priority)
        };
        (b.tier as u8)
            .cmp(&(a.tier as u8))
            .then_with(|| priority(b).total_cmp(&priority(a)))
            .then_with(|| a.order.cmp(&b.order))
    }

    /// Registers `binding` in `context`, standing on the claim of its trigger, which is made when no registration holds it.
    ///
    /// # Errors
    ///
    /// The compositor has no claim for the trigger (a wheel), or the claim could not be made.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn register(
        &mut self,
        cx: &Cx<'_>,
        context: Obj,
        binding: Obj,
        tier: BindingTier,
        owns: Option<Closure>,
        layout: Option<Obj>,
        device: &str,
    ) -> Res<u64> {
        let (id, parsed) = {
            let held = self
                .bindings
                .get(&binding)
                .ok_or_else(|| Fail::stale("the binding is gone"))?;
            (held.id.clone(), held.parsed.clone())
        };
        let Some(spec) = trigger::claim_of(&parsed) else {
            return Err(Fail::unsupported(format!(
                "Binding \"{id}\": the compositor has no claim for {}; wheel triggers cannot be bound yet",
                parsed.text
            )));
        };
        let spec_key = spec.key();
        let pointer = parsed.kind == trigger::TriggerClass::Pointer;
        let claim = self.acquire(cx, context, &spec, &spec_key, device, pointer)?;
        self.order += 1;
        let order = self.order;
        let held = self.context(context)?;
        held.next_registration += 1;
        let registration = BindingRegistration {
            id: held.next_registration,
            binding,
            tier,
            order,
            owns,
            layout,
            device: device.to_owned(),
            spec_key,
            claim,
        };
        let registered = registration.id;
        held.registrations.push(registration);
        Ok(registered)
    }

    /// Takes a registration out. An input its binding runs ends first, and the claim goes when no other registration stands on it.
    pub(super) fn unregister(&mut self, cx: &Cx<'_>, context: Obj, registration: u64) {
        let Some(held) = self.contexts.get_mut(&context) else {
            return;
        };
        let Some(at) = held.registrations.iter().position(|item| item.id == registration) else {
            return;
        };
        let gone = held.registrations.remove(at);
        matchers(cx.ctx(), |matcher| matcher.retire(cx, context, &gone));
        self.unstand(context, &gone.claim);
    }

    /// One registration less stands on the claim `key`; the last one to leave gives it up.
    fn unstand(&mut self, context: Obj, key: &str) {
        let Some(held) = self.contexts.get_mut(&context) else {
            return;
        };
        let Some(at) = held
            .claims
            .iter()
            .position(|claim| ClaimEntry::key_of(&claim.spec_key, &claim.device) == key)
        else {
            return;
        };
        held.claims[at].refs = held.claims[at].refs.saturating_sub(1);
        if held.claims[at].refs == 0 {
            let claim = held.claims.remove(at);
            self.routes.remove(&claim.id);
            crate::registry::remove_id(claim.reg);
        }
    }

    /// The claim of `spec` for `device`: the one a registration holds already, or a new one. Answers its key.
    fn acquire(
        &mut self,
        cx: &Cx<'_>,
        context: Obj,
        spec: &SeatClaim,
        spec_key: &str,
        device: &str,
        pointer: bool,
    ) -> Res<String> {
        let key = ClaimEntry::key_of(spec_key, device);
        let manager = self.manager;
        let held = self.context(context)?;
        if let Some(entry) = held
            .claims
            .iter_mut()
            .find(|claim| ClaimEntry::key_of(&claim.spec_key, &claim.device) == key)
        {
            entry.refs += 1;
            return Ok(key);
        }
        if held.life != Life::Live {
            return Err(Fail::new(
                abi::E_STALE,
                format!(
                    "InputContext \"{}\": add the context to the input manager first",
                    held.name
                ),
            ));
        }
        let seat = held.seat;
        let name = held.name.clone();
        let (claim_spec, subject) = match spec {
            SeatClaim::Chord { text, grab } => (
                (abi::CLAIM_CHORD, if *grab { abi::CLAIM_FLAG_GRAB } else { 0 }, 0),
                text.clone(),
            ),
            SeatClaim::Gesture { gesture, fingers } => {
                ((abi::CLAIM_GESTURE, 0, i64::from(*fingers)), (*gesture).to_owned())
            }
        };
        let (id, reg) = claim_for(cx, manager, seat, claim_spec, &subject, device).map_err(|(code, text)| {
            Fail::new(
                code,
                format!("InputContext \"{name}\": the compositor refused the claim for {spec_key} ({text})"),
            )
        })?;
        self.routes.insert(id, context);
        self.context(context)?.claims.push(ClaimEntry {
            id,
            reg,
            spec_key: spec_key.to_owned(),
            device: device.to_owned(),
            spec: spec.clone(),
            pointer,
            refs: 1,
        });
        Ok(key)
    }

    /// The registrations of `context` that can take an input on the claim `spec_key` of `device`, best first. A claim for one device
    /// shadows the claim for every device, so what the context bound for every device still answers for that device's events: it is a
    /// later tier of the same list.
    pub(super) fn candidates(&self, context: Obj, spec_key: &str, device: &str) -> Vec<BindingRegistration> {
        let Some(held) = self.contexts.get(&context) else {
            return Vec::new();
        };
        let mut list: Vec<BindingRegistration> = held
            .registrations
            .iter()
            .filter(|registration| {
                registration.spec_key == spec_key
                    && (registration.device == device || registration.device.is_empty())
                    && self
                        .bindings
                        .get(&registration.binding)
                        .is_some_and(|binding| binding.enabled)
            })
            .cloned()
            .collect();
        list.sort_by(|a, b| self.precedence(a, b));
        list
    }

    /// Gives up every claim of the context. The registrations go with them.
    pub(super) fn drop_claims(&mut self, context: Obj) {
        let Some(held) = self.contexts.get_mut(&context) else {
            return;
        };
        held.registrations.clear();
        for claim in std::mem::take(&mut held.claims) {
            self.routes.remove(&claim.id);
            crate::registry::remove_id(claim.reg);
        }
    }
}

/// Claims `spec` on `seat` for the generation of `cx` and routes the events of the claim to the input module.
fn claim_for(
    cx: &Cx<'_>,
    manager: Obj,
    seat: u64,
    kind: (u32, u32, i64),
    subject: &str,
    device: &str,
) -> Result<(i64, u64), (i32, String)> {
    let ctx = cx.ctx();
    let handler: ClaimHandler = Arc::new(move |notice: &ClaimNotice| {
        let notice = notice.clone();
        let _ = crate::object::run_in(ctx, manager, move |cx| super::claim_event(cx, &notice));
        CallbackOutcome::Handled
    });
    claim_with(
        ctx,
        seat,
        &ClaimSpec {
            kind: kind.0,
            flags: kind.1,
            fingers: kind.2,
            subject,
            device,
            node: 0,
        },
        Some(handler),
        std::ptr::null_mut(),
        None,
    )
}

/// Claims `device` for the context: its keys reach no client and fire only the bindings made for it or for every device. Answers the
/// claim's id and the registration that owns it.
pub(super) fn claim_device(cx: &Cx<'_>, seat: u64, device: &str) -> Result<(i64, u64), (i32, String)> {
    claim_with(
        cx.ctx(),
        seat,
        &ClaimSpec {
            kind: abi::CLAIM_DEVICE,
            flags: 0,
            fingers: 0,
            subject: "",
            device,
            node: 0,
        },
        None,
        std::ptr::null_mut(),
        None,
    )
}
