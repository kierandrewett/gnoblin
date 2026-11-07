//! The guardian state machine.
//!
//! One [`Guardian`] owns the session: it decides when each child starts, when it
//! is restarted, when it is killed and in which order everything stops. It owns
//! no process, no socket, no timer and no file: the caller spawns, signals, kills
//! and persists what [`Guardian::apply`] returns, and reports back through
//! [`Event`].

use crate::effect::{Effect, Level};
use crate::environment::{EnvTarget, Environment, EnvironmentRules, InheritedEnvironment};
use crate::event::{AutostartEntry, Event, EventKind, ExitStatus, ShutdownReason, Signal};
use crate::ids::ChildId;
use crate::plan::{ChildKind, ChildSpec, Plan, Restart};
use crate::recovery::{ConfigStatus, Recovery};
use crate::state::{ChildState, ChildStatus, SessionState, StopCause, StopReason};
use crate::time::{Tick, secs};

/// How the guardian behaves.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Rules {
    /// How long a child may take to exit after a graceful signal before it is
    /// killed. Two seconds matches `guardian_stop_supervisor` in
    /// `src/session/gnoblin-runtime.c`.
    pub stop_grace: Tick,
    /// How many session starts may fail in a row before the guardian asks for
    /// safe mode.
    pub safe_mode_after: u32,
    /// Which environment the session owns.
    pub environment: EnvironmentRules,
}

impl Rules {
    /// The rules the C session applies today.
    #[must_use]
    pub fn standard(inherited: &InheritedEnvironment<'_>) -> Self {
        Self {
            stop_grace: secs(2),
            safe_mode_after: 3,
            environment: EnvironmentRules::standard(inherited),
        }
    }
}

/// Why a child is waiting for its exit after a kill.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct StopPending {
    /// What the stop is for.
    cause: StopCause,
    /// Whether a kill has already been sent, in which case the next deadline is
    /// the caller's last chance to report the exit before the guardian assumes
    /// it.
    killed: bool,
}

/// One child's supervision record.
struct Child {
    /// The plan's declaration.
    spec: ChildSpec,
    /// Where the child is.
    state: ChildState,
    /// How many times it has been spawned.
    generation: u64,
    /// The absolute deadline of the phase in `state`: the readiness deadline
    /// while `Starting`, the grace deadline while `Stopping`.
    deadline: Option<Tick>,
    /// When the current instance last became usable, for the restart-budget
    /// reset.
    ready_at: Option<Tick>,
    /// When a restart is due while `Backoff`.
    restart_at: Option<Tick>,
    /// Restarts scheduled since the child was last stable.
    restarts: u32,
    /// How the last instance ended.
    last_exit: Option<ExitStatus>,
    /// Why the child is `Stopping`, and whether it has been killed.
    stop: Option<StopPending>,
}

impl Child {
    /// A record for a child that has not been spawned.
    fn new(spec: ChildSpec) -> Self {
        Self {
            spec,
            state: ChildState::Pending,
            generation: 0,
            deadline: None,
            ready_at: None,
            restart_at: None,
            restarts: 0,
            last_exit: None,
            stop: None,
        }
    }

    /// An autostart application the session still means to run: one a configuration asked for
    /// and that has not been stopped, failed or given up.
    fn live_autostart(&self) -> bool {
        self.spec.kind == ChildKind::Autostart
            && matches!(
                self.state,
                ChildState::Pending | ChildState::Ready | ChildState::Running | ChildState::Backoff
            )
    }
}

/// The reverse-order stop in progress.
struct Stop {
    /// Children to stop, grouped into contiguous runs of the same kind and
    /// ordered so the last-started run goes first. Everything in one run is
    /// signalled together, so ten autostart applications do not cost ten grace
    /// periods.
    ranks: Vec<Vec<usize>>,
    /// Which run is being stopped.
    current: usize,
}

/// The session guardian.
pub struct Guardian {
    plan: Plan,
    rules: Rules,
    recovery: Recovery,
    children: Vec<Child>,
    /// The display variables the compositor published, in `forwarded` order.
    published: Vec<(String, String)>,
    session: SessionState,
    /// The position in the plan the start sequence is waiting at.
    cursor: usize,
    /// The child that exhausted its restart budget before the session was up, so
    /// the sequence must not continue past it: a session without policy must not
    /// launch user applications.
    stalled: Option<ChildId>,
    stop: Option<Stop>,
    stop_reason: Option<StopReason>,
    /// Whether this session ever became usable, which decides whether its end
    /// counts as a failed start.
    reached_up: bool,
    /// The newest time any event carried.
    now: Tick,
    /// The identifier the next autostart child added by a configuration takes.
    next_id: u32,
}

