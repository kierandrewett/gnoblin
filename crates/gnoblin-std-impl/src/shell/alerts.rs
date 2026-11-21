//! The not-responding alert: which windows get one, and the service that looks at the windows from the toolkit's turn.
//!
//! The compositor pings every client. A window that misses pings for longer than `window-management.check-alive-timeout` is marked
//! `unresponsive`, and the alert offers the user Wait or Force quit. Force quit asks the compositor to kill the window's process.
//!
//! `unresponsive` is a property the compositor writes, so the service reads it: a change of it, of a window or of the lock asks for a
//! turn, and the turn looks at the windows when the interval since the last look has passed. A look a turn skipped for being too soon
//! is made when the interval is over.

use std::cell::RefCell;
use std::rc::Rc;

use gnoblin_core::Subscription;
use gnoblin_object::server::Cx;
use gnoblin_object::{Closure, Fail, Obj, Res, Val};

use super::answer;
use super::dialog;
use crate::nodes;
use crate::ui::classes::enumeration;
use crate::ui::update::now_ms;
use crate::ui::widget::{Cid, Ui};

/// What the service keeps of the window an alert is open for.
pub(crate) struct AlertCard {
    pub(crate) client: u64,
    pub(crate) dialog: Cid,
    /// The node that held the keyboard before the alert took it.
    pub(crate) before: Option<u64>,
    pub(crate) arm_at: f64,
}

/// What a look at a window found.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WindowLook {
    pub(crate) node: u64,
    pub(crate) alive: bool,
    pub(crate) unresponsive: bool,
}

/// The alerts a look decided on.
#[derive(Default, Debug, PartialEq, Eq)]
pub(crate) struct AlertChanges {
    pub(crate) open: Vec<u64>,
    pub(crate) close: Vec<u64>,
}

/// Which windows get an alert.
#[derive(Default, Debug)]
pub(crate) struct AlertTracker {
    /// The windows whose user chose Wait or Force quit: no alert for them until they have answered a ping.
    waited: Vec<u64>,
    open: Vec<u64>,
}

impl AlertTracker {
    pub(crate) fn is_open(&self, client: u64) -> bool {
        self.open.contains(&client)
    }

    pub(crate) fn is_waiting(&self, client: u64) -> bool {
        self.waited.contains(&client)
    }

    /// Looks at every window once and says which alerts to open and which to close.
    pub(crate) fn check(&mut self, windows: &[WindowLook], locked: bool) -> AlertChanges {
        let mut changes = AlertChanges::default();
        let mut live = Vec::new();
        for window in windows {
            if !window.alive {
                self.waited.retain(|held| *held != window.node);
                continue;
            }
            live.push(window.node);
            if window.unresponsive {
                if !self.waited.contains(&window.node) && !self.open.contains(&window.node) && !locked {
                    self.open.push(window.node);
                    changes.open.push(window.node);
                }
            } else {
                self.waited.retain(|held| *held != window.node);
                if self.open.contains(&window.node) {
                    changes.close.push(window.node);
                }
            }
        }
        for window in &self.open {
            if (locked || !live.contains(window)) && !changes.close.contains(window) {
                changes.close.push(*window);
            }
        }
        changes
    }

    /// The alert for `client` ended with `answer`. Whether the window is to be killed.
    pub(crate) fn answered(&mut self, client: u64, answer: u32) -> bool {
        self.open.retain(|held| *held != client);
        if answer == answer::RECOVERED {
            return false;
        }
        if !self.waited.contains(&client) {
            self.waited.push(client);
        }
        answer == answer::FORCE
    }
}

/// Text from a peer is for display only: no control characters, and no more than `max` characters.
pub(crate) fn clean_text(text: &str, max: usize) -> String {
    let plain: String = text
        .chars()
        .map(|c| {
            let code = u32::from(c);
            if code <= 0x1f || (0x7f..=0x9f).contains(&code) {
                ' '
            } else {
                c
            }
        })
        .collect();
    if plain.chars().count() > max {
        let mut cut: String = plain.chars().take(max).collect();
        cut.push('\u{2026}');
        cut
    } else {
        plain
    }
}

