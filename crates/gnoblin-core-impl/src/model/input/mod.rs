//! `input`: the input matcher of the core. A shell binds a trigger on an input context; the library claims the chord, the pointer button or the
//! gesture on the seat, shares one claim between the bindings of one trigger, and runs the right binding when the input happens.
//!
//! `data/js/gnoblin-core/{bindings,input}/*` is the behaviour it ports:
//!
//! - [`trigger`]: the grammar, one implementation for every language, and the seat claim a trigger needs;
//! - [`binding`]: the `binding` and `binding_collection` classes, and the rules a collection keeps (ids, conflicts, who follows it);
//! - [`claims`]: the claims a context holds on its seat and the registrations that stand on them, in the order of precedence;
//! - [`context`]: the `input_context` class, one for each seat and one for each context a shell makes;
//! - [`devices`] and [`settings`]: the `device` classes and the roster a seat context keeps of the devices the seat reports, and the ledger
//!   of the settings a shell wrote to them;
//! - [`manager`]: the `input_manager` service, the contexts it holds and how a claim's events find their context;
//! - [`keys`], [`pointer`] and [`gesture`]: the matchers of a key chord, of a pointer button (a click or a drag) and of a touchpad gesture,
//!   and [`hooks`], which they share. [`Matcher`] is the seam they plug into.
//!
//! One [`InputTable`] holds all the state of a generation. A reload hands a chord from one generation to the next without this module doing
//! anything of its own: a generation that is only staged keeps its claims in the session and the session sends them when it takes over,
//! behind the releases of the old generation's, in one transaction. What the old generation was in the middle of is the matchers' to end: a
//! drag or a gesture that is under way ends when a window closes, a binding goes, a context ends ([`Matcher::cancel_all`]), and when the
//! generation does ([`end_inflight`], from the shutdown hook of the object library).
//!
//! Nothing here calls the shell while the table or the matchers are borrowed: a binding's function, a hook and a scope run from a turn
//! ([`gnoblin_object::server::Cx::later`]) on what the table said, and the table is looked at again after each call. The one call that is
//! not from a turn is the `cancel` of a generation that ends, which no turn would follow.

use std::collections::HashMap;

use gnoblin_core_abi::GnoblinCtx;
use gnoblin_object::server::{Builder, Cx};
use gnoblin_object::{Closure, Obj, Res, Val};

use super::glue::PerGeneration;
use crate::events::ClaimNotice;

mod binding;
mod claims;
mod context;
mod devices;
mod gesture;
mod hooks;
mod keys;
mod manager;
mod pointer;
mod seat;
mod settings;
mod trigger;

use trigger::{ParsedTrigger, SeatClaim};

/// Where a binding comes from. A device's binding runs before a layout's, and a layout's before the context's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BindingTier {
    ContextRec = 0,
    Layout = 1,
    Device = 2,
}

impl BindingTier {
    fn name(self) -> &'static str {
        match self {
            BindingTier::ContextRec => "context",
            BindingTier::Layout => "layout",
            BindingTier::Device => "device",
        }
    }
}

/// What a binding does when its chord is pressed.
#[derive(Clone)]
pub(super) enum BindingAction {
    /// The trigger is claimed and swallowed: the press reaches no client and nothing runs.
    Swallow,
    /// A function the shell gave. It answers `false` to decline.
    Closure(Closure),
    /// An object the shell implements (`key_binding`), held for as long as the binding is.
    Handler(Obj),
    /// An object the shell implements (`pointer_drag_binding`), held for as long as the binding is.
    Drag(Obj),
    /// An object the shell implements (`gesture_binding`), held for as long as the binding is.
    Gesture(Obj),
}

/// One binding of a collection.
pub(super) struct BindingRec {
    pub id: String,
    /// The trigger as it was written.
    pub written: String,
    pub parsed: ParsedTrigger,
    pub priority: f64,
    pub enabled: bool,
    /// Which windows the binding applies to, when it has a say.
    pub scope: Option<Closure>,
    /// How far the pointer must go from the press before a drag starts. Only a drag uses it.
    pub threshold: f64,
    pub action: BindingAction,
    /// The collection that holds it, none once it was unbound.
    pub collection: Option<Obj>,
}