impl Guardian {
    /// A guardian for `plan`, with no recovery state from an earlier session.
    #[must_use]
    pub fn new(plan: Plan, rules: Rules) -> Self {
        Self::with_recovery(plan, rules, Recovery::default())
    }

    /// A guardian that continues from the recovery state the last session left.
    #[must_use]
    pub fn with_recovery(plan: Plan, rules: Rules, recovery: Recovery) -> Self {
        // Autostart children are added while the session runs, so their identifiers continue
        // above every declared one and no removed record is ever reused.
        let next_id = plan
            .children()
            .iter()
            .map(|spec| spec.id.get())
            .max()
            .map_or(1, |max| max.saturating_add(1));
        let children = plan.children().iter().map(|spec| Child::new(spec.clone())).collect();
        Self {
            plan,
            rules,
            recovery,
            children,
            published: Vec::new(),
            session: SessionState::Idle,
            cursor: 0,
            stalled: None,
            stop: None,
            stop_reason: None,
            reached_up: false,
            now: Tick::ZERO,
            next_id,
        }
    }

    /// Apply one event and return the effects the caller must perform, in order.
    ///
    /// This is the whole interface. It is deterministic: the same plan, rules,
    /// recovery and event sequence always produce the same effects, because the
    /// guardian reads no clock, no environment, no process table and no file.
    pub fn apply(&mut self, event: Event) -> Vec<Effect> {
        let mut effects = Vec::new();
        if event.now > self.now {
            self.now = event.now;
        }
        // Deadlines are armed from the newest time seen, never from this event's
        // own stamp: a caller whose clock went backwards must not arm a deadline
        // that has already passed.
        let now = self.now;
        match event.kind {
            EventKind::Start => self.start(now, &mut effects),
            EventKind::Tick => self.tick(now, &mut effects),
            EventKind::Ready {
                child,
                generation,
                environment,
            } => self.ready(child, generation, environment, now, &mut effects),
            EventKind::Exited {
                child,
                generation,
                status,
            } => self.exited(child, generation, status, now, &mut effects),
            EventKind::Shutdown { reason } => self.end(reason, now, &mut effects),
            EventKind::ConfigAccepted { revision, autostart } => {
                self.config_accepted(revision, autostart, now, &mut effects);
            }
            EventKind::ConfigRejected { revision, diagnostic } => {
                self.config_rejected(revision, &diagnostic, &mut effects);
            }
            EventKind::ConfigFallback { reason } => self.config_fallback(&reason, &mut effects),
        }
        effects
    }

    /// Where the session is.
    #[must_use]
    pub const fn session(&self) -> SessionState {
        self.session
    }

    /// Why the session is stopping or ended, once that is known.
    #[must_use]
    pub const fn stop_reason(&self) -> Option<StopReason> {
        self.stop_reason
    }

    /// The recovery state the caller should have persisted.
    #[must_use]
    pub const fn recovery(&self) -> &Recovery {
        &self.recovery
    }

    /// The rules this guardian applies.
    #[must_use]
    pub const fn rules(&self) -> &Rules {
        &self.rules
    }

    /// The plan this guardian supervises.
    #[must_use]
    pub const fn plan(&self) -> &Plan {
        &self.plan
    }

    /// The newest time seen in an event.
    #[must_use]
    pub const fn now(&self) -> Tick {
        self.now
    }

    /// The complete environment a child spawned now would inherit: the session's
    /// own rules, the display the compositor published, and the safe-mode marker
    /// while the session runs in safe mode.
    #[must_use]
    pub fn environment(&self) -> Environment {
        let mut environment = self.rules.environment.session.clone();
        for (key, value) in &self.published {
            environment.set(key.clone(), value.clone());
        }
        if self.recovery.safe_mode {
            environment.set(self.rules.environment.safe_mode_key.clone(), "1");
        }
        environment
    }

    /// One child's status.
    #[must_use]
    pub fn status(&self, child: ChildId) -> Option<ChildStatus> {
        self.index(child).map(|index| self.status_at(index))
    }

    /// One child's declaration, as the guardian currently holds it: a child a configuration
    /// added is declared here and nowhere in the plan.
    #[must_use]
    pub fn spec(&self, child: ChildId) -> Option<&ChildSpec> {
        self.index(child).map(|index| &self.children[index].spec)
    }

    /// The position of a child in the record list. Records are never removed, so the
    /// position stays valid for the whole session.
    fn index(&self, child: ChildId) -> Option<usize> {
        self.children.iter().position(|record| record.spec.id == child)
    }

