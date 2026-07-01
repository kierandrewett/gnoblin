//! gnoblin-notifyd owns org.freedesktop.Notifications and renders each active
//! notification as its own content-sized layer surface. The compositor owns the
//! card chrome; Slint only draws the card tint and content.

mod server;

use gnoblin_core::config::Config;
use gnoblin_core::{file_mtime, RuntimeError};
use gnoblin_desktop::find_icon;
use gnoblin_runtime::{
    run, BarApp, BarConfig, BarMargins, PopoutConfig, PopoutHandle, RuntimeControl,
};
slint::include_modules!(); // NotificationHost, NotificationCardSurface, NotifCard, NotifAction
use server::{NotifyCommand, NotifyEvent};
use slint::ComponentHandle;
use smithay_client_toolkit::shell::wlr_layer::{Anchor, Layer};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// Close reasons (org.freedesktop.Notifications spec).
const CLOSE_EXPIRED: u32 = 1;
const CLOSE_DISMISSED: u32 = 2;
const CLOSE_REQUESTED: u32 = 3;

const DEFAULT_MS: u32 = 5000;
const MAX_MS: u32 = 15000;

const HOST_NAMESPACE: &str = "gnoblin-notifyd-host";
const CARD_NAMESPACE: &str = "gnoblin-notifyd";

// Card geometry - mirror notifications.slint and keep the layer surface exactly
// the size of the card content.
const CARD_W: i32 = 360;
const HEADER_H: i32 = 90;
const ACTIONS_EXTRA: i32 = 54;
const GAP: i32 = 8;
const MARGIN: i32 = 8;
const TOPBAR_H: i32 = 34;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CardLayout {
    top: i32,
    right: i32,
    width: u32,
    height: u32,
}

