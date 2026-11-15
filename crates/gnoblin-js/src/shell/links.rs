//! The shell's links beside the tree: the dialog slots, the idle connection, the session link and the host
//! API, and what each hands the script every frame.

use std::os::fd::BorrowedFd;
use std::time::{Duration, Instant};

use gnoblin_tree_client::slot_link::{SlotEvent, SlotLink};
use rquickjs::{Array, Function};
use serde_json::Value as Json;

use super::{JsShell, JsShellError};
use crate::dialogs::{events_to_js, verdicts_from_js};
use crate::host_idle::{JsIdleConnector, JsIdlePoll, commands_from_js, reading_to_js};
use crate::host_session::JsSessionLink;

impl JsShell {
    /// Gives the shell its connection for slots: the observation of the compositor's own dialogs runs on it. The
    /// shell holds `dialogs.observe` while the running generation has a dialog listener. Without a link no
    /// listener ever runs and the compositor's dialogs serve every request, which is reported once.
    pub fn set_slot_link(&mut self, link: Box<dyn SlotLink>) {
        self.note_script_ran();
        self.dialogs.set_link(link);
    }

    /// Gives the shell its connection for the session: `gnoblin.session`, `gnoblin.runtime.reloadConfig` and the
    /// schema's signals of `gnoblin.on` run over it. Without one the session verbs are refused `unavailable`.
    pub fn set_session_link(&mut self, link: Box<dyn JsSessionLink>) {
        self.note_script_ran();
        self.state.borrow_mut().api.session = Some(link);
    }

    /// Sleeps until one of the shell's connections (the tree, the slots, and the idle connection while it is open) is
    /// readable, `own` (the role's own connection, which the shell does not hold) is, or `timeout` has passed. It says
    /// whether something was readable. A loop that waits here instead of in a timed read of each connection spends no
    /// time in a turn that finds every connection quiet: the reads that follow are made only for a connection the kernel
    /// has said holds bytes.
    ///
    /// # Errors
    ///
    /// The kernel refused the wait, which means a connection's descriptor is no longer open.
    pub fn wait_for_input(&self, own: Option<BorrowedFd<'_>>, timeout: Duration) -> Result<bool, JsShellError> {
        let sources = [self.link.wait_fd(), self.dialogs.wait_fd(), self.idle.wait_fd(), own];
        let fds: Vec<BorrowedFd<'_>> = sources.into_iter().flatten().collect();
        gnoblin_client::wait_readable(&fds, timeout)
            .map_err(|error| JsShellError::Link(format!("waiting for the compositor: {error}")))
    }

    /// The host API's frame: what happened outside the script (a process ended, the compositor finished a call or
    /// raised a signal) goes to the module, which queues the listeners; then its timers are checked. The listeners
    /// run under the usual per-call budget, so one that never returns costs a strike and not the frame.
    pub(super) fn run_host_api(
        &mut self,
        backlog: Vec<Json>,
        diagnostics: &mut Vec<String>,
    ) -> Result<(), JsShellError> {
        let mut events = backlog;
        events.extend(self.state.borrow_mut().api.poll());
        let batch = Json::Array(events).to_string();
        let started = Instant::now();
        let pending: usize = self.call_entry(self.frame_budget, move |entry| {
            entry.get::<_, Function<'_>>("apiFrame")?.call((batch,))
        })?;
        if pending == 0 {
            return Ok(());
        }
        self.run_pending(pending, started, diagnostics)
    }

    /// Gives the shell the way to open its connection for the compositor's idle state. It is called when a script
    /// first makes an idle watch or hold, and the connection is closed again when the script has neither. Without
    /// one, watches never fire and holds are not recorded, which is reported once.
    pub fn set_idle_connector(&mut self, connector: JsIdleConnector) {
        self.note_script_ran();
        self.idle.set_connector(connector);
    }

