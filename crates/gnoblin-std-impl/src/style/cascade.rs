//! The cascade: the layers a client sits under, resolved into one style, and what the resolved style becomes on the node.
//!
//! `data/js/gnoblin-std/host/style_scopes.ts`, `host/styles.ts`, `style/resolve.ts` and `style/map.ts`. A scope holds only what was set
//! on it; a property nobody set is inherited. The layers of a client, lowest first, are the global style (the root collection of
//! clients), its workspace's, the `where` selections that hold it (in the order they first got a style) and its own. Per property the
//! highest layer that sets it wins, and a list a layer sets replaces the inherited list whole.
//!
//! The application runs in the turn, after the layout pass: when any style changed, or the layers of a client changed, the client's
//! effects (shadows, borders, the backdrop blur), its opacity and its corner radius are written to its node. What the tree cannot
//! express is kept as a note the client reports (`style_notes`), never faked.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

use gnoblin_core::{Effect, EffectKind, NodeId};

use super::values::{Entry, ListProp, MAX_GRADE, MAX_SHADERS, MAX_STYLE_LIST, Prop, Radii, ScalarProp};
use crate::animation::staging;
use crate::collection::Source;
use crate::state::{Id, State};

/// Where a style lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Scope {
    /// A collection of clients: the root, a workspace's clients or a selection.
    Collection(Id),
    /// One client.
    Client(NodeId64),
}

/// A client's node id.
pub(crate) type NodeId64 = u64;

/// The effects a generation left on a window's node.
#[derive(Clone, Copy, Debug, PartialEq)]
enum EffectsLeft {
    /// None: the list it wrote was empty, or it wrote none.
    None,
    /// A list the core could not number (a core from before `node_effects_serial`): the node's effects are taken to be it.
    Unknown,
    /// A list written at this serial of the core (`Node::effects_serial`): the node holds it while the serial is the same.
    Serial(u64),
}

/// What a generation wrote to a window's node and left there: the opacity and the corner radius it set when they are not what a window
/// with no style holds, and the effects. This is what the generation that takes over may take off, no more.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Left {
    opacity: Option<f64>,
    corner: Option<f64>,
    effects: EffectsLeft,
}

impl Left {
    /// Whether the node holds `wrote` still: a value somebody else wrote since is not this style's to take off. The tolerance is the
    /// compositor's single-precision storage of a double.
    fn holds(read: Result<f64, gnoblin_core::CoreError>, wrote: f64) -> bool {
        read.is_ok_and(|read| (read - wrote).abs() <= 1e-6 * wrote.abs().max(1.0))
    }

    /// Whether the node still holds the effects this left: the core numbers every write of a node's effects, and a list that was
    /// written since (by a generation that started after this one, or by the shell's own node writes) has another serial. The effects
    /// cannot be read, so a serial that cannot be read is no proof, and the list is left.
    fn effects_held(&self, node: &gnoblin_core::Node) -> bool {
        match self.effects {
            EffectsLeft::None => false,
            EffectsLeft::Unknown => true,
            EffectsLeft::Serial(wrote) => node.effects_serial().is_ok_and(|now| now == wrote),
        }
    }
}

thread_local! {
    /// The windows a generation that retired left styled, with what it left on each, for the generation that takes over. A reload
    /// commits the new generation before the old one retires, and the new one's first turn comes after, so the old one hands its
    /// windows over here. An owning successor writes the style it holds for each of them, an empty one included; one with no style of
    /// its own takes off what the old one wrote and the node still holds. The set is the shell thread's, as every generation of the
    /// shell is. Compositor node ids are never handed out twice in a session, so an entry that is never taken costs one write at most.
    static HANDOVER: RefCell<HashMap<NodeId64, Left>> = RefCell::new(HashMap::new());
}

/// A scalar a scope sets.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Scalar {
    Opacity(f64),
    CornerRadius(Radii),
    Background(Option<String>),
}

/// What one scope sets.
#[derive(Debug)]
pub(crate) struct Store {
    pub(crate) scope: Scope,
    pub(crate) scalars: BTreeMap<ScalarProp, Scalar>,
    pub(crate) lists: BTreeMap<ListProp, Vec<Entry>>,
}

/// What the last application wrote for one client.
#[derive(Debug, Default)]
struct Applied {
    applied: bool,
    epoch: u64,
    layers: String,
    computed: Option<Computed>,
    notes: Vec<String>,
    effects: Option<Vec<Effect>>,
    /// The core's serial of the write that put `effects` on the node, when it could say.
    effects_serial: Option<u64>,
    opacity: Option<f64>,
    corner: Option<f64>,
}

impl Applied {
    /// What this application wrote and the node still carries: an effect, an opacity other than 1 or a corner radius.
    fn left_on_node(&self) -> Option<Left> {
        let left = Left {
            opacity: self.opacity.filter(|opacity| *opacity != 1.0),
            corner: self.corner.filter(|corner| *corner != 0.0),
            effects: match (&self.effects, self.effects_serial) {
                (Some(effects), Some(serial)) if !effects.is_empty() => EffectsLeft::Serial(serial),
                (Some(effects), None) if !effects.is_empty() => EffectsLeft::Unknown,
                _ => EffectsLeft::None,
            },
        };
        (left.opacity.is_some() || left.corner.is_some() || left.effects != EffectsLeft::None).then_some(left)
    }
}