/// A context following a collection: every binding of the collection is registered in the context, and so is each one added later.
pub(super) struct ContextFollow {
    pub id: u64,
    pub context: Obj,
    pub tier: BindingTier,
    /// Tells whether a window is one the bindings apply to. A layout's.
    pub owns: Option<Closure>,
    /// The device the bindings are for, or empty for every device.
    pub device: String,
    /// The registrations made, by binding.
    pub held: Vec<(Obj, u64)>,
}

/// The bindings of one holder, by id, in the order they were added.
pub(super) struct BindingSet {
    pub tier: BindingTier,
    /// The name of the context or the device that owns it, empty for a collection a shell made.
    pub owner: String,
    pub bindings: Vec<Obj>,
    pub follows: Vec<ContextFollow>,
}

/// One binding registered in a context, standing on a claim.
#[derive(Clone)]
pub(super) struct BindingRegistration {
    pub id: u64,
    pub binding: Obj,
    pub tier: BindingTier,
    /// Registration order, which breaks a tie between two bindings of one tier and priority.
    pub order: u64,
    /// The scope of the layout that supplied it.
    pub owns: Option<Closure>,
    /// The collection of the layout that supplied it.
    pub layout: Option<Obj>,
    /// The device it is for, or empty for every device.
    pub device: String,
    /// Names the claim's chord or gesture: registrations with one key are alternatives for one input.
    pub spec_key: String,
    /// The key of the claim it stands on.
    pub claim: String,
}

/// One claim a context holds on its seat, and how many registrations stand on it.
#[derive(Clone)]
pub(super) struct ClaimEntry {
    /// The compositor's id for it, as the core allocated it.
    pub id: i64,
    /// The registration of the library that owns the claim: removing it gives the claim up.
    pub reg: u64,
    pub spec_key: String,
    /// The device it is for, or empty for every device.
    pub device: String,
    pub spec: SeatClaim,
    /// Whether it is for a pointer button.
    pub pointer: bool,
    pub refs: usize,
}

impl ClaimEntry {
    /// The key a claim is looked up by: one claim for one trigger and one device.
    pub(super) fn key_of(spec_key: &str, device: &str) -> String {
        format!("{spec_key}\n{device}")
    }
}

/// Where a context is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Life {
    /// Made, and in no list: it collects bindings and holds no device and makes no claim.
    Detached,
    Live,
    Removed,
}

/// An input context.
pub(super) struct ContextRec {
    pub name: String,
    /// The seat whose claims it makes, or 0 while it is in no list.
    pub seat: u64,
    /// Made by a shell: it holds devices of a seat, not a seat.
    pub scripted: bool,
    pub life: Life,
    /// The collection of the bindings added to the context itself.
    pub own: Obj,
    pub claims: Vec<ClaimEntry>,
    pub registrations: Vec<BindingRegistration>,
    pub next_registration: u64,
    /// The collections it follows, and the follow of each.
    pub follows: Vec<(Obj, u64)>,
    /// The layouts attached by hand, by the collection.
    pub layouts: Vec<Obj>,
    /// Every device the seat reports, whichever context holds it. Only a seat context has any.
    pub roster: Vec<(String, Obj)>,
    /// The devices this context holds, each with the follows its bindings stand on.
    pub owned: Vec<(Obj, Vec<u64>)>,
    /// The claim of each binding-only device, by its name: the claim's id and the registration that owns it.
    pub only: Vec<(String, i64, u64)>,
    pub disposed: bool,
}

/// An input device of a seat.
pub(super) struct InputDevice {
    pub id: String,
    pub name: String,
    pub capabilities: Vec<String>,
    /// The class its capabilities make it.
    pub kind: devices::InputDeviceKind,
    pub alive: bool,
    /// The context that holds it.
    pub context: Obj,
    /// Its own bindings.
    pub bindings: Obj,
}

