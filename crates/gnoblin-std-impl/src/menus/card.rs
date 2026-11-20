//! The card: the menu layer, its columns of rows, and what they do with the pointer and the keys.
//!
//! The layer (`window_menu`) is a canvas as big as the screen that takes every press outside the cards and holds the keyboard focus. It has
//! a card (`menu_panel`), and a second one beside it while a submenu is open. A card holds a `menu_item` for each entry and a separator for
//! each gap. Rows are made in a fixed order (edge, check, arrow, label, edge), so the nodes they make number the same on every run.

use std::rc::Rc;

use gnoblin_object::server::Cx;
use gnoblin_object::{Fail, Obj, Res, Val};
use gnoblin_paint::PaintOp;

use super::entry::{Entry, EntryRun, Sub};
use super::metrics::{
    CARD, CLEAR, EDGE, HOT, HOT_TEXT, ITEM_H, KeyAction, LEFT, LINE, MARK, MIN_WIDTH, OFF, PAD, SEPARATOR_H, TEXT,
    key_action, menu_height, place_card,
};
use super::navigator::{Nav, NavRow};
use super::windows;
use crate::ui::{CanvasEvent, CanvasHandle, Cid, Special, Ui, WidgetKind, en, paint};

/// What a mark draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MarkKind {
    None,
    Check,
    Arrow,
}

/// A check or a submenu arrow, drawn from lines so it does not rely on a font glyph.
#[derive(Clone, Debug)]
pub(crate) struct MarkPart {
    pub(crate) kind: MarkKind,
    pub(crate) color: String,
}

/// What a row adds to a box.
#[derive(Clone)]
pub(crate) struct ItemPart {
    pub(crate) entry: Entry,
    pub(crate) selected: bool,
    pub(crate) label: Cid,
    pub(crate) check: Cid,
    pub(crate) arrow: Cid,
}

/// Where a card goes along x.
#[derive(Clone, Copy, Debug)]
pub(crate) enum CardPlace {
    /// The main card: anchored where the menu was asked for, and kept inside the screen.
    Anchor(f64),
    /// A submenu: beside this card, on the other side when there is no room.
    Beside(Cid),
}

/// What a card adds to a box.
#[derive(Clone)]
pub(crate) struct PanelPart {
    pub(crate) rows: Vec<Cid>,
    pub(crate) place: CardPlace,
}

/// What the menu layer adds to a box.
#[derive(Clone)]
pub(crate) struct MenuPart {
    pub(crate) client: u64,
    pub(crate) before: u64,
    pub(crate) panel: Cid,
    pub(crate) sub: Option<Cid>,
    pub(crate) nav: Nav,
    pub(crate) nav_obj: Obj,
}

/// A row as the navigator asks about it.
pub(crate) fn row_of(ui: &Ui) -> impl Fn(Cid) -> NavRow + '_ {
    move |cid| {
        ui.read(cid, |widget| match &widget.sp {
            Special::Item(item) => NavRow {
                label: item.entry.label.clone(),
                enabled: item.entry.enabled,
            },
            _ => NavRow {
                label: String::new(),
                enabled: false,
            },
        })
        .unwrap_or(NavRow {
            label: String::new(),
            enabled: false,
        })
    }
}

/// Makes a canvas of `kind` in an object of its own, owned by `owner` unless there is none: the layer owns itself until it closes.
fn spawn(ui: &Rc<Ui>, cx: &mut Cx<'_>, kind: WidgetKind, owner: Option<Obj>) -> Res<(Obj, Cid)> {
    match owner {
        Some(owner) => ui.spawn(cx, kind, owner),
        None => {
            let cid = ui.make(kind)?;
            let obj = cx.spawn(kind.class(), CanvasHandle { ui: Rc::clone(ui), cid })?;
            ui.tweak(cid, |widget| widget.obj = obj);
            ui.schedule();
            Ok((obj, cid))
        }
    }
}

/// A plain box of a given width: an edge's spacer.
fn spacer(ui: &Rc<Ui>, cx: &mut Cx<'_>, owner: Obj, width: f64) -> Res<Cid> {
    let (_, cid) = ui.spawn(cx, WidgetKind::Box, owner)?;
    ui.tweak(cid, |widget| widget.width = Some(width));
    Ok(cid)
}

