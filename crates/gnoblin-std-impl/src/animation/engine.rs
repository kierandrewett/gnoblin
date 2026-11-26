//! The transition engine: stages a callback's writes, sends them as timed writes that share one `tween` node, and follows the
//! compositor's word on how each track ended.
//!
//! This is `data/js/gnoblin-std/transition/engine.ts` and `transition.ts`. The compositor plays the tracks; the library writes the
//! end state when the transition is requested (the tree, the layout and the styles hold it) and the compositor draws the way there.
//! A transition ends when its tracks do (`track_ended`), not by the library's clock. Two transitions that write one property: the
//! newer owns it, the older goes on with what it still owns.
//!
//! What is not carried over from the TypeScript engine, because the native objects have no equivalent: the rollback of the script
//! state a failed callback changed (a layout object the callback assigned, an array edited in place) and the `Promise` of `finished`
//! (a listener is the native form).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use gnoblin_core::{ErrorKind, Gnoblin, LogLevel, NodeId, Timing, WriteStage};
use gnoblin_std_abi as abi;

use super::easing::Easing;
use super::staging::{self, Curve, Prop, Staged};
use crate::error::{Fail, Res};
use crate::nodes;
use crate::state::{Id, Notice, State};
use gnoblin_object::ErrorCode;

/// The properties of a node whose change is a place: a node that is not drawn yet is put there at once, not tweened.
const PLACE_KEYS: [&str; 4] = ["x", "y", "width", "height"];

/// The checked options of a transition.
#[derive(Clone, Debug)]
pub(crate) struct Options {
    pub(crate) duration_ms: f64,
    pub(crate) delay_ms: f64,
    pub(crate) samples: Vec<f64>,
    pub(crate) autoplay: bool,
}

impl Options {
    /// Checks the caller's options as `checkOptions` does: a finite duration and delay of at least 0, a known easing. `site` names the
    /// call in the error (`gnoblin.requestTransition`), as the path of the member it was about.
    pub(crate) fn check(raw: &abi::GnoblinStdTransitionOptions, site: &str) -> Res<Options> {
        if !raw.duration_ms.is_finite() || raw.duration_ms < 0.0 {
            return Err(Fail::type_error(format!(
                "{site}: duration must be a finite number of milliseconds of at least 0"
            )));
        }
        if !raw.delay_ms.is_finite() || raw.delay_ms < 0.0 {
            return Err(Fail::type_error(format!(
                "{site}: delay must be a finite number of milliseconds of at least 0"
            )));
        }
        let easing = Easing::from_abi(raw.easing).ok_or_else(|| {
            Fail::type_error(format!(
                "{site}: easing must be one of linear, ease, ease-in, ease-out, ease-in-out"
            ))
        })?;
        Ok(Options {
            duration_ms: raw.duration_ms,
            delay_ms: raw.delay_ms,
            samples: easing.samples(),
            autoplay: raw.autoplay != 0,
        })
    }
}

/// A track a transition started: where its property starts and ends, so a cancel can land it and a run back can restore it.
#[derive(Clone, Debug)]
struct Owned {
    node: u64,
    key: String,
    value: Prop,
    from: Option<Prop>,
}

/// One transition.
#[derive(Debug)]
pub(crate) struct Transition {
    id: Id,
    duration_ms: f64,
    state: u32,
    paused: bool,
    reversed: bool,
    progress: f64,
    /// The compositor's `tween` node its tracks joined.
    tween: Option<u64>,
    /// The tracks that have not ended.
    owned: BTreeMap<(u64, String), Owned>,
    /// Every property it wrote with a track.
    all: BTreeMap<(u64, String), Owned>,
    /// The properties a newer write took from it.
    released: BTreeSet<(u64, String)>,
    /// Some track ended other than by reaching its target.
    cancelled: bool,
    /// Whether `reverse` is the last word on the direction.
    running_back: bool,
}