    /// Every child's status, in start order.
    #[must_use]
    pub fn statuses(&self) -> Vec<ChildStatus> {
        (0..self.children.len()).map(|index| self.status_at(index)).collect()
    }

    fn status_at(&self, index: usize) -> ChildStatus {
        let child = &self.children[index];
        ChildStatus {
            id: child.spec.id,
            name: child.spec.name.clone(),
            kind: child.spec.kind,
            state: child.state,
            generation: child.generation,
            restarts: child.restarts,
            last_exit: child.last_exit,
        }
    }

    fn start(&mut self, now: Tick, effects: &mut Vec<Effect>) {
        if self.session != SessionState::Idle {
            note(
                Level::Warning,
                format!(
                    "[guardian] ignoring start: the session is already {}",
                    self.session.as_str()
                ),
                effects,
            );
            return;
        }
        self.session = SessionState::Starting;
        note(
            Level::Info,
            format!("[guardian] session start: {} planned children", self.children.len()),
            effects,
        );
        if !self.recovery.safe_mode && self.recovery.failed_starts >= self.rules.safe_mode_after {
            self.recovery.safe_mode = true;
            note(
                Level::Warning,
                format!(
                    "[guardian] safe mode after {} failed starts: the user configuration is skipped",
                    self.recovery.failed_starts
                ),
                effects,
            );
            self.persist(effects);
        }
        effects.push(Effect::ExportEnv {
            target: EnvTarget::Session,
            environment: self.environment(),
        });
        effects.push(Effect::ExportEnv {
            target: EnvTarget::Activation,
            environment: self.rules.environment.activation.clone(),
        });
        self.ladder(now, effects);
    }

    /// Walk the start sequence as far as it can go: spawn the child at the cursor
    /// and stop when it needs to report readiness, or finish the sequence when
    /// every child has been spawned.
    fn ladder(&mut self, now: Tick, effects: &mut Vec<Effect>) {
        if self.stalled.is_some() {
            return;
        }
        if !matches!(self.session, SessionState::Starting | SessionState::Degraded) {
            return;
        }
        while self.cursor < self.children.len() {
            let index = self.cursor;
            match self.children[index].state {
                // Already started, or already given up on: nothing to do here.
                ChildState::Ready | ChildState::Running | ChildState::Failed | ChildState::Stopped => {
                    self.cursor += 1;
                }
                // The next child must not start until this one is usable, and a
                // child waiting for its restart is not usable.
                ChildState::Starting | ChildState::Stopping | ChildState::Backoff => return,
                ChildState::Pending => {
                    self.spawn(index, now, effects);
                    if self.children[index].state == ChildState::Starting {
                        return;
                    }
                    self.cursor += 1;
                }
            }
        }
        self.reach_up(effects);
    }

    fn reach_up(&mut self, effects: &mut Vec<Effect>) {
        if !matches!(self.session, SessionState::Starting | SessionState::Degraded) {
            return;
        }
        if self
            .children
            .iter()
            .any(|child| matches!(child.state, ChildState::Starting | ChildState::Stopping))
        {
            return;
        }
        self.session = SessionState::Up;
        self.reached_up = true;
        for child in &mut self.children {
            if child.state == ChildState::Ready {
                child.state = ChildState::Running;
            }
        }
        if self.recovery.failed_starts != 0 {
            self.recovery.failed_starts = 0;
            self.persist(effects);
        }
        note(Level::Info, "[guardian] session ready".to_owned(), effects);
    }

    fn spawn(&mut self, index: usize, now: Tick, effects: &mut Vec<Effect>) {
        let environment = self.environment();
        let child = &mut self.children[index];
        child.generation += 1;
        let needs_ready = child.spec.ready_within.is_some();
        child.state = if needs_ready {
            ChildState::Starting
        } else {
            ChildState::Running
        };
        child.deadline = child.spec.ready_within.map(|within| now.saturating_add(within.get()));
        child.ready_at = if needs_ready { None } else { Some(now) };
        child.restart_at = None;
        child.stop = None;
        effects.push(Effect::Spawn {
            child: child.spec.id,
            generation: child.generation,
            name: child.spec.name.clone(),
            environment,
        });
    }