/// Makes a row for `entry`: a check column, the label, and a mark when it opens a submenu.
///
/// # Errors
///
/// A canvas cannot be made or put in place.
pub(crate) fn build_item(ui: &Rc<Ui>, cx: &mut Cx<'_>, entry: &Entry, owner: Obj) -> Res<(Obj, Cid)> {
    let (me, row) = ui.spawn(cx, WidgetKind::MenuItem, owner)?;
    fill_item(ui, cx, me, row, entry)?;
    Ok((me, row))
}

/// Makes what a row has of its own, under the canvas `row` of the object `me`.
///
/// # Errors
///
/// A canvas cannot be made or put in place.
pub(crate) fn fill_item(ui: &Rc<Ui>, cx: &mut Cx<'_>, me: Obj, row: Cid, entry: &Entry) -> Res<()> {
    let (_, check) = ui.spawn(cx, WidgetKind::MenuMark, me)?;
    let (_, arrow) = ui.spawn(cx, WidgetKind::MenuMark, me)?;
    let (_, label) = ui.spawn(cx, WidgetKind::Text, me)?;
    ui.tweak(row, |widget| {
        widget.b.direction = en::DIRECTION_ROW;
        widget.b.gap = 4.0;
        widget.height = Some(ITEM_H);
        widget.b.radius = 6.0;
    });
    for (mark, kind) in [
        (check, if entry.checked { MarkKind::Check } else { MarkKind::None }),
        (arrow, if entry.opens() { MarkKind::Arrow } else { MarkKind::None }),
    ] {
        ui.tweak(mark, |widget| {
            widget.width = Some(MARK);
            widget.sp = Special::Mark(MarkPart {
                kind,
                color: TEXT.to_owned(),
            });
        });
    }
    ui.tweak(label, |widget| {
        widget.t.text.clone_from(&entry.label);
        widget.t.size = 14.0;
        widget.b.grow = 1.0;
    });
    ui.tweak(row, |widget| {
        widget.sp = Special::Item(Box::new(ItemPart {
            entry: entry.clone(),
            selected: false,
            label,
            check,
            arrow,
        }));
    });
    let edge = spacer(ui, cx, me, EDGE)?;
    ui.link(row, edge)?;
    ui.link(row, check)?;
    ui.link(row, label)?;
    ui.link(row, arrow)?;
    let edge = spacer(ui, cx, me, EDGE)?;
    ui.link(row, edge)?;
    Ok(())
}

/// Makes the rule between groups of entries.
fn build_separator(ui: &Rc<Ui>, cx: &mut Cx<'_>, owner: Obj) -> Res<Cid> {
    let (_, rule) = ui.spawn(cx, WidgetKind::MenuSeparator, owner)?;
    ui.tweak(rule, |widget| widget.height = Some(SEPARATOR_H));
    Ok(rule)
}

/// Makes a card of `entries` under the menu, its top at `y`.
fn build_panel(
    ui: &Rc<Ui>,
    cx: &mut Cx<'_>,
    menu: (Obj, Cid),
    entries: &[Entry],
    y: f64,
    place: CardPlace,
    secure: bool,
) -> Res<Cid> {
    let (me, panel) = ui.spawn(cx, WidgetKind::MenuPanel, menu.0)?;
    ui.tweak(panel, |widget| {
        widget.b.direction = en::DIRECTION_COLUMN;
        widget.b.radius = 10.0;
        widget.b.fill = CARD.to_owned();
        widget.b.padding = PAD;
        widget.b.gap = 0.0;
        widget.secure = secure;
        widget.height = Some(menu_height(entries.iter().map(|entry| entry.separator)));
        widget.y = y;
    });
    ui.link(menu.1, panel)?;
    let mut rows = Vec::new();
    for entry in entries {
        if entry.separator {
            let rule = build_separator(ui, cx, me)?;
            ui.link(panel, rule)?;
        } else {
            let (_, row) = build_item(ui, cx, entry, me)?;
            rows.push(row);
            ui.link(panel, row)?;
        }
    }
    ui.tweak(panel, |widget| {
        widget.sp = Special::Panel(Box::new(PanelPart { rows, place }))
    });
    Ok(panel)
}

/// The rows of a card.
fn rows_of(ui: &Ui, panel: Cid) -> Vec<Cid> {
    ui.read(panel, |widget| match &widget.sp {
        Special::Panel(part) => part.rows.clone(),
        _ => Vec::new(),
    })
    .unwrap_or_default()
}

