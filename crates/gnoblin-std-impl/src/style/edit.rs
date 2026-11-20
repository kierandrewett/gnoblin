//! The edits of a style that the functions of the std table and the classes of `style.toml` share: making the style of a scope, setting
//! and unsetting what it holds, and reading what it holds or inherits. Every check is here, so a value is refused with the same words
//! whichever way it came (`Client#42.style.opacity: must be finite and between 0 and 1`).

use gnoblin_core_abi as abi;
use gnoblin_std_abi as sabi;

use super::cascade::{Scalar, Scope, Store};
use super::values::{self, Entry, ListProp, Prop, Radii, ScalarProp};
use crate::collection::Want;
use crate::error::{Fail, Res};
use crate::state::{Id, State, next_id};

/// A scalar as it arrives, before it is checked.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ScalarIn {
    Opacity(f64),
    CornerRadius(Radii),
    /// A colour, or none for no background.
    Background(Option<String>),
}

impl ScalarIn {
    /// Which scalar property it is for.
    pub(crate) fn prop(&self) -> ScalarProp {
        match self {
            ScalarIn::Opacity(_) => ScalarProp::Opacity,
            ScalarIn::CornerRadius(_) => ScalarProp::CornerRadius,
            ScalarIn::Background(_) => ScalarProp::Background,
        }
    }

    /// The scalar it stands for, checked, with `path` at the front of every failure.
    pub(crate) fn check(&self, path: &str) -> Res<Scalar> {
        Ok(match self {
            ScalarIn::Opacity(value) => Scalar::Opacity(values::unit(path, *value)?),
            ScalarIn::CornerRadius(radii) => Scalar::CornerRadius(values::corner_radius(path, *radii)?),
            ScalarIn::Background(None) => Scalar::Background(None),
            ScalarIn::Background(Some(text)) => Scalar::Background(Some(values::colour(path, text)?)),
        })
    }
}

impl State {
    /// The scope a style is for, or why there is none.
    pub(crate) fn style_scope_of(&self, style: Id) -> Res<Scope> {
        self.styles
            .stores
            .get(&style)
            .map(|store| store.scope)
            .ok_or_else(|| Fail::stale(format!("style {style} does not exist")))
    }

    /// Fails for a style whose client is gone.
    pub(crate) fn style_guard(&self, scope: Scope) -> Res<()> {
        match scope {
            Scope::Client(node) if !crate::nodes::alive(&self.g, node) => Err(Fail::new(
                abi::E_NO_SUCH_NODE,
                format!("{}: the client is gone", self.style_path(scope)),
            )),
            _ => Ok(()),
        }
    }

    /// The store of `style`, for a change.
    fn style_store_mut(&mut self, style: Id) -> Res<&mut Store> {
        self.styles
            .stores
            .get_mut(&style)
            .ok_or_else(|| Fail::stale(format!("style {style} does not exist")))
    }

    /// The style of `scope`, made when there is none: a collection of clients, or one client.
    pub(crate) fn style_for(&mut self, scope: Scope) -> Res<Id> {
        match scope {
            Scope::Collection(id) => {
                let coll = self
                    .colls
                    .get(&id)
                    .ok_or_else(|| Fail::stale(format!("collection {id} does not exist")))?;
                if coll.kind != sabi::STD_COLLECTION_CLIENTS {
                    return Err(Fail::type_error("style: only a collection of clients has a style"));
                }
            }
            Scope::Client(node) => {
                if !self.rec.contains_key(&node) || !crate::nodes::alive(&self.g, node) {
                    return Err(Fail::new(abi::E_NO_SUCH_NODE, format!("Client#{node} is not a client")));
                }
            }
        }
        if let Some(existing) = self.styles.by_scope.get(&scope) {
            return Ok(*existing);
        }
        let id = next_id();
        self.styles.stores.insert(
            id,
            Store {
                scope,
                scalars: Default::default(),
                lists: Default::default(),
            },
        );
        self.styles.by_scope.insert(scope, id);
        if let Scope::Collection(coll) = scope
            && self.colls.get(&coll).is_some_and(|coll| coll.parent.is_some())
        {
            self.styles.selections.push(coll);
        }
        Ok(id)
    }

