//! gnoblin-notifyd — the notification daemon. Owns org.freedesktop.Notifications
//! (on a background zbus thread) and renders active notifications as a top-right
//! column of Slint cards on a full-screen, mostly-passthrough overlay. Cards
//! expire on a timer or on click; both emit NotificationClosed.

mod server;

use gnoblin_shell_ui::config::Config;
use gnoblin_shell_ui::{file_mtime, find_icon, run, BarApp, BarConfig, RuntimeError};
slint::include_modules!(); // Notifications, NotifCard, NotifAction
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

// Card geometry — mirror notifications.slint for the input region.
// Card content is a VerticalLayout(padding 16, spacing 16): 32px of vertical
// padding + a ~58px icon/title/body header row = 90px with no actions; an
// actions row adds spacing 16 + a 38px button = 54px.
const CARD_W: i32 = 360;
const HEADER_H: i32 = 90;
const ACTIONS_EXTRA: i32 = 54; // extra height for the button row
const GAP: i32 = 8;
const MARGIN: i32 = 8;
const TOPBAR_H: i32 = 34;

struct Active {
    id: u32,
    summary: String,
    body: String,
    icon_name: String,
    has_default: bool,
    critical: bool,
    actions: Vec<(String, String)>, // (id, label), excluding "default"
    deadline: Option<Instant>,
}

type Notifs = Rc<RefCell<Vec<Active>>>;
/// `(x, y, w, h)` input rects for each card, shared with `input_rects`.
type CardRects = Rc<RefCell<Vec<(i32, i32, i32, i32)>>>;

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

struct NotifyApp {
    win: Option<Notifications>,
    rx: Receiver<NotifyEvent>,
    cmd_tx: Sender<NotifyCommand>,
    notifs: Notifs,
    dirty: Rc<Cell<bool>>,
    surface_w: Rc<Cell<i32>>,
    surface_h: Rc<Cell<i32>>,
    // Card rects (x,y,w,h) from the last rebuild, mirrored into the input region.
    card_rects: CardRects,
    // Shared notification history.
    history: History,
    config_path: Option<PathBuf>,
    config_mtime: Option<SystemTime>,
}

fn apply_theme(w: &Notifications) {
    gnoblin_shell_ui::apply_shell_theme!(w);
}

fn apply_shell_motion(w: &Notifications) -> bool {
    let motion = gnoblin_shell_ui::prefs::shell_motion();
    let theme = w.global::<Theme>();
    gnoblin_shell_ui::apply_shell_motion_to_theme!(theme, motion)
}

impl NotifyApp {
    fn rebuild_model(&self) {
        let Some(win) = &self.win else { return };
        let w = self.surface_w.get();
        let mut cards = Vec::new();
        let mut rects = Vec::new();
        let mut y = TOPBAR_H + MARGIN;
        for n in self.notifs.borrow().iter() {
            let icon = find_icon(&n.icon_name, "");
            let card_h = HEADER_H
                + if n.actions.is_empty() {
                    0
                } else {
                    ACTIONS_EXTRA
                };
            let actions: Vec<NotifAction> = n
                .actions
                .iter()
                .map(|(id, label)| NotifAction {
                    id: id.clone().into(),
                    label: label.clone().into(),
                })
                .collect();
            cards.push(NotifCard {
                id: n.id as i32,
                summary: n.summary.clone().into(),
                body: n.body.clone().into(),
                has_icon: icon.is_some(),
                icon: icon.unwrap_or_default(),
                critical: n.critical,
                actions: Rc::new(slint::VecModel::from(actions)).into(),
                card_y: y as f32,
                card_h: card_h as f32,
            });
            rects.push((w - CARD_W - MARGIN, y, CARD_W, card_h));
            y += card_h + GAP;
        }
        win.set_cards(Rc::new(slint::VecModel::from(cards)).into());
        *self.card_rects.borrow_mut() = rects;

        // The topbar's bell reads this for its unread dot.
        let history = self.history.borrow();
        gnoblin_shell_ui::notifcenter::set_pending(!history.is_empty());
        let summary = history.first().map_or_else(
            gnoblin_shell_ui::notifcenter::Summary::default,
            |latest| gnoblin_shell_ui::notifcenter::Summary {
                count: history.len(),
                latest_summary: latest.summary.clone(),
                latest_body: latest.body.clone(),
            },
        );
        gnoblin_shell_ui::notifcenter::write_summary(&summary);
        let entries: Vec<gnoblin_shell_ui::notifcenter::Entry> = history
            .iter()
            .map(|h| gnoblin_shell_ui::notifcenter::Entry {
                app_name: h.app_name.clone(),
                summary: h.summary.clone(),
                body: h.body.clone(),
                icon_name: h.icon_name.clone(),
                timestamp_secs: h.timestamp_secs,
            })
            .collect();
        gnoblin_shell_ui::notifcenter::write_history(&entries);
    }
}

