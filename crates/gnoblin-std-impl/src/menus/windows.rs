//! `window_menus`: answers the compositor's `menu` requests, opens one menu at a time, restores the keyboard when it closes, and keeps
//! the windows a person pinned "Always on top" above the others.
//!
//! The entries a window's menu has depend on the window: whether it is maximised, fullscreen, tiled or snapped, which workspaces there
//! are and which it is on. They read the library's own state (`State`: the client records and the workspaces) and the window's node, and
//! what an entry does is done here too. The two things that belong to the window policy and are not the library's yet (the keyboard move
//! and resize modes) are callbacks the shell gives the service, and an entry whose callback is missing is shown disabled.

use std::collections::HashMap;
use std::rc::Rc;

use gnoblin_core::Rect;
use gnoblin_core_abi as abi;
use gnoblin_object::server::{Cx, ShellImpl};
use gnoblin_object::{Closure, Obj, Res, Val};

use super::card;
use super::entry::{Entry, EntryRun, MenuAction, Sub};
use crate::floating::shrink;
use crate::nodes;
use crate::object::record::{edit, look};
use crate::object::with_lib;
use crate::ui::{Cid, Ui};

/// What a shell implemented of `menu_provider`.
#[derive(Clone)]
pub(crate) struct MenuProviderRef {
    pub(crate) obj: Obj,
    pub(crate) shell: Rc<ShellImpl>,
}

/// The window menu of one generation.
#[derive(Default)]
pub(crate) struct State {
    pub(crate) started: bool,
    /// The `window_menus` object, which owns what the library makes when a request opens a menu.
    pub(crate) owner: Option<Obj>,
    /// The open menu.
    pub(crate) current: Option<Cid>,
    pub(crate) provider: Option<MenuProviderRef>,
    /// The windows pinned above the others.
    pub(crate) above: Vec<u64>,
    pub(crate) move_with_keys: Option<Closure>,
    pub(crate) resize_with_keys: Option<Closure>,
    /// The rectangle a window had before "Maximise" filled the work area, which "Restore" gives back.
    pub(crate) kept: HashMap<u64, Rect>,
}

/// The ids of `menu_provider`'s methods, by the shell's implementation of them: asked once.
fn provider_member(name: &str) -> Option<u32> {
    with_lib(|lib| {
        let class = lib.class_id("menu_provider")?;
        lib.member_id(class, abi::MEMBER_METHOD, name)
    })
    .ok()
}

impl MenuProviderRef {
    /// Calls `name` when the shell defined it: `None` when it did not and the library's own runs.
    fn call(&self, name: &str, args: &[Val]) -> Option<Res<Val>> {
        let member = provider_member(name)?;
        self.shell.has(member).then(|| self.shell.call(member, args))
    }
}

// ---- the window, as the menu reads it ------------------------------------------------------------------------------------------------

/// Whether `client` is a window that is still there.
pub(crate) fn alive(ui: &Ui, client: u64) -> bool {
    let node = ui.node(client);
    node.is_alive() && node.class_name().is_ok_and(|class| class == "window")
}

fn flag(ui: &Ui, client: u64, key: &str) -> bool {
    nodes::flag(&ui.g, client, key)
}

/// The workspace `client` is on.
fn workspace_of(ui: &Ui, client: u64) -> Option<u64> {
    look(ui.ctx, |st| Ok(st.rec.get(&client).and_then(|rec| rec.workspace)))
        .ok()
        .flatten()
}

/// Whether the library holds the window as snapped to an edge.
fn snapped(ui: &Ui, client: u64) -> bool {
    look(ui.ctx, |st| {
        Ok(st.rec.get(&client).is_some_and(|rec| rec.snap.is_some()))
    })
    .ok()
    .unwrap_or(false)
}

/// The windows under the root, back to front.
fn stack(ui: &Ui) -> Vec<u64> {
    let Ok(root) = ui.g.root() else {
        return Vec::new();
    };
    root.children()
        .unwrap_or_default()
        .into_iter()
        .filter(|node| node.class_name().is_ok_and(|class| class == "window"))
        .map(|node| node.id().0)
        .collect()
}