    /// The style of the windows of the workspace `workspace`, by its id.
    pub(crate) fn style_of_workspace(&mut self, workspace: &str) -> Res<Id> {
        let coll = self
            .ws
            .values()
            .find(|held| held.id == workspace)
            .map(|held| held.coll)
            .ok_or_else(|| Fail::no_such(format!("there is no workspace `{workspace}`")))?;
        self.style_for(Scope::Collection(coll))
    }

    /// The style of the selection of the windows of `style`'s scope that match `terms`, ranked by `priority` when it is given.
    pub(crate) fn style_where(&mut self, style: Id, terms: Vec<(String, Want)>, priority: Option<f64>) -> Res<Id> {
        let Scope::Collection(parent) = self.style_scope_of(style)? else {
            return Err(Fail::type_error(
                "only the style of a collection of clients has a selection",
            ));
        };
        if let Some(rank) = priority
            && !rank.is_finite()
        {
            return Err(Fail::type_error("where: priority must be a finite number"));
        }
        let view = self.make_view(parent, terms)?;
        let made = self.style_for(Scope::Collection(view))?;
        if let Some(rank) = priority {
            self.styles.set_rank(view, rank);
        }
        Ok(made)
    }

    /// Sets a scalar, checked.
    pub(crate) fn style_put_scalar(&mut self, style: Id, input: &ScalarIn) -> Res<()> {
        let scope = self.style_scope_of(style)?;
        self.style_guard(scope)?;
        let prop = input.prop();
        let path = format!("{}.{}", self.style_path(scope), Prop::Scalar(prop).name());
        let checked = input.check(&path)?;
        self.style_store_mut(style)?.scalars.insert(prop, checked);
        self.style_changed();
        Ok(())
    }