    fn ready(
        &mut self,
        child: ChildId,
        generation: u64,
        environment: Vec<(String, String)>,
        now: Tick,
        effects: &mut Vec<Effect>,
    ) {
        let Some(index) = self.index(child) else {
            note(
                Level::Warning,
                format!("[guardian] readiness from unknown child {}", child.get()),
                effects,
            );
            return;
        };
        if generation != self.children[index].generation {
            note(
                Level::Debug,
                format!(
                    "[guardian] readiness from an earlier {} instance ignored",
                    self.children[index].spec.name
                ),
                effects,
            );
            return;
        }
        match self.children[index].state {
            ChildState::Starting => {}
            ChildState::Ready | ChildState::Running => {
                note(
                    Level::Debug,
                    format!(
                        "[guardian] duplicate readiness from {} ignored",
                        self.children[index].spec.name
                    ),
                    effects,
                );
                return;
            }
            state => {
                note(
                    Level::Warning,
                    format!(
                        "[guardian] {} reported ready while {}; ignored",
                        self.children[index].spec.name,
                        state.as_str()
                    ),
                    effects,
                );
                return;
            }
        }
        let kind = self.children[index].spec.kind;
        self.children[index].state = if matches!(self.session, SessionState::Up | SessionState::Degraded) {
            ChildState::Running
        } else {
            ChildState::Ready
        };
        self.children[index].deadline = None;
        self.children[index].ready_at = Some(now);
        if kind == ChildKind::Compositor {
            self.publish_display(environment, now, effects);
        }
        self.ladder(now, effects);
    }

    /// Take the display variables the compositor published and make them the
    /// session's, so every later child and the D-Bus activation environment see
    /// the display. A missing required variable ends the session instead of
    /// launching children that cannot reach their server.
    fn publish_display(&mut self, environment: Vec<(String, String)>, now: Tick, effects: &mut Vec<Effect>) {
        let mut published = Vec::new();
        let mut missing = Vec::new();
        for name in &self.rules.environment.forwarded {
            match environment.iter().find(|(key, _)| key == name) {
                Some((_, value)) => published.push((name.clone(), value.clone())),
                None => {
                    if self.rules.environment.required.contains(name) {
                        missing.push(name.clone());
                    }
                }
            }
        }
        if !missing.is_empty() {
            note(
                Level::Error,
                format!(
                    "[guardian] the compositor reported ready without {}; ending the session",
                    missing.join(", ")
                ),
                effects,
            );
            self.begin_stop(StopReason::StartFailed, now, effects);
            return;
        }
        self.published = published;
        let mut export = Environment::new();
        for (key, value) in &self.published {
            export.set(key.clone(), value.clone());
        }
        effects.push(Effect::ExportEnv {
            target: EnvTarget::Session,
            environment: export.clone(),
        });
        effects.push(Effect::ExportEnv {
            target: EnvTarget::Activation,
            environment: export,
        });
    }

    fn exited(&mut self, child: ChildId, generation: u64, status: ExitStatus, now: Tick, effects: &mut Vec<Effect>) {
        let Some(index) = self.index(child) else {
            note(
                Level::Warning,
                format!("[guardian] exit from unknown child {}", child.get()),
                effects,
            );
            return;
        };
        if generation != self.children[index].generation {
            note(
                Level::Debug,
                format!(
                    "[guardian] exit of an earlier {} instance ignored",
                    self.children[index].spec.name
                ),
                effects,
            );
            return;
        }
        let name = self.children[index].spec.name.clone();
        if self.session == SessionState::Ended {
            note(
                Level::Debug,
                format!("[guardian] exit of {name} after the session ended ignored"),
                effects,
            );
            return;
        }
        self.children[index].last_exit = Some(status);
        if self.session == SessionState::Stopping {
            // No restart outlives the session, and no rank waits for a child
            // that the caller reports late.
            if self.children[index].state != ChildState::Pending {
                self.children[index].state = ChildState::Stopped;
            }
            self.children[index].stop = None;
            self.children[index].deadline = None;
            self.children[index].restart_at = None;
            note(Level::Debug, format!("[guardian] {name} stopped"), effects);
            self.stop_step(now, effects);
            return;
        }
        if self.children[index].state == ChildState::Stopping
            && self.children[index].stop.map(|stop| stop.cause) == Some(StopCause::Retired)
        {
            // The configuration dropped this application while the session keeps running. Its
            // exit is what the retirement asked for, so it is not a failure and never restarts.
            self.children[index].state = ChildState::Stopped;
            self.children[index].stop = None;
            self.children[index].deadline = None;
            note(
                Level::Info,
                format!(
                    "[guardian] {} stopped; the configuration no longer starts it",
                    self.children[index].spec.name
                ),
                effects,
            );
            // A stopping child holds the start sequence and the Up transition back, so
            // letting it go may let either proceed.
            self.ladder(now, effects);
            return;
        }
        match self.children[index].state {
            ChildState::Starting | ChildState::Ready | ChildState::Running => self.crashed(index, status, now, effects),
            ChildState::Stopping => {
                // A child killed for missing its readiness deadline. Its exit is
                // the failure the deadline detected.
                self.crashed(index, status, now, effects);
            }
            ChildState::Backoff => {
                note(
                    Level::Debug,
                    format!("[guardian] exit of {name} while it waits to restart ignored"),
                    effects,
                );
            }
            ChildState::Pending | ChildState::Failed | ChildState::Stopped => {
                note(
                    Level::Debug,
                    format!(
                        "[guardian] unexpected exit of {name} in state {} ignored",
                        self.children[index].state.as_str()
                    ),
                    effects,
                );
            }
        }
    }

