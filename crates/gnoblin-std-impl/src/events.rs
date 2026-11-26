//! Opening the library for a generation, and what it hears from the core.
//!
//! `open` takes stock of the windows and outputs that exist, then watches the tree through the core's own subscriptions: a window
//! appearing, going, and changing; an output appearing, going, and changing. Each handler updates the object model, queues the
//! notices the shell's listeners are owed, posts a turn, and delivers what is queued, outside any borrow.

use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

use gnoblin_core::{Gnoblin, LogLevel, ShellValue, Value};
use gnoblin_core_abi::GnoblinCtx;
use gnoblin_std_abi as abi;

use crate::collection::Source;
use crate::desks::Policy;
use crate::error::{Fail, Res};
use crate::nodes;
use crate::state::{self, ClientRec, Counters, NodeId, Notice, State, Std};
use crate::turn::schedule;

/// The window properties whose changes the library watches.
const WINDOW_PROPERTIES: [&str; 12] = [
    "title",
    "app_id",
    "focused",
    "x",
    "y",
    "width",
    "height",
    "decoration",
    "fullscreen",
    "maximised",
    "minimised",
    "parent_window",
];

/// The screen properties whose changes the library watches.
const SCREEN_PROPERTIES: [&str; 12] = [
    "x",
    "y",
    "width",
    "height",
    "mode",
    "enabled",
    "mirror_of",
    "primary",
    "connector",
    "make",
    "model",
    "identity",
];

impl State {
    pub(crate) fn new(g: Gnoblin, ctx: GnoblinCtx) -> State {
        let mut state = State {
            g,
            ctx,
            opened: false,
            desktop: false,
            clients: Vec::new(),
            rec: HashMap::new(),
            outputs: Vec::new(),
            disp: crate::displays::Displays::default(),
            insets: HashMap::new(),
            workspaces: Vec::new(),
            ws: HashMap::new(),
            colls: HashMap::new(),
            roots: [0; 3],
            desks: Vec::new(),
            regs: HashMap::new(),
            notices: Vec::new(),
            graveyard: Vec::new(),
            slides: Vec::new(),
            holding: HashMap::new(),
            policy: Policy::default(),
            last_focus: None,
            active_written: None,
            publish_active: false,
            counters: Counters::default(),
            placements_written: false,
            unverified: BTreeSet::new(),
            scan_all: false,
            cover_unconfirmed: BTreeSet::new(),
            geometry_waiting: false,
            compute_failed: None,
            anim: crate::animation::engine::AnimState::default(),
            styles: crate::style::cascade::StyleState::default(),
            scene: crate::scene::SceneState::default(),
        };
        let clients = state.new_root(abi::STD_COLLECTION_CLIENTS, Source::Clients);
        let outputs = state.new_root(abi::STD_COLLECTION_OUTPUTS, Source::Outputs);
        let workspaces = state.new_root(abi::STD_COLLECTION_WORKSPACES, Source::Workspaces);
        state.roots = [clients, outputs, workspaces];
        state
    }

    /// Starts tracking a window.
    fn add_client(&mut self, id: NodeId, preexisting: bool) {
        let position = nodes::rect_of(&self.g, id);
        // Only a window that was in the tree when this generation opened can carry the baseline an earlier generation fitted it at:
        // a window that appears later is new, and a record under its id belongs to some other window of the thread's earlier trees
        // (the oracle runs one scenario after another on a thread).
        let carried = if preexisting {
            crate::fit::carried_fit(id)
        } else {
            crate::fit::forget_fit(id);
            None
        };
        self.clients.push(id);
        self.rec.insert(
            id,
            ClientRec {
                preexisting,
                last_pos: (position.x, position.y),
                parent: nodes::node_ref(&self.g, id, "parent_window"),
                // A window an earlier generation fitted keeps that frame's room as its baseline, so this generation's frame (or
                // none) moves the content by what the frame gained or lost.
                fitted: carried,
                ..ClientRec::default()
            },
        );
        crate::object::client::client_arrived(self.ctx, id);
    }

    /// A window the library did not know came into the tree: the listeners hear it, and the window gets a workspace.
    fn client_created(&mut self, id: NodeId) {
        if self.rec.contains_key(&id) {
            return;
        }
        self.add_client(id, false);
        let root = self.roots[0];
        self.notify_one(abi::STD_EVENT_ADDED, id, root);
        self.recompute(abi::STD_COLLECTION_CLIENTS, id);
        let holders = self.holders(abi::STD_COLLECTION_CLIENTS, id);
        self.notices.push(Notice::new(abi::STD_EVENT_READY, id, holders));
        // Before the first turn the windows wait: the turn gives them a workspace, once the shell has made its own.
        if self.opened
            && !self.place_dialog(id)
            && let Some(target) = self.new_window_workspace()
        {
            self.move_client(id, Some(target), true);
        }
    }