    /// The dialog observation of one frame: what the compositor offered goes to the listeners, which run under the
    /// usual budgets, and the verdicts they gave go back. A request the listeners did not refuse is answered
    /// `pass`, so it never waits on a script. The slot is claimed or released to match the generation's listeners.
    pub(super) fn run_dialogs(
        &mut self,
        backlog: Vec<SlotEvent>,
        diagnostics: &mut Vec<String>,
    ) -> Result<(), JsShellError> {
        let mut events = backlog;
        events.extend(self.dialogs.poll(diagnostics));
        if !events.is_empty() {
            let started = Instant::now();
            let pending: usize = self.call_entry(self.frame_budget, move |entry| {
                let list = events_to_js(entry.ctx(), &events)?;
                entry.get::<_, Function<'_>>("dialogs")?.call((list,))
            })?;
            self.run_pending(pending, started, diagnostics)?;
        }
        let verdicts = self.call_entry(self.entry_budget, |entry| {
            let list: Array<'_> = entry.get::<_, Function<'_>>("takeVerdicts")?.call(())?;
            verdicts_from_js(&list)
        })?;
        for verdict in &verdicts {
            self.dialogs.answer(verdict);
        }
        let wanted: bool = self.call_entry(self.entry_budget, |entry| {
            entry.get::<_, Function<'_>>("dialogsWanted")?.call(())
        })?;
        self.dialogs.sync(wanted, diagnostics);
        Ok(())
    }

    /// The host API that talks to the compositor beyond the tree: idle state. One call asks the script what it wants,
    /// so a shell that uses none pays for that call and nothing else. (Authentication is not here: a polkit request is
    /// observed through the dialog slot, which `run_dialogs` serves.)
    pub(super) fn run_compositor_links(
        &mut self,
        backlog: JsIdlePoll,
        diagnostics: &mut Vec<String>,
    ) -> Result<(), JsShellError> {
        let wants: u32 = self.call_entry(self.entry_budget, |entry| {
            entry.get::<_, Function<'_>>("wants")?.call(())
        })?;
        let idle = wants & 1 != 0;
        if idle || self.idle.is_open() {
            self.run_idle(idle, backlog, diagnostics)?;
        }
        Ok(())
    }

    /// Sends the holds the script has asked for since the last send; one the compositor refuses is told to the script.
    fn send_idle_commands(&mut self, diagnostics: &mut Vec<String>) -> Result<(), JsShellError> {
        let commands = self.call_entry(self.entry_budget, |entry| {
            let list: Array<'_> = entry.get::<_, Function<'_>>("takeIdleCommands")?.call(())?;
            commands_from_js(&list)
        })?;
        if !commands.is_empty() {
            for holder in self.idle.run(&commands, diagnostics) {
                self.call_entry(self.entry_budget, move |entry| {
                    entry.get::<_, Function<'_>>("idleFailed")?.call::<_, ()>((holder,))
                })?;
            }
        }
        Ok(())
    }

    /// The idle state of one frame: the connection is opened or closed to match what the script wants, the holds it
    /// asked for are sent, and then the newest reading goes to its watches (their handlers run under the usual budgets).
    fn run_idle(
        &mut self,
        wanted: bool,
        backlog: JsIdlePoll,
        diagnostics: &mut Vec<String>,
    ) -> Result<(), JsShellError> {
        if wanted {
            self.idle.sync(true, diagnostics);
        }
        // Holds go out before the reading is handed to the watches: the first reading after a connect predates the
        // script's own holds, and a watch that heard it would fire `onIdle` for a session the script has just kept
        // awake. The poll that follows sees the compositor's count with the holds in it.
        self.send_idle_commands(diagnostics)?;
        let polled = self.idle.poll(diagnostics);
        let poll = JsIdlePoll {
            reading: polled.reading.or(backlog.reading),
            lost: polled.lost || backlog.lost,
        };
        let started = Instant::now();
        if poll.lost {
            let pending: usize = self.call_entry(self.entry_budget, |entry| {
                entry.get::<_, Function<'_>>("idleLost")?.call(())
            })?;
            self.run_pending(pending, started, diagnostics)?;
        }
        if let Some(reading) = poll.reading {
            let pending: usize = self.call_entry(self.frame_budget, move |entry| {
                let item = reading_to_js(entry.ctx(), reading)?;
                entry.get::<_, Function<'_>>("idleReading")?.call((item,))
            })?;
            self.run_pending(pending, started, diagnostics)?;
        }
        // A handler may have asked for holds too, and they go before the connection can close.
        self.send_idle_commands(diagnostics)?;
        // The last hold given back leaves the connection with nothing to do.
        let wanted: u32 = self.call_entry(self.entry_budget, |entry| {
            entry.get::<_, Function<'_>>("wants")?.call(())
        })?;
        self.idle.sync(wanted & 1 != 0, diagnostics);
        Ok(())
    }
}