/// Opens a menu layer over the screen: its first card holds `entries`, anchored at `at`. The caller mounts it and gives it the keyboard.
///
/// # Errors
///
/// A canvas or an object cannot be made.
pub(crate) fn build(
    ui: &Rc<Ui>,
    cx: &mut Cx<'_>,
    client: u64,
    entries: &[Entry],
    at: (f64, f64),
    before: u64,
    secure: bool,
) -> Res<(Obj, Cid)> {
    let (screen_width, screen_height) = ui.primary_screen().unwrap_or((0.0, 0.0));
    let (me, menu) = spawn(ui, cx, WidgetKind::Menu, None)?;
    ui.tweak(menu, |widget| {
        widget.x = 0.0;
        widget.y = 0.0;
        widget.width = Some(screen_width);
        widget.height = Some(screen_height);
    });
    let height = menu_height(entries.iter().map(|entry| entry.separator));
    let y = place_card(at, (0.0, height), (screen_width, screen_height)).1;
    let panel = build_panel(ui, cx, (me, menu), entries, y, CardPlace::Anchor(at.0), secure)?;
    let rows = rows_of(ui, panel);
    let nav_obj = cx.spawn("menu_navigator", super::classes::NavHandle::of_menu(ui, menu))?;
    cx.set_parent(nav_obj, me)?;
    ui.tweak(menu, |widget| {
        widget.sp = Special::Menu(Box::new(MenuPart {
            client,
            before,
            panel,
            sub: None,
            nav: Nav::new(rows),
            nav_obj,
        }));
    });
    Ok((me, menu))
}

// ---- what the menu reads of itself --------------------------------------------------------------------------------------------------

/// The menu part of a canvas.
pub(crate) fn part(ui: &Ui, menu: Cid) -> Option<MenuPart> {
    ui.read(menu, |widget| match &widget.sp {
        Special::Menu(part) => Some((**part).clone()),
        _ => None,
    })
    .flatten()
}

/// The selection of a menu.
pub(crate) fn nav_of(ui: &Ui, menu: Cid) -> Option<Nav> {
    part(ui, menu).map(|held| held.nav)
}

/// Writes a menu's selection back, and says what moved on the navigator object a shell may be listening to.
pub(crate) fn set_nav(ui: &Ui, menu: Cid, nav: Nav) {
    let before = part(ui, menu).map(|held| (held.nav_obj, held.nav.depth(), held.nav.index()));
    let now = (nav.depth(), nav.index());
    ui.tweak(menu, |widget| {
        if let Special::Menu(part) = &mut widget.sp {
            part.nav = nav;
        }
    });
    if let Some((obj, depth, index)) = before
        && (depth, index) != now
    {
        ui.announce(obj, |cx| {
            cx.changed("selected");
            cx.changed("index");
            if depth != now.0 {
                cx.changed("depth");
            }
        });
    }
    // The rows show the selection from the toolkit's next turn.
    ui.schedule();
}

/// Changes the selection of `menu` with `work`, which may ask about rows.
pub(crate) fn with_nav<R>(ui: &Ui, menu: Cid, work: impl FnOnce(&mut Nav) -> R) -> Option<R> {
    let mut nav = nav_of(ui, menu)?;
    let out = work(&mut nav);
    set_nav(ui, menu, nav);
    Some(out)
}

/// The menu a canvas is in: itself, or the nearest one above it.
fn menu_over(ui: &Ui, cid: Cid) -> Option<Cid> {
    let w = ui.w.borrow();
    let mut at = Some(cid);
    while let Some(next) = at {
        let widget = w.get(next)?;
        if widget.kind == WidgetKind::Menu {
            return Some(next);
        }
        at = widget.parent;
    }
    None
}

// ---- what a row does -----------------------------------------------------------------------------------------------------------------

/// Runs an entry: opens its submenu, or closes the menu and then runs it.
pub(crate) fn activate(ui: &Rc<Ui>, menu: Cid, row: Cid) {
    let Some(entry) = item_entry(ui, row) else {
        return;
    };
    let Some(client) = part(ui, menu).map(|held| held.client) else {
        return;
    };
    if !entry.enabled {
        return;
    }
    if entry.opens() {
        open_submenu(ui, menu, row);
        return;
    }
    windows::close(ui);
    match entry.run {
        EntryRun::None => {}
        EntryRun::Closure(run) => {
            if let Err(fail) = run.call(0, &[Val::Text(client.to_string())]) {
                ui.warn(&format!("menu entry {:?} threw: {fail}", entry.label));
            }
        }
        EntryRun::MenuAction(action) => windows::act(ui, client, &action),
    }
}