/// Raises `client` above the other windows.
fn raise(ui: &Ui, client: u64) {
    if let Ok(root) = ui.g.root() {
        let _ = ui.node(client).insert_into(&root, None);
    }
}

// ---- the stock entries --------------------------------------------------------------------------------------------------------------

/// The stock entries for a window. Move and Resize are disabled for a window that is not floating or when no keyboard mode was given.
pub(crate) fn default_entries(ui: &Ui, client: u64) -> Vec<Entry> {
    let maximised = flag(ui, client, "maximised");
    let fullscreen = flag(ui, client, "fullscreen");
    let floating = !(maximised || fullscreen || flag(ui, client, "tiled"));
    let restores = maximised || fullscreen || snapped(ui, client);
    let menus = ui.menus.borrow();
    let mut entries = vec![
        if restores {
            Entry::of("Restore", MenuAction::Restore)
        } else {
            Entry::of("Maximise", MenuAction::Maximise)
        },
        Entry::of("Minimise", MenuAction::Minimise),
        Entry::gap(),
    ];
    let mut shown = Entry::of("Move", MenuAction::MoveWithKeys);
    shown.enabled = floating && (menus.move_with_keys.is_some() || crate::policy::repositions(ui));
    entries.push(shown);
    let mut shown = Entry::of("Resize", MenuAction::ResizeWithKeys);
    shown.enabled = floating && (menus.resize_with_keys.is_some() || crate::policy::repositions(ui));
    entries.push(shown);
    let mut above = Entry::of("Always on top", MenuAction::ToggleAbove);
    above.checked = menus.above.contains(&client);
    entries.push(above);
    entries.push(Entry {
        label: "Move to workspace".to_owned(),
        enabled: !fullscreen,
        checked: false,
        separator: false,
        run: EntryRun::None,
        sub: Sub::Workspaces,
    });
    entries.push(Entry::gap());
    entries.push(Entry::of("Close", MenuAction::Close));
    entries
}

/// The "Move to workspace" list: a workspace each, ticked and disabled for the one the window is on.
pub(crate) fn workspace_entries(ui: &Ui, client: u64) -> Vec<Entry> {
    let current = workspace_of(ui, client);
    look(ui.ctx, |st| {
        Ok(st
            .workspaces
            .iter()
            .filter_map(|id| {
                let workspace = st.workspace(*id).ok()?;
                let mut entry = Entry::of(
                    if workspace.name.is_empty() {
                        &workspace.id
                    } else {
                        &workspace.name
                    },
                    MenuAction::MoveToWorkspace(workspace.id.clone()),
                );
                entry.checked = current == Some(*id);
                entry.enabled = current != Some(*id);
                Some(entry)
            })
            .collect())
    })
    .unwrap_or_default()
}

// ---- what an entry does -------------------------------------------------------------------------------------------------------------

/// Does what a stock entry says, for `client`.
pub(crate) fn act(ui: &Rc<Ui>, client: u64, action: &MenuAction) {
    match action {
        MenuAction::Maximise => maximise(ui, client, true),
        MenuAction::Restore => {
            if flag(ui, client, "fullscreen") {
                nodes::write_flag(&ui.g, client, "fullscreen", false);
            } else if flag(ui, client, "maximised") {
                maximise(ui, client, false);
            } else {
                let _ = edit(ui.ctx, |st| Ok(st.unsnap_client(client, false)));
            }
        }
        MenuAction::Minimise => minimise(ui, client),
        MenuAction::MoveWithKeys | MenuAction::ResizeWithKeys => {
            let run = {
                let menus = ui.menus.borrow();
                if *action == MenuAction::MoveWithKeys {
                    menus.move_with_keys.clone()
                } else {
                    menus.resize_with_keys.clone()
                }
            };
            match run {
                Some(run) => {
                    if let Err(fail) = run.call(0, &[Val::Text(client.to_string())]) {
                        ui.warn(&format!("the keyboard move or resize threw: {fail}"));
                    }
                }
                // No function of the shell's: the policy's own keyboard modes, when they were started.
                None => crate::policy::begin_reposition(ui, client, *action == MenuAction::ResizeWithKeys),
            }
        }
        MenuAction::ToggleAbove => {
            let on = !is_above(ui, client);
            set_above(ui, client, on);
        }
        MenuAction::Close => {
            let _ = ui.node(client).call("close", &[]);
        }
        MenuAction::MoveToWorkspace(id) => {
            let _ = edit(ui.ctx, |st| {
                if let Some(workspace) = st
                    .workspaces
                    .iter()
                    .copied()
                    .find(|held| st.workspace(*held).is_ok_and(|workspace| workspace.id == *id))
                {
                    let _ = st.transfer_client(workspace, client);
                }
                Ok(())
            });
        }
    }
}