impl BarApp for NotifyApp {
    fn show(&mut self, w: u32, _h: u32, _screen_w: u32, screen_h: u32) -> Result<(), RuntimeError> {
        let win = Notifications::new()
            .map_err(|e| gnoblin_shell_ui::runtime_error(format!("Notifications::new: {e}")))?;
        apply_theme(&win);
        apply_shell_motion(&win);
        self.surface_w.set(w as i32);
        self.surface_h.set(screen_h.max(1) as i32);

        // Click a card → dismiss (invoking the default action if any), emit
        // NotificationClosed, and flag a rebuild.
        let notifs = self.notifs.clone();
        let dirty = self.dirty.clone();
        let cmd_tx = self.cmd_tx.clone();
        win.on_dismissed(move |id| {
            let id = id as u32;
            let mut list = notifs.borrow_mut();
            if let Some(pos) = list.iter().position(|n| n.id == id) {
                if list[pos].has_default {
                    let _ = cmd_tx.send(NotifyCommand::Action {
                        id,
                        action: "default".into(),
                    });
                }
                list.remove(pos);
                let _ = cmd_tx.send(NotifyCommand::Closed {
                    id,
                    reason: CLOSE_DISMISSED,
                });
                dirty.set(true);
            }
        });

        // An action button → invoke that action, then close the notification.
        let notifs = self.notifs.clone();
        let dirty = self.dirty.clone();
        let cmd_tx = self.cmd_tx.clone();
        win.on_action_invoked(move |id, action| {
            let id = id as u32;
            let mut list = notifs.borrow_mut();
            if let Some(pos) = list.iter().position(|n| n.id == id) {
                let _ = cmd_tx.send(NotifyCommand::Action {
                    id,
                    action: action.to_string(),
                });
                list.remove(pos);
                let _ = cmd_tx.send(NotifyCommand::Closed {
                    id,
                    reason: CLOSE_DISMISSED,
                });
                dirty.set(true);
            }
        });

        win.show()
            .map_err(|e| gnoblin_shell_ui::runtime_error(format!("notifications.show: {e}")))?;
        self.win = Some(win);
        Ok(())
    }

    fn resized(&mut self, w: u32, _h: u32, _screen_w: u32, screen_h: u32) {
        self.surface_w.set(w.max(1) as i32);
        self.surface_h.set(screen_h.max(1) as i32);
        self.rebuild_model();
    }

    fn tick(&mut self) -> bool {
        let mut changed = false;

        let config_mtime = file_mtime(self.config_path.as_deref());
        if config_mtime != self.config_mtime {
            self.config_mtime = config_mtime;
            if let Some(w) = &self.win {
                apply_theme(w);
                changed |= apply_shell_motion(w);
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
                    if gnoblin_shell_ui::dnd::is_on() {
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
                    // actions arrive flat: [id1, label1, id2, label2, …]. Pair
                    // them up; "default" is the click-the-body action, not a button.
                    let mut actions = Vec::new();
                    let mut it = inc.actions.chunks_exact(2);
                    for pair in it.by_ref() {
                        if pair[0] != "default" {
                            actions.push((pair[0].clone(), pair[1].clone()));
                        }
                    }
                    let active = Active {
                        id: inc.id,
                        summary: inc.summary,
                        body: inc.body,
                        icon_name: inc.app_icon,
                        has_default: inc.actions.iter().any(|a| a == "default"),
                        critical,
                        actions,
                        deadline,
                    };
                    let mut list = self.notifs.borrow_mut();
                    if let Some(pos) = list.iter().position(|n| n.id == active.id) {
                        list[pos] = active; // replace in place
                    } else {
                        list.push(active);
                    }
                    self.dirty.set(true);
                }
                NotifyEvent::Close(id) => {
                    let mut list = self.notifs.borrow_mut();
                    if let Some(pos) = list.iter().position(|n| n.id == id) {
                        list.remove(pos);
                        let _ = self.cmd_tx.send(NotifyCommand::Closed {
                            id,
                            reason: CLOSE_REQUESTED,
                        });
                        self.dirty.set(true);
                    }
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
                    let id = list[i].id;
                    list.remove(i);
                    let _ = self.cmd_tx.send(NotifyCommand::Closed {
                        id,
                        reason: CLOSE_EXPIRED,
                    });
                    self.dirty.set(true);
                } else {
                    i += 1;
                }
            }
        }

        // The old right-side notification center has been folded into the
        // quick-settings popout. Keep the compatibility flag inert.
        if gnoblin_shell_ui::notifcenter::is_open() {
            gnoblin_shell_ui::notifcenter::set(false);
        }

        if self.dirty.replace(false) {
            self.rebuild_model();
            return true;
        }
        changed
    }

    fn window(&self) -> Option<&slint::Window> {
        self.win.as_ref().map(|w| w.window())
    }

    fn input_rects(&self) -> Option<Vec<(i32, i32, i32, i32)>> {
        Some(self.card_rects.borrow().clone())
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
            namespace: "gnoblin-notifyd",
            anchor: Anchor::TOP
                .union(Anchor::BOTTOM)
                .union(Anchor::LEFT)
                .union(Anchor::RIGHT),
            layer: Layer::Overlay,
            height: 1,
            exclusive_zone: 0,
            full_height: true,
            input_passthrough: false,
            keyboard: false,
        },
        Box::new(NotifyApp {
            win: None,
            rx,
            cmd_tx,
            notifs: Rc::new(RefCell::new(Vec::new())),
            dirty: Rc::new(Cell::new(false)),
            surface_w: Rc::new(Cell::new(1280)),
            surface_h: Rc::new(Cell::new(800)),
            card_rects: Rc::new(RefCell::new(Vec::new())),
            history: Rc::new(RefCell::new(Vec::new())),
            config_path,
            config_mtime,
        }),
    );
}