/// The entry a row was made from.
pub(crate) fn item_entry(ui: &Ui, row: Cid) -> Option<Entry> {
    ui.read(row, |widget| match &widget.sp {
        Special::Item(item) => Some(item.entry.clone()),
        _ => None,
    })
    .flatten()
}

/// Opens `row`'s submenu to the right of its card, or to the left when there is no room.
pub(crate) fn open_submenu(ui: &Rc<Ui>, menu: Cid, row: Cid) {
    let Some(obj) = ui.read(menu, |widget| widget.obj) else {
        return;
    };
    let _ = ui.with_cx(obj, |cx| {
        if let Err(fail) = open_submenu_in(ui, cx, menu, row) {
            ui.warn(&format!("the submenu did not open: {fail}"));
        }
    });
}

fn open_submenu_in(ui: &Rc<Ui>, cx: &mut Cx<'_>, menu: Cid, row: Cid) -> Res<()> {
    let Some(held) = part(ui, menu) else {
        return Ok(());
    };
    let Some(entry) = item_entry(ui, row) else {
        return Ok(());
    };
    let entries = match entry.sub {
        Sub::None => return Ok(()),
        Sub::List(list) => list,
        Sub::Source(source) => {
            let answer = source
                .call(0, &[Val::Text(held.client.to_string())])
                .map_err(|fail| Fail::err(format!("the submenu's entries: {fail}")))?;
            let read = super::entry::read_all(cx, &answer);
            super::entry::settle(cx, &answer);
            read?
        }
        Sub::Workspaces => windows::workspace_entries(ui, held.client),
    };
    if entries.is_empty() {
        return Ok(());
    }
    close_submenu(ui, menu);
    let Some(menu_obj) = ui.read(menu, |widget| widget.obj) else {
        return Ok(());
    };
    let (panel_y, menu_height_now) = (
        ui.read(held.panel, |widget| widget.y).unwrap_or(0.0),
        ui.height_of(menu),
    );
    let row_y = ui.read(row, |widget| widget.y).unwrap_or(0.0);
    let at = row_y + panel_y;
    let wanted = menu_height(entries.iter().map(|entry| entry.separator));
    let y = 0.0_f64.max((at - PAD).min(menu_height_now - wanted));
    let secure = ui.read(held.panel, |widget| widget.secure).unwrap_or(false);
    let sub = build_panel(
        ui,
        cx,
        (menu_obj, menu),
        &entries,
        y,
        CardPlace::Beside(held.panel),
        secure,
    )?;
    let rows = rows_of(ui, sub);
    ui.tweak(menu, |widget| {
        if let Special::Menu(part) = &mut widget.sp {
            part.sub = Some(sub);
        }
    });
    let info = row_of(ui);
    let _ = with_nav(ui, menu, |nav| nav.open_submenu(rows, &info));
    Ok(())
}

/// Closes the submenu and gives the selection back to the main card.
pub(crate) fn close_submenu(ui: &Rc<Ui>, menu: Cid) {
    let Some(held) = part(ui, menu) else {
        return;
    };
    if held.nav.depth() != 2 {
        return;
    }
    if let Some(sub) = held.sub {
        ui.kill(sub);
    }
    ui.tweak(menu, |widget| {
        if let Special::Menu(part) = &mut widget.sp {
            part.sub = None;
        }
    });
    let _ = with_nav(ui, menu, Nav::close_submenu);
}

/// Selecting a row outside the open submenu closes it in the navigator; its card goes with it.
fn end_orphaned_submenu(ui: &Rc<Ui>, menu: Cid) {
    let Some(held) = part(ui, menu) else {
        return;
    };
    if held.nav.depth() == 1
        && let Some(sub) = held.sub
    {
        ui.kill(sub);
        ui.tweak(menu, |widget| {
            if let Special::Menu(part) = &mut widget.sp {
                part.sub = None;
            }
        });
    }
}

// ---- what the toolkit asks of a card ---------------------------------------------------------------------------------------------------