/// Fills the work area and keeps the floating rectangle (`on`), or leaves maximised and returns the floating rectangle. Refused when
/// the window is already maximised or fullscreen (`on`), or is not maximised.
fn maximise(ui: &Rc<Ui>, client: u64, on: bool) {
    let maximised = flag(ui, client, "maximised");
    if on {
        if maximised || flag(ui, client, "fullscreen") {
            return;
        }
        let before = nodes::rect_of(&ui.g, client);
        ui.menus.borrow_mut().kept.insert(client, before);
        let _ = edit(ui.ctx, |st| {
            if st.rec.get(&client).is_some_and(|rec| rec.snap.is_some()) {
                st.unsnap_client(client, false);
            }
            let before = nodes::rect_of(&st.g, client);
            let inset = st.inset_of(client);
            if let Some(rec) = st.rec.get_mut(&client) {
                rec.float_rect = Some(before);
            }
            nodes::write_flag(&st.g, client, "maximised", true);
            if let Some(area) = st.work_area_of_client(client) {
                nodes::write_rect(&st.g, client, &shrink(&area, &inset));
            }
            Ok(())
        });
    } else {
        if !maximised {
            return;
        }
        nodes::write_flag(&ui.g, client, "maximised", false);
        if let Some(before) = ui.menus.borrow_mut().kept.remove(&client) {
            nodes::write_rect(&ui.g, client, &before);
        }
    }
}

/// Hides the window and hands the keyboard on, as the window manager did: the topmost sibling still showing, or nobody. A window that
/// did not hold the keyboard leaves it where it is.
fn minimise(ui: &Rc<Ui>, client: u64) {
    if flag(ui, client, "minimised") {
        return;
    }
    nodes::write_flag(&ui.g, client, "minimised", true);
    if ui.seat_focus() != Some(client) {
        return;
    }
    let siblings = ui
        .node(client)
        .parent()
        .ok()
        .flatten()
        .and_then(|parent| parent.children().ok())
        .unwrap_or_default();
    let next = siblings
        .iter()
        .rev()
        .map(|node| node.id().0)
        .find(|id| *id != client && alive(ui, *id) && !flag(ui, *id, "minimised"));
    let _ = ui.set_focus(next.unwrap_or(0));
}

// ---- opening and closing --------------------------------------------------------------------------------------------------------------

/// Whether `client` is pinned above the windows that are not.
pub(crate) fn is_above(ui: &Ui, client: u64) -> bool {
    ui.menus.borrow().above.contains(&client)
}

/// Pins `client` above the others, or lets it go. A pinned window is raised at once.
pub(crate) fn set_above(ui: &Rc<Ui>, client: u64, on: bool) {
    if on {
        {
            let mut menus = ui.menus.borrow_mut();
            if !menus.above.contains(&client) {
                menus.above.push(client);
            }
        }
        raise(ui, client);
        ui.schedule();
    } else {
        ui.menus.borrow_mut().above.retain(|held| *held != client);
    }
}

/// The entries of `client`'s menu: the provider's, or the stock ones.
///
/// # Errors
///
/// The provider's answer is not a list of entries.
pub(crate) fn items_in(ui: &Rc<Ui>, cx: &Cx<'_>, client: u64) -> Res<Vec<Entry>> {
    let provider = ui.menus.borrow().provider.clone();
    match provider.and_then(|held| held.call("items", &[Val::Text(client.to_string())])) {
        Some(answer) => {
            let answer = answer?;
            let read = super::entry::read_all(cx, &answer);
            super::entry::settle(cx, &answer);
            read
        }
        None => Ok(default_entries(ui, client)),
    }
}