/// What every style of a generation shares.
#[derive(Debug, Default)]
pub(crate) struct StyleState {
    pub(crate) stores: HashMap<Id, Store>,
    pub(crate) by_scope: HashMap<Scope, Id>,
    /// The selections that have a style, in the order they first got one: the tie-break between selections.
    pub(crate) selections: Vec<Id>,
    /// What a selection ranks by among the selections that style a client: higher wins (`style_rank`); absent is 0.
    ranks: HashMap<Id, f64>,
    /// The library owns the styles of the windows (`style_reset` or a first scope): its pass applies them, an empty style included.
    owned: bool,
    /// An earlier script generation may have left effects on the windows (the library held styles or applied ones when `style_reset`
    /// ran, or this generation is a reload): the tree's effects cannot be read back, so the first application of a window writes its
    /// list, an empty one included. A first start has nothing to take off, and writes nothing for an empty list.
    inherited: bool,
    /// The windows an earlier generation left styled (`HANDOVER`), with what it wrote to each, that this generation has not dealt with
    /// yet. A generation that holds styles writes its own for each, an empty list included. One that holds none takes off what the
    /// earlier one wrote and the node still holds, so a reload that drops the styles takes them off the windows and leaves a value the
    /// new generation wrote itself.
    carried: HashMap<NodeId64, Left>,
    /// Goes up by one for every accepted change of any style; the application compares it between turns.
    pub(crate) epoch: u64,
    clients: HashMap<u64, Applied>,
}

impl StyleState {
    /// Drops every scope and what was applied, and makes the library the styles' owner: a script generation that starts over a library
    /// state an earlier one left (a reload that stays in one core generation) styles on a clean slate, and its first pass takes
    /// what the earlier one drew off the windows.
    pub(crate) fn reset(&mut self, reloaded: bool) {
        self.inherited = reloaded || !self.stores.is_empty() || !self.clients.is_empty();
        self.stores.clear();
        self.by_scope.clear();
        self.selections.clear();
        self.ranks.clear();
        self.clients.clear();
        self.owned = true;
        self.epoch += 1;
    }

    /// The rank of the selection `coll`.
    pub(crate) fn rank(&self, coll: Id) -> f64 {
        self.ranks.get(&coll).copied().unwrap_or(0.0)
    }

    /// Sets the rank of the selection `coll`.
    pub(crate) fn set_rank(&mut self, coll: Id, rank: f64) {
        self.ranks.insert(coll, rank);
        self.epoch += 1;
    }

    /// What every style holds now.
    pub(crate) fn snapshot(&self) -> StyleSnapshot {
        StyleSnapshot {
            stores: self
                .stores
                .iter()
                .map(|(id, store)| (*id, (store.scalars.clone(), store.lists.clone())))
                .collect(),
            ranks: self.ranks.clone(),
        }
    }

    /// Puts back what a snapshot held: a style that is not in it (one made since) is emptied.
    pub(crate) fn restore(&mut self, snapshot: StyleSnapshot) {
        let StyleSnapshot { mut stores, ranks } = snapshot;
        for (id, store) in &mut self.stores {
            (store.scalars, store.lists) = stores.remove(id).unwrap_or_default();
        }
        self.ranks = ranks;
        self.epoch += 1;
    }
}

/// What one style holds.
type Held = (BTreeMap<ScalarProp, Scalar>, BTreeMap<ListProp, Vec<Entry>>);

/// What the styles held at one moment: a transition that does not commit puts it back.
#[derive(Clone, Debug)]
pub(crate) struct StyleSnapshot {
    stores: HashMap<Id, Held>,
    ranks: HashMap<Id, f64>,
}

/// A layer of a client's cascade.
#[derive(Clone, Debug)]
pub(crate) struct Layer {
    /// `default`, `global`, `workspace:<id>`, `where(<query>)` or `client`.
    pub(crate) origin: String,
    pub(crate) store: Id,
}

/// A client's resolved style.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Computed {
    pub(crate) opacity: f64,
    pub(crate) corner_radius: Radii,
    pub(crate) background: Option<String>,
    pub(crate) lists: BTreeMap<ListProp, Vec<Entry>>,
    pub(crate) origin: BTreeMap<&'static str, String>,
}

/// What a resolved style becomes on the node, and what of it nothing draws.
#[derive(Debug)]
pub(crate) struct Mapping {
    pub(crate) effects: Vec<Effect>,
    pub(crate) corner_radius: f64,
    pub(crate) unapplied: Vec<String>,
}

/// The compositor's fixed backdrop blur radius.
const BACKDROP_BLUR_RADIUS: f64 = 12.0;

/// The largest blur a `filter` effect takes, in logical pixels (`gnoblin-render`'s `MAX_BLUR_RADIUS`).
const MAX_FILTER_BLUR: f64 = 100.0;

impl Computed {
    pub(crate) fn list(&self, prop: ListProp) -> &[Entry] {
        self.lists.get(&prop).map_or(&[], Vec::as_slice)
    }
}

impl State {
    /// The path of a collection of clients in a failure: `gnoblin.clients`, `Workspace#dev.clients`, then `.where({...})` for each
    /// selection.
    fn coll_path(&self, id: Id) -> String {
        let Some(coll) = self.colls.get(&id) else {
            return "collection".to_owned();
        };
        if let Some(parent) = coll.parent {
            let terms: Vec<String> = coll
                .query
                .iter()
                .map(|(key, wants)| {
                    let render = |want: &crate::collection::Want| match want {
                        crate::collection::Want::Text(text) => format!("{text:?}"),
                        crate::collection::Want::Number(number) => number.to_string(),
                        crate::collection::Want::Bool(flag) => flag.to_string(),
                    };
                    let value = match wants.as_slice() {
                        [one] => render(one),
                        many => format!("[{}]", many.iter().map(render).collect::<Vec<_>>().join(",")),
                    };
                    format!("\"{key}\":{value}")
                })
                .collect();
            return format!("{}.where({{{}}})", self.coll_path(parent), terms.join(","));
        }
        match coll.source {
            Some(Source::WorkspaceClients(ws)) => format!(
                "Workspace#{}.clients",
                self.ws.get(&ws).map_or_else(|| "?".to_owned(), |ws| ws.id.clone())
            ),
            _ => "gnoblin.clients".to_owned(),
        }
    }

