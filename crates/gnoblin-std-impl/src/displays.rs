//! Output profiles, logical displays and what reconciles them with the connected outputs.
//!
//! An *output profile* is a persistent selector for one monitor (by connector, make, model or serial) and the settings it gets when it
//! is connected, and again when it returns. A *display* is one viewport presented on one output (`single`), copied to several
//! (`mirror`) or stretched over their union (`span`); its selectors are profiles or outputs. The library gives every output that no
//! display of the shell presents a `single` display of its own, so the list of displays always says what each monitor shows.
//!
//! Reconciling (`reflow`) is what `display_manager/manager.ts` was: apply the profiles, settle who presents which output, and enact
//! each display. The output requests it sends go to the compositor's `screen` node as one `apply` call each, the way a script's own
//! `Output.apply` does, and the answers come back through the core: a refusal is named by the profile (`outputs.profiles.DP-1.scale:
//! ...`), an answer reaches the shell as an event, and an output that has a request in flight is not asked again until it is answered.
//! Everything the shell hears (the lists changed, an answer arrived, a sentence for its diagnostics) is an event of the root
//! collection of outputs.

use std::collections::{HashMap, HashSet};

use gnoblin_core::host::Value;
use gnoblin_std_abi as abi;

use crate::error::{Fail, Res};
use crate::nodes;
use crate::state::{self, Id, NodeId, Notice, State};
use crate::turn::schedule;
use gnoblin_object::ErrorCode;

/// A profile or a display, by the library's own id.
pub(crate) type Hid = u64;

/// A viewport, by the library's own id.
pub(crate) type Vid = u64;

/// Which hardware a profile is for: every key given must match.
#[derive(Clone, Debug, Default)]
pub(crate) struct Match {
    pub(crate) connector: Option<String>,
    pub(crate) make: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) serial: Option<String>,
}

/// What a relative position is against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlaceTarget {
    Profile(Hid),
    Output(NodeId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NeighbourSide {
    RightOf,
    LeftOf,
    Above,
    Below,
}

/// Where a profile puts its output.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Position {
    Absolute(f64, f64),
    Relative {
        side: NeighbourSide,
        align: u32,
        target: PlaceTarget,
    },
}

/// What a profile asks for, apart from which hardware it is for.
#[derive(Clone, Debug, Default)]
pub(crate) struct ProfileSettings {
    pub(crate) mode: Option<String>,
    pub(crate) scale: Option<f64>,
    pub(crate) supersample: Option<f64>,
    pub(crate) transform: Option<String>,
    pub(crate) vrr: Option<bool>,
    pub(crate) enabled: Option<bool>,
    pub(crate) position: Option<Position>,
}

/// What was last sent to one output for one profile.
#[derive(Clone, Debug, Default)]
struct SentRequests {
    base: Option<String>,
    place: Option<String>,
}

#[derive(Debug)]
pub(crate) struct ProfileRec {
    pub(crate) id: String,
    pub(crate) matcher: Match,
    pub(crate) settings: ProfileSettings,
    pub(crate) listed: bool,
    sent: HashMap<NodeId, SentRequests>,
}

/// Something a display presents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sel {
    Profile(Hid),
    Output(NodeId),
}

#[derive(Debug)]
pub(crate) struct DisplayRec {
    pub(crate) id: String,
    pub(crate) mode: u32,
    pub(crate) selectors: Vec<Sel>,
    /// A display the library made for an output nobody else presents.
    pub(crate) implicit: bool,
    pub(crate) listed: bool,
    /// The connected outputs the display presents now, in selector order.
    pub(crate) live: Vec<NodeId>,
    /// The workspace assigned while the display could not show it.
    pub(crate) wanted: Option<Id>,
    /// The workspace last assigned or last seen shown.
    pub(crate) content: Option<Id>,
    /// The viewport the display shows, once the shell has one for it.
    pub(crate) viewport: Option<Vid>,
    /// The mirrors this display made: output and what it mirrors.
    mirrored: Vec<(NodeId, NodeId)>,
    /// The output this display spans from, while it does.
    span_lead: Option<NodeId>,
}

/// What the library keeps of displays and profiles.
#[derive(Debug, Default)]
pub(crate) struct Displays {
    pub(crate) profiles: HashMap<Hid, ProfileRec>,
    pub(crate) profile_list: Vec<Hid>,
    pub(crate) displays: HashMap<Hid, DisplayRec>,
    pub(crate) display_list: Vec<Hid>,
    /// The viewports the shell made, by the library's own id.
    pub(crate) viewports: HashMap<Vid, ViewportRec>,
    /// How many displays were named `display-N` for want of an id.
    pub(crate) numbered: u64,
    /// Who presents each output, as the last reflow settled it.
    owners: HashMap<NodeId, Hid>,
    /// The display the library made for each output it made one for.
    implicit: HashMap<NodeId, Hid>,
    /// How many requests of each output are in flight.
    pending: HashMap<NodeId, u32>,
    /// The sentences already told, once per generation.
    said: HashSet<String>,
    /// What `display_snapshot` kept, by token.
    snapshots: HashMap<u64, DisplaysSnapshot>,
}