    /// A window left the tree: its listeners hear it close, it leaves its workspace and the clients, and the keyboard passes on.
    fn client_destroyed(&mut self, id: NodeId) {
        crate::fit::forget_fit(id);
        let Some(rec) = self.rec.get(&id) else {
            return;
        };
        let (workspace, parent) = (rec.workspace, rec.parent);
        let holders = self.holders(abi::STD_COLLECTION_CLIENTS, id);
        self.notices.push(Notice::new(abi::STD_EVENT_CLOSED, id, holders));
        let was_focused = self.last_focus == Some(id);
        if let Some(wid) = workspace {
            let index = self
                .ws
                .get(&wid)
                .and_then(|ws| ws.clients.iter().position(|one| *one == id));
            let removes = self.ws.get(&wid).is_some_and(|ws| ws.layout.handles_removal());
            if let Some(ws) = self.ws.get_mut(&wid) {
                ws.clients.retain(|one| *one != id);
                ws.dirty = true;
                let coll = ws.coll;
                self.forget_arrival(wid, id);
                self.notices.push(Notice::new(abi::STD_EVENT_REMOVED, id, vec![coll]));
            }
            if was_focused
                && !removes
                && let Some(index) = index
            {
                self.hand_off_focus(wid, index, parent);
            }
        }
        self.clients.retain(|one| *one != id);
        self.unverified.remove(&id);
        let root = self.roots[0];
        self.notify_one(abi::STD_EVENT_REMOVED, id, root);
        self.rec.remove(&id);
        crate::object::client::forget_client(self.ctx, id);
        self.recompute(abi::STD_COLLECTION_CLIENTS, id);
    }

    /// A window property changed.
    fn client_changed(&mut self, id: NodeId, property: &str, value: &Value) {
        if !self.rec.contains_key(&id) {
            return;
        }
        let holders = self.holders(abi::STD_COLLECTION_CLIENTS, id);
        let text = value.as_str().unwrap_or_default().to_owned();
        let mut push = |kind: u32, text: String, flag: u32| {
            let mut notice = Notice::new(kind, id, holders.clone());
            notice.text = text;
            notice.flag = flag;
            self.notices.push(notice);
        };
        match property {
            "title" => push(abi::STD_EVENT_TITLE_CHANGED, text, 0),
            "app_id" => push(abi::STD_EVENT_APP_ID_CHANGED, text, 0),
            "focused" => push(
                abi::STD_EVENT_FOCUS_CHANGED,
                String::new(),
                u32::from(value.as_bool().unwrap_or(false)),
            ),
            "decoration" => {
                push(abi::STD_EVENT_DECORATION_CHANGED, text, 0);
                // The frame's room round the window came or went: the workspace's layout places it again.
                if let Some(ws) = self
                    .rec
                    .get(&id)
                    .and_then(|rec| rec.workspace)
                    .and_then(|workspace| self.ws.get_mut(&workspace))
                {
                    ws.dirty = true;
                }
            }
            "x" | "y" | "width" | "height" => {
                if let Some(rec) = self.rec.get_mut(&id) {
                    rec.geometry_pending = true;
                    self.geometry_waiting = true;
                    // A rectangle the pass wrote and that was seen held may have moved: it is read again for a lost write.
                    if rec.expected.is_some() {
                        self.unverified.insert(id);
                    }
                }
                if property == "x" || property == "y" {
                    self.dialogs_follow(id);
                }
            }
            "parent_window" => {
                let parent = nodes::node_ref(&self.g, id, "parent_window");
                if let Some(rec) = self.rec.get_mut(&id) {
                    rec.parent = parent;
                }
                self.place_dialog(id);
            }
            _ => {}
        }
        if matches!(property, "focused" | "fullscreen" | "maximised") {
            // The layout pass reads these three flags of every window of a workspace (`plan_workspace`): it reads them again.
            self.scan_again(id);
        }
        self.recompute(abi::STD_COLLECTION_CLIENTS, id);
    }