/// What a window is called in its alert: its title as text, or "This application" when it has none.
fn clean_title(title: &str) -> String {
    let name = clean_text(title, 60);
    if name.is_empty() {
        "This application".to_owned()
    } else {
        name
    }
}

/// What the service keeps.
pub(crate) struct Alerts {
    /// The `not_responding_alerts` object, and the tracker object beneath it.
    pub(crate) obj: Obj,
    pub(crate) tracker_obj: Obj,
    pub(crate) started: bool,
    pub(crate) arm_ms: f64,
    pub(crate) interval_ms: f64,
    pub(crate) now: Option<Closure>,
    pub(crate) close_menu: Option<Closure>,
    pub(crate) tracker: Rc<RefCell<AlertTracker>>,
    pub(crate) cards: Vec<AlertCard>,
    pub(crate) last_look: f64,
    /// A turn ran inside the interval and skipped the look, so one is due when the interval has passed.
    pub(crate) look_deferred: bool,
    pub(crate) listening: Vec<Subscription>,
}

impl Default for Alerts {
    fn default() -> Alerts {
        Alerts {
            obj: 0,
            tracker_obj: 0,
            started: false,
            arm_ms: 500.0,
            interval_ms: 250.0,
            now: None,
            close_menu: None,
            tracker: Rc::new(RefCell::new(AlertTracker::default())),
            cards: Vec::new(),
            last_look: f64::NEG_INFINITY,
            look_deferred: false,
            listening: Vec::new(),
        }
    }
}

/// The alert's clock, in milliseconds: the shell's, or the toolkit's.
fn clock(ui: &Ui) -> f64 {
    let source = ui.shell.borrow().alerts.now.clone();
    match source {
        Some(source) => source
            .call(0, &[])
            .and_then(|value| value.double())
            .unwrap_or_else(|fail| {
                ui.warn(&format!("the alert's clock threw: {fail}"));
                now_ms()
            }),
        None => now_ms(),
    }
}

/// The window `client` as the compositor's tree has it.
fn seen(ui: &Ui, client: u64) -> WindowLook {
    WindowLook {
        node: client,
        alive: nodes::alive(&ui.g, client),
        unresponsive: nodes::flag(&ui.g, client, "unresponsive"),
    }
}

/// Every window, in the order it appeared.
fn windows(ui: &Ui) -> Vec<u64> {
    let mut ids: Vec<u64> =
        ui.g.windows()
            .unwrap_or_default()
            .into_iter()
            .map(|window| window.id().0)
            .collect();
    ids.sort_unstable();
    ids
}

/// The node number a client value names.
pub(crate) fn client_of(value: &Val) -> Res<u64> {
    let text = value.text()?;
    text.parse::<u64>()
        .map_err(|_| Fail::type_error(format!("a client is the number of its node as text, got `{text}`")))
}

impl AlertTracker {
    /// `check` over windows as the tree has them now.
    pub(crate) fn look(&mut self, ui: &Ui, clients: &[u64], locked: bool) -> AlertChanges {
        let views: Vec<WindowLook> = clients.iter().map(|client| seen(ui, *client)).collect();
        self.check(&views, locked)
    }
}

/// Starts the alert: from now the windows are looked at.
pub(crate) fn start(ui: &Rc<Ui>) -> Res<()> {
    if ui.shell.borrow().alerts.started {
        return Ok(());
    }
    let ctx = ui.ctx;
    let g = &ui.g;
    let refuse = |error: gnoblin_core::CoreError| Fail::new(error.kind().code(), error.to_string());
    let wake = move || {
        if let Some(ui) = crate::ui::tree::find(ctx) {
            ui.schedule();
        }
    };
    let listening = vec![
        g.on_class_property_changed("window", "unresponsive", move |_, _| wake())
            .map_err(refuse)?,
        g.on_node_destroyed("window", move |_| wake()).map_err(refuse)?,
        g.on_class_property_changed("lock", "state", move |_, _| wake())
            .map_err(refuse)?,
        g.on_class_property_changed("lock", "engaged", move |_, _| wake())
            .map_err(refuse)?,
    ];
    {
        let mut shell = ui.shell.borrow_mut();
        shell.alerts.listening = listening;
        shell.alerts.started = true;
    }
    ui.schedule();
    Ok(())
}