/// A viewport: a workspace seen through a camera.
#[derive(Debug)]
pub(crate) struct ViewportRec {
    /// The workspace last assigned or last seen shown.
    pub(crate) content: Option<Id>,
    /// The object of the camera, once one is given or made.
    pub(crate) camera: Option<u64>,
    /// The colour behind the content.
    pub(crate) clear_color: String,
    /// The display that shows the viewport.
    pub(crate) display: Option<Hid>,
}

/// What a snapshot keeps: for each display the workspace it waits to show and the one it last showed, for each viewport its camera and its
/// colour, and the workspace each desk shows.
#[derive(Debug, Default)]
pub(crate) struct DisplaysSnapshot {
    displays: Vec<(Hid, Option<Id>, Option<Id>)>,
    viewports: Vec<(Vid, Option<u64>, String)>,
    desks: Vec<(u64, Option<Id>)>,
}

const RECOMMENDED_SUPERSAMPLE: [f64; 3] = [1.0, 2.0, 4.0];

/// The name a message gives an output.
fn name_of(st: &State, output: NodeId) -> String {
    let connector = nodes::text(&st.g, output, "connector");
    if connector.is_empty() {
        nodes::text(&st.g, output, "name")
    } else {
        connector
    }
}

/// The serial number in an EDID identity `MAN:product:serial`, or empty.
fn serial_of(identity: &str) -> String {
    let parts: Vec<&str> = identity.split(':').collect();
    if parts.len() < 3 {
        String::new()
    } else {
        parts[2..].join(":")
    }
}

/// Whether `text` starts with a lowercase word and a colon: a refusal that names its own key.
fn keyed(text: &str) -> bool {
    let word = text
        .bytes()
        .take_while(|b| b.is_ascii_lowercase() || *b == b'_')
        .count();
    word > 0 && text[word..].starts_with(": ")
}

/// The top-left corner that puts an output of `size` against `target` on `side`, aligned as asked.
fn placed(side: NeighbourSide, align: u32, target: (f64, f64, f64, f64), size: (f64, f64)) -> (f64, f64) {
    let (tx, ty, tw, th) = target;
    let along = |own: f64, theirs: f64| -> f64 {
        match align {
            abi::STD_ALIGN_CENTER => ((theirs - own) / 2.0).round(),
            abi::STD_ALIGN_END => theirs - own,
            _ => 0.0,
        }
    };
    match side {
        NeighbourSide::RightOf => (tx + tw, ty + along(size.1, th)),
        NeighbourSide::LeftOf => (tx - size.0, ty + along(size.1, th)),
        NeighbourSide::Above => (tx + along(size.0, tw), ty - size.1),
        NeighbourSide::Below => (tx + along(size.0, tw), ty + th),
    }
}

/// The compositor answered a request the library sent for `output`: the request is no longer in flight, the shell hears the answer, and
/// a refusal is a sentence for its diagnostics under the name of what asked.
fn answered(ctx: gnoblin_core_abi::GnoblinCtx, output: NodeId, path: &str, refusal: Option<String>) {
    let Some(std) = state::find(ctx) else {
        return;
    };
    {
        let mut st = std.st.borrow_mut();
        if let Some(count) = st.disp.pending.get_mut(&output) {
            *count = count.saturating_sub(1);
        }
        let root = st.roots[1];
        let mut notice = Notice::new(abi::STD_EVENT_APPLIED, output, vec![root]);
        notice.text = refusal.clone().unwrap_or_default();
        st.notices.push(notice);
        if let Some(error) = refusal {
            // `mode: ...` is a key's own refusal, so it extends the path; anything else is about the entry as a whole.
            let text = if keyed(&error) {
                format!("{path}.{error}")
            } else {
                format!("{path}: {error}")
            };
            st.note(text);
        }
    }
    schedule(&std);
    std.drain();
}

impl State {
    // ---- the shell's side of it ----------------------------------------------------------------------------------------

    fn announce_displays(&mut self) {
        let root = self.roots[1];
        self.notices.push(Notice::new(abi::STD_EVENT_DISPLAYS, 0, vec![root]));
    }

    /// A sentence for the shell's diagnostics, once.
    pub(crate) fn note(&mut self, text: String) {
        if !self.disp.said.insert(text.clone()) {
            return;
        }
        let root = self.roots[1];
        let mut notice = Notice::new(abi::STD_EVENT_NOTE, 0, vec![root]);
        notice.text = text;
        self.notices.push(notice);
    }

    // ---- profiles ------------------------------------------------------------------------------------------------------

    pub(crate) fn new_profile(&mut self, id: String, matcher: Match, settings: ProfileSettings) -> Hid {
        let handle = state::next_id();
        self.disp.profiles.insert(
            handle,
            ProfileRec {
                id,
                matcher,
                settings,
                listed: false,
                sent: HashMap::new(),
            },
        );
        handle
    }