/// What the engine keeps for one generation.
#[derive(Debug, Default)]
pub(crate) struct AnimState {
    transitions: HashMap<Id, Transition>,
    /// The transition with tracks running on each `tween` node.
    by_tween: HashMap<u64, Id>,
    /// The transition whose track runs on each property.
    owners: HashMap<(u64, String), Id>,
    /// The options of the transition being staged.
    pending: Option<Options>,
    /// Transitions with no track: they settle with the next turn, after the call that made them returns.
    settle_next: Vec<Id>,
    /// Finished transitions in the order they ended, so the oldest can be forgotten.
    ended: Vec<Id>,
    /// What the transition being staged can put back, when it was begun through the object protocol.
    undo: Option<staging::Undo>,
    /// The transitions that settled since the turn last told the objects that stand for them: `(transition, cancelled)`.
    settled: Vec<(Id, bool)>,
    /// The core's write stage while a transition is being staged: the writes of the core's object setters (`client.opacity = 0.5`) are
    /// held with the others. None on a core built before it, whose setters send every write at once.
    hook: Option<WriteStage>,
}

/// How many finished transitions are remembered for `transition_info`.
const KEEP_ENDED: usize = 256;

/// The milliseconds an animation of `duration` takes: 0 while the seat asks for reduced motion.
pub(crate) fn motion(g: &Gnoblin, duration: f64) -> f64 {
    match nodes::seat(g) {
        Some(seat) if seat.get_bool("reduced_motion").unwrap_or(false) => 0.0,
        _ => duration,
    }
}

/// Whether a node has a place that is drawn: a size, and not hidden.
fn placed(g: &Gnoblin, node: u64) -> bool {
    nodes::number(g, node, "width") > 0.0
        && nodes::number(g, node, "height") > 0.0
        && nodes::flag_or(g, node, "visible", true)
}

/// Reads a property as the engine writes them back.
pub(crate) fn read_prop(g: &Gnoblin, node: u64, key: &str) -> Option<Prop> {
    Prop::from_value(&g.node(NodeId(node)).get(key).ok()?)
}

/// Writes a value that is not a script's (a transition's own): a timed write when `play` is given, else a plain one that ends the
/// compositor's track on the property. `Ok(false)` is a node that is gone, which is not a refusal.
fn land(g: &Gnoblin, node: u64, key: &str, value: &Prop, play: Option<(&Curve, f64, Option<u64>)>) -> Res<bool> {
    let handle = g.node(NodeId(node));
    let result = match play {
        Some((curve, delay_ms, tween)) => handle.set_timed(
            key,
            value.to_value(),
            &Timing {
                seconds: curve.duration_ms / 1000.0,
                delay: delay_ms / 1000.0,
                tween: tween.map(NodeId),
                samples: curve.samples.clone(),
            },
        ),
        None => handle.set(key, value.to_value()),
    };
    match result {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NoSuchNode => Ok(false),
        Err(error) => Err(Fail::from(error)),
    }
}

/// Makes the `tween` node the tracks of a transition join.
fn make_tween(g: &Gnoblin) -> Res<u64> {
    g.create_node("tween").map(|node| node.id().0).map_err(|error| {
        Fail::refused(
            ErrorCode::Internal,
            format!("the shell could not make the node that holds the transition's play state: {error}"),
        )
    })
}

/// Drops a `tween` node; a node already gone is not an error.
fn drop_tween(g: &Gnoblin, tween: u64) {
    let _ = g.node(NodeId(tween)).destroy();
}

impl State {
    /// Starts staging a transition's writes.
    pub(crate) fn transition_begin(&mut self, raw: &abi::GnoblinStdTransitionOptions) -> Res<()> {
        const SITE: &str = "gnoblin.requestTransition";
        if staging::active() {
            return Err(Fail::busy(format!(
                "{SITE}: was called inside a requestTransition callback; stage every change in the outer callback instead"
            )).with_code(ErrorCode::InvalidState));
        }
        self.anim.pending = Some(Options::check(raw, SITE)?);
        staging::begin();
        self.anim.hook = self.write_stage();
        Ok(())
    }

