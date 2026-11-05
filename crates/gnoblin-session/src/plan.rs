//! The plan: which children exist, in which order, and how each is supervised.

use crate::Tick;
use crate::ids::ChildId;
use crate::time::secs;
use core::fmt;

/// How long a child may take to report readiness before the guardian kills it.
///
/// `STARTUP_TIMEOUT_MS` in `src/session/gnoblin-runtime-internal.h` gives the
/// runtime ten seconds; one number for every child keeps the plan readable and a
/// child that needs longer can override it in its own [`ChildSpec`].
pub const DEFAULT_READY_WITHIN: Tick = secs(10);

/// What a planned child is.
///
/// The kind decides the default supervision policy, the position in the start
/// order and how the guardian reports it. It is not an enum of behaviours: two
/// children of the same kind can still carry different [`ChildSpec`] fields.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChildKind {
    /// `gnoblin --role compositor`: Wayland, rendering, input, the policy core
    /// and the IPC socket.
    Compositor,
    /// `gnoblin --role services`: the D-Bus agents.
    Services,
    /// `gnoblin --role runtime`: the Lua configuration host.
    Runtime,
    /// The greeter a session manager starts in front of the session.
    Greeter,
    /// One autostart application, whether from XDG or from configuration.
    Autostart,
}

impl ChildKind {
    /// Whether this kind reports readiness over a channel of its own.
    ///
    /// An autostart application is a plain desktop application: it has no way to
    /// say "ready", so it counts as running as soon as it is spawned and the
    /// guardian never kills it for missing a deadline.
    #[must_use]
    pub const fn reports_ready(self) -> bool {
        !matches!(self, Self::Autostart)
    }
}

/// What the guardian does when a child's process ends.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Restart {
    /// The compositor: its exit ends the session, so it is never restarted.
    ///
    /// Every other child is stopped in reverse start order, because a client
    /// that outlives the compositor it talks to is an orphan.
    EndsSession,
    /// Never restarted. A failure is logged and the session continues, because
    /// the child is not something the session cannot run without.
    Never,
    /// Restarted with bounded exponential backoff and a circuit breaker.
    Backoff(Backoff),
}

/// Bounded exponential backoff with a crash-loop circuit breaker.
///
/// The numbers default to the ones the C supervisor uses
/// (`sleep_before_worker_restart`, `WORKER_MAX_RESTARTS`,
/// `WORKER_STABLE_RESET_MS`): 250 ms doubling to a 4 s cap, five restarts, and a
/// budget that resets after a minute of uptime. Bounded is the point: an
/// unbounded ladder would burn a core for the rest of the login.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Backoff {
    /// The wait before the first restart.
    pub initial: Tick,
    /// The multiplier applied for each later restart.
    pub factor: u32,
    /// The longest wait the ladder may reach.
    pub max: Tick,
    /// How many restarts in a row are attempted before the child is failed and
    /// the session is reported degraded.
    pub max_restarts: u32,
    /// How long a child must stay up before its restart budget is reset.
    pub stable_after: Tick,
}

impl Backoff {
    /// The ladder the C supervisor uses today.
    #[must_use]
    pub const fn guardian() -> Self {
        Self {
            initial: Tick::new(250),
            factor: 2,
            max: Tick::new(4000),
            max_restarts: 5,
            stable_after: secs(60),
        }
    }

    /// How long to wait before the `restart`th restart, counting from one.
    ///
    /// A `factor` below two cannot grow the wait, and the ladder stops as soon
    /// as it reaches `max`, so this terminates for every input, including a
    /// `restart` large enough to make an unguarded shift overflow.
    #[must_use]
    pub fn delay(&self, restart: u32) -> Tick {
        let factor = u64::from(self.factor);
        let mut delay = self.initial;
        if factor < 2 {
            return if delay > self.max { self.max } else { delay };
        }
        let mut step = 1;
        while step < restart {
            if delay >= self.max {
                break;
            }
            // The doubling step: a shared tick has no scale operation because no
            // other crate needs one, and doing it here keeps the saturation at
            // the value that reaches the caller.
            delay = Tick::new(delay.get().saturating_mul(factor));
            step += 1;
        }
        if delay > self.max { self.max } else { delay }
    }
}

/// One planned child and how it is supervised.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChildSpec {
    /// The identifier effects and events use.
    pub id: ChildId,
    /// What the child is.
    pub kind: ChildKind,
    /// The name used in logs, such as `runtime` or `org.gnome.Terminal.desktop`.
    pub name: String,
    /// How long the child may take to report readiness, or `None` when it never
    /// reports any.
    pub ready_within: Option<Tick>,
    /// What happens when its process ends.
    pub restart: Restart,
}