    fn crashed(&mut self, index: usize, status: ExitStatus, now: Tick, effects: &mut Vec<Effect>) {
        let name = self.children[index].spec.name.clone();
        let restart = self.children[index].spec.restart;
        let up = self.reached_up;
        match restart {
            Restart::EndsSession => {
                self.children[index].state = ChildState::Stopped;
                self.children[index].deadline = None;
                self.children[index].stop = None;
                if status.success() {
                    note(
                        Level::Warning,
                        format!("[guardian] {name} exited cleanly; the session cannot continue without it"),
                        effects,
                    );
                } else {
                    note(
                        Level::Warning,
                        format!("[guardian] {name} exited ({status}); the session cannot continue"),
                        effects,
                    );
                }
                let reason = if up {
                    StopReason::CompositorLost
                } else {
                    StopReason::StartFailed
                };
                self.begin_stop(reason, now, effects);
            }
            Restart::Never => {
                self.children[index].state = ChildState::Failed;
                self.children[index].deadline = None;
                note(
                    Level::Warning,
                    format!("[guardian] {name} exited ({status}); it is not restarted"),
                    effects,
                );
                if !up && index == self.cursor {
                    self.cursor += 1;
                    self.ladder(now, effects);
                }
            }
            Restart::Backoff(backoff) => {
                let restarts = self.children[index].restarts;
                if restarts >= backoff.max_restarts {
                    self.children[index].state = ChildState::Failed;
                    self.children[index].deadline = None;
                    note(
                        Level::Error,
                        format!("[guardian] {name} failed after {restarts} restarts; the session is degraded"),
                        effects,
                    );
                    self.degrade(index);
                } else {
                    let attempt = restarts + 1;
                    let delay = backoff.delay(attempt);
                    self.children[index].restarts = attempt;
                    self.children[index].state = ChildState::Backoff;
                    self.children[index].restart_at = Some(now.saturating_add(delay.get()));
                    self.children[index].deadline = None;
                    self.children[index].stop = None;
                    note(
                        Level::Warning,
                        format!(
                            "[guardian] restarting {name} in {} ms (attempt {attempt} of {})",
                            delay.get(),
                            backoff.max_restarts
                        ),
                        effects,
                    );
                }
            }
        }
    }

    /// A child gave up its restarts. The session keeps whatever still works, so
    /// one services crash cannot take the desktop with it, but the start sequence
    /// stops at a child that never came up.
    fn degrade(&mut self, index: usize) {
        if matches!(self.session, SessionState::Starting | SessionState::Up) {
            self.session = SessionState::Degraded;
        }
        if !self.reached_up && self.cursor <= index {
            self.stalled = Some(self.children[index].spec.id);
        }
    }