    pub(crate) fn profile(&self, profile: Hid) -> Res<&ProfileRec> {
        self.disp
            .profiles
            .get(&profile)
            .ok_or_else(|| Fail::stale(format!("profile {profile} does not exist")))
    }

    /// Replaces the settings of a profile. A position that depends on itself through other profiles is refused.
    pub(crate) fn set_profile(&mut self, profile: Hid, settings: ProfileSettings) -> Res<()> {
        let before = self.profile(profile)?.settings.clone();
        if let Some(rec) = self.disp.profiles.get_mut(&profile) {
            rec.settings = settings;
        }
        if let Some(through) = self.profile_cycle(profile) {
            if let Some(rec) = self.disp.profiles.get_mut(&profile) {
                rec.settings = before;
            }
            let own = self.disp.profiles.get(&profile).map_or("", |rec| rec.id.as_str());
            let via = self.disp.profiles.get(&through).map_or("", |rec| rec.id.as_str());
            return Err(Fail::type_error(format!(
                "OutputProfile \"{own}\".position: placed relative to itself through \"{via}\""
            )));
        }
        Ok(())
    }

    /// The profile that following the relative positions from `profile` comes back to, if it does.
    fn profile_cycle(&self, profile: Hid) -> Option<Hid> {
        let mut seen = HashSet::from([profile]);
        let mut at = profile;
        while let Some(Position::Relative {
            target: PlaceTarget::Profile(next),
            ..
        }) = self.disp.profiles.get(&at).and_then(|rec| rec.settings.position)
        {
            if !seen.insert(next) {
                return Some(next);
            }
            at = next;
        }
        None
    }

    pub(crate) fn add_profile(&mut self, profile: Hid) -> Res<bool> {
        let rec = self.profile(profile)?;
        if rec.listed {
            return Ok(false);
        }
        let id = rec.id.clone();
        if self
            .disp
            .profile_list
            .iter()
            .any(|other| self.disp.profiles.get(other).is_some_and(|rec| rec.id == id))
        {
            return Err(Fail::busy(format!(
                "there is already a profile \"{id}\"; give the profile an id of its own"
            ))
            .with_code(ErrorCode::AlreadyExists));
        }
        if let Some(rec) = self.disp.profiles.get_mut(&profile) {
            rec.listed = true;
        }
        self.disp.profile_list.push(profile);
        self.announce_displays();
        self.reflow_displays();
        Ok(true)
    }

    pub(crate) fn remove_profile(&mut self, profile: Hid) -> bool {
        let Some(at) = self.disp.profile_list.iter().position(|one| *one == profile) else {
            return false;
        };
        self.disp.profile_list.remove(at);
        if let Some(rec) = self.disp.profiles.get_mut(&profile) {
            rec.listed = false;
        }
        self.announce_displays();
        true
    }

    fn profile_matches(&self, profile: Hid, output: NodeId) -> bool {
        let Some(rec) = self.disp.profiles.get(&profile) else {
            return false;
        };
        let m = &rec.matcher;
        let g = &self.g;
        m.connector
            .as_ref()
            .is_none_or(|v| nodes::text(g, output, "connector") == *v)
            && m.make.as_ref().is_none_or(|v| nodes::text(g, output, "make") == *v)
            && m.model.as_ref().is_none_or(|v| nodes::text(g, output, "model") == *v)
            && m.serial
                .as_ref()
                .is_none_or(|v| serial_of(&nodes::text(g, output, "identity")) == *v)
    }

    /// The first connected output `profile` is for, or `None` while its monitor is not connected.
    fn hardware(&self, profile: Hid, live: &[NodeId]) -> Option<NodeId> {
        live.iter()
            .copied()
            .find(|output| self.profile_matches(profile, *output))
    }

    /// The request lines a profile makes of its output, leaving out a relative position, which needs its neighbour.
    fn profile_request(&self, rec: &ProfileRec) -> Vec<String> {
        let s = &rec.settings;
        let mut lines = Vec::new();
        if let Some(mode) = &s.mode {
            lines.push(format!("mode={mode}"));
        }
        if let Some(scale) = s.scale {
            lines.push(format!("scale={scale}"));
        }
        if let Some(supersample) = s.supersample {
            lines.push(format!("supersample={supersample}"));
        }
        if let Some(transform) = &s.transform {
            lines.push(format!("transform={transform}"));
        }
        if let Some(enabled) = s.enabled {
            lines.push(format!("enabled={enabled}"));
        }
        if let Some(Position::Absolute(x, y)) = s.position {
            lines.push(format!("position={x},{y}"));
        }
        lines
    }

    fn profile_depth(&self, profile: Hid) -> usize {
        let mut depth = 0;
        let mut at = profile;
        while let Some(Position::Relative {
            target: PlaceTarget::Profile(next),
            ..
        }) = self.disp.profiles.get(&at).and_then(|rec| rec.settings.position)
        {
            depth += 1;
            at = next;
            if depth > self.disp.profiles.len() {
                break;
            }
        }
        depth
    }