    /// Installs the core's write stage: a write an object's setter makes (a window's `opacity`) is staged here when `staging::check`
    /// accepts it, and a read of it sees the staged value. What the check refuses goes on as a plain write, as it does without a stage.
    /// A core without the write stage leaves those writes plain.
    fn write_stage(&self) -> Option<WriteStage> {
        let g = self.g.clone();
        let staged = self.g.stage_writes(
            move |node, key, value| {
                let Some(prop) = Prop::from_value(value) else {
                    return false;
                };
                staging::check(&g, node.0, key, &prop, false).is_ok() && staging::capture(node.0, key, &prop)
            },
            |node, key| staging::peek(node.0, key).map(|prop| prop.to_value()),
        );
        match staged {
            Ok(stage) => Some(stage),
            Err(error) if error.kind() == ErrorKind::Unsupported => None,
            Err(error) => {
                self.g.log_at(
                    LogLevel::Warn,
                    &format!("gnoblin-std: a transition could not stage the writes of the core's objects: {error}"),
                );
                None
            }
        }
    }

    /// Ends staging for a commit and hands over what was staged, in the order the properties were first written. The core's write
    /// stage goes first, so nothing offered from here on is kept.
    pub(crate) fn transition_end_staging(&mut self) -> Vec<Staged> {
        self.anim.hook = None;
        staging::end()
    }

    /// Ends staging and drops what was staged.
    pub(crate) fn transition_abort(&mut self) {
        self.anim.hook = None;
        staging::end();
        self.anim.pending = None;
    }

    /// Starts staging for the object protocol: the callback's writes are held, and what the library holds that they may change, with the
    /// shell's rollback, is kept to put back.
    pub(crate) fn transition_begin_undoable(
        &mut self,
        raw: &abi::GnoblinStdTransitionOptions,
        rollback: Option<std::rc::Rc<dyn gnoblin_object::Callable>>,
    ) -> Res<()> {
        self.transition_begin(raw)?;
        self.anim.undo = Some(staging::Undo {
            rollback,
            styles: self.styles.snapshot(),
            workspaces: self.snapshot_workspaces(),
            displays: self.snapshot_displays_value(),
        });
        Ok(())
    }

    /// The undo of the transition that was being staged: none for one begun through the table. A transition that committed has none
    /// to put back; its rollback is let go.
    pub(crate) fn transition_take_undo(&mut self) -> Option<staging::Undo> {
        self.anim.undo.take()
    }

    /// The transitions that settled since the last call.
    pub(crate) fn anim_take_settled(&mut self) -> Vec<(Id, bool)> {
        std::mem::take(&mut self.anim.settled)
    }

    /// Stages a write of the shell's, or makes it at once when nothing is staged (and lets go of the property a transition held).
    pub(crate) fn transition_set(&mut self, node: u64, key: &str, value: Prop) -> Res<()> {
        staging::check(&self.g, node, key, &value, false)?;
        if staging::capture(node, key, &value) {
            return Ok(());
        }
        staging::released(node, key);
        self.process_released();
        self.g.node(NodeId(node)).set(key, value.to_value()).map_err(Fail::from)
    }

    /// A write of `key` of `node` was made by the shell's own engine, outside the library: the transition that played the property
    /// lets it go.
    pub(crate) fn transition_release(&mut self, node: u64, key: &str) {
        staging::released(node, key);
        self.process_released();
    }

    /// The staged value of a property while a transition is being staged, else the tree's.
    pub(crate) fn transition_get(&self, node: u64, key: &str) -> Option<Prop> {
        staging::peek(node, key).or_else(|| read_prop(&self.g, node, key))
    }