impl CardLayout {
    fn margins(self) -> BarMargins {
        BarMargins {
            top: self.top,
            right: self.right,
            ..BarMargins::default()
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CardSurface {
    handle: PopoutHandle,
    layout: CardLayout,
}

struct Active {
    id: u32,
    summary: String,
    body: String,
    icon_name: String,
    has_default: bool,
    critical: bool,
    actions: Vec<(String, String)>, // (id, label), excluding "default"
    deadline: Option<Instant>,
    surface: Option<CardSurface>,
}

type Notifs = Rc<RefCell<Vec<Active>>>;

/// Max notifications kept in shared notification history.
const HISTORY_CAP: usize = 20;

/// A past notification, kept for quick-settings history.
struct HistoryItem {
    /// The spec notification id, so an app replacing a notification (same id)
    /// updates its history entry instead of stacking duplicates.
    notif_id: u32,
    app_name: String,
    summary: String,
    body: String,
    icon_name: String,
    timestamp_secs: u64,
}

type History = Rc<RefCell<Vec<HistoryItem>>>;

struct CardWindow {
    handle: PopoutHandle,
    win: NotificationCardSurface,
}

struct NotifyApp {
    host: Option<NotificationHost>,
    runtime: Option<RuntimeControl>,
    rx: Receiver<NotifyEvent>,
    cmd_tx: Sender<NotifyCommand>,
    notifs: Notifs,
    dirty: Rc<Cell<bool>>,
    cards: Vec<CardWindow>,
    history: History,
    config_path: Option<PathBuf>,
    config_mtime: Option<SystemTime>,
}

fn apply_theme_host(host: &NotificationHost) {
    gnoblin_runtime::apply_shell_theme!(host);
}

fn apply_motion_host(host: &NotificationHost) -> bool {
    gnoblin_runtime::apply_shell_motion!(host)
}

fn apply_theme_card(card: &NotificationCardSurface) {
    gnoblin_runtime::apply_shell_theme!(card);
}

fn apply_motion_card(card: &NotificationCardSurface) -> bool {
    gnoblin_runtime::apply_shell_motion!(card)
}

fn card_height(n: &Active) -> i32 {
    HEADER_H
        + if n.actions.is_empty() {
            0
        } else {
            ACTIONS_EXTRA
        }
}

fn card_model(n: &Active) -> NotifCard {
    let icon = find_icon(&n.icon_name, "");
    let actions: Vec<NotifAction> = n
        .actions
        .iter()
        .map(|(id, label)| NotifAction {
            id: id.clone().into(),
            label: label.clone().into(),
        })
        .collect();
    NotifCard {
        id: n.id as i32,
        summary: n.summary.clone().into(),
        body: n.body.clone().into(),
        has_icon: icon.is_some(),
        icon: icon.unwrap_or_default(),
        critical: n.critical,
        actions: Rc::new(slint::VecModel::from(actions)).into(),
        card_h: card_height(n) as f32,
    }
}

fn card_popout_config(layout: CardLayout) -> PopoutConfig {
    PopoutConfig {
        namespace: CARD_NAMESPACE,
        anchor: Anchor::TOP.union(Anchor::RIGHT),
        layer: Layer::Overlay,
        width: layout.width,
        height: layout.height,
        margins: layout.margins(),
        keyboard: false,
    }
}

fn close_active_at(
    list: &mut Vec<Active>,
    pos: usize,
    cmd_tx: &Sender<NotifyCommand>,
    runtime: Option<&RuntimeControl>,
    reason: u32,
    invoke_default: bool,
) -> bool {
    if pos >= list.len() {
        return false;
    }
    let mut active = list.remove(pos);
    if invoke_default && active.has_default {
        let _ = cmd_tx.send(NotifyCommand::Action {
            id: active.id,
            action: "default".into(),
        });
    }
    if let Some(surface) = active.surface.take() {
        if let Some(runtime) = runtime {
            runtime.close_popout(surface.handle);
        }
    }
    let _ = cmd_tx.send(NotifyCommand::Closed {
        id: active.id,
        reason,
    });
    true
}

impl NotifyApp {
    fn publish_history(&self) {
        let history = self.history.borrow();
        gnoblin_runtime::notifcenter::set_pending(!history.is_empty());
        let summary =
            history
                .first()
                .map_or_else(gnoblin_runtime::notifcenter::Summary::default, |latest| {
                    gnoblin_runtime::notifcenter::Summary {
                        count: history.len(),
                        latest_summary: latest.summary.clone(),
                        latest_body: latest.body.clone(),
                    }
                });
        gnoblin_runtime::notifcenter::write_summary(&summary);
        let entries: Vec<gnoblin_runtime::notifcenter::Entry> = history
            .iter()
            .map(|h| gnoblin_runtime::notifcenter::Entry {
                app_name: h.app_name.clone(),
                summary: h.summary.clone(),
                body: h.body.clone(),
                icon_name: h.icon_name.clone(),
                timestamp_secs: h.timestamp_secs,
            })
            .collect();
        gnoblin_runtime::notifcenter::write_history(&entries);
    }

    fn sync_surfaces(&mut self) {
        self.publish_history();
        let Some(runtime) = self.runtime.clone() else {
            return;
        };

        let mut pending_cards = Vec::new();
        let mut y = TOPBAR_H + MARGIN;
        {
            let mut list = self.notifs.borrow_mut();
            for n in list.iter_mut() {
                let height = card_height(n);
                let layout = CardLayout {
                    top: y,
                    right: MARGIN,
                    width: CARD_W as u32,
                    height: height as u32,
                };

                let surface = match n.surface {
                    Some(surface) => {
                        if surface.layout != layout {
                            runtime.configure_popout(
                                surface.handle,
                                layout.width,
                                layout.height,
                                layout.margins(),
                            );
                            let moved = CardSurface {
                                handle: surface.handle,
                                layout,
                            };
                            n.surface = Some(moved);
                            moved
                        } else {
                            surface
                        }
                    }
                    None => {
                        let handle = runtime.open_popout(card_popout_config(layout));
                        let surface = CardSurface { handle, layout };
                        n.surface = Some(surface);
                        surface
                    }
                };
                pending_cards.push((surface.handle, card_model(n)));
                y += height + GAP;
            }
        }

        for (handle, card) in pending_cards {
            if let Some(surface) = self.cards.iter().find(|surface| surface.handle == handle) {
                surface.win.set_card(card);
            }
        }
    }

    fn close_active_by_id(&self, id: u32, reason: u32, invoke_default: bool) -> bool {
        let mut list = self.notifs.borrow_mut();
        if let Some(pos) = list.iter().position(|n| n.id == id) {
            let closed = close_active_at(
                &mut list,
                pos,
                &self.cmd_tx,
                self.runtime.as_ref(),
                reason,
                invoke_default,
            );
            if closed {
                self.dirty.set(true);
            }
            closed
        } else {
            false
        }
    }
}

impl BarApp for NotifyApp {
    fn set_runtime(&mut self, runtime: RuntimeControl) {
        self.runtime = Some(runtime);
    }

    fn show(
        &mut self,
        _w: u32,
        _h: u32,
        _screen_w: u32,
        _screen_h: u32,
    ) -> Result<(), RuntimeError> {
        let host = NotificationHost::new()
            .map_err(|e| gnoblin_core::runtime_error(format!("NotificationHost::new: {e}")))?;
        apply_theme_host(&host);
        apply_motion_host(&host);
        host.show()
            .map_err(|e| gnoblin_core::runtime_error(format!("notification host.show: {e}")))?;
        self.host = Some(host);
        Ok(())
    }

    fn show_popout(
        &mut self,
        handle: PopoutHandle,
        namespace: &'static str,
        _width: u32,
        _height: u32,
        _screen_w: u32,
        _screen_h: u32,
    ) -> Result<(), RuntimeError> {
        if namespace != CARD_NAMESPACE {
            return Err(gnoblin_core::runtime_error(format!(
                "unknown notifyd popout namespace: {namespace}"
            )));
        }
        let (id, card_model) = {
            let list = self.notifs.borrow();
            list.iter()
                .find_map(|n| {
                    (n.surface.map(|s| s.handle) == Some(handle)).then(|| (n.id, card_model(n)))
                })
                .ok_or_else(|| {
                    gnoblin_core::runtime_error("notifyd popout has no active notification")
                })?
        };

        let card = NotificationCardSurface::new().map_err(|e| {
            gnoblin_core::runtime_error(format!("NotificationCardSurface::new: {e}"))
        })?;
        apply_theme_card(&card);
        apply_motion_card(&card);
        card.set_card(card_model);

        let notifs = self.notifs.clone();
        let dirty = self.dirty.clone();
        let cmd_tx = self.cmd_tx.clone();
        let runtime = self
            .runtime
            .clone()
            .ok_or_else(|| gnoblin_core::runtime_error("notifyd runtime handle missing"))?;
        card.on_dismissed(move |id| {
            let id = id as u32;
            let mut list = notifs.borrow_mut();
            if let Some(pos) = list.iter().position(|n| n.id == id) {
                close_active_at(
                    &mut list,
                    pos,
                    &cmd_tx,
                    Some(&runtime),
                    CLOSE_DISMISSED,
                    true,
                );
                dirty.set(true);
            }
        });

        let notifs = self.notifs.clone();
        let dirty = self.dirty.clone();
        let cmd_tx = self.cmd_tx.clone();
        let runtime = self
            .runtime
            .clone()
            .ok_or_else(|| gnoblin_core::runtime_error("notifyd runtime handle missing"))?;
        card.on_action_invoked(move |id, action| {
            let id = id as u32;
            let mut list = notifs.borrow_mut();
            if let Some(pos) = list.iter().position(|n| n.id == id) {
                let _ = cmd_tx.send(NotifyCommand::Action {
                    id,
                    action: action.to_string(),
                });
                close_active_at(
                    &mut list,
                    pos,
                    &cmd_tx,
                    Some(&runtime),
                    CLOSE_DISMISSED,
                    false,
                );
                dirty.set(true);
            }
        });

        card.show()
            .map_err(|e| gnoblin_core::runtime_error(format!("notification card.show: {e}")))?;
        self.cards.push(CardWindow { handle, win: card });
        eprintln!("gnoblin-notifyd: opened card id={id} handle={handle:?}");
        Ok(())
    }

    fn popout_window(&self, handle: PopoutHandle) -> Option<&slint::Window> {
        self.cards
            .iter()
            .find(|surface| surface.handle == handle)
            .map(|surface| surface.win.window())
    }

    fn popout_closed(&mut self, handle: PopoutHandle, _namespace: &'static str) {
        self.cards.retain(|surface| surface.handle != handle);
        let mut list = self.notifs.borrow_mut();
        if let Some(active) = list
            .iter_mut()
            .find(|n| n.surface.map(|surface| surface.handle) == Some(handle))
        {
            active.surface = None;
            self.dirty.set(true);
        }
    }

    fn tick(&mut self) -> bool {
        let mut changed = false;

        let config_mtime = file_mtime(self.config_path.as_deref());
        if config_mtime != self.config_mtime {
            self.config_mtime = config_mtime;
            if let Some(w) = &self.host {
                apply_theme_host(w);
                changed |= apply_motion_host(w);
            }
            for surface in &self.cards {
                apply_theme_card(&surface.win);
                changed |= apply_motion_card(&surface.win);
            }
        }

        // Drain bus events.
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                NotifyEvent::Show(inc) => {
                    // A replacement = this id is already an active notification.
                    let is_replacement = self.notifs.borrow().iter().any(|n| n.id == inc.id);
                    // Record in history (even if DND hides the popup).
                    {
                        let mut h = self.history.borrow_mut();
                        // Replacing a still-shown notification updates its existing
                        // history entry (moved to top) rather than stacking dupes.
                        let existing = is_replacement
                            .then(|| h.iter().position(|e| e.notif_id == inc.id))
                            .flatten();
                        if let Some(pos) = existing {
                            h.remove(pos);
                        }
                        h.insert(
                            0,
                            HistoryItem {
                                notif_id: inc.id,
                                app_name: if inc.app_name.is_empty() {
                                    "Notification".to_string()
                                } else {
                                    inc.app_name.clone()
                                },
                                summary: inc.summary.clone(),
                                body: inc.body.clone(),
                                icon_name: inc.app_icon.clone(),
                                timestamp_secs: SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .map(|d| d.as_secs())
                                    .unwrap_or_default(),
                            },
                        );
                        h.truncate(HISTORY_CAP);
                        self.dirty.set(true);
                    }
                    // Do-Not-Disturb: drop the popup (the app still got its id back).
                    if gnoblin_core::dnd::is_on() {
                        continue;
                    }
                    let critical = inc.urgency >= 2;
                    let ms = match inc.expire_ms {
                        -1 => DEFAULT_MS,
                        0 => 0,
                        n => (n as u32).min(MAX_MS),
                    };
                    // Critical notifications persist until dismissed.
                    let deadline = (ms != 0 && !critical)
                        .then(|| Instant::now() + Duration::from_millis(ms as u64));
                    // actions arrive flat: [id1, label1, id2, label2, ...]. Pair
                    // them up; "default" is the click-the-body action, not a button.
                    let mut actions = Vec::new();
                    let mut it = inc.actions.chunks_exact(2);
                    for pair in it.by_ref() {
                        if pair[0] != "default" {
                            actions.push((pair[0].clone(), pair[1].clone()));
                        }
                    }
                    let mut active = Active {
                        id: inc.id,
                        summary: inc.summary,
                        body: inc.body,
                        icon_name: inc.app_icon,
                        has_default: inc.actions.iter().any(|a| a == "default"),
                        critical,
                        actions,
                        deadline,
                        surface: None,
                    };
                    let mut list = self.notifs.borrow_mut();
                    if let Some(pos) = list.iter().position(|n| n.id == active.id) {
                        active.surface = list[pos].surface;
                        list[pos] = active;
                    } else {
                        list.push(active);
                    }
                    self.dirty.set(true);
                }
                NotifyEvent::Close(id) => {
                    self.close_active_by_id(id, CLOSE_REQUESTED, false);
                }
            }
        }

        // Expire by deadline.
        let now = Instant::now();
        {
            let mut list = self.notifs.borrow_mut();
            let mut i = 0;
            while i < list.len() {
                if list[i].deadline.map(|d| now >= d).unwrap_or(false) {
                    close_active_at(
                        &mut list,
                        i,
                        &self.cmd_tx,
                        self.runtime.as_ref(),
                        CLOSE_EXPIRED,
                        false,
                    );
                    self.dirty.set(true);
                } else {
                    i += 1;
                }
            }
        }

        // The old right-side notification center has been folded into the
        // quick-settings popout. Keep the compatibility flag inert.
        if gnoblin_runtime::notifcenter::is_open() {
            gnoblin_runtime::notifcenter::clear_legacy_flag();
        }

        if self.dirty.replace(false) {
            self.sync_surfaces();
            return true;
        }
        changed
    }

    fn window(&self) -> Option<&slint::Window> {
        self.host.as_ref().map(|w| w.window())
    }
}

fn main() {
    let (tx, rx) = std::sync::mpsc::channel();
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();
    server::spawn(tx, cmd_rx);
    let config_path = Config::path();
    let config_mtime = file_mtime(config_path.as_deref());

    run(
        BarConfig {
            namespace: HOST_NAMESPACE,
            anchor: Anchor::TOP.union(Anchor::LEFT),
            layer: Layer::Overlay,
            width: 1,
            height: 1,
            exclusive_zone: 0,
            full_height: false,
            input_passthrough: true,
            keyboard: false,
            ..BarConfig::default()
        },
        Box::new(NotifyApp {
            host: None,
            runtime: None,
            rx,
            cmd_tx,
            notifs: Rc::new(RefCell::new(Vec::new())),
            dirty: Rc::new(Cell::new(false)),
            cards: Vec::new(),
            history: Rc::new(RefCell::new(Vec::new())),
            config_path,
            config_mtime,
        }),
    );
}