/// The pointer on a canvas of a card, before a handler of the shell hears it: whether the card used the event up.
pub(crate) fn on_pointer(ui: &Rc<Ui>, cid: Cid, kind: u32, raw: &CanvasEvent) -> bool {
    let Some(widget_kind) = ui.read(cid, |widget| widget.kind) else {
        return false;
    };
    match widget_kind {
        WidgetKind::Menu if kind == en::POINTER_PRESS => {
            windows::close(ui);
            true
        }
        WidgetKind::MenuPanel if kind == en::POINTER_PRESS => true,
        WidgetKind::MenuItem if kind == en::POINTER_ENTER => {
            let enabled = ui
                .read(
                    cid,
                    |widget| matches!(&widget.sp, Special::Item(item) if item.entry.enabled),
                )
                .unwrap_or(false);
            if let Some(menu) = menu_over(ui, cid)
                && enabled
            {
                let _ = with_nav(ui, menu, |nav| nav.select(cid));
                end_orphaned_submenu(ui, menu);
            }
            false
        }
        WidgetKind::MenuItem if kind == en::POINTER_PRESS => {
            let enabled = ui
                .read(
                    cid,
                    |widget| matches!(&widget.sp, Special::Item(item) if item.entry.enabled),
                )
                .unwrap_or(false);
            if let Some(menu) = menu_over(ui, cid)
                && enabled
                && raw.button == LEFT
            {
                let _ = with_nav(ui, menu, |nav| nav.select(cid));
                end_orphaned_submenu(ui, menu);
                activate(ui, menu, cid);
            }
            true
        }
        _ => false,
    }
}

/// A key for the menu layer: every key is taken while it is pressed, a menu is modal.
pub(crate) fn on_key(ui: &Rc<Ui>, cid: Cid, raw: &CanvasEvent) -> bool {
    if !ui.read(cid, |widget| widget.kind == WidgetKind::Menu).unwrap_or(false) || !raw.pressed {
        return false;
    }
    let action = key_action(&raw.key, &raw.text);
    let info = row_of(ui);
    match action {
        Some(KeyAction::Escape) => {
            if nav_of(ui, cid).is_some_and(|nav| nav.depth() == 2) {
                close_submenu(ui, cid);
            } else {
                windows::close(ui);
            }
        }
        Some(KeyAction::Move(step)) => {
            let _ = with_nav(ui, cid, |nav| nav.step(step, &info));
        }
        Some(KeyAction::Home) => {
            let _ = with_nav(ui, cid, |nav| nav.home(&info));
        }
        Some(KeyAction::End) => {
            let _ = with_nav(ui, cid, |nav| nav.end(&info));
        }
        Some(KeyAction::Activate) => {
            if let Some(row) = nav_of(ui, cid).and_then(|nav| nav.selected()) {
                activate(ui, cid, row);
            }
        }
        Some(KeyAction::Right) => {
            if let Some(nav) = nav_of(ui, cid)
                && let Some(row) = nav.selected()
                && nav.depth() == 1
                && ui
                    .read(
                        row,
                        |widget| matches!(&widget.sp, Special::Item(item) if item.entry.opens()),
                    )
                    .unwrap_or(false)
            {
                open_submenu(ui, cid, row);
            }
        }
        Some(KeyAction::Left) => close_submenu(ui, cid),
        Some(KeyAction::Jump(letter)) => {
            let _ = with_nav(ui, cid, |nav| nav.jump(&letter, &info));
        }
        None => {}
    }
    true
}

/// Brings a canvas of a card up to date from live state, before its children are arranged.
pub(crate) fn refresh(ui: &Rc<Ui>, cid: Cid, kind: WidgetKind) {
    match kind {
        WidgetKind::MenuItem => refresh_item(ui, cid),
        WidgetKind::MenuPanel => refresh_panel(ui, cid),
        WidgetKind::Menu => refresh_menu(ui, cid),
        _ => {}
    }
}

/// The colour of a row's label and marks.
pub(crate) fn ink(entry: &Entry, selected: bool) -> &'static str {
    if !entry.enabled {
        OFF
    } else if selected {
        HOT_TEXT
    } else {
        TEXT
    }
}

fn refresh_item(ui: &Ui, cid: Cid) {
    let Some(item) = ui
        .read(cid, |widget| match &widget.sp {
            Special::Item(item) => Some((**item).clone()),
            _ => None,
        })
        .flatten()
    else {
        return;
    };
    let ink = ink(&item.entry, item.selected);
    ui.tweak(cid, |widget| {
        widget.b.fill = if item.selected && item.entry.enabled {
            HOT
        } else {
            CLEAR
        }
        .to_owned();
    });
    ui.tweak(item.label, |widget| widget.t.color = ink.to_owned());
    for mark in [item.check, item.arrow] {
        ui.tweak(mark, |widget| {
            if let Special::Mark(part) = &mut widget.sp {
                part.color = ink.to_owned();
            }
        });
    }
}