    fn apply_profiles(&mut self, live: &[NodeId]) {
        // A neighbour first: its geometry is what a relative position reads.
        let mut ordered = self.disp.profile_list.clone();
        ordered.sort_by_key(|profile| self.profile_depth(*profile));
        for profile in ordered {
            if let Some(output) = self.hardware(profile, live) {
                self.apply_profile(profile, output, live);
            }
        }
    }

    /// One step of applying a profile: its own settings first, then, once those are answered, its relative position.
    fn apply_profile(&mut self, profile: Hid, output: NodeId, live: &[NodeId]) {
        let Some(rec) = self.disp.profiles.get(&profile) else {
            return;
        };
        let id = rec.id.clone();
        let path = format!("outputs.profiles.{id}");
        let lines = self.profile_request(rec);
        let text = lines.join("\n");
        let vrr = rec.settings.vrr == Some(true);
        let supersample = rec.settings.supersample;
        let position = rec.settings.position;
        let sent = rec.sent.get(&output).cloned().unwrap_or_default();
        if vrr {
            self.note(format!(
                "not applied: OutputProfile \"{id}\" vrr (the output request has no vrr key yet)"
            ));
        }
        if !text.is_empty() && sent.base.as_deref() != Some(text.as_str()) {
            if !self.is_pending(output) {
                if let Some(rec) = self.disp.profiles.get_mut(&profile) {
                    rec.sent.entry(output).or_default().base = Some(text.clone());
                }
                self.send_apply(output, text, &path, supersample);
            }
            return;
        }
        let Some(Position::Relative { side, align, target }) = position else {
            return;
        };
        if self.is_pending(output) {
            return;
        }
        let neighbour = match target {
            PlaceTarget::Profile(other) => self.hardware(other, live),
            PlaceTarget::Output(node) => live.contains(&node).then_some(node),
        };
        let Some(neighbour) = neighbour else {
            self.note(format!(
                "not applied: OutputProfile \"{id}\" position (the output it is placed against is not connected)"
            ));
            return;
        };
        if self.is_pending(neighbour) {
            return;
        }
        let geometry = |o: NodeId| {
            let g = &self.g;
            (
                nodes::number(g, o, "x"),
                nodes::number(g, o, "y"),
                nodes::number(g, o, "width"),
                nodes::number(g, o, "height"),
            )
        };
        let theirs = geometry(neighbour);
        let own = geometry(output);
        let (x, y) = placed(side, align, theirs, (own.2, own.3));
        let place = format!("{x},{y}");
        if sent.place.as_deref() == Some(place.as_str()) {
            return;
        }
        if let Some(rec) = self.disp.profiles.get_mut(&profile) {
            rec.sent.entry(output).or_default().place = Some(place.clone());
        }
        if own.0 != x || own.1 != y {
            self.send_apply(output, format!("position={place}"), &path, None);
        }
    }

    // ---- output requests -----------------------------------------------------------------------------------------------

    fn is_pending(&self, output: NodeId) -> bool {
        self.disp.pending.get(&output).is_some_and(|count| *count > 0)
    }

    /// Sends one request for `output`. A refusal is named under `path`; a request that cannot be sent is reported, not raised.
    fn send_apply(&mut self, output: NodeId, request: String, path: &str, supersample: Option<f64>) {
        if let Some(factor) = supersample
            && !RECOMMENDED_SUPERSAMPLE.contains(&factor)
        {
            self.note(format!("supersample {factor} is not recommended: use 1, 2 or 4"));
        }
        if !nodes::alive(&self.g, output) {
            self.note(format!("{path}: the output is gone"));
            return;
        }
        let ctx = self.ctx;
        let path = path.to_owned();
        let answered_path = path.clone();
        let g = self.g.clone();
        let node = self.g.node(gnoblin_core::host::NodeId(output));
        let sent = node.call_with("apply", &[Value::Text(request)], move |answer| {
            let refusal = answer.error.filter(|error| !error.is_empty());
            // The answer to a call that is refused at once comes while the state is borrowed: it is taken a turn later.
            let busy = state::find(ctx).is_some_and(|std| std.st.try_borrow_mut().is_err());
            if busy {
                let _ = g
                    .poster()
                    .post(move |_g| answered(ctx, output, &answered_path, refusal));
            } else {
                answered(ctx, output, &answered_path, refusal);
            }
        });
        match sent {
            Ok(_) => *self.disp.pending.entry(output).or_insert(0) += 1,
            Err(error) => self.note(format!("{path}: {error}")),
        }
    }

    fn mirror_of(&self, output: NodeId) -> Option<NodeId> {
        let connector = nodes::text(&self.g, output, "mirror_of");
        if connector.is_empty() {
            return None;
        }
        self.outputs
            .iter()
            .copied()
            .find(|other| nodes::text(&self.g, *other, "connector") == connector)
    }

    // ---- displays ------------------------------------------------------------------------------------------------------