/// What a matcher is told of: the claims' events and the end of what its bindings run. The key matcher keeps nothing in flight; the
/// matchers of a pointer drag and of a touchpad gesture do, and an in-flight input ends here, whatever ends it: the compositor, a window
/// that closes, a binding that goes, a context that ends, or the generation (which ends them at once, from [`end_inflight`]).
pub(super) trait Matcher {
    /// A claim of `context` reported `notice`. A matcher answers the kinds of event it owns and ignores the rest.
    fn claim_event(&mut self, cx: &Cx<'_>, context: Obj, claim: &ClaimEntry, notice: &ClaimNotice);

    /// A registration is going: an input its binding runs ends first, so nothing is left running for a binding that is gone.
    fn retire(&mut self, cx: &Cx<'_>, context: Obj, registration: &BindingRegistration);

    /// A window is gone: what runs on it ends.
    fn cancel_on(&mut self, cx: &Cx<'_>, client: Obj);

    /// The context ends: everything in flight in it ends.
    fn cancel_all(&mut self, cx: &Cx<'_>, context: Obj);
}

/// The matchers of a generation. A matcher added to the input module is a field here and a line in [`Matchers::each`]. They are kept
/// apart from the [`InputTable`] so that a matcher is told of an event while the table is free: what it decides it reads from the table
/// again, from the turn it queued.
#[derive(Default)]
struct Matchers {
    keys: keys::KeyMatcher,
    pointer: pointer::PointerMatcher,
    gesture: gesture::GestureMatcher,
}

impl Matchers {
    fn each(&mut self, mut work: impl FnMut(&mut dyn Matcher)) {
        work(&mut self.keys);
        work(&mut self.pointer);
        work(&mut self.gesture);
    }
}

/// Everything the input module keeps for a generation.
#[derive(Default)]
pub(super) struct InputTable {
    /// The `input_manager` service, or 0 before it was made.
    pub manager: Obj,
    /// Numbers the bindings a bare function made, so each has an id of its own.
    pub serial: u64,
    /// Numbers the registrations of every context, so an older one runs first.
    pub order: u64,
    pub follow_serial: u64,
    /// The contexts of the seats, by the seat's node, in the order the seats appeared.
    pub seats: Vec<(u64, Obj)>,
    /// The contexts a shell added, in the order it added them.
    pub scripted: Vec<Obj>,
    pub contexts: HashMap<Obj, ContextRec>,
    pub collections: HashMap<Obj, BindingSet>,
    pub bindings: HashMap<Obj, BindingRec>,
    pub devices: HashMap<Obj, InputDevice>,
    /// The context that holds each claim, by the claim's id.
    pub routes: HashMap<i64, Obj>,
    /// What a shell wrote to the settings of the devices.
    pub settings: settings::SettingsLedger,
}

thread_local! {
    static INPUT: PerGeneration<InputTable> = PerGeneration::new();
    static MATCHERS: PerGeneration<Matchers> = PerGeneration::new();
}

/// Runs `work` on the table of the generation `ctx`. `work` must not call the shell or reach the table again.
fn with<R>(ctx: GnoblinCtx, work: impl FnOnce(&mut InputTable) -> R) -> R {
    INPUT.with(|input| input.with(ctx, work))
}

/// Runs `work` on the matchers of the generation `ctx`. `work` must not call the shell.
fn with_matchers<R>(ctx: GnoblinCtx, work: impl FnOnce(&mut Matchers) -> R) -> R {
    MATCHERS.with(|matchers| matchers.with(ctx, work))
}

/// Runs `work` on each matcher of the generation `ctx`.
fn matchers(ctx: GnoblinCtx, work: impl FnMut(&mut dyn Matcher)) {
    MATCHERS.with(|matchers| matchers.with(ctx, |held| held.each(work)));
}

/// A claim of a context reported `notice`: the matchers are told of it.
fn claim_event(cx: &Cx<'_>, notice: &ClaimNotice) {
    let ctx = cx.ctx();
    let found = with(ctx, |table| {
        let context = table.routes.get(&notice.claim).copied()?;
        let claim = table
            .contexts
            .get(&context)?
            .claims
            .iter()
            .find(|claim| claim.id == notice.claim)?;
        Some((context, claim.clone()))
    });
    if let Some((context, claim)) = found {
        matchers(ctx, |matcher| matcher.claim_event(cx, context, &claim, notice));
    }
}

/// A window is gone: what the matchers run on it ends.
pub(super) fn client_gone(cx: &Cx<'_>, client: Obj) {
    matchers(cx.ctx(), |matcher| matcher.cancel_on(cx, client));
}

/// The generation is going: every drag and gesture its bindings run ends now, with the `cancel` of its hooks, so the binding that took it
/// is told while the shell's code is still mapped. The next generation hears nothing of what the old one had begun. A turn queued for later
/// would never run, so this calls the shell at once.
pub(crate) fn end_inflight(ctx: GnoblinCtx) {
    let Some(manager) = with(ctx, |table| (table.manager != 0).then_some(table.manager)) else {
        return;
    };
    let _ = crate::object::run_in(ctx, manager, |cx| {
        let (presses, runs) = with_matchers(ctx, |held| (held.pointer.take_inflight(), held.gesture.take_inflight()));
        for press in presses {
            pointer::cancel(cx, &press, true);
        }
        for run in runs {
            gesture::cancel(cx, &run, true);
        }
    });
}

/// What a pump of the tree link brought: the seats and devices are looked at again.
pub(crate) fn pumped(ctx: GnoblinCtx) {
    let Some(manager) = with(ctx, |table| (table.manager != 0).then_some(table.manager)) else {
        return;
    };
    let _ = crate::object::run_in(ctx, manager, |cx| manager::sync(cx));
}

/// Implements the classes of the catalog.
pub(super) fn register(builder: &mut Builder) {
    binding::register(builder);
    context::register(builder);
    devices::register(builder);
    manager::register(builder);
    trigger_class(builder);
}

/// The generation ended: its input state goes with it. The claims were given up with the registrations that owned them.
pub(super) fn forget(ctx: GnoblinCtx) {
    MATCHERS.with(|held| held.take(ctx));
    INPUT.with(|input| input.take(ctx));
}

/// `triggers`: the grammar as functions.
fn trigger_class(builder: &mut Builder) {
    let _ = builder
        .class::<()>("triggers")
        .ctor(|_, _| Err(gnoblin_object::Fail::refused("`triggers` holds functions only")))
        .function("parse", |cx, args| {
            let trigger = parse_arg(args)?;
            trigger_value(cx, &trigger)
        })
        .function("check", |_, args| {
            let trigger = parse_arg(args)?;
            Ok(Val::Text(trigger.text))
        })
        .function("claim_text", |_, args| {
            let trigger = parse_arg(args)?;
            Ok(Val::Text(match trigger::claim_of(&trigger) {
                Some(SeatClaim::Chord { text, .. }) => text,
                Some(SeatClaim::Gesture { gesture, fingers }) => format!("{gesture}{fingers}"),
                None => String::new(),
            }))
        });
}

/// The trigger `args` names: its text, and `what` for the message when given.
fn parse_arg(args: &[Val]) -> Res<ParsedTrigger> {
    let text = args[0].text()?;
    let what = match args.get(1) {
        Some(value) if !value.is_none() => value.text()?,
        _ => "trigger",
    };
    parse_trigger(text, what)
}

/// Reads `text` as a trigger, or refuses it as the grammar does.
pub(super) fn parse_trigger(text: &str, what: &str) -> Res<ParsedTrigger> {
    trigger::parse(text, what).map_err(|refusal| gnoblin_object::Fail::type_error(refusal.message))
}

/// A parsed trigger as a value of the catalog's `trigger` record.
pub(super) fn trigger_value(cx: &Cx<'_>, trigger: &ParsedTrigger) -> Res<Val> {
    let optional_text = |text: Option<&str>| text.map_or(Val::None, |text| Val::Text(text.to_owned()));
    super::values::record(
        cx,
        "trigger",
        vec![
            ("text", Val::Text(trigger.text.clone())),
            ("kind", cx.enum_val("trigger_kind", trigger.kind.name())?),
            (
                "modifiers",
                Val::List(
                    trigger
                        .modifiers
                        .iter()
                        .map(|modifier| Val::Text((*modifier).to_owned()))
                        .collect(),
                ),
            ),
            ("key", optional_text(trigger.key.as_deref())),
            ("button", optional_text(trigger.button)),
            ("action", optional_text(trigger.action)),
            ("direction", optional_text(trigger.direction)),
            ("gesture", optional_text(trigger.gesture)),
            (
                "fingers",
                trigger
                    .fingers
                    .map_or(Val::None, |fingers| Val::Uint(u64::from(fingers))),
            ),
        ],
    )
}