    /// Sends the staged writes: the transition's tracks join one `tween` node.
    pub(crate) fn transition_commit(&mut self, entries: Vec<Staged>, options: &Options) -> Res<Id> {
        self.process_released();
        let g = self.g.clone();
        let duration = motion(&g, options.duration_ms);
        // Under reduced motion nothing waits either.
        let delay = if duration > 0.0 { options.delay_ms } else { 0.0 };
        let mut appears: HashMap<u64, bool> = HashMap::new();
        let mut plan: Vec<(Staged, Option<Curve>)> = Vec::with_capacity(entries.len());
        for entry in entries {
            let curve = entry.curve.clone().or_else(|| {
                (duration > 0.0).then(|| Curve {
                    duration_ms: duration,
                    samples: options.samples.clone(),
                })
            });
            // A node that was not drawn has no place to tween from: its place is written at once.
            let appearing = *appears.entry(entry.node).or_insert_with(|| !placed(&g, entry.node));
            let timed = curve.as_ref().is_some_and(|curve| curve.duration_ms > 0.0)
                && staging::animated(&entry.key)
                && !(appearing && entry.curve.is_none() && PLACE_KEYS.contains(&entry.key.as_str()));
            let curve = if timed { curve } else { None };
            plan.push((entry, curve));
        }
        let tracks = plan.iter().filter(|(_, curve)| curve.is_some()).count();
        let tween = if tracks > 0 { Some(make_tween(&g)?) } else { None };
        let id = crate::state::next_id();
        let mut transition = Transition {
            id,
            duration_ms: duration,
            state: abi::STD_TRANSITION_RUNNING,
            paused: !options.autoplay,
            reversed: false,
            progress: 0.0,
            tween,
            owned: BTreeMap::new(),
            all: BTreeMap::new(),
            released: BTreeSet::new(),
            cancelled: false,
            running_back: false,
        };
        if let (Some(tween), false) = (tween, options.autoplay) {
            let _ = g.node(NodeId(tween)).set("paused", true);
        }
        let mut refused: Vec<String> = Vec::new();
        for (entry, curve) in plan {
            let key_id = (entry.node, entry.key.clone());
            self.release(&key_id);
            let from = read_prop(&g, entry.node, &entry.key);
            let sent = curve.as_ref().filter(|_| tween.is_some());
            match land(
                &g,
                entry.node,
                &entry.key,
                &entry.value,
                sent.map(|curve| (curve, delay, tween)),
            ) {
                Ok(true) => {
                    if sent.is_some() {
                        let owned = Owned {
                            node: entry.node,
                            key: entry.key.clone(),
                            value: entry.value.clone(),
                            from,
                        };
                        transition.owned.insert(key_id.clone(), owned.clone());
                        transition.all.insert(key_id.clone(), owned);
                        self.anim.owners.insert(key_id, id);
                    }
                }
                Ok(false) => {}
                Err(fail) => refused.push(format!("Client#{}.{} ({})", entry.node, entry.key, fail.message)),
            }
        }
        if !refused.is_empty() {
            for key_id in transition.owned.keys() {
                self.anim.owners.remove(key_id);
            }
            if let Some(tween) = tween {
                drop_tween(&g, tween);
            }
            return Err(Fail::refused(
                ErrorCode::Denied,
                format!("the host refused {}", refused.join(", ")),
            ));
        }
        let has_tracks = !transition.owned.is_empty();
        self.anim.transitions.insert(id, transition);
        match tween {
            None => self.anim.settle_next.push(id),
            Some(tween) if has_tracks => {
                self.anim.by_tween.insert(tween, id);
            }
            // Every track was refused as for an unknown node: there is nothing for the compositor to end.
            Some(_) => self.finish(id),
        }
        Ok(id)
    }

    /// One timed write of the shell's own: a transition of one property (or, while staging, a staged write with its own curve).
    pub(crate) fn transition_animate(
        &mut self,
        node: u64,
        key: &str,
        value: Prop,
        options: &Options,
    ) -> Res<Option<Id>> {
        let duration = motion(&self.g, options.duration_ms);
        let curve = Curve {
            duration_ms: duration,
            samples: options.samples.clone(),
        };
        if staging::active() {
            staging::check(&self.g, node, key, &value, true)?;
            staging::keep(Staged {
                node,
                key: key.to_owned(),
                value,
                curve: Some(curve),
            });
            return Ok(None);
        }
        let entry = Staged {
            node,
            key: key.to_owned(),
            value,
            curve: Some(curve),
        };
        self.transition_commit(vec![entry], options).map(Some)
    }

