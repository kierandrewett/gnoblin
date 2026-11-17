//! Collections: live views of clients, outputs and workspaces.
//!
//! A *root* collection holds what the library tracks (`state.clients`, `state.outputs`, the added workspaces) or what one workspace
//! holds (its clients). A *selection* (`where`) is a view over its parent that holds the members whose properties match a query,
//! and follows the parent as members come, go and change. A collection with listeners is *tracked*: the library remembers which
//! members it last told them of, so a member that stops matching is announced as removed. This is
//! `data/js/gnoblin-core/host/collection_state.ts` and `collection_events.ts`.

use std::collections::HashSet;

use gnoblin_std_abi as abi;

use crate::error::{Fail, Res};
use crate::state::{Id, Notice, State};

/// One value a query compares a property with.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Want {
    Text(String),
    Number(f64),
    Bool(bool),
}

/// Where a root collection's members come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    Clients,
    Outputs,
    Workspaces,
    /// The clients of one workspace.
    WorkspaceClients(Id),
}

/// A collection.
#[derive(Debug)]
pub(crate) struct Coll {
    /// `STD_COLLECTION_*`: what the members are.
    pub(crate) kind: u32,
    /// For a root: where its members come from.
    pub(crate) source: Option<Source>,
    /// For a selection: the collection it narrows.
    pub(crate) parent: Option<Id>,
    /// For a selection: the conditions, grouped by key. A member matches when every key has an alternative it equals.
    pub(crate) query: Vec<(String, Vec<Want>)>,
    /// The members the listeners were last told of; kept only while a selection has listeners.
    pub(crate) tracked: Option<HashSet<u64>>,
}

/// The properties `where` can select on, by the kind of member.
pub(crate) fn query_keys(kind: u32) -> &'static [&'static str] {
    match kind {
        abi::STD_COLLECTION_CLIENTS => &[
            "id",
            "app_id",
            "title",
            "focused",
            "fullscreen",
            "maximised",
            "minimised",
            "x",
            "y",
            "width",
            "height",
            "workspace",
        ],
        abi::STD_COLLECTION_OUTPUTS => &[
            "id",
            "name",
            "connector",
            "make",
            "model",
            "mode",
            "scale",
            "primary",
            "enabled",
            "x",
            "y",
            "width",
            "height",
        ],
        abi::STD_COLLECTION_WORKSPACES => &["id", "name", "active", "index"],
        _ => &[],
    }
}

impl State {
    /// Makes a root collection.
    pub(crate) fn new_root(&mut self, kind: u32, source: Source) -> Id {
        let id = crate::state::next_id();
        self.colls.insert(
            id,
            Coll {
                kind,
                source: Some(source),
                parent: None,
                query: Vec::new(),
                tracked: None,
            },
        );
        id
    }

    fn coll(&self, id: Id) -> Res<&Coll> {
        self.colls
            .get(&id)
            .ok_or_else(|| Fail::stale(format!("collection {id} does not exist")))
    }

    /// The members of the collection, in order.
    pub(crate) fn items(&self, id: Id) -> Vec<u64> {
        let Some(coll) = self.colls.get(&id) else {
            return Vec::new();
        };
        match (coll.source, coll.parent) {
            (Some(Source::Clients), _) => self.clients.clone(),
            (Some(Source::Outputs), _) => self.outputs.clone(),
            (Some(Source::Workspaces), _) => self.workspaces.clone(),
            (Some(Source::WorkspaceClients(workspace)), _) => self
                .ws
                .get(&workspace)
                .map(|workspace| workspace.clients.clone())
                .unwrap_or_default(),
            (None, Some(parent)) => self
                .items(parent)
                .into_iter()
                .filter(|member| self.matches(coll.kind, *member, &coll.query))
                .collect(),
            (None, None) => Vec::new(),
        }
    }

    /// Whether `member` belongs to the collection now, worked out from the objects and not from what was told.
    pub(crate) fn contains(&self, id: Id, member: u64) -> bool {
        let Some(coll) = self.colls.get(&id) else {
            return false;
        };
        match (coll.source, coll.parent) {
            (Some(_), _) => self.items(id).contains(&member),
            (None, Some(parent)) => self.contains(parent, member) && self.matches(coll.kind, member, &coll.query),
            (None, None) => false,
        }
    }

    fn matches(&self, kind: u32, member: u64, query: &[(String, Vec<Want>)]) -> bool {
        query.iter().all(|(key, alternatives)| {
            self.property(kind, member, key)
                .is_some_and(|have| alternatives.contains(&have))
        })
    }