    fn tick(&mut self, now: Tick, effects: &mut Vec<Effect>) {
        match self.session {
            SessionState::Idle | SessionState::Ended => return,
            SessionState::Stopping => {
                self.stop_step(now, effects);
                return;
            }
            SessionState::Starting | SessionState::Up | SessionState::Degraded => {}
        }

        let grace = self.rules.stop_grace;

        // A child that missed its readiness deadline is unusable: kill it and
        // treat its exit as the failure it announced.
        for index in 0..self.children.len() {
            let child = &mut self.children[index];
            if child.state != ChildState::Starting {
                continue;
            }
            let Some(deadline) = child.deadline else {
                continue;
            };
            if deadline > now {
                continue;
            }
            let within = child.spec.ready_within.unwrap_or(Tick::ZERO);
            note(
                Level::Warning,
                format!(
                    "[guardian] {} did not report ready within {} ms; killing it",
                    child.spec.name,
                    within.get()
                ),
                effects,
            );
            effects.push(Effect::Kill { child: child.spec.id });
            child.state = ChildState::Stopping;
            child.stop = Some(StopPending {
                cause: StopCause::Unready,
                killed: true,
            });
            child.deadline = Some(now.saturating_add(grace.get()));
        }

        // A retired application that ignored its SIGTERM gets the same forced kill a session stop
        // gives, so a wedged application cannot hold the start sequence open indefinitely.
        for index in 0..self.children.len() {
            let child = &mut self.children[index];
            if child.state != ChildState::Stopping || child.stop.map(|stop| stop.cause) != Some(StopCause::Retired) {
                continue;
            }
            let Some(deadline) = child.deadline else {
                continue;
            };
            if deadline > now {
                continue;
            }
            if child.stop.is_some_and(|stop| stop.killed) {
                note(
                    Level::Error,
                    format!("[guardian] {} did not exit after the forced kill", child.spec.name),
                    effects,
                );
                child.state = ChildState::Stopped;
                child.stop = None;
                child.deadline = None;
            } else {
                note(
                    Level::Warning,
                    format!(
                        "[guardian] {} did not stop within {} ms; killing it",
                        child.spec.name,
                        grace.get()
                    ),
                    effects,
                );
                effects.push(Effect::Kill { child: child.spec.id });
                if let Some(stop) = &mut child.stop {
                    stop.killed = true;
                }
                child.deadline = Some(now.saturating_add(grace.get()));
            }
        }

        // A killed child the caller never reported: the kernel guarantees the
        // process is gone, so the guardian stops waiting for the report rather
        // than holding the session at a child that will never answer.
        let mut unreaped = Vec::new();
        for index in 0..self.children.len() {
            let child = &self.children[index];
            if child.state != ChildState::Stopping {
                continue;
            }
            if child.stop.map(|stop| stop.cause) != Some(StopCause::Unready) {
                continue;
            }
            if child.deadline.is_some_and(|deadline| deadline <= now) {
                unreaped.push(index);
            }
        }
        for index in unreaped {
            let name = self.children[index].spec.name.clone();
            note(
                Level::Error,
                format!("[guardian] {name} did not exit after the forced kill"),
                effects,
            );
            self.crashed(index, ExitStatus::Signalled(Signal::Kill), now, effects);
        }

        // Restarts are caller-driven: nothing here runs without a tick.
        let mut restarted = false;
        for index in 0..self.children.len() {
            let child = &self.children[index];
            if child.state != ChildState::Backoff {
                continue;
            }
            if child.restart_at.is_some_and(|at| at <= now) {
                self.spawn(index, now, effects);
                restarted = true;
            }
        }
        if restarted {
            self.ladder(now, effects);
        }

        // A child that stayed up long enough gets its budget back, exactly as
        // `WORKER_STABLE_RESET_MS` does for the worker.
        for index in 0..self.children.len() {
            let child = &mut self.children[index];
            let Restart::Backoff(backoff) = child.spec.restart else {
                continue;
            };
            if child.restarts == 0 || !matches!(child.state, ChildState::Ready | ChildState::Running) {
                continue;
            }
            let Some(ready_at) = child.ready_at else {
                continue;
            };
            if now.since(ready_at) >= backoff.stable_after.get() {
                child.restarts = 0;
                note(
                    Level::Debug,
                    format!(
                        "[guardian] {} has been stable; its restart budget is reset",
                        child.spec.name
                    ),
                    effects,
                );
            }
        }
    }

    fn end(&mut self, reason: ShutdownReason, now: Tick, effects: &mut Vec<Effect>) {
        let reason = match reason {
            ShutdownReason::Logout => StopReason::Logout,
            ShutdownReason::Signal => StopReason::Signal,
        };
        match self.session {
            SessionState::Idle => {
                self.session = SessionState::Ended;
                self.stop_reason = Some(reason);
                note(
                    Level::Info,
                    format!("[guardian] session ended: {} before it started", reason.as_str()),
                    effects,
                );
            }
            SessionState::Ended => {
                note(
                    Level::Debug,
                    "[guardian] session already ended; ignoring".to_owned(),
                    effects,
                );
            }
            SessionState::Stopping => {
                note(
                    Level::Debug,
                    "[guardian] session already stopping; ignoring".to_owned(),
                    effects,
                );
            }
            SessionState::Starting | SessionState::Up | SessionState::Degraded => {
                self.begin_stop(reason, now, effects);
            }
        }
    }