impl ChildSpec {
    /// A child with the policy its kind implies.
    ///
    /// The compositor ends the session when it exits; the runtime and services
    /// restart with [`Backoff::guardian`]; the greeter and autostart
    /// applications are not restarted, because the session manager owns greeter
    /// retries and a user application that dies is not the session's to raise.
    /// Every kind except autostart applications reports readiness within
    /// [`DEFAULT_READY_WITHIN`].
    #[must_use]
    pub fn new(id: ChildId, kind: ChildKind, name: impl Into<String>) -> Self {
        let restart = match kind {
            ChildKind::Compositor => Restart::EndsSession,
            ChildKind::Runtime | ChildKind::Services => Restart::Backoff(Backoff::guardian()),
            ChildKind::Greeter | ChildKind::Autostart => Restart::Never,
        };
        Self {
            id,
            kind,
            name: name.into(),
            ready_within: if kind.reports_ready() {
                Some(DEFAULT_READY_WITHIN)
            } else {
                None
            },
            restart,
        }
    }
}

/// Why a plan cannot describe a session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanError {
    /// A session needs at least one child.
    Empty,
    /// Two children share an identifier, so an effect could name either.
    DuplicateId(ChildId),
    /// Nothing in the plan is the compositor.
    MissingCompositor,
    /// The compositor is not the first child. Everything else needs the display
    /// it publishes, so it must start before them.
    CompositorNotFirst,
    /// A session has one compositor.
    SecondCompositor,
    /// A session has at most one greeter; the session manager starts it.
    SecondGreeter,
}

impl fmt::Display for PlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("a session plan needs at least one child"),
            Self::DuplicateId(id) => write!(formatter, "child id {} is declared twice", id.get()),
            Self::MissingCompositor => formatter.write_str("a session plan needs a compositor"),
            Self::CompositorNotFirst => formatter
                .write_str("the compositor must be the first child, because every other child needs its display"),
            Self::SecondCompositor => formatter.write_str("a session plan has one compositor"),
            Self::SecondGreeter => formatter.write_str("a session plan has at most one greeter"),
        }
    }
}

impl std::error::Error for PlanError {}

/// The children of one session, in start order.
///
/// The order is the contract: children start in this order, one at a time, each
/// after the previous one reports ready, and stop in the reverse of it. Stop
/// order is reverse start order because a later child depends on an earlier one,
/// so a client is never left talking to a server that has already gone.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Plan {
    children: Vec<ChildSpec>,
}

impl Plan {
    /// A plan over exactly these children, validated.
    pub fn new(children: Vec<ChildSpec>) -> Result<Self, PlanError> {
        if children.is_empty() {
            return Err(PlanError::Empty);
        }
        for (index, child) in children.iter().enumerate() {
            if children[..index].iter().any(|other| other.id == child.id) {
                return Err(PlanError::DuplicateId(child.id));
            }
        }
        if !children.iter().any(|child| child.kind == ChildKind::Compositor) {
            return Err(PlanError::MissingCompositor);
        }
        if children[0].kind != ChildKind::Compositor {
            return Err(PlanError::CompositorNotFirst);
        }
        if children
            .iter()
            .filter(|child| child.kind == ChildKind::Compositor)
            .count()
            > 1
        {
            return Err(PlanError::SecondCompositor);
        }
        if children.iter().filter(|child| child.kind == ChildKind::Greeter).count() > 1 {
            return Err(PlanError::SecondGreeter);
        }
        Ok(Self { children })
    }

    /// The session Gnoblin starts today: compositor, services, runtime, then the
    /// greeter when the session manager asks for one, then each autostart entry
    /// in configuration order.
    ///
    /// Services start before the runtime because the runtime's user scripts can
    /// reach them during their own start, and because a session whose agents are
    /// missing must be visible before user policy runs. Identifiers are assigned
    /// `1..=n` in this order.
    pub fn standard(greeter: bool, autostart: impl IntoIterator<Item = String>) -> Result<Self, PlanError> {
        let mut next = 1;
        let mut children = Vec::new();
        for (kind, name) in [
            (ChildKind::Compositor, "compositor"),
            (ChildKind::Services, "services"),
            (ChildKind::Runtime, "runtime"),
        ] {
            children.push(ChildSpec::new(ChildId::new(next), kind, name));
            next += 1;
        }
        if greeter {
            children.push(ChildSpec::new(ChildId::new(next), ChildKind::Greeter, "greeter"));
            next += 1;
        }
        for name in autostart {
            children.push(ChildSpec::new(ChildId::new(next), ChildKind::Autostart, name));
            next += 1;
        }
        Self::new(children)
    }

    /// Every child, in start order.
    #[must_use]
    pub fn children(&self) -> &[ChildSpec] {
        &self.children
    }

    /// The position in start order of a child, or `None` when the plan does not
    /// contain it. An event naming an unknown child is a caller bug, and the
    /// guardian ignores it rather than guessing.
    #[must_use]
    pub fn index(&self, id: ChildId) -> Option<usize> {
        self.children.iter().position(|child| child.id == id)
    }

    /// The declaration of a child.
    #[must_use]
    pub fn spec(&self, id: ChildId) -> Option<&ChildSpec> {
        self.index(id).map(|index| &self.children[index])
    }
}