    pub(crate) fn new_display(&mut self, id: String, mode: u32, selectors: Vec<Sel>, wanted: Option<Id>) -> Hid {
        let handle = state::next_id();
        self.disp.displays.insert(
            handle,
            DisplayRec {
                id,
                mode,
                selectors,
                implicit: false,
                listed: false,
                live: Vec::new(),
                wanted,
                content: wanted,
                mirrored: Vec::new(),
                span_lead: None,
                viewport: None,
            },
        );
        handle
    }

    pub(crate) fn display(&self, display: Hid) -> Res<&DisplayRec> {
        self.disp
            .displays
            .get(&display)
            .ok_or_else(|| Fail::stale(format!("display {display} does not exist")))
    }

    pub(crate) fn set_display_mode(&mut self, display: Hid, mode: u32) -> Res<()> {
        let rec = self.display(display)?;
        if mode == abi::STD_PRESENT_SINGLE && rec.selectors.len() > 1 {
            return Err(Fail::type_error(format!(
                "Display \"{}\".mode: \"single\" presents one output; use \"mirror\" or \"span\" for {}",
                rec.id,
                rec.selectors.len()
            )));
        }
        if let Some(rec) = self.disp.displays.get_mut(&display) {
            rec.mode = mode;
        }
        Ok(())
    }

    /// The connected outputs `display` presents, in selector order. A profile gives the first match not used yet.
    fn resolve(&self, display: Hid, live: &[NodeId]) -> Vec<NodeId> {
        let mut found: Vec<NodeId> = Vec::new();
        let Some(rec) = self.disp.displays.get(&display) else {
            return found;
        };
        for selector in &rec.selectors {
            let output = match selector {
                Sel::Profile(profile) => live
                    .iter()
                    .copied()
                    .find(|one| !found.contains(one) && self.profile_matches(*profile, *one)),
                Sel::Output(node) => (live.contains(node) && !found.contains(node)).then_some(*node),
            };
            if let Some(output) = output {
                found.push(output);
            }
        }
        found
    }

    /// What `a` and `b` both present, as a phrase, or `None` when they share nothing.
    fn overlap(&self, a: Hid, b: Hid) -> Option<String> {
        let (left, right) = (self.disp.displays.get(&a)?, self.disp.displays.get(&b)?);
        for selector in &left.selectors {
            if right.selectors.contains(selector) {
                return Some(match selector {
                    Sel::Profile(profile) => {
                        format!(
                            "profile \"{}\"",
                            self.disp.profiles.get(profile).map_or("", |rec| rec.id.as_str())
                        )
                    }
                    Sel::Output(node) => format!("output {}", name_of(self, *node)),
                });
            }
        }
        let live = self.outputs.clone();
        let theirs = self.resolve(b, &live);
        let shared = self
            .resolve(a, &live)
            .into_iter()
            .find(|output| theirs.contains(output))?;
        Some(format!("output {}", name_of(self, shared)))
    }

    /// Fails when `display` clashes with a display of the script that is not in `leaving`.
    fn check_add(&self, display: Hid, leaving: &HashSet<Hid>) -> Res<()> {
        let rec = self.display(display)?;
        if rec.mode == abi::STD_PRESENT_SINGLE && rec.selectors.len() > 1 {
            return Err(Fail::type_error(format!(
                "Display \"{}\".mode: \"single\" presents one output; use \"mirror\" or \"span\" for {}",
                rec.id,
                rec.selectors.len()
            )));
        }
        let id = rec.id.clone();
        for other in &self.disp.display_list {
            let Some(held) = self.disp.displays.get(other) else {
                continue;
            };
            if held.implicit || leaving.contains(other) {
                continue;
            }
            if held.id == id {
                return Err(
                    Fail::busy(format!("there is already a display \"{id}\"")).with_code(ErrorCode::AlreadyExists)
                );
            }
            if let Some(shared) = self.overlap(display, *other) {
                return Err(Fail::refused(
                    ErrorCode::Busy,
                    format!(
                        "display \"{id}\" cannot present {shared}: display \"{}\" does already (use gnoblin.displays.replace(\"{}\", ...) to swap them)",
                        held.id, held.id
                    ),
                ));
            }
        }
        Ok(())
    }

    fn insert_display(&mut self, display: Hid, at: usize) {
        // The library's own display of the same name gives way: it is what the script replaces.
        let id = self
            .disp
            .displays
            .get(&display)
            .map(|rec| rec.id.clone())
            .unwrap_or_default();
        let gives_way: Vec<Hid> = self
            .disp
            .display_list
            .iter()
            .copied()
            .filter(|other| {
                self.disp
                    .displays
                    .get(other)
                    .is_some_and(|held| held.implicit && held.id == id)
            })
            .collect();
        for other in gives_way {
            self.drop_display(other);
        }
        let at = at.min(self.disp.display_list.len());
        self.disp.display_list.insert(at, display);
        if let Some(rec) = self.disp.displays.get_mut(&display) {
            rec.listed = true;
        }
    }