    /// A dialog moves with its parent.
    fn dialogs_follow(&mut self, parent: NodeId) {
        let now = nodes::rect_of(&self.g, parent);
        let Some(rec) = self.rec.get_mut(&parent) else {
            return;
        };
        let (dx, dy) = (now.x - rec.last_pos.0, now.y - rec.last_pos.1);
        rec.last_pos = (now.x, now.y);
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        for dialog in self.dialogs_of(parent) {
            let own = nodes::rect_of(&self.g, dialog);
            nodes::write_num(&self.g, dialog, "x", own.x + dx);
            nodes::write_num(&self.g, dialog, "y", own.y + dy);
        }
    }

    /// An output came into the tree.
    fn output_created(&mut self, id: NodeId) {
        if self.outputs.contains(&id) {
            return;
        }
        self.outputs.push(id);
        let root = self.roots[1];
        self.notify_one(abi::STD_EVENT_ADDED, id, root);
        self.recompute(abi::STD_COLLECTION_OUTPUTS, id);
        if self.output_added(id) {
            self.focus_returned(id);
        }
    }

    /// An output left the tree.
    fn output_destroyed(&mut self, id: NodeId) {
        if !self.outputs.contains(&id) {
            return;
        }
        self.outputs.retain(|one| *one != id);
        self.insets.remove(&id);
        let root = self.roots[1];
        self.notify_one(abi::STD_EVENT_REMOVED, id, root);
        self.recompute(abi::STD_COLLECTION_OUTPUTS, id);
        self.output_left(id);
    }

    /// An output property changed.
    fn output_property(&mut self, id: NodeId, property: &str, value: &Value) {
        if !self.outputs.contains(&id) {
            return;
        }
        let holders = self.holders(abi::STD_COLLECTION_OUTPUTS, id);
        match property {
            "x" | "y" | "width" | "height" => {
                let mut notice = Notice::new(abi::STD_EVENT_GEOMETRY_CHANGED, id, holders);
                notice.rect = crate::desks::output_rect(&self.g, id).unwrap_or_default();
                self.notices.push(notice);
                // The area the workspaces are laid out in changed: each computes again when its pass finds the area is not the one
                // it computed for (`begin_compute`), so nothing is marked here: the pass may have seen the new area already.
            }
            "mode" => {
                let mut notice = Notice::new(abi::STD_EVENT_MODE_CHANGED, id, holders);
                notice.text = value.as_str().unwrap_or_default().to_owned();
                self.notices.push(notice);
            }
            "enabled" | "mirror_of" => self.output_changed(id),
            _ => {}
        }
        self.recompute(abi::STD_COLLECTION_OUTPUTS, id);
    }
}

/// Runs a handler over the state, then posts a turn and tells the listeners what it queued.
fn handle(std: &Rc<Std>, f: impl FnOnce(&mut State)) {
    f(&mut std.st.borrow_mut());
    schedule(std);
    std.drain();
}

/// A shell handed this library a window it has not heard of yet: the core announced the window to the shell's listeners (its
/// `onAdded`) as it read the tree, before this library's own subscription to the same tree change ran. The library takes the window
/// in now, as that subscription would: it gets a record, a workspace and its notices, which are told with the next turn. The
/// subscription's own call, when it comes, finds the window known and does nothing.
///
/// Without this a listener that gave a new window its floating bounds, size limits or frame room in `onAdded` was refused (the
/// window "is not a client"), and the floating layout placed the window itself.
pub(crate) fn hear_of_client(ctx: GnoblinCtx, node: NodeId) {
    // A library with no model yet takes stock of every window when it opens.
    let Some(std) = state::find(ctx) else {
        return;
    };
    {
        // The library calls into the shell only outside a borrow of its state, so it is not borrowed when a shell's member runs; were it,
        // the subscription to the tree change takes the window in as it always did.
        let Ok(mut st) = std.st.try_borrow_mut() else {
            return;
        };
        if st.rec.contains_key(&node) {
            return;
        }
        st.client_created(node);
    }
    schedule(&std);
}

/// Makes the library the generation's desktop, attaching it first when it is not: the default workspace, the adoption of windows
/// and the window policy are the library's from here on.
///
/// # Errors
///
/// The core refuses the ABI, the tree cannot be attached, or a subscription is refused.
pub(crate) fn open(ctx: GnoblinCtx) -> Res<()> {
    let std = open_model(ctx)?;
    let g = {
        let mut st = std.st.borrow_mut();
        if st.desktop {
            return Ok(());
        }
        st.desktop = true;
        st.g.clone()
    };
    schedule(&std);
    register_providers(&g, ctx);
    Ok(())
}