/// Closes any open menu, then opens the stock one at (`x`, `y`) unless the window or the session cannot show it or it is empty.
///
/// # Errors
///
/// A canvas or an object of the card cannot be made.
pub(crate) fn open_in(ui: &Rc<Ui>, cx: &mut Cx<'_>, client: u64, x: f64, y: f64) -> Res<Option<Obj>> {
    close(ui);
    if !ui.menus.borrow().started
        || !alive(ui, client)
        || flag(ui, client, "minimised")
        || ui.locked()
        || ui.primary_screen().is_none()
    {
        return Ok(None);
    }
    if let Some(workspace) = workspace_of(ui, client)
        && !look(ui.ctx, |st| Ok(st.is_shown(workspace))).unwrap_or(true)
    {
        return Ok(None);
    }
    let entries = match items_in(ui, cx, client) {
        Ok(entries) => entries,
        Err(fail) => {
            ui.warn(&format!("the window menu's items: {fail}"));
            return Ok(None);
        }
    };
    if entries.is_empty() {
        return Ok(None);
    }
    let before = ui.seat_focus().unwrap_or(0);
    let (obj, menu) = card::build(ui, cx, client, &entries, (x, y), before, flag(ui, client, "secure"))?;
    ui.mount_top(menu)?;
    ui.menus.borrow_mut().current = Some(menu);
    if let Some(node) = ui.read(menu, |widget| widget.node) {
        ui.focus_node(node)?;
    }
    announce_current(ui);
    Ok(Some(obj))
}

/// Opens `client`'s menu at (`x`, `y`), or at the pointer when they are left out. The provider's `show` is asked first.
///
/// # Errors
///
/// As [`open_in`].
pub(crate) fn show_in(ui: &Rc<Ui>, cx: &mut Cx<'_>, client: u64, x: Option<f64>, y: Option<f64>) -> Res<Option<Obj>> {
    let pointer = ui.pointer_place().unwrap_or((0.0, 0.0));
    let (x, y) = (x.unwrap_or(pointer.0), y.unwrap_or(pointer.1));
    let provider = ui.menus.borrow().provider.clone();
    if let Some(shown) =
        provider.and_then(|held| held.call("show", &[Val::Text(client.to_string()), Val::Double(x), Val::Double(y)]))
    {
        shown?;
        return Ok(None);
    }
    open_in(ui, cx, client, x, y)
}

/// Closes the open menu and gives the keyboard back. Answers whether a menu was open.
pub(crate) fn close(ui: &Rc<Ui>) -> bool {
    let Some(menu) = ui.menus.borrow_mut().current.take() else {
        return false;
    };
    let held = card::part(ui, menu);
    // `closed` is told from the library's next turn, and the kill below ends the card's object at once when no shell holds a handle to it (a
    // script's is gone when its handler returns, while the listener it added stays), which takes the listener with it. The library holds
    // the card until the turn that tells `closed` has run.
    let told = ui.read(menu, |widget| widget.obj).filter(|obj| {
        ui.with_cx(*obj, |cx| {
            let kept = cx.hold(*obj).is_ok();
            if kept {
                cx.emit("closed", Vec::new());
            }
            kept
        }) == Some(true)
    });
    ui.kill(menu);
    if let Some(obj) = told {
        // Queued after what the kill told, so the turn lets the card go once all of it has been heard.
        ui.with_cx(obj, |cx| cx.later(move |cx| cx.unhold(obj)));
    }
    // The node it came from may be gone: the compositor refuses a focus on a node it has not got, so ask and go on.
    let before = held.map_or(0, |held| held.before);
    if ui.focus_node(before).is_err() {
        let _ = ui.focus_node(0);
    }
    true
}

/// Tells a shell that listens to the service that a menu opened. The menu says itself when it closes (`closed`), so a shell has the
/// menu in hand when it hears of the end.
fn announce_current(ui: &Ui) {
    let owner = ui.menus.borrow().owner;
    if let Some(owner) = owner {
        ui.announce(owner, |cx| cx.changed("current"));
    }
}