    /// Takes a display out: its mirror and span end, and it presents nothing.
    fn drop_display(&mut self, display: Hid) {
        self.release_display(display);
        self.disp.display_list.retain(|one| *one != display);
        self.disp.implicit.retain(|_, held| *held != display);
        let implicit = self.disp.displays.get(&display).is_some_and(|rec| rec.implicit);
        if implicit {
            if let Some(vid) = self.disp.displays.get(&display).and_then(|rec| rec.viewport)
                && let Some(viewport) = self.disp.viewports.get_mut(&vid)
            {
                viewport.display = None;
            }
            self.disp.displays.remove(&display);
        } else if let Some(rec) = self.disp.displays.get_mut(&display) {
            rec.live.clear();
            rec.listed = false;
        }
    }

    pub(crate) fn add_display(&mut self, display: Hid) -> Res<bool> {
        if self.display(display)?.listed {
            return Ok(false);
        }
        self.check_add(display, &HashSet::new())?;
        let end = self.disp.display_list.len();
        self.insert_display(display, end);
        self.announce_displays();
        self.reflow_displays();
        Ok(true)
    }

    pub(crate) fn remove_display(&mut self, display: Hid) -> Res<bool> {
        let rec = self.display(display)?;
        if !rec.listed || rec.implicit {
            return Ok(false);
        }
        self.drop_display(display);
        self.reflow_displays();
        self.announce_displays();
        Ok(true)
    }

    pub(crate) fn replace_display(&mut self, old: Hid, next: Hid) -> Res<()> {
        let old_rec = self.display(old)?;
        if !old_rec.listed || old_rec.implicit {
            return Err(Fail::stale(format!(
                "Display#{} is not a display of the script",
                old_rec.id
            )));
        }
        let next_rec = self.display(next)?;
        if next_rec.listed {
            return Err(
                Fail::busy(format!("Display#{} is in the collection already", next_rec.id))
                    .with_code(ErrorCode::AlreadyExists),
            );
        }
        let mut leaving = HashSet::from([old]);
        for other in &self.disp.display_list {
            let held = self.disp.displays.get(other);
            if held.is_some_and(|held| !held.implicit) && self.overlap(next, *other).is_some() {
                leaving.insert(*other);
            }
        }
        self.check_add(next, &leaving)?;
        let at = self.disp.display_list.iter().position(|one| *one == old).unwrap_or(0);
        let before = self.disp.display_list[..at]
            .iter()
            .filter(|display| !leaving.contains(display))
            .count();
        let leaving: Vec<Hid> = self
            .disp
            .display_list
            .iter()
            .copied()
            .filter(|display| leaving.contains(display))
            .collect();
        for display in leaving {
            self.drop_display(display);
        }
        let at = before.min(self.disp.display_list.len());
        self.insert_display(next, at);
        self.reflow_displays();
        self.announce_displays();
        Ok(())
    }

    pub(crate) fn clear_displays(&mut self) {
        for display in self.disp.display_list.clone() {
            if self.disp.displays.get(&display).is_some_and(|rec| !rec.implicit) {
                self.drop_display(display);
            }
        }
        self.reflow_displays();
        self.announce_displays();
    }

    /// The display under `point`, else the primary output's.
    pub(crate) fn display_at(&mut self, x: f64, y: f64) -> Option<Hid> {
        self.reflow_displays();
        for output in self.outputs.clone() {
            let g = &self.g;
            let (ox, oy) = (nodes::number(g, output, "x"), nodes::number(g, output, "y"));
            let (w, h) = (nodes::number(g, output, "width"), nodes::number(g, output, "height"));
            let inside = x >= ox && x < ox + w && y >= oy && y < oy + h;
            if inside && let Some(owner) = self.disp.owners.get(&output) {
                return Some(*owner);
            }
        }
        let primary = self.primary_output()?;
        self.disp.owners.get(&primary).copied()
    }

    /// The first output of the display when it is connected and has a desk of its own.
    fn desk_lead(&self, display: Hid) -> Option<NodeId> {
        let lead = *self.disp.displays.get(&display)?.live.first()?;
        (self.outputs.contains(&lead) && self.desk_of_output(lead).is_some()).then_some(lead)
    }

    /// The workspace the output shows (the one its span shows, for a spanned output).
    fn shown_on(&self, output: NodeId) -> Option<Id> {
        let desk = self.desk_of_output(output).and_then(|id| self.desk(id))?;
        let effective = desk.spanned_by.unwrap_or(desk.id);
        self.desk(effective).and_then(|desk| desk.shown)
    }

    /// What the display's viewport shows: `Some` while its first output has a desk (the workspace or nothing), `None` while not.
    pub(crate) fn display_shown(&self, display: Hid) -> Option<Option<Id>> {
        self.desk_lead(display).map(|lead| self.shown_on(lead))
    }