/// The width a row needs: its label, two marks, the gaps between and the edges.
pub(crate) fn natural(ui: &Ui, row: Cid) -> f64 {
    let label = ui
        .read(row, |widget| match &widget.sp {
            Special::Item(item) => Some(item.label),
            _ => None,
        })
        .flatten();
    label.map_or(0.0, |label| {
        ui.metrics(label).width + 2.0 * EDGE + 2.0 * MARK + 4.0 * 4.0
    })
}

fn refresh_panel(ui: &Ui, cid: Cid) {
    let Some(part) = ui
        .read(cid, |widget| match &widget.sp {
            Special::Panel(part) => Some((**part).clone()),
            _ => None,
        })
        .flatten()
    else {
        return;
    };
    let widest = part
        .rows
        .iter()
        .fold(MIN_WIDTH, |widest, row| widest.max(natural(ui, *row) + 2.0 * PAD));
    ui.tweak(cid, |widget| widget.width = Some(widest));
    let x = match part.place {
        CardPlace::Anchor(at) => {
            let (width, height) = ui
                .read(cid, |widget| widget.parent)
                .flatten()
                .map_or((0.0, 0.0), |menu| (ui.width_of(menu), ui.height_of(menu)));
            place_card((at, 0.0), (widest, 0.0), (width, height)).0
        }
        CardPlace::Beside(parent) => {
            let px = ui.read(parent, |widget| widget.x).unwrap_or(0.0);
            let pw = ui.width_of(parent);
            let menu_width = ui
                .read(cid, |widget| widget.parent)
                .flatten()
                .map_or(0.0, |menu| ui.width_of(menu));
            let right = px + pw - 2.0;
            if right + widest > menu_width {
                0.0_f64.max(px - widest + 2.0)
            } else {
                right
            }
        }
    };
    ui.tweak(cid, |widget| widget.x = x);
}

fn refresh_menu(ui: &Ui, cid: Cid) {
    let Some(held) = part(ui, cid) else {
        return;
    };
    let rows: Vec<Cid> = held.nav.rows.iter().chain(&held.nav.sub).copied().collect();
    for row in rows {
        let selected = held.nav.is_selected(row);
        ui.tweak(row, |widget| {
            if let Special::Item(item) = &mut widget.sp {
                item.selected = selected;
            }
        });
    }
}

// ---- what a card paints ---------------------------------------------------------------------------------------------------------------

/// What a mark or a separator paints: the box's rectangle is not drawn, only the mark's lines or the rule.
pub(crate) fn ops(ui: &Ui, cid: Cid, kind: WidgetKind) -> Result<Vec<PaintOp>, String> {
    let height = ui.height_of(cid);
    match kind {
        WidgetKind::MenuSeparator => {
            let width = ui.width_of(cid);
            Ok(vec![paint::rect(
                EDGE,
                (height / 2.0).floor(),
                0.0_f64.max(width - 2.0 * EDGE),
                1.0,
                0.0,
                LINE,
            )?])
        }
        WidgetKind::MenuMark => {
            let Some(mark) = ui
                .read(cid, |widget| match &widget.sp {
                    Special::Mark(part) => Some(part.clone()),
                    _ => None,
                })
                .flatten()
            else {
                return Ok(Vec::new());
            };
            let top = ((height - 10.0) / 2.0).floor();
            let color = mark.color.as_str();
            match mark.kind {
                MarkKind::Check => Ok(vec![
                    paint::line(2.0, top + 5.0, 5.0, top + 9.0, 1.5, color)?,
                    paint::line(5.0, top + 9.0, 12.0, top + 1.0, 1.5, color)?,
                ]),
                MarkKind::Arrow => Ok(vec![
                    paint::line(4.0, top, 9.0, top + 5.0, 1.5, color)?,
                    paint::line(9.0, top + 5.0, 4.0, top + 10.0, 1.5, color)?,
                ]),
                MarkKind::None => Ok(Vec::new()),
            }
        }
        _ => Ok(Vec::new()),
    }
}