/// Stops the alert and closes the alerts that are open.
pub(crate) fn stop(ui: &Rc<Ui>) {
    if !ui.shell.borrow().alerts.started {
        return;
    }
    let clients: Vec<u64> = ui.shell.borrow().alerts.cards.iter().map(|card| card.client).collect();
    for client in clients {
        finish(ui, client, answer::RECOVERED);
    }
    let mut shell = ui.shell.borrow_mut();
    shell.alerts.started = false;
    shell.alerts.look_deferred = false;
    shell.alerts.listening.clear();
}

/// One turn of the service: the cards whose delay is over are armed, and the windows are looked at when the interval since the last look has
/// passed.
pub(crate) fn frame(ui: &Rc<Ui>) {
    if !ui.shell.borrow().alerts.started {
        return;
    }
    let now = clock(ui);
    let due: Vec<Cid> = ui
        .shell
        .borrow()
        .alerts
        .cards
        .iter()
        .filter(|card| now >= card.arm_at)
        .map(|card| card.dialog)
        .collect();
    for dialog in due {
        dialog::arm(ui, dialog);
    }
    let (interval, last) = {
        let shell = ui.shell.borrow();
        (shell.alerts.interval_ms, shell.alerts.last_look)
    };
    if now - last >= interval {
        {
            let mut shell = ui.shell.borrow_mut();
            shell.alerts.last_look = now;
            shell.alerts.look_deferred = false;
        }
        check(ui);
    } else {
        ui.shell.borrow_mut().alerts.look_deferred = true;
    }
}

/// How long until the service next needs a turn: a card is to be armed, or a look a turn skipped is due.
pub(crate) fn wake_in(ui: &Rc<Ui>) -> Option<u64> {
    if !ui.shell.borrow().alerts.started {
        return None;
    }
    let now = clock(ui);
    let mut at: Option<f64> = None;
    {
        let shell = ui.shell.borrow();
        if shell.alerts.look_deferred {
            at = Some(shell.alerts.last_look + shell.alerts.interval_ms);
        }
        for card in &shell.alerts.cards {
            if !dialog::is_armed(ui, card.dialog) {
                at = Some(at.map_or(card.arm_at, |held| held.min(card.arm_at)));
            }
        }
    }
    let delay = (at? - now).ceil().max(1.0);
    Some(delay as u64)
}

/// One look at the windows, now.
pub(crate) fn check(ui: &Rc<Ui>) {
    if !ui.shell.borrow().alerts.started {
        return;
    }
    let tracker = Rc::clone(&ui.shell.borrow().alerts.tracker);
    let screen = ui.primary_screen();
    let clients = windows(ui);
    let changes = tracker.borrow_mut().look(ui, &clients, ui.locked());
    for client in changes.close {
        finish(ui, client, answer::RECOVERED);
    }
    // No screen, no card: the window is looked at again once there is one.
    for client in changes.open {
        match screen {
            None => {
                tracker.borrow_mut().answered(client, answer::RECOVERED);
            }
            Some(screen) => {
                if let Err(fail) = show(ui, client, screen) {
                    ui.warn(&format!("NotRespondingAlerts.check threw: {fail}"));
                }
            }
        }
    }
}

/// Opens the alert for `client`: the menu is closed, the card is made over the windows and takes the keyboard.
fn show(ui: &Rc<Ui>, client: u64, screen: (f64, f64)) -> Res<()> {
    let close_menu = ui.shell.borrow().alerts.close_menu.clone();
    if let Some(close_menu) = close_menu
        && let Err(fail) = close_menu.call(0, &[])
    {
        ui.warn(&format!("the alert could not close the window menu: {fail}"));
    }
    let win = nodes::text(&ui.g, client, "title");
    let secure = nodes::flag(&ui.g, client, "secure");
    let owner = ui.shell.borrow().alerts.obj;
    let ctx = ui.ctx;
    let (_, card) = crate::object::with_lib(|lib| {
        lib.with_cx(ctx, owner, |cx| {
            dialog::make(ui, cx, owner, &clean_title(&win), secure, Some(screen), Some(client))
        })
    })??;
    let arm_ms = ui.shell.borrow().alerts.arm_ms;
    let arm_at = clock(ui) + arm_ms;
    ui.shell.borrow_mut().alerts.cards.push(AlertCard {
        client,
        dialog: card,
        before: ui.seat_focus(),
        arm_at,
    });
    ui.mount_top(card)?;
    super::focus_canvas(ui, card)
}