    pub(crate) fn assign_display(&mut self, display: Hid, workspace: Id) -> Res<()> {
        self.display(display)?;
        match self.desk_lead(display) {
            None => {
                if let Some(rec) = self.disp.displays.get_mut(&display) {
                    rec.wanted = Some(workspace);
                    rec.content = Some(workspace);
                }
                Ok(())
            }
            Some(lead) => {
                self.show_workspace(workspace, Some(lead), 0.0, 0.0)?;
                if let Some(rec) = self.disp.displays.get_mut(&display) {
                    rec.wanted = None;
                    rec.content = Some(workspace);
                }
                Ok(())
            }
        }
    }

    pub(crate) fn display_outputs(&self, display: Hid) -> Vec<NodeId> {
        self.disp
            .displays
            .get(&display)
            .map(|rec| rec.live.clone())
            .unwrap_or_default()
    }

    /// What a transition that does not commit may have to put back: the workspace each display waits to show and the one it last showed,
    /// and the camera and colour of each viewport.
    pub(crate) fn snapshot_displays_value(&self) -> DisplaysSnapshot {
        DisplaysSnapshot {
            displays: self
                .disp
                .displays
                .iter()
                .map(|(handle, rec)| (*handle, rec.wanted, rec.content))
                .collect(),
            viewports: self
                .disp
                .viewports
                .iter()
                .map(|(vid, rec)| (*vid, rec.camera, rec.clear_color.clone()))
                .collect(),
            desks: self.desks.iter().map(|desk| (desk.id, desk.shown)).collect(),
        }
    }

    /// Puts back what [`State::snapshot_displays_value`] kept, for the displays and viewports that are still there. A transition that
    /// fails puts back the desks too (`desks`): the workspace an assignment showed goes, and the one it hid returns.
    pub(crate) fn restore_displays_value(&mut self, kept: DisplaysSnapshot, desks: bool) {
        for (handle, wanted, content) in kept.displays {
            if let Some(rec) = self.disp.displays.get_mut(&handle) {
                rec.wanted = wanted;
                rec.content = content;
            }
        }
        for (vid, camera, clear_color) in kept.viewports {
            if let Some(rec) = self.disp.viewports.get_mut(&vid) {
                rec.camera = camera;
                rec.clear_color = clear_color;
            }
        }
        if desks {
            for (desk, shown) in kept.desks {
                self.set_shown(desk, shown);
            }
        }
        self.reflow_displays();
    }

    pub(crate) fn snapshot_displays(&mut self) -> u64 {
        let token = state::next_id();
        let kept = self.snapshot_displays_value();
        self.disp.snapshots.insert(token, kept);
        token
    }

    pub(crate) fn restore_displays(&mut self, token: u64) {
        if let Some(kept) = self.disp.snapshots.remove(&token) {
            self.restore_displays_value(kept, false);
        }
    }

    // ---- reconciling ---------------------------------------------------------------------------------------------------

    /// Applies the profiles, settles who presents which output, and enacts each display.
    pub(crate) fn reflow_displays(&mut self) {
        if self.disp.profile_list.is_empty() && self.disp.display_list.is_empty() && self.outputs.is_empty() {
            return;
        }
        let before = self.display_signature();
        let live = self.outputs.clone();
        self.apply_profiles(&live);
        let mut owners: HashMap<NodeId, Hid> = HashMap::new();
        for display in self.disp.display_list.clone() {
            if self.disp.displays.get(&display).is_none_or(|rec| rec.implicit) {
                continue;
            }
            let resolved = self.resolve(display, &live);
            let id = self.disp.displays[&display].id.clone();
            let mut kept = Vec::new();
            for output in resolved {
                if let Some(held) = owners.get(&output) {
                    let held_id = self
                        .disp
                        .displays
                        .get(held)
                        .map(|rec| rec.id.clone())
                        .unwrap_or_default();
                    let name = name_of(self, output);
                    self.note(format!(
                        "not applied: display \"{id}\" presents {name}, which display \"{held_id}\" already does"
                    ));
                    continue;
                }
                owners.insert(output, display);
                kept.push(output);
            }
            if let Some(rec) = self.disp.displays.get_mut(&display) {
                rec.live = kept;
            }
        }
        self.sync_implicit(&live, &mut owners);
        self.disp.owners = owners;
        for display in self.disp.display_list.clone() {
            self.enact(display);
        }
        if self.display_signature() != before {
            self.announce_displays();
        }
    }

    /// The lists and what each display presents, to tell whether a reflow changed anything the shell reads.
    fn display_signature(&self) -> Vec<(Hid, Vec<NodeId>)> {
        self.disp
            .display_list
            .iter()
            .map(|display| {
                (
                    *display,
                    self.disp
                        .displays
                        .get(display)
                        .map(|rec| rec.live.clone())
                        .unwrap_or_default(),
                )
            })
            .collect()
    }

