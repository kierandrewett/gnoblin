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

use std::collections::{BTreeMap, HashMap};

use gnoblin_core::{Effect, EffectKind, NodeId};

use super::values::{Entry, ListProp, MAX_GRADE, MAX_STYLE_LIST, Prop, Radii, ScalarProp};
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
    opacity: Option<f64>,
    corner: Option<f64>,
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
    /// The library held styles or applied ones when `style_reset` ran: an earlier script generation may have left effects on the windows.
    inherited: bool,
    /// Goes up by one for every accepted change of any style; the application compares it between turns.
    pub(crate) epoch: u64,
    clients: HashMap<u64, Applied>,
}

impl StyleState {
    /// Drops every scope and what was applied, and makes the library the styles' owner: a script generation that starts over a library
    /// state an earlier one left (a reload that stays in one core generation) styles on a clean slate, and its first pass takes
    /// what the earlier one drew off the windows.
    pub(crate) fn reset(&mut self) {
        self.inherited = !self.stores.is_empty() || !self.clients.is_empty();
        self.stores.clear();
        self.by_scope.clear();
        self.selections.clear();
        self.ranks.clear();
        self.clients.clear();
        self.owned = true;
        self.epoch += 1;
    }

    /// The rank of the selection `coll`.
    fn rank(&self, coll: Id) -> f64 {
        self.ranks.get(&coll).copied().unwrap_or(0.0)
    }

    /// Sets the rank of the selection `coll`.
    pub(crate) fn set_rank(&mut self, coll: Id, rank: f64) {
        self.ranks.insert(coll, rank);
        self.epoch += 1;
    }
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

    /// Writes the resolved style of every client whose style or layers changed since it was last applied.
    pub(crate) fn apply_styles(&mut self) {
        // A shell that keeps its styles itself (a native one, which has made no scope here) has this pass leave its windows alone: the
        // first scope made here, or `style_reset`, is what makes the library the styles' owner.
        if self.styles.by_scope.is_empty() && !self.styles.owned {
            return;
        }
        let epoch = self.styles.epoch;
        for client in self.clients.clone() {
            if !crate::nodes::alive(&self.g, client) {
                continue;
            }
            let layers = self.client_layers(client);
            let key = layers
                .iter()
                .map(|layer| layer.origin.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            let seen = self.styles.clients.entry(client).or_default();
            if seen.applied && seen.epoch == epoch && seen.layers == key {
                continue;
            }
            seen.applied = true;
            seen.epoch = epoch;
            seen.layers = key;
            let computed = self.resolve_style(&layers);
            self.apply_client_style(client, computed);
        }
    }

    fn apply_client_style(&mut self, client: NodeId64, computed: Computed) {
        let g = self.g.clone();
        let node = g.node(NodeId(client));
        let mapping = map_style(&computed);
        let mut notes = mapping.unapplied.clone();
        let mut applied = self.styles.clients.remove(&client).unwrap_or_default();
        if mapping.effects.len() > MAX_STYLE_LIST * 2 {
            notes.push(format!(
                "Client#{client}.style: a window carries at most {} effects, the style makes {}",
                MAX_STYLE_LIST * 2,
                mapping.effects.len()
            ));
        } else {
            // A window an earlier generation styled keeps its effects: the first application clears them when the style has none.
            // The tree's effects cannot be read back, so what an earlier generation left is not known: over a state one left
            // (`inherited`) the first application writes, even an empty list; otherwise an empty list is what the window has.
            let unchanged = match &applied.effects {
                None => !self.styles.inherited && mapping.effects.is_empty(),
                Some(written) => *written == mapping.effects,
            };
            if !unchanged {
                let _ = node.set_effects(&mapping.effects);
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
/// `shadow` effects that follow the window's whole frame, `borders` are `border` effects and the `backdropFilter` is one backdrop blur
/// whose saturation, brightness and contrast are the products of the chain's. `background`, `filter` and inset shadows stay in the
/// computed style; each comes back as an `unapplied` text when the style has a non-default value for it.
pub(crate) fn map_style(computed: &Computed) -> Mapping {
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
    let shadows = computed.list(ListProp::Shadows);
    for entry in shadows.iter().rev() {
        if let Entry::Shadow {
            x,
            y,
            blur,
            spread,
            color,
            inset: false,
        } = entry
        {
            let mut shadow = Effect::new(EffectKind::Shadow);
            shadow.offset_x = *x;
            shadow.offset_y = *y;
            shadow.blur_radius = *blur;
            shadow.spread = *spread;
            shadow.colour = wire_colour(color);
            // A window's shadow is the whole frame's, titlebar and all, not the content's alone.
            shadow.frame = true;
            effects.push(shadow);
        }
    }
    if shadows
        .iter()
        .any(|entry| matches!(entry, Entry::Shadow { inset: true, .. }))
    {
        unapplied.push("shadows: an inset shadow has no effect kind".to_owned());
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
    if computed.background.is_some() {
        unapplied.push("background: the tree has no background for a window".to_owned());
    }
    if !computed.list(ListProp::Filter).is_empty() {
        unapplied.push("filter: no effect kind filters a window's own content".to_owned());
    }
    Mapping {
        effects,
        corner_radius,
        unapplied,
    }
}