/// Ends the alert for `client`: the card goes, the keyboard goes back, and a Force quit kills.
pub(crate) fn finish(ui: &Rc<Ui>, client: u64, answer: u32) {
    let card = {
        let mut shell = ui.shell.borrow_mut();
        let Some(at) = shell.alerts.cards.iter().position(|card| card.client == client) else {
            return;
        };
        shell.alerts.cards.remove(at)
    };
    ui.kill(card.dialog);
    if ui.focus_node(card.before.unwrap_or(0)).is_err() {
        let _ = ui.focus_node(0);
    }
    let tracker = Rc::clone(&ui.shell.borrow().alerts.tracker);
    let kill = tracker.borrow_mut().answered(client, answer);
    if kill && let Err(error) = ui.node(client).call("kill", &[]) {
        ui.warn(&format!("Client.kill threw: {error}"));
    }
    let service = ui.shell.borrow().alerts.obj;
    ui.announce(service, |cx| {
        cx.emit("answered", vec![Val::Text(client.to_string()), enumeration(answer)]);
    });
}

/// The object of the alert open for `client`, handed out.
pub(crate) fn card_of(ui: &Ui, cx: &Cx<'_>, client: u64) -> Res<Val> {
    let dialog = ui
        .shell
        .borrow()
        .alerts
        .cards
        .iter()
        .find(|card| card.client == client)
        .map(|card| card.dialog);
    match dialog.and_then(|cid| ui.read(cid, |widget| widget.obj)) {
        Some(obj) => cx.handout(obj),
        None => Ok(Val::None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(node: u64, alive: bool, unresponsive: bool) -> WindowLook {
        WindowLook {
            node,
            alive,
            unresponsive,
        }
    }

    #[test]
    fn a_window_that_stops_answering_gets_one_alert() {
        let mut tracker = AlertTracker::default();
        assert_eq!(tracker.check(&[seen(3, true, false)], false), AlertChanges::default());
        let first = tracker.check(&[seen(3, true, true)], false);
        assert_eq!(first.open, vec![3]);
        assert!(tracker.is_open(3));
        assert_eq!(tracker.check(&[seen(3, true, true)], false), AlertChanges::default());
    }

    #[test]
    fn a_chosen_window_is_waited_on_until_it_answers() {
        let mut tracker = AlertTracker::default();
        let _ = tracker.check(&[seen(3, true, true)], false);
        assert!(!tracker.answered(3, answer::WAIT));
        assert!(tracker.is_waiting(3));
        assert_eq!(tracker.check(&[seen(3, true, true)], false), AlertChanges::default());
        assert_eq!(tracker.check(&[seen(3, true, false)], false), AlertChanges::default());
        assert!(!tracker.is_waiting(3));
        assert_eq!(tracker.check(&[seen(3, true, true)], false).open, vec![3]);
        assert!(tracker.answered(3, answer::FORCE));
    }

    #[test]
    fn a_lock_or_a_missing_window_closes_the_alert() {
        let mut tracker = AlertTracker::default();
        let _ = tracker.check(&[seen(3, true, true)], false);
        assert_eq!(tracker.check(&[seen(3, true, true)], true).close, vec![3]);
        let mut tracker = AlertTracker::default();
        let _ = tracker.check(&[seen(3, true, true)], false);
        assert_eq!(tracker.check(&[], false).close, vec![3]);
        assert!(tracker.check(&[seen(4, true, true)], true).open.is_empty());
    }

    #[test]
    fn text_from_a_peer_is_for_display_only() {
        assert_eq!(clean_text("a\nb", 80), "a b");
        assert_eq!(clean_text("tab\there", 80), "tab here");
        assert_eq!(clean_text(&"x".repeat(70), 60).chars().count(), 61);
        assert_eq!(clean_text("caf\u{e9} \u{2713}", 80), "caf\u{e9} \u{2713}");
        assert_eq!(clean_title(""), "This application");
    }
}