    /// Gives each output with a desk that no display of the script presents a display of the library's, and retires the rest.
    fn sync_implicit(&mut self, live: &[NodeId], owners: &mut HashMap<NodeId, Hid>) {
        for (output, display) in self.disp.implicit.clone() {
            if !live.contains(&output) || self.desk_of_output(output).is_none() || owners.contains_key(&output) {
                self.drop_display(display);
            }
        }
        for output in live {
            if owners.contains_key(output) || self.desk_of_output(*output).is_none() {
                continue;
            }
            let display = match self.disp.implicit.get(output) {
                Some(display) => *display,
                None => {
                    let handle = state::next_id();
                    self.disp.displays.insert(
                        handle,
                        DisplayRec {
                            id: format!("display-{}", name_of(self, *output)),
                            mode: abi::STD_PRESENT_SINGLE,
                            selectors: vec![Sel::Output(*output)],
                            implicit: true,
                            listed: true,
                            live: Vec::new(),
                            wanted: None,
                            content: None,
                            mirrored: Vec::new(),
                            span_lead: None,
                            viewport: None,
                        },
                    );
                    self.disp.implicit.insert(*output, handle);
                    self.disp.display_list.push(handle);
                    handle
                }
            };
            if let Some(rec) = self.disp.displays.get_mut(&display) {
                rec.live = vec![*output];
            }
            owners.insert(*output, display);
        }
    }

    /// Undoes what a display did to its outputs: the mirrors it made and the span it holds.
    fn release_display(&mut self, display: Hid) {
        self.unmirror(display);
        self.end_span(display);
    }

    fn unmirror(&mut self, display: Hid) {
        let Some(rec) = self.disp.displays.get_mut(&display) else {
            return;
        };
        let id = rec.id.clone();
        let mirrored = std::mem::take(&mut rec.mirrored);
        for (output, source) in mirrored {
            if self.outputs.contains(&output) && self.mirror_of(output) == Some(source) {
                self.send_apply(output, "mirror_of=".to_owned(), &format!("displays.{id}"), None);
            }
        }
    }

    fn end_span(&mut self, display: Hid) {
        let Some(lead) = self
            .disp
            .displays
            .get_mut(&display)
            .and_then(|rec| rec.span_lead.take())
        else {
            return;
        };
        if self.outputs.contains(&lead) {
            self.span_desks(lead, None);
        }
    }

    /// Brings the hardware and the desks to what `display` says.
    fn enact(&mut self, display: Hid) {
        let Some(rec) = self.disp.displays.get(&display) else {
            return;
        };
        let (mode, live) = (rec.mode, rec.live.clone());
        let Some((&lead, rest)) = live.split_first() else {
            return;
        };
        let members: Vec<NodeId> = if mode == abi::STD_PRESENT_SINGLE {
            Vec::new()
        } else {
            rest.to_vec()
        };
        if mode == abi::STD_PRESENT_MIRROR && !members.is_empty() {
            self.mirror(display, lead, &members);
        } else {
            self.unmirror(display);
        }
        if mode == abi::STD_PRESENT_SPAN && !members.is_empty() {
            if self.disp.displays.get(&display).and_then(|rec| rec.span_lead) != Some(lead) {
                self.end_span(display);
            }
            let mut all = vec![lead];
            all.extend(members);
            self.span_desks(lead, Some(&all));
            if let Some(rec) = self.disp.displays.get_mut(&display) {
                rec.span_lead = Some(lead);
            }
        } else {
            self.end_span(display);
        }
        self.settle_content(display, lead);
    }

    fn mirror(&mut self, display: Hid, lead: NodeId, members: &[NodeId]) {
        let Some(rec) = self.disp.displays.get(&display) else {
            return;
        };
        let id = rec.id.clone();
        let connector = nodes::text(&self.g, lead, "connector");
        for output in members {
            if let Some(rec) = self.disp.displays.get_mut(&display) {
                match rec.mirrored.iter_mut().find(|(held, _)| held == output) {
                    Some(entry) => entry.1 = lead,
                    None => rec.mirrored.push((*output, lead)),
                }
            }
            if self.mirror_of(*output) != Some(lead) && !self.is_pending(*output) {
                self.send_apply(
                    *output,
                    format!("mirror_of={connector}"),
                    &format!("displays.{id}"),
                    None,
                );
            }
        }
    }

    /// Shows the workspace a display waits to show, and remembers what the desk shows for when the display is parked.
    fn settle_content(&mut self, display: Hid, lead: NodeId) {
        if self.desk_of_output(lead).is_none() {
            return;
        }
        let Some(rec) = self.disp.displays.get(&display) else {
            return;
        };
        let (id, wanted) = (rec.id.clone(), rec.wanted);
        if let Some(workspace) = wanted {
            if self.ws.get(&workspace).is_some_and(|ws| ws.attached) {
                let _ = self.show_workspace(workspace, Some(lead), 0.0, 0.0);
                if let Some(rec) = self.disp.displays.get_mut(&display) {
                    rec.wanted = None;
                }
            } else {
                self.note(format!(
                    "not applied: display \"{id}\" shows a workspace that is not in gnoblin.workspaces (add it first)"
                ));
            }
        }
        if let Some(shown) = self.shown_on(lead)
            && let Some(rec) = self.disp.displays.get_mut(&display)
        {
            rec.content = Some(shown);
        }
    }
}