    fn begin_stop(&mut self, reason: StopReason, now: Tick, effects: &mut Vec<Effect>) {
        if matches!(self.session, SessionState::Stopping | SessionState::Ended) {
            return;
        }
        self.session = SessionState::Stopping;
        self.stop_reason = Some(reason);
        note(
            Level::Info,
            format!("[guardian] session stopping: {}", reason.as_str()),
            effects,
        );
        let mut ranks: Vec<Vec<usize>> = Vec::new();
        let mut start = 0;
        while start < self.children.len() {
            let kind = self.children[start].spec.kind;
            let mut end = start + 1;
            while end < self.children.len() && self.children[end].spec.kind == kind {
                end += 1;
            }
            ranks.push((start..end).rev().collect());
            start = end;
        }
        ranks.reverse();
        self.stop = Some(Stop { ranks, current: 0 });
        self.stop_step(now, effects);
    }

    /// Stop the current rank, then the next one, until nothing is left.
    ///
    /// The first call signals the last-started rank, so a caller that shuts the
    /// session down and then only reports exits and ticks drives the whole
    /// sequence without another instruction from the guardian.
    fn stop_step(&mut self, now: Tick, effects: &mut Vec<Effect>) {
        let grace = self.rules.stop_grace;
        loop {
            let Some(rank) = self
                .stop
                .as_ref()
                .and_then(|stop| stop.ranks.get(stop.current))
                .cloned()
            else {
                self.finish_stop(effects);
                return;
            };

            // Anything past its grace period is killed; anything already killed
            // and still unreported is assumed dead, so one wedged child cannot
            // hold the whole session open.
            for index in &rank {
                let child = &mut self.children[*index];
                if child.state != ChildState::Stopping {
                    continue;
                }
                let Some(deadline) = child.deadline else {
                    continue;
                };
                if deadline > now {
                    continue;
                }
                if child.stop.map(|stop| stop.killed) == Some(true) {
                    note(
                        Level::Error,
                        format!("[guardian] {} did not exit after the forced kill", child.spec.name),
                        effects,
                    );
                    child.state = ChildState::Stopped;
                    child.stop = None;
                    child.deadline = None;
                } else {
                    note(
                        Level::Warning,
                        format!(
                            "[guardian] {} did not stop within {} ms; killing it",
                            child.spec.name,
                            grace.get()
                        ),
                        effects,
                    );
                    effects.push(Effect::Kill { child: child.spec.id });
                    if let Some(stop) = &mut child.stop {
                        stop.killed = true;
                    }
                    child.deadline = Some(now.saturating_add(grace.get()));
                }
            }

            if rank
                .iter()
                .any(|index| self.children[*index].state == ChildState::Stopping)
            {
                return;
            }

            let mut signalled = false;
            for index in &rank {
                let child = &mut self.children[*index];
                match child.state {
                    ChildState::Starting | ChildState::Ready | ChildState::Running => {
                        effects.push(Effect::Signal {
                            child: child.spec.id,
                            signal: Signal::Term,
                        });
                        child.state = ChildState::Stopping;
                        child.stop = Some(StopPending {
                            cause: StopCause::Session,
                            killed: false,
                        });
                        child.deadline = Some(now.saturating_add(grace.get()));
                        signalled = true;
                    }
                    ChildState::Backoff => {
                        // Nothing to signal: the process is already gone and only
                        // its restart was pending.
                        child.state = ChildState::Stopped;
                        child.restart_at = None;
                        child.deadline = None;
                        child.stop = None;
                    }
                    ChildState::Pending | ChildState::Stopping | ChildState::Failed | ChildState::Stopped => {}
                }
            }
            if signalled {
                return;
            }
            if let Some(stop) = &mut self.stop {
                stop.current += 1;
            }
        }
    }

    fn finish_stop(&mut self, effects: &mut Vec<Effect>) {
        self.stop = None;
        self.session = SessionState::Ended;
        let reason = self.stop_reason.map_or("unknown", StopReason::as_str);
        if self.reached_up {
            note(Level::Info, format!("[guardian] session ended: {reason}"), effects);
        } else {
            self.recovery.failed_starts = self.recovery.failed_starts.saturating_add(1);
            note(
                Level::Warning,
                format!(
                    "[guardian] session ended after a failed start ({reason}); {} failed starts in a row",
                    self.recovery.failed_starts
                ),
                effects,
            );
            self.persist(effects);
        }
    }