/// A menu request from the compositor: opens the menu of the window it names.
pub(crate) fn on_request(ui: &Rc<Ui>, node: u64, x: f64, y: f64) {
    let owner = {
        let menus = ui.menus.borrow();
        if !menus.started {
            return;
        }
        menus.owner
    };
    if !alive(ui, node) {
        return;
    }
    let Some(owner) = owner else {
        return;
    };
    let ui = Rc::clone(ui);
    let _ = ui.with_cx(owner, |cx| {
        if let Err(fail) = show_in(&ui, cx, node, Some(x), Some(y)) {
            ui.warn(&format!("the window menu did not open: {fail}"));
        }
    });
}

/// Opens the menu of window `node` at the pointer, for a click on a frame that asks for it. Nothing happens when the service has not started.
pub(crate) fn show_at_pointer(ui: &Rc<Ui>, node: u64) {
    let owner = {
        let menus = ui.menus.borrow();
        if !menus.started {
            return;
        }
        menus.owner
    };
    let Some(owner) = owner else {
        return;
    };
    let ui = Rc::clone(ui);
    let _ = ui.with_cx(owner, |cx| {
        if let Err(fail) = show_in(&ui, cx, node, None, None) {
            ui.warn(&format!("the window menu did not open: {fail}"));
        }
    });
}

/// Closes the menu when its window goes or the session locks, and keeps pinned windows above unpinned ones. Runs once a frame of the
/// toolkit.
pub(crate) fn watch(ui: &Rc<Ui>) {
    if !ui.menus.borrow().started {
        return;
    }
    let open = ui.menus.borrow().current;
    if let Some(menu) = open {
        let gone = card::part(ui, menu).is_none_or(|held| !alive(ui, held.client));
        if gone || ui.locked() {
            close(ui);
        }
    }
    keep_pinned(ui);
}

/// Whether the toolkit has to run each frame for the menu: while a window is pinned, something else may raise a window over it.
pub(crate) fn polls(ui: &Ui) -> bool {
    let menus = ui.menus.borrow();
    menus.started && !menus.above.is_empty()
}

/// Raises the pinned windows when an unpinned window would cover one. A fullscreen window stays above the pinned ones.
fn keep_pinned(ui: &Rc<Ui>) {
    {
        let mut menus = ui.menus.borrow_mut();
        if menus.above.is_empty() {
            return;
        }
        let gone: Vec<u64> = menus
            .above
            .iter()
            .copied()
            .filter(|client| !alive(ui, *client))
            .collect();
        menus.above.retain(|client| !gone.contains(client));
        if menus.above.is_empty() {
            return;
        }
    }
    let above = ui.menus.borrow().above.clone();
    let order = stack(ui);
    let Some(first) = order.iter().position(|client| above.contains(client)) else {
        return;
    };
    let loose = order
        .iter()
        .enumerate()
        .any(|(at, client)| at > first && !above.contains(client) && !flag(ui, *client, "fullscreen"));
    if !loose {
        return;
    }
    for client in order.iter().filter(|client| above.contains(client)) {
        raise(ui, *client);
    }
    for client in order.iter().filter(|client| flag(ui, **client, "fullscreen")) {
        raise(ui, *client);
    }
}

/// Starts answering menu requests, once.
pub(crate) fn start(ui: &Rc<Ui>, cx: &Cx<'_>) {
    {
        let mut menus = ui.menus.borrow_mut();
        if menus.started {
            return;
        }
        menus.started = true;
        menus.owner = Some(cx.me());
    }
    // A window going and the lock engaging close the menu from the toolkit's next frame.
    let ctx = ui.ctx;
    let wake = move || {
        if let Some(ui) = crate::ui::find(ctx) {
            ui.schedule();
        }
    };
    let _ = ui.g.on_client_destroyed(move |_| wake());
    for property in ["state", "engaged"] {
        let _ = ui.g.on_class_property_changed("lock", property, move |_, _| wake());
    }
    ui.schedule();
}