    /// Sets a list from drafts: every entry is checked first, and nothing changes when one is refused.
    pub(crate) fn style_put_drafts(&mut self, style: Id, list: ListProp, drafts: &[values::Draft<'_>]) -> Res<()> {
        let scope = self.style_scope_of(style)?;
        self.style_guard(scope)?;
        let path = format!("{}.{}", self.style_path(scope), Prop::List(list).name());
        if drafts.len() > values::MAX_STYLE_LIST {
            return Err(values::fail(
                &path,
                &format!("holds at most {} entries, got {}", values::MAX_STYLE_LIST, drafts.len()),
            ));
        }
        let checked = drafts
            .iter()
            .enumerate()
            .map(|(at, draft)| values::entry_from(list, draft, &format!("{path}[{at}]")))
            .collect::<Res<Vec<Entry>>>()?;
        self.style_put_entries(style, list, checked)
    }

    /// Sets a list from entries that are checked already.
    pub(crate) fn style_put_entries(&mut self, style: Id, list: ListProp, entries: Vec<Entry>) -> Res<()> {
        self.style_store_mut(style)?.lists.insert(list, entries);
        self.style_changed();
        Ok(())
    }

    /// Takes one property back to inheriting, or every one.
    pub(crate) fn style_remove(&mut self, style: Id, which: Option<Prop>) -> Res<()> {
        let scope = self.style_scope_of(style)?;
        self.style_guard(scope)?;
        let store = self.style_store_mut(style)?;
        for each in Prop::ALL {
            if which.is_none_or(|which| which == each) {
                match each {
                    Prop::Scalar(scalar) => {
                        store.scalars.remove(&scalar);
                    }
                    Prop::List(list) => {
                        store.lists.remove(&list);
                    }
                }
            }
        }
        self.style_changed();
        Ok(())
    }

    /// The scalar a scope sets itself, or none.
    pub(crate) fn style_own_scalar(&self, style: Id, prop: ScalarProp) -> Res<Option<Scalar>> {
        let store = self
            .styles
            .stores
            .get(&style)
            .ok_or_else(|| Fail::stale(format!("style {style} does not exist")))?;
        Ok(store.scalars.get(&prop).cloned())
    }

    /// The list a scope sets itself, if it does, else what the scopes below it give, and whether the scope sets it.
    pub(crate) fn style_entries(&self, style: Id, list: ListProp) -> Res<(Vec<Entry>, bool)> {
        let store = self
            .styles
            .stores
            .get(&style)
            .ok_or_else(|| Fail::stale(format!("style {style} does not exist")))?;
        Ok(match store.lists.get(&list) {
            Some(own) => (own.clone(), true),
            None => (self.layered_list(&self.layers_below(store.scope), list), false),
        })
    }

    /// The scalar a scope sets itself, else what the scopes below it resolve to.
    pub(crate) fn style_scalar(&self, style: Id, prop: ScalarProp) -> Res<Scalar> {
        if let Some(own) = self.style_own_scalar(style, prop)? {
            return Ok(own);
        }
        let scope = self.style_scope_of(style)?;
        let below = self.resolve_style(&self.layers_below(scope));
        Ok(match prop {
            ScalarProp::Opacity => Scalar::Opacity(below.opacity),
            ScalarProp::CornerRadius => Scalar::CornerRadius(below.corner_radius),
            ScalarProp::Background => Scalar::Background(below.background),
        })
    }

    /// Makes the scope set exactly the scalars and lists given, after every one of them was checked.
    pub(crate) fn style_replace(
        &mut self,
        style: Id,
        scalars: Vec<ScalarIn>,
        lists: Vec<(ListProp, Vec<values::Draft<'_>>)>,
    ) -> Res<()> {
        let scope = self.style_scope_of(style)?;
        self.style_guard(scope)?;
        let front = self.style_path(scope);
        let mut checked_scalars = Vec::new();
        for input in &scalars {
            let path = format!("{front}.{}", Prop::Scalar(input.prop()).name());
            checked_scalars.push((input.prop(), input.check(&path)?));
        }
        let mut checked_lists = Vec::new();
        for (list, drafts) in &lists {
            let path = format!("{front}.{}", Prop::List(*list).name());
            if drafts.len() > values::MAX_STYLE_LIST {
                return Err(values::fail(
                    &path,
                    &format!("holds at most {} entries, got {}", values::MAX_STYLE_LIST, drafts.len()),
                ));
            }
            let entries = drafts
                .iter()
                .enumerate()
                .map(|(at, draft)| values::entry_from(*list, draft, &format!("{path}[{at}]")))
                .collect::<Res<Vec<Entry>>>()?;
            checked_lists.push((*list, entries));
        }
        let store = self.style_store_mut(style)?;
        store.scalars.clear();
        store.lists.clear();
        store.scalars.extend(checked_scalars);
        store.lists.extend(checked_lists);
        self.style_changed();
        Ok(())
    }
}

impl State {
    /// Ranks a selection's style among the selections that style a client: higher wins.
    pub(crate) fn style_set_rank(&mut self, style: Id, rank: f64) -> Res<()> {
        if !rank.is_finite() {
            return Err(Fail::type_error("a selection's rank must be a finite number"));
        }
        match self.style_scope_of(style)? {
            Scope::Collection(coll) if self.colls.get(&coll).is_some_and(|coll| coll.parent.is_some()) => {
                self.styles.set_rank(coll, rank);
                Ok(())
            }
            _ => Err(Fail::type_error("only the style of a `where` selection has a rank")),
        }
    }

    /// What a selection's style ranks by: zero for a scope that is not a selection.
    pub(crate) fn style_rank_of(&self, style: Id) -> Res<f64> {
        Ok(match self.style_scope_of(style)? {
            Scope::Collection(coll) => self.styles.rank(coll),
            Scope::Client(_) => 0.0,
        })
    }
}