    /// Plays, pauses, reverses, seeks or cancels a transition.
    pub(crate) fn transition_control(&mut self, id: Id, op: u32, value: f64) -> Res<()> {
        if staging::active() {
            return Err(Fail::busy("a transition cannot be controlled inside a requestTransition callback: it has a side effect, and a callback only stages property writes").with_code(ErrorCode::InvalidState));
        }
        self.process_released();
        let g = self.g.clone();
        let Some(transition) = self.anim.transitions.get_mut(&id) else {
            return Err(Fail::stale(format!("transition {id} does not exist")));
        };
        if transition.state != abi::STD_TRANSITION_RUNNING {
            return Ok(());
        }
        let tween = transition.tween;
        let write = |key: &str, value: Prop| {
            if let Some(tween) = tween {
                let _ = g.node(NodeId(tween)).set(key, value.to_value());
            }
        };
        match op {
            abi::STD_CONTROL_SEEK => {
                if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                    return Err(Fail::type_error(format!(
                        "Transition.progress: expects a number from 0 to 1, not {value}"
                    )));
                }
                transition.progress = value;
                write("progress", Prop::Number(value));
            }
            abi::STD_CONTROL_PLAY => {
                transition.paused = false;
                transition.reversed = false;
                transition.running_back = false;
                write("reverse", Prop::Bool(false));
                write("paused", Prop::Bool(false));
            }
            abi::STD_CONTROL_PAUSE => {
                transition.paused = true;
                write("paused", Prop::Bool(true));
            }
            abi::STD_CONTROL_REVERSE => {
                transition.paused = false;
                transition.reversed = true;
                transition.running_back = true;
                write("reverse", Prop::Bool(true));
                write("paused", Prop::Bool(false));
            }
            abi::STD_CONTROL_CANCEL => self.cancel(id),
            other => return Err(Fail::type_error(format!("{other} is not a transition operation"))),
        }
        Ok(())
    }

    /// Stops a transition: every property it still owns is written at its end value at once, which ends the compositor's track.
    fn cancel(&mut self, id: Id) {
        let g = self.g.clone();
        let Some(transition) = self.anim.transitions.get_mut(&id) else {
            return;
        };
        let owned: Vec<Owned> = std::mem::take(&mut transition.owned).into_values().collect();
        let tween = transition.tween;
        for track in &owned {
            self.anim.owners.remove(&(track.node, track.key.clone()));
            // The instant write lands the end value and ends the track the compositor runs on the property.
            let _ = land(&g, track.node, &track.key, &track.value, None);
        }
        if let Some(tween) = tween {
            self.anim.by_tween.remove(&tween);
            drop_tween(&g, tween);
        }
        self.settle(id, true);
    }

    /// The compositor's word that a timed write ended. A track that is not one a transition still owns is ignored: a newer write took
    /// the property, or the transition was cancelled, and it has already settled for it.
    pub(crate) fn track_ended(&mut self, node: u64, property: &str, completed: bool, tween: u64) {
        self.process_released();
        let Some(id) = self.anim.by_tween.get(&tween).copied() else {
            return;
        };
        let key_id = (node, property.to_owned());
        let Some(transition) = self.anim.transitions.get_mut(&id) else {
            return;
        };
        if transition.owned.remove(&key_id).is_none() {
            return;
        }
        self.anim.owners.remove(&key_id);
        if !completed {
            transition.cancelled = true;
        }
        if transition.owned.is_empty() {
            self.finish(id);
        }
    }

    /// A property written again: the transition that owned it no longer does, and one that has no track left is over.
    fn release(&mut self, key_id: &(u64, String)) {
        let Some(id) = self.anim.owners.remove(key_id) else {
            return;
        };
        let Some(transition) = self.anim.transitions.get_mut(&id) else {
            return;
        };
        transition.owned.remove(key_id);
        transition.released.insert(key_id.clone());
        if transition.owned.is_empty() {
            // One that lost its last property to a newer write is over, cancelled; one that lost some goes on.
            transition.cancelled = true;
            self.finish(id);
        }
    }

    /// Takes the plain writes made since the last call and lets go of the properties they took.
    pub(crate) fn process_released(&mut self) {
        for key_id in staging::take_released() {
            self.release(&key_id);
        }
    }

    /// A transition whose tracks have all ended: it settles, cancelled when any did not reach its target. One that ran back to its
    /// start has the start values put back first (a property a newer write took stays the newer write's).
    fn finish(&mut self, id: Id) {
        let g = self.g.clone();
        let Some(transition) = self.anim.transitions.get_mut(&id) else {
            return;
        };
        let tween = transition.tween;
        let cancelled = transition.cancelled;
        if let Some(tween) = tween {
            self.anim.by_tween.remove(&tween);
        }
        let restore: Vec<Owned> = if transition.running_back && cancelled {
            transition
                .all
                .iter()
                .filter(|(key_id, _)| !transition.released.contains(*key_id))
                .map(|(_, owned)| owned.clone())
                .collect()
        } else {
            Vec::new()
        };
        for owned in restore {
            if let Some(from) = &owned.from {
                let _ = land(&g, owned.node, &owned.key, from, None);
            }
        }
        if let Some(tween) = tween {
            drop_tween(&g, tween);
        }
        self.settle(id, cancelled);
    }

    /// Settles a transition once and tells the listeners that wait for it.
    fn settle(&mut self, id: Id, cancelled: bool) {
        let Some(transition) = self.anim.transitions.get_mut(&id) else {
            return;
        };
        if transition.state != abi::STD_TRANSITION_RUNNING {
            return;
        }
        transition.state = if cancelled {
            abi::STD_TRANSITION_CANCELLED
        } else {
            abi::STD_TRANSITION_FINISHED
        };
        self.anim.ended.push(id);
        self.anim.settled.push((id, cancelled));
        let heard: Vec<Id> = self
            .regs
            .iter()
            .filter(|(_, reg)| reg.event == abi::STD_EVENT_TRANSITION_FINISHED && reg.coll == id)
            .map(|(reg_id, _)| *reg_id)
            .collect();
        for reg in heard {
            self.notify_finished(id, reg, cancelled);
        }
        while self.anim.ended.len() > KEEP_ENDED {
            let oldest = self.anim.ended.remove(0);
            self.anim.transitions.remove(&oldest);
        }
    }

    /// Queues the end of `transition` for the listener `reg`.
    pub(crate) fn notify_finished(&mut self, transition: Id, reg: Id, cancelled: bool) {
        let mut notice = Notice::new(abi::STD_EVENT_TRANSITION_FINISHED, transition, Vec::new());
        notice.only = Some(reg);
        notice.flag = u32::from(cancelled);
        self.notices.push(notice);
    }

    /// Work the next turn does: the writes that released a property, and the transitions that had no track to wait for.
    pub(crate) fn anim_flush(&mut self) {
        self.process_released();
        for id in std::mem::take(&mut self.anim.settle_next) {
            self.settle(id, false);
        }
    }

    /// What a transition is now.
    pub(crate) fn transition_info(&self, id: Id) -> Res<abi::GnoblinStdTransitionInfo> {
        let transition = self
            .anim
            .transitions
            .get(&id)
            .ok_or_else(|| Fail::stale(format!("transition {id} does not exist or ended long ago")))?;
        Ok(abi::GnoblinStdTransitionInfo {
            size: size_of::<abi::GnoblinStdTransitionInfo>() as u32,
            state: transition.state,
            id: transition.id,
            duration_ms: transition.duration_ms,
            progress: transition.progress,
            paused: u32::from(transition.paused),
            reversed: u32::from(transition.reversed),
        })
    }

    /// The state of a transition, for a listener that arrives after it ended.
    pub(crate) fn transition_state(&self, id: Id) -> Option<u32> {
        self.anim.transitions.get(&id).map(|transition| transition.state)
    }
}

impl State {
    /// Whether a transition has a track running on `key` of `node`.
    pub(crate) fn plays(&self, node: u64, key: &str) -> bool {
        self.anim.owners.contains_key(&(node, key.to_owned()))
    }

    /// Ends the generation's hold on the compositor: the `tween` nodes of the transitions still running are destroyed.
    pub(crate) fn anim_release(&mut self) {
        for (tween, _) in std::mem::take(&mut self.anim.by_tween) {
            drop_tween(&self.g, tween);
        }
        self.anim.owners.clear();
    }

    /// The options of the transition being staged, taken as it commits.
    pub(crate) fn anim_take_pending(&mut self) -> Option<Options> {
        self.anim.pending.take()
    }
}