/// Attaches the library to the generation `ctx` with no desktop: it takes stock of the windows and outputs and watches the tree, which
/// is all animation, styles and the scene need. [`open`] adds the desktop later.
///
/// # Errors
///
/// The core refuses the ABI, the tree cannot be attached, or a subscription is refused.
pub(crate) fn open_model(ctx: GnoblinCtx) -> Res<Rc<Std>> {
    if let Some(std) = state::find(ctx) {
        return Ok(std);
    }
    let g = gnoblin_core::private::gnoblin_for(ctx)?;
    g.attach()?;
    let mut st = State::new(g.clone(), ctx);
    for window in g.windows()? {
        st.add_client(window.id().0, true);
    }
    let screen = g.class("screen")?;
    for child in g.root()?.children()? {
        // A node of a class the core's catalog does not have (a compositor newer than the core) is not an output.
        if child.class().is_ok_and(|class| class == screen) {
            st.outputs.push(child.id().0);
        }
    }
    for output in st.outputs.clone() {
        let _ = st.output_added(output);
    }
    let std = Rc::new(Std::new(st));
    state::insert(ctx, Rc::clone(&std));
    if let Err(fail) = subscribe(&std, &g) {
        state::remove(ctx);
        return Err(fail);
    }
    // The windows already open have their `Client`s looked up now: the library runs no turn until something changes, and a window
    // that closes first must still be told of as the core's object.
    crate::object::client::look_up_arrivals(ctx);
    Ok(std)
}

/// Registers std's desktop policy with the core on the generation that opened std (`providers.rs`). A kind someone already holds
/// (the compositor process registered std's own for the stock session, or another shell did) stays with its holder; a failure
/// is the log's business, because a shell with no window manager provider still runs.
fn register_providers(g: &Gnoblin, ctx: GnoblinCtx) {
    match crate::providers::register_defaults(gnoblin_core::private::api_of(g), ctx) {
        Ok(done) if done.kept_by_others.is_empty() => {}
        Ok(done) => g.log_at(
            LogLevel::Debug,
            &format!(
                "std did not register {}: already registered",
                done.kept_by_others.join(", ")
            ),
        ),
        Err(why) => g.log_at(LogLevel::Warn, &format!("std registered no providers: {why}")),
    }
}

/// The core subscriptions that keep the object model current. Each closure holds the library state, which the generation's
/// retirement frees with the subscription.
fn subscribe(std: &Rc<Std>, g: &Gnoblin) -> Res<()> {
    let ctx = std.st.borrow().ctx;
    g.on_shutdown(move || state::retire(ctx))?;

    // The compositor's word that a timed write ended: the transition engine settles the transitions whose tracks all have.
    let ended = Rc::clone(std);
    g.on_signal_named("tree", "track_ended", move |event| {
        if let (
            Some(ShellValue::Uint(node)),
            Some(ShellValue::Text(property)),
            Some(ShellValue::Text(reason)),
            Some(ShellValue::Uint(tween)),
        ) = (event.value(0), event.value(1), event.value(2), event.value(3))
        {
            handle(&ended, |st| {
                st.track_ended(node, property, reason == "completed", tween)
            });
        }
    })?;

    let created = Rc::clone(std);
    g.on_client_created(move |window| handle(&created, |st| st.client_created(window.id().0)))?;
    let destroyed = Rc::clone(std);
    g.on_client_destroyed(move |window| {
        handle(&destroyed, |st| st.client_destroyed(window.id().0));
        // A drag or a gesture that pressed the window is cancelled once it is gone.
        crate::bindings::client_closed(&destroyed, window.id().0);
    })?;
    for property in WINDOW_PROPERTIES {
        let changed = Rc::clone(std);
        g.on_window_property_changed(property, move |window, value| {
            handle(&changed, |st| st.client_changed(window.id().0, property, value));
        })?;
    }

    // The keyboard moved: nothing to update now (a window that closes in the same batch as the focus changes is the one that had
    // it), but the next turn notes who has it.
    let focused = Rc::clone(std);
    g.on_class_property_changed("seat", "focus", move |_node, _value| handle(&focused, |_st| {}))
        .map_err(Fail::from)?;

    let appeared = Rc::clone(std);
    g.on_node_created("screen", move |node| {
        handle(&appeared, |st| st.output_created(node.id().0))
    })?;
    let gone = Rc::clone(std);
    g.on_node_destroyed("screen", move |node| {
        handle(&gone, |st| st.output_destroyed(node.id().0))
    })?;
    for property in SCREEN_PROPERTIES {
        let changed = Rc::clone(std);
        g.on_class_property_changed("screen", property, move |node, value| {
            handle(&changed, |st| st.output_property(node.id().0, property, value));
        })
        .map_err(Fail::from)?;
    }
    Ok(())
}