    fn config_accepted(&mut self, revision: u64, autostart: Vec<AutostartEntry>, now: Tick, effects: &mut Vec<Effect>) {
        self.recovery.config.last_good = Some(revision);
        self.recovery.config.status = ConfigStatus::Accepted;
        note(
            Level::Info,
            format!("[guardian] configuration revision {revision} accepted"),
            effects,
        );
        if self.recovery.safe_mode {
            self.recovery.safe_mode = false;
            note(
                Level::Info,
                "[guardian] the user configuration was accepted; leaving safe mode".to_owned(),
                effects,
            );
        }
        self.persist(effects);
        self.reconcile_autostart(&autostart, now, effects);
    }

    /// Bring the autostart children in line with the configuration just accepted.
    ///
    /// A live record the configuration still lists keeps running untouched. A listed entry with no
    /// record gets a new child. A live record the configuration dropped is retired: an unspawned one
    /// is simply forgotten, and a running one is asked to exit, so it can save its state, with the
    /// same grace period a session stop gives.
    fn reconcile_autostart(&mut self, entries: &[AutostartEntry], now: Tick, effects: &mut Vec<Effect>) {
        // A session that is stopping or has ended starts nothing new.
        if matches!(self.session, SessionState::Stopping | SessionState::Ended) {
            return;
        }
        let mut claimed = vec![false; entries.len()];
        for index in 0..self.children.len() {
            if !self.children[index].live_autostart() {
                continue;
            }
            let spec = &self.children[index].spec;
            let entry = entries.iter().enumerate().position(|(entry, wanted)| {
                !claimed[entry] && wanted.name == spec.name && wanted.command == spec.command
            });
            match entry {
                Some(entry) => claimed[entry] = true,
                None => self.retire_autostart(index, now, effects),
            }
        }
        for (entry, wanted) in entries.iter().enumerate() {
            if claimed[entry] {
                continue;
            }
            let id = ChildId::new(self.next_id);
            self.next_id += 1;
            self.children.push(Child::new(ChildSpec::autostart(
                id,
                wanted.name.clone(),
                wanted.command.clone(),
            )));
            note(
                Level::Info,
                format!("[guardian] autostart {} added by the configuration", wanted.name),
                effects,
            );
        }
        if self.reached_up {
            // The start sequence is over, so nothing else waits on a new application: start it now.
            for index in 0..self.children.len() {
                let child = &self.children[index];
                if child.state == ChildState::Pending && child.spec.kind == ChildKind::Autostart {
                    self.spawn(index, now, effects);
                }
            }
        } else {
            // Still starting: the start sequence reaches the new entries in order.
            self.ladder(now, effects);
        }
    }

    /// Stop one autostart child the configuration no longer starts.
    fn retire_autostart(&mut self, index: usize, now: Tick, effects: &mut Vec<Effect>) {
        let grace = self.rules.stop_grace;
        let child = &mut self.children[index];
        if matches!(child.state, ChildState::Pending | ChildState::Backoff) {
            // Never spawned, or waiting for a restart: there is no process to stop.
            child.state = ChildState::Stopped;
            child.restart_at = None;
            note(
                Level::Info,
                format!("[guardian] {} removed by the configuration", child.spec.name),
                effects,
            );
            return;
        }
        child.state = ChildState::Stopping;
        child.stop = Some(StopPending {
            cause: StopCause::Retired,
            killed: false,
        });
        child.deadline = Some(now.saturating_add(grace.get()));
        effects.push(Effect::Signal {
            child: child.spec.id,
            signal: Signal::Term,
        });
        note(
            Level::Info,
            format!(
                "[guardian] stopping {}; the configuration no longer starts it",
                child.spec.name
            ),
            effects,
        );
    }

    fn config_rejected(&mut self, revision: u64, diagnostic: &str, effects: &mut Vec<Effect>) {
        self.recovery.config.status = ConfigStatus::Rejected {
            revision,
            diagnostic: diagnostic.to_owned(),
        };
        note(
            Level::Warning,
            format!("[guardian] configuration revision {revision} rejected: {diagnostic}"),
            effects,
        );
        self.persist(effects);
    }

    fn config_fallback(&mut self, reason: &str, effects: &mut Vec<Effect>) {
        self.recovery.config.status = ConfigStatus::Defaults {
            reason: reason.to_owned(),
        };
        note(
            Level::Warning,
            format!("[guardian] the saved configuration was unusable ({reason}); running the embedded defaults"),
            effects,
        );
        self.persist(effects);
    }

    fn persist(&self, effects: &mut Vec<Effect>) {
        effects.push(Effect::SetRecovery(self.recovery.clone()));
    }
}

/// Push one log line. A free function so the handlers can hold a mutable borrow
/// of a child while they log.
fn note(level: Level, message: String, effects: &mut Vec<Effect>) {
    effects.push(Effect::Log { level, message });
}