    /// The front of an error path for a scope's style.
    pub(crate) fn style_path(&self, scope: Scope) -> String {
        match scope {
            Scope::Collection(id) => format!("{}.style", self.coll_path(id)),
            Scope::Client(node) => format!("Client#{node}.style"),
        }
    }

    /// The name `origin` gives a scope.
    fn coll_origin(&self, id: Id) -> String {
        let Some(coll) = self.colls.get(&id) else {
            return "global".to_owned();
        };
        if coll.parent.is_some() {
            return self.coll_path(id).replacen("gnoblin.clients.", "", 1);
        }
        match coll.source {
            Some(Source::WorkspaceClients(ws)) => format!(
                "workspace:{}",
                self.ws.get(&ws).map_or_else(|| "?".to_owned(), |ws| ws.id.clone())
            ),
            _ => "global".to_owned(),
        }
    }

    /// The root collection a selection narrows.
    fn root_of(&self, mut id: Id) -> Id {
        while let Some(parent) = self.colls.get(&id).and_then(|coll| coll.parent) {
            id = parent;
        }
        id
    }

    fn layer_of(&self, scope: Scope, origin: String) -> Option<Layer> {
        self.styles
            .by_scope
            .get(&scope)
            .map(|store| Layer { origin, store: *store })
    }

    /// The scopes a client sits under, lowest precedence first: global, its workspace, then its `where` selections.
    fn scope_layers(&self, client: NodeId64) -> Vec<Layer> {
        let mut layers = Vec::new();
        layers.extend(self.layer_of(Scope::Collection(self.roots[0]), "global".to_owned()));
        if let Some(ws) = self
            .rec
            .get(&client)
            .and_then(|rec| rec.workspace)
            .and_then(|ws| self.ws.get(&ws))
        {
            layers.extend(self.layer_of(Scope::Collection(ws.coll), format!("workspace:{}", ws.id)));
        }
        // Higher rank beats lower; selections of one rank keep the order they first got a style (the sort is stable).
        let mut held: Vec<Id> = self
            .styles
            .selections
            .iter()
            .copied()
            .filter(|selection| self.contains(*selection, client))
            .collect();
        held.sort_by(|a, b| self.styles.rank(*a).total_cmp(&self.styles.rank(*b)));
        for selection in held {
            layers.extend(self.layer_of(Scope::Collection(selection), self.coll_origin(selection)));
        }
        layers
    }

    /// The layers below a scope: what a list it does not override is inherited from.
    pub(crate) fn layers_below(&self, scope: Scope) -> Vec<Layer> {
        match scope {
            Scope::Client(client) => self.scope_layers(client),
            Scope::Collection(id) if id == self.roots[0] => Vec::new(),
            Scope::Collection(id) => {
                let mut layers = Vec::new();
                layers.extend(self.layer_of(Scope::Collection(self.roots[0]), "global".to_owned()));
                let root = self.root_of(id);
                if root != self.roots[0] && root != id {
                    layers.extend(self.layer_of(Scope::Collection(root), self.coll_origin(root)));
                }
                layers
            }
        }
    }

    /// `scope_layers` and the client's own overrides on top.
    pub(crate) fn client_layers(&self, client: NodeId64) -> Vec<Layer> {
        let mut layers = self.scope_layers(client);
        layers.extend(self.layer_of(Scope::Client(client), "client".to_owned()));
        layers
    }

    /// The list the highest layer that overrides it holds, or none.
    pub(crate) fn layered_list(&self, layers: &[Layer], prop: ListProp) -> Vec<Entry> {
        layers
            .iter()
            .rev()
            .find_map(|layer| self.styles.stores.get(&layer.store)?.lists.get(&prop))
            .cloned()
            .unwrap_or_default()
    }

    /// The style the layers resolve to: per property, the highest layer that sets it wins.
    pub(crate) fn resolve_style(&self, layers: &[Layer]) -> Computed {
        let mut computed = Computed {
            opacity: 1.0,
            corner_radius: Radii::All(0.0),
            background: None,
            lists: BTreeMap::new(),
            origin: BTreeMap::new(),
        };
        for prop in Prop::ALL {
            computed.origin.insert(prop.name(), "default".to_owned());
        }
        for prop in [ScalarProp::Opacity, ScalarProp::CornerRadius, ScalarProp::Background] {
            let hit = layers.iter().rev().find_map(|layer| {
                let scalar = self.styles.stores.get(&layer.store)?.scalars.get(&prop)?;
                Some((layer, scalar))
            });
            if let Some((layer, scalar)) = hit {
                match scalar {
                    Scalar::Opacity(value) => computed.opacity = *value,
                    Scalar::CornerRadius(value) => computed.corner_radius = *value,
                    Scalar::Background(value) => computed.background.clone_from(value),
                }
                computed.origin.insert(Prop::Scalar(prop).name(), layer.origin.clone());
            }
        }
        for prop in [
            ListProp::Borders,
            ListProp::Shadows,
            ListProp::Filter,
            ListProp::BackdropFilter,
        ] {
            let hit = layers.iter().rev().find_map(|layer| {
                let list = self.styles.stores.get(&layer.store)?.lists.get(&prop)?;
                Some((layer, list))
            });
            computed
                .lists
                .insert(prop, hit.map_or_else(Vec::new, |(_, list)| list.clone()));
            if let Some((layer, _)) = hit {
                computed.origin.insert(Prop::List(prop).name(), layer.origin.clone());
            }
        }
        computed
    }