    /// A member's property as a query value, `None` for a key the kind has not.
    fn property(&self, kind: u32, member: u64, key: &str) -> Option<Want> {
        let g = &self.g;
        let text = |name: &str| Want::Text(crate::nodes::text(g, member, name));
        let flag = |name: &str| Want::Bool(crate::nodes::flag(g, member, name));
        let number = |name: &str| Want::Number(crate::nodes::number(g, member, name));
        match kind {
            abi::STD_COLLECTION_CLIENTS => Some(match key {
                "id" => Want::Text(member.to_string()),
                "app_id" | "appId" => text("app_id"),
                "title" => text("title"),
                "focused" | "fullscreen" | "maximised" | "minimised" => flag(key),
                "x" | "y" | "width" | "height" => number(key),
                "workspace" => Want::Text(
                    self.rec
                        .get(&member)
                        .and_then(|rec| rec.workspace)
                        .and_then(|workspace| self.ws.get(&workspace))
                        .map(|workspace| workspace.id.clone())
                        .unwrap_or_default(),
                ),
                _ => return None,
            }),
            abi::STD_COLLECTION_OUTPUTS => Some(match key {
                "id" => Want::Text(member.to_string()),
                "name" | "connector" | "make" | "model" | "mode" => text(key),
                "scale" => number("output_scale"),
                "primary" | "enabled" => flag(key),
                "x" | "y" | "width" | "height" => number(key),
                _ => return None,
            }),
            abi::STD_COLLECTION_WORKSPACES => {
                let workspace = self.ws.get(&member)?;
                Some(match key {
                    "id" => Want::Text(workspace.id.clone()),
                    "name" => Want::Text(workspace.name.clone()),
                    "active" => Want::Bool(self.is_shown(member)),
                    "index" => Want::Number(
                        self.workspaces
                            .iter()
                            .position(|w| *w == member)
                            .map_or(-1.0, |i| i as f64),
                    ),
                    _ => return None,
                })
            }
            _ => None,
        }
    }

    /// Makes the live selection of `parent` that holds the members matching `terms`.
    pub(crate) fn make_view(&mut self, parent: Id, terms: Vec<(String, Want)>) -> Res<Id> {
        let kind = self.coll(parent)?.kind;
        let known = query_keys(kind);
        let mut query: Vec<(String, Vec<Want>)> = Vec::new();
        for (key, want) in terms {
            if !known.contains(&key.as_str()) && key != "appId" {
                return Err(Fail::no_such(format!(
                    "where: no property `{key}` to select on (known: {})",
                    known.join(", ")
                )));
            }
            let key = if key == "appId" { "app_id".to_owned() } else { key };
            match query.iter_mut().find(|(have, _)| *have == key) {
                Some((_, alternatives)) => alternatives.push(want),
                None => query.push((key, vec![want])),
            }
        }
        let id = crate::state::next_id();
        self.colls.insert(
            id,
            Coll {
                kind,
                source: None,
                parent: Some(parent),
                query,
                tracked: None,
            },
        );
        Ok(id)
    }

    /// Starts remembering which members the listeners of a selection were told of.
    pub(crate) fn track(&mut self, id: Id) {
        let needs = self
            .colls
            .get(&id)
            .is_some_and(|coll| coll.parent.is_some() && coll.tracked.is_none());
        if needs {
            let members: HashSet<u64> = self.items(id).into_iter().collect();
            if let Some(coll) = self.colls.get_mut(&id) {
                coll.tracked = Some(members);
            }
        }
    }

    /// Stops remembering, for a selection nobody listens to any more.
    pub(crate) fn untrack_if_idle(&mut self, id: Id) {
        if self.regs.values().any(|reg| reg.coll == id) {
            return;
        }
        if let Some(coll) = self.colls.get_mut(&id) {
            coll.tracked = None;
        }
    }

    /// The collections that hold `member` now: its root, its workspace's clients, and every tracked selection with it.
    pub(crate) fn holders(&self, kind: u32, member: u64) -> Vec<Id> {
        let mut held = vec![self.roots[(kind - 1) as usize]];
        if kind == abi::STD_COLLECTION_CLIENTS
            && let Some(workspace) = self.rec.get(&member).and_then(|rec| rec.workspace)
            && let Some(workspace) = self.ws.get(&workspace)
        {
            held.push(workspace.coll);
        }
        let mut views: Vec<Id> = self
            .colls
            .iter()
            .filter(|(_, coll)| coll.kind == kind && coll.tracked.as_ref().is_some_and(|set| set.contains(&member)))
            .map(|(id, _)| *id)
            .collect();
        views.sort_unstable();
        held.extend(views);
        held
    }

    /// Queues a notice for the listeners of one collection.
    pub(crate) fn notify_one(&mut self, kind: u32, member: u64, coll: Id) {
        self.notices.push(Notice::new(kind, member, vec![coll]));
    }

    /// Tells the tracked selections whether `member` joined or left them, as its properties or its membership now say.
    pub(crate) fn recompute(&mut self, kind: u32, member: u64) {
        let mut views: Vec<Id> = self
            .colls
            .iter()
            .filter(|(_, coll)| coll.kind == kind && coll.tracked.is_some())
            .map(|(id, _)| *id)
            .collect();
        views.sort_unstable();
        for view in views {
            let was = self
                .colls
                .get(&view)
                .and_then(|coll| coll.tracked.as_ref())
                .is_some_and(|set| set.contains(&member));
            let is = self.contains(view, member);
            if was == is {
                continue;
            }
            if let Some(set) = self.colls.get_mut(&view).and_then(|coll| coll.tracked.as_mut()) {
                if is {
                    set.insert(member);
                } else {
                    set.remove(&member);
                }
            }
            let event = if is {
                abi::STD_EVENT_ADDED
            } else {
                abi::STD_EVENT_REMOVED
            };
            self.notify_one(event, member, view);
        }
    }
}