    /// Whether a style of any scope changed: the next turn applies it.
    pub(crate) fn style_changed(&mut self) {
        self.styles.epoch += 1;
    }

    /// Writes the resolved style of every client whose style or layers changed since it was last applied. `takes_over` is the turn of a
    /// generation that serves: only it takes the windows an earlier generation handed over. A pass that runs inside a script's call
    /// (`settle`) may be a generation that is only staged, and one that fails must leave the windows to the one that serves.
    pub(crate) fn apply_styles(&mut self, takes_over: bool) {
        if takes_over {
            self.styles
                .carried
                .extend(HANDOVER.with(|left| std::mem::take(&mut *left.borrow_mut())));
        }
        // A shell that keeps its styles itself (a native one, which has made no scope here) has this pass leave its windows alone: the
        // first scope made here, or `style_reset`, is what makes the library the styles' owner. The windows an earlier generation left
        // styled are the exception: whoever takes over restyles them, and with nothing to say that is taking the style off.
        let owning = !self.styles.by_scope.is_empty() || self.styles.owned;
        if !owning && self.styles.carried.is_empty() {
            return;
        }
        let epoch = self.styles.epoch;
        for client in self.clients.clone() {
            let carried = self.styles.carried.remove(&client);
            if !owning {
                if let Some(left) = carried {
                    self.take_off(client, &left);
                }
                continue;
            }
            let layers = self.client_layers(client);
            let key = layers
                .iter()
                .map(|layer| layer.origin.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            // What was applied is compared first and the window's liveness asked only for one that has something to apply: a window
            // that is gone and one that is up to date are both left alone, and the second needs no trip to the core.
            if self
                .styles
                .clients
                .get(&client)
                .is_some_and(|seen| seen.applied && seen.epoch == epoch && seen.layers == key)
                || !crate::nodes::alive(&self.g, client)
            {
                continue;
            }
            let seen = self.styles.clients.entry(client).or_default();
            seen.applied = true;
            seen.epoch = epoch;
            seen.layers = key;
            let computed = self.resolve_style(&layers);
            // The first application of a window the earlier generation left effects on writes this generation's list, an empty one
            // included, unless the list on the node is no longer that generation's.
            let take_over = carried.is_some_and(|left| left.effects_held(&self.g.node(NodeId(client))));
            self.apply_client_style(client, computed, take_over);
        }
        // Every window this generation knows has had its turn: a handed-over window it never meets is gone.
        self.styles.carried.clear();
    }

    /// The generation ends. A window it left styled (an effect, an opacity other than 1 or a corner radius on the node, or one it was
    /// handed and never restyled) is the next generation's to restyle when `successor` says one is open: it is handed over, so a
    /// reload that keeps the style writes it once more and one that drops it takes it off. With no generation to take over (a
    /// shutdown, or a reload to a shell that does not use the library) the style comes off the windows now. Whether windows were
    /// handed over.
    pub(crate) fn retire_styles(&mut self, successor: bool) -> bool {
        let mut left: HashMap<NodeId64, Left> = std::mem::take(&mut self.styles.carried);
        // What this generation applied is newer than what it was handed, and a window it applied bare has nothing left.
        for (client, applied) in &self.styles.clients {
            match applied.left_on_node() {
                Some(now) => {
                    left.insert(*client, now);
                }
                None if applied.applied => {
                    left.remove(client);
                }
                None => {}
            }
        }
        // A window that is gone has no style to hand on or take off.
        left.retain(|client, _| crate::nodes::alive(&self.g, *client));
        if left.is_empty() {
            return false;
        }
        if successor {
            HANDOVER.with(|handed| handed.borrow_mut().extend(left));
            return true;
        }
        for (client, left) in left {
            self.take_off(client, &left);
        }
        false
    }

    /// Puts back what a window with no style holds, for what `left` says an earlier generation wrote: an opacity, a corner radius and
    /// effects only while the node still holds them (a later write of the shell's own stays; the effects are told by the core's serial
    /// of their last write). A window whose opacity a running transition plays keeps it: that is the script's own.
    fn take_off(&self, client: NodeId64, left: &Left) {
        if !crate::nodes::alive(&self.g, client) {
            return;
        }
        let node = self.g.node(NodeId(client));
        if left.effects_held(&node) {
            let _ = node.set_effects(&[]);
        }
        if let Some(wrote) = left.opacity
            && !self.plays(client, "opacity")
            && Left::holds(node.get_f64("opacity"), wrote)
        {
            let _ = node.set("opacity", 1.0);
        }
        if let Some(wrote) = left.corner
            && Left::holds(node.get_f64("corner_radius"), wrote)
        {
            let _ = node.set("corner_radius", 0.0);
        }
    }

    fn apply_client_style(&mut self, client: NodeId64, computed: Computed, take_over: bool) {
        let g = self.g.clone();
        let node = g.node(NodeId(client));
        // The seat says whether the compositor accepts a shader effect (`compositor.user-shaders`); a tree with no seat, or a seat
        // that does not carry the property (a compositor from before the pass), does not.
        let user_shaders = crate::nodes::seat(&g).is_some_and(|seat| seat.get_bool("user_shaders").unwrap_or(false));
        let mapping = map_style(&computed, user_shaders);
        let mut notes = mapping.unapplied.clone();
        let mut applied = self.styles.clients.remove(&client).unwrap_or_default();
        if mapping.effects.len() > MAX_STYLE_LIST * 2 {
            notes.push(format!(
                "Client#{client}.style: a window carries at most {} effects, the style makes {}",
                MAX_STYLE_LIST * 2,
                mapping.effects.len()
            ));
        } else {
            // A window an earlier generation styled keeps its effects: the first application clears them when the style has none. The
            // tree's effects cannot be read back, so over a library that is a reload or holds an earlier script's state (`inherited`),
            // and for a window whose effects an earlier generation left and the node still holds (`take_over`), the first application
            // writes even an empty list; at a first start an empty list is what the window has.
            let unchanged = match &applied.effects {
                None => !(self.styles.inherited || take_over) && mapping.effects.is_empty(),
                Some(written) => *written == mapping.effects,
            };
            if !unchanged {
                let _ = node.set_effects(&mapping.effects);
                // Read straight after the write: the list is this application's while the node's serial is this one.
                applied.effects_serial = node.effects_serial().ok();
            }
            applied.effects = Some(mapping.effects.clone());
        }
        // The first application takes the opacity of a window an earlier generation styled back to what the style says. A window whose
        // opacity a running transition plays is the script's own: the tree holds the transition's target, which no style left there.
        let opacity_before = applied.opacity.or_else(|| {
            if self.plays(client, "opacity") {
                Some(computed.opacity)
            } else {
                node.get_f64("opacity").ok()
            }
        });
        if opacity_before != Some(computed.opacity) {
            write_number(&g, client, "opacity", computed.opacity);
        }
        applied.opacity = Some(computed.opacity);
        let corner_before = applied.corner.or_else(|| node.get_f64("corner_radius").ok());
        if corner_before != Some(mapping.corner_radius) {
            write_number(&g, client, "corner_radius", mapping.corner_radius);
        }
        applied.corner = Some(mapping.corner_radius);
        // What the style asks for and the window does not get is said once per generation, as every sentence the library has for the
        // shell's diagnostics is (`note`); `style_warnings` keeps the list for the window it is about.
        for note in &notes {
            self.note(format!("not applied: style.{note}"));
        }
        applied.notes = notes;
        applied.computed = Some(computed);
        self.styles.clients.insert(client, applied);
    }

    /// A client's resolved style: what was last applied for one that is gone, else what its layers resolve to now.
    pub(crate) fn computed_style(&self, client: NodeId64) -> Computed {
        if !crate::nodes::alive(&self.g, client) {
            return self
                .styles
                .clients
                .get(&client)
                .and_then(|applied| applied.computed.clone())
                .unwrap_or_else(|| self.resolve_style(&[]));
        }
        self.resolve_style(&self.client_layers(client))
    }

    /// What a client's style asks for that is not drawn, then what the compositor reports it skipped.
    pub(crate) fn style_warnings(&self, client: NodeId64) -> Vec<String> {
        let mut warnings: Vec<String> = self
            .styles
            .clients
            .get(&client)
            .map(|applied| {
                applied
                    .notes
                    .iter()
                    .map(|note| format!("not applied: style.{note}"))
                    .collect()
            })
            .unwrap_or_default();
        if let Ok(skipped) = self.g.node(NodeId(client)).get("skipped")
            && let gnoblin_core::Value::Texts(skipped) = skipped
        {
            warnings.extend(
                skipped
                    .into_iter()
                    .map(|text| format!("the compositor does not draw the effect {text}")),
            );
        }
        warnings
    }
}

/// A plain write of a number, held when a transition is staging and otherwise made at once (letting go of a property a transition
/// still plays).
fn write_number(g: &gnoblin_core::Gnoblin, node: u64, key: &str, value: f64) {
    if staging::capture(node, key, &staging::Prop::Number(value)) {
        return;
    }
    staging::released(node, key);
    let _ = g.node(NodeId(node)).set(key, value);
}

/// A colour as the wire spells it: `#rrggbbaa`, lower case, with a full alpha added to a six-digit colour.
fn wire_colour(text: &str) -> String {
    let mut digits = text.trim_start_matches('#').to_ascii_lowercase();
    if digits.len() <= 4 {
        digits = digits.chars().flat_map(|digit| [digit, digit]).collect();
    }
    if digits.len() == 6 {
        digits.push_str("ff");
    }
    format!("#{digits}")
}

/// What a resolved style becomes on the node: a node `corner_radius`, and the `effects` list in paint order. Outer `shadows` are
/// `shadow` effects that follow the window's whole frame, inset `shadows` are `shadow` effects with `inset` that follow its own
/// shape, `borders` are `border` effects and the `backdropFilter` is one backdrop blur whose saturation, brightness and contrast are
/// the products of the chain's. A `ShaderFilter` of either filter list is one `shader` effect (the `backdropFilter`'s first), placed
/// after the blur: it is written only while `user_shaders` (the seat's capability) is on, and at most [`MAX_SHADERS`] of them. The
/// `background` is one `background` effect. The `filter`'s other elements are one `filter` effect: its blurs add as Gaussians do (the
/// root of the sum of their squares) and its saturation, brightness and contrast are the products of the chain's, as for the
/// backdrop. Whether the renderer draws a kind is the compositor's to say (the node's `skipped`), not the style's.
pub(crate) fn map_style(computed: &Computed, user_shaders: bool) -> Mapping {
    let mut effects = Vec::new();
    let mut unapplied = Vec::new();
    let backdrop = computed.list(ListProp::BackdropFilter);
    let blurs: Vec<f64> = backdrop
        .iter()
        .filter_map(|entry| {
            if let Entry::Blur(radius) = entry {
                Some(*radius)
            } else {
                None
            }
        })
        .collect();
    let grade =
        |pick: fn(&Entry) -> Option<f64>| backdrop.iter().filter_map(pick).fold(1.0, |acc, amount| acc * amount);
    let saturation = grade(|entry| {
        if let Entry::Saturate(amount) = entry {
            Some(*amount)
        } else {
            None
        }
    });
    let brightness = grade(|entry| {
        if let Entry::Brightness(amount) = entry {
            Some(*amount)
        } else {
            None
        }
    });
    let contrast = grade(|entry| {
        if let Entry::Contrast(amount) = entry {
            Some(*amount)
        } else {
            None
        }
    });
    if blurs.iter().any(|radius| *radius > 0.0) {
        if blurs
            .iter()
            .any(|radius| *radius > 0.0 && *radius != BACKDROP_BLUR_RADIUS)
        {
            unapplied.push(format!(
                "backdropFilter: the compositor blurs at a fixed radius of {BACKDROP_BLUR_RADIUS}, so a BlurFilter's radius only switches the blur on"
            ));
        }
        if [saturation, brightness, contrast].iter().any(|n| *n > MAX_GRADE) {
            unapplied.push(format!(
                "backdropFilter: a combined saturate, brightness or contrast above {MAX_GRADE} is drawn as {MAX_GRADE}"
            ));
        }
        let mut blur = Effect::new(EffectKind::Blur);
        blur.saturation = saturation.min(MAX_GRADE);
        blur.brightness = brightness.min(MAX_GRADE);
        blur.contrast = contrast.min(MAX_GRADE);
        effects.push(blur);
    } else if [saturation, brightness, contrast].iter().any(|n| *n != 1.0) {
        unapplied.push(
            "backdropFilter: saturate, brightness and contrast are drawn with the backdrop blur, and there is none in the chain"
                .to_owned(),
        );
    }
    let mut shaders = Vec::new();
    for (list, name) in [
        (ListProp::BackdropFilter, "backdropFilter"),
        (ListProp::Filter, "filter"),
    ] {
        let held: Vec<&Entry> = computed
            .list(list)
            .iter()
            .filter(|entry| matches!(entry, Entry::Shader { .. }))
            .collect();
        if held.is_empty() {
            continue;
        }
        if user_shaders {
            shaders.extend(held);
        } else {
            unapplied.push(format!(
                "{name}: a ShaderFilter is not drawn because the compositor's custom shaders are switched off (`compositor.user-shaders` is false); the window is drawn without it"
            ));
        }
    }
    if shaders.len() > MAX_SHADERS {
        unapplied.push(format!(
            "filter: a window draws at most {MAX_SHADERS} shaders, so the first {MAX_SHADERS} of the {} ShaderFilters are drawn",
            shaders.len()
        ));
        shaders.truncate(MAX_SHADERS);
    }
    for entry in shaders {
        if let Entry::Shader { source, params } = entry {
            effects.push(Effect::shader(source, params));
        }
    }
    if let Some(background) = &computed.background {
        let mut fill = Effect::new(EffectKind::Background);
        fill.colour = wire_colour(background);
        effects.push(fill);
    }
    if let Some(filter) = map_filter(computed.list(ListProp::Filter), &mut unapplied) {
        effects.push(filter);
    }
    // The style's first shadow is the top one, as in CSS, and a list's later effects paint on top: the list is walked backwards.
    let shadows = computed.list(ListProp::Shadows);
    for inset in [false, true] {
        for entry in shadows.iter().rev() {
            if let Entry::Shadow {
                x,
                y,
                blur,
                spread,
                color,
                inset: held,
            } = entry
                && *held == inset
            {
                let mut shadow = Effect::new(EffectKind::Shadow);
                shadow.offset_x = *x;
                shadow.offset_y = *y;
                shadow.blur_radius = *blur;
                shadow.spread = *spread;
                shadow.colour = wire_colour(color);
                shadow.inset = inset;
                // A window's shadow is the whole frame's, titlebar and all, not the content's alone; an inset one lies inside
                // the shape it is cast from and has no frame.
                shadow.frame = !inset;
                effects.push(shadow);
            }
        }
    }
    for entry in computed.list(ListProp::Borders) {
        if let Entry::Border {
            width,
            color,
            offset,
            inset,
        } = entry
        {
            let mut border = Effect::new(EffectKind::Border);
            border.width = *width;
            border.offset = *offset;
            border.inset = *inset;
            border.colour = wire_colour(color);
            effects.push(border);
        }
    }
    let corner_radius = match computed.corner_radius {
        Radii::All(radius) => radius,
        Radii::Corners(corners) => {
            let largest = corners.iter().copied().fold(f64::MIN, f64::max);
            if !corners.iter().all(|corner| *corner == largest) {
                unapplied.push(format!(
                    "cornerRadius: a window has one radius for every corner, so the largest of the four ({largest}) is drawn"
                ));
            }
            largest
        }
    };
    Mapping {
        effects,
        corner_radius,
        unapplied,
    }
}

/// The one `filter` effect the non-shader elements of a `filter` list make, `None` when they change nothing. The blurs of a chain
/// run one after the other, and Gaussians add their variances, so the radius is the root of the sum of the squares; the grades
/// multiply like kinds, as the `backdropFilter`'s do. A radius or a grade over the renderer's limit is drawn as the limit and
/// said in `unapplied`.
fn map_filter(filter: &[Entry], unapplied: &mut Vec<String>) -> Option<Effect> {
    let mut variance = 0.0;
    let (mut saturation, mut brightness, mut contrast) = (1.0, 1.0, 1.0);
    for entry in filter {
        match entry {
            Entry::Blur(radius) => variance += radius * radius,
            Entry::Saturate(amount) => saturation *= amount,
            Entry::Brightness(amount) => brightness *= amount,
            Entry::Contrast(amount) => contrast *= amount,
            Entry::Border { .. } | Entry::Shadow { .. } | Entry::Shader { .. } => {}
        }
    }
    let blur = f64::sqrt(variance);
    if blur == 0.0 && saturation == 1.0 && brightness == 1.0 && contrast == 1.0 {
        return None;
    }
    if blur > MAX_FILTER_BLUR {
        unapplied.push(format!(
            "filter: the blurs of a filter add up to {blur}, and the compositor blurs at most {MAX_FILTER_BLUR}, so {MAX_FILTER_BLUR} is drawn"
        ));
    }
    if [saturation, brightness, contrast].iter().any(|n| *n > MAX_GRADE) {
        unapplied.push(format!(
            "filter: a combined saturate, brightness or contrast above {MAX_GRADE} is drawn as {MAX_GRADE}"
        ));
    }
    let mut effect = Effect::new(EffectKind::Filter);
    effect.blur_radius = blur.min(MAX_FILTER_BLUR);
    effect.saturation = saturation.min(MAX_GRADE);
    effect.brightness = brightness.min(MAX_GRADE);
    effect.contrast = contrast.min(MAX_GRADE);
    Some(effect)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shader(source: &str, params: &[f64]) -> Entry {
        Entry::Shader {
            source: source.to_owned(),
            params: params.to_vec(),
        }
    }

    fn style_with(lists: &[(ListProp, Vec<Entry>)]) -> Computed {
        Computed {
            opacity: 1.0,
            corner_radius: Radii::All(0.0),
            background: None,
            lists: lists.iter().cloned().collect(),
            origin: BTreeMap::new(),
        }
    }

    fn kinds(mapping: &Mapping) -> Vec<(EffectKind, String)> {
        mapping
            .effects
            .iter()
            .map(|effect| (effect.kind, effect.source.clone()))
            .collect()
    }

    #[test]
    fn a_shader_filter_is_one_shader_effect_after_the_blur_and_before_the_shadows() {
        let computed = style_with(&[
            (
                ListProp::BackdropFilter,
                vec![Entry::Blur(12.0), shader("back", &[1.0])],
            ),
            (ListProp::Filter, vec![shader("content", &[0.5, 2.0])]),
            (
                ListProp::Shadows,
                vec![Entry::Shadow {
                    x: 0.0,
                    y: 8.0,
                    blur: 24.0,
                    spread: 0.0,
                    color: "#000".to_owned(),
                    inset: false,
                }],
            ),
        ]);
        let mapping = map_style(&computed, true);
        assert_eq!(
            kinds(&mapping),
            [
                (EffectKind::Blur, String::new()),
                (EffectKind::Shader, "back".to_owned()),
                (EffectKind::Shader, "content".to_owned()),
                (EffectKind::Shadow, String::new()),
            ],
            "the backdrop's shaders come first, then the content's, and the style's other effects keep their places"
        );
        assert_eq!(mapping.effects[2].params, [0.5, 2.0]);
        assert!(
            mapping.unapplied.is_empty(),
            "a shader filter alone leaves nothing unapplied: {:?}",
            mapping.unapplied
        );
    }

    #[test]
    fn with_user_shaders_off_a_shader_filter_is_not_written_and_each_list_that_holds_one_says_why() {
        let computed = style_with(&[
            (ListProp::BackdropFilter, vec![Entry::Blur(12.0), shader("back", &[])]),
            (ListProp::Filter, vec![shader("content", &[])]),
        ]);
        let mapping = map_style(&computed, false);
        assert_eq!(
            kinds(&mapping),
            [(EffectKind::Blur, String::new())],
            "the blur is written, the shaders are not"
        );
        assert_eq!(mapping.unapplied.len(), 2, "{:?}", mapping.unapplied);
        for (note, list) in mapping.unapplied.iter().zip(["backdropFilter", "filter"]) {
            assert!(
                note.starts_with(&format!("{list}: a ShaderFilter is not drawn"))
                    && note.contains("compositor.user-shaders"),
                "{note}"
            );
        }
    }

    #[test]
    fn a_window_draws_four_shaders_and_the_style_says_when_it_asked_for_more() {
        let many: Vec<Entry> = (0..6).map(|at| shader(&format!("s{at}"), &[])).collect();
        let mapping = map_style(&style_with(&[(ListProp::Filter, many)]), true);
        assert_eq!(
            kinds(&mapping)
                .into_iter()
                .map(|(_, source)| source)
                .collect::<Vec<_>>(),
            ["s0", "s1", "s2", "s3"]
        );
        assert_eq!(mapping.unapplied.len(), 1, "{:?}", mapping.unapplied);
        assert!(
            mapping.unapplied[0].contains("at most 4 shaders") && mapping.unapplied[0].contains("of the 6"),
            "{}",
            mapping.unapplied[0]
        );
    }

    fn inset(x: f64, y: f64, blur: f64, spread: f64, color: &str) -> Entry {
        Entry::Shadow {
            x,
            y,
            blur,
            spread,
            color: color.to_owned(),
            inset: true,
        }
    }

    fn outer(blur: f64, color: &str) -> Entry {
        Entry::Shadow {
            x: 0.0,
            y: 0.0,
            blur,
            spread: 0.0,
            color: color.to_owned(),
            inset: false,
        }
    }

    #[test]
    fn a_content_filter_is_one_filter_effect_and_nothing_is_left_unapplied() {
        let only_a_shader = style_with(&[(ListProp::Filter, vec![shader("content", &[])])]);
        let mapping = map_style(&only_a_shader, true);
        assert!(mapping.unapplied.is_empty());
        assert_eq!(kinds(&mapping).len(), 1, "a shader alone is no filter effect");
        let with_a_blur = style_with(&[(ListProp::Filter, vec![shader("content", &[]), Entry::Blur(4.0)])]);
        let mapping = map_style(&with_a_blur, true);
        assert!(mapping.unapplied.is_empty(), "{:?}", mapping.unapplied);
        assert_eq!(
            kinds(&mapping).iter().map(|(kind, _)| *kind).collect::<Vec<_>>(),
            [EffectKind::Shader, EffectKind::Filter],
            "the shader, then the filter"
        );
        let filter = &mapping.effects[1];
        assert_eq!(
            (
                filter.blur_radius,
                filter.saturation,
                filter.brightness,
                filter.contrast
            ),
            (4.0, 1.0, 1.0, 1.0)
        );
    }

    #[test]
    fn a_filter_adds_its_blurs_as_gaussians_and_multiplies_its_grades() {
        let computed = style_with(&[(
            ListProp::Filter,
            vec![
                Entry::Blur(3.0),
                Entry::Saturate(0.5),
                Entry::Blur(4.0),
                Entry::Contrast(1.5),
                Entry::Saturate(0.5),
                Entry::Brightness(1.2),
            ],
        )]);
        let mapping = map_style(&computed, true);
        assert!(mapping.unapplied.is_empty(), "{:?}", mapping.unapplied);
        let [filter] = mapping.effects.as_slice() else {
            panic!("one filter effect: {:?}", mapping.effects);
        };
        assert_eq!(filter.kind, EffectKind::Filter);
        assert_eq!(filter.blur_radius, 5.0, "the root of 3 squared and 4 squared");
        assert_eq!(
            (filter.saturation, filter.brightness, filter.contrast),
            (0.25, 1.2, 1.5)
        );
    }

    #[test]
    fn a_filter_that_changes_nothing_writes_no_effect_and_one_over_the_limits_says_so() {
        let neutral = style_with(&[(
            ListProp::Filter,
            vec![Entry::Blur(0.0), Entry::Saturate(1.0), Entry::Contrast(1.0)],
        )]);
        let mapping = map_style(&neutral, true);
        assert!(mapping.effects.is_empty() && mapping.unapplied.is_empty());

        let over = style_with(&[(
            ListProp::Filter,
            vec![
                Entry::Blur(80.0),
                Entry::Blur(80.0),
                Entry::Brightness(2.0),
                Entry::Brightness(2.0),
            ],
        )]);
        let mapping = map_style(&over, true);
        let [filter] = mapping.effects.as_slice() else {
            panic!("one filter effect: {:?}", mapping.effects);
        };
        assert_eq!((filter.blur_radius, filter.brightness), (100.0, 2.0));
        assert_eq!(mapping.unapplied.len(), 2, "{:?}", mapping.unapplied);
        assert!(mapping.unapplied[0].starts_with("filter: the blurs of a filter add up to"));
        assert!(mapping.unapplied[1].starts_with("filter: a combined saturate, brightness or contrast above 2"));
    }

    #[test]
    fn a_background_is_one_background_effect_with_its_colour_in_full() {
        let mut computed = style_with(&[]);
        computed.background = Some("#123".to_owned());
        let mapping = map_style(&computed, true);
        assert!(mapping.unapplied.is_empty(), "{:?}", mapping.unapplied);
        let [fill] = mapping.effects.as_slice() else {
            panic!("one background effect: {:?}", mapping.effects);
        };
        assert_eq!((fill.kind, fill.colour.as_str()), (EffectKind::Background, "#112233ff"));
        computed.background = None;
        assert!(
            map_style(&computed, true).effects.is_empty(),
            "no background, no effect"
        );
    }

    #[test]
    fn inset_shadows_are_shadow_effects_with_inset_after_the_outer_ones_and_follow_no_frame() {
        let computed = style_with(&[(
            ListProp::Shadows,
            vec![
                inset(1.0, 2.0, 6.0, 3.0, "#000"),
                outer(8.0, "#111"),
                inset(0.0, 0.0, 2.0, 0.0, "#fff8"),
                outer(4.0, "#222"),
            ],
        )]);
        let mapping = map_style(&computed, true);
        assert!(mapping.unapplied.is_empty(), "{:?}", mapping.unapplied);
        let held: Vec<(bool, bool, f64)> = mapping
            .effects
            .iter()
            .map(|effect| (effect.inset, effect.frame, effect.blur_radius))
            .collect();
        assert_eq!(
            held,
            [
                (false, true, 4.0),
                (false, true, 8.0),
                (true, false, 2.0),
                (true, false, 6.0)
            ],
            "outer shadows last to first, then the inset ones last to first"
        );
        let first_inset = &mapping.effects[3];
        assert_eq!(
            (
                first_inset.offset_x,
                first_inset.offset_y,
                first_inset.spread,
                first_inset.colour.as_str()
            ),
            (1.0, 2.0, 3.0, "#000000ff")
        );
    }

    #[test]
    fn the_background_and_the_filter_sit_after_the_shaders_and_before_the_shadows() {
        let mut computed = style_with(&[
            (ListProp::BackdropFilter, vec![Entry::Blur(12.0)]),
            (ListProp::Filter, vec![shader("content", &[]), Entry::Saturate(0.5)]),
            (
                ListProp::Shadows,
                vec![outer(8.0, "#000"), inset(0.0, 0.0, 3.0, 0.0, "#000")],
            ),
        ]);
        computed.background = Some("#fff".to_owned());
        let mapping = map_style(&computed, true);
        assert_eq!(
            kinds(&mapping).iter().map(|(kind, _)| *kind).collect::<Vec<_>>(),
            [
                EffectKind::Blur,
                EffectKind::Shader,
                EffectKind::Background,
                EffectKind::Filter,
                EffectKind::Shadow,
                EffectKind::Shadow
            ]
        );
        assert!(mapping.unapplied.is_empty(), "{:?}", mapping.unapplied);
    }
}
