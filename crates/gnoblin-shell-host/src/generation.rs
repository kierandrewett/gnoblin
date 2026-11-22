//! Generation contexts, the registrations they own, the event queue and the dispatch loop.
//!
//! One [`GenerationHost`] holds every generation of a shell and every registration a generation made, plus the queues the
//! compositor side publishes into. It knows nothing of a language or of the C ABI: a registration is a boxed function and
//! a boxed release, so a native shell's `extern "C"` callbacks, a script's functions and a test's closures are all the
//! same thing to it. The invariants this module keeps:
//!
//! - Each registration is owned by the generation that made it, and removed with it. Its release (a native shell's
//!   `drop_user`, a script's handle) runs exactly once, on the shell thread, before the generation's code can go away.
//! - A retired generation's calls are refused with [`CtxError::Retired`] and do nothing. Its record is kept so the answer
//!   stays "retired" rather than becoming "no such generation".
//! - A registration of a generation that is not active (staged for a reload) receives nothing, and abandoning the stage
//!   discards it without a call.
//! - Shell code never runs with the lock held. A callback may remove registrations, including its own: removing the running
//!   one defers its release until the callback returns.
//! - An event is delivered to a registration only if the registration still exists, is not marked dead, and its generation
//!   is active at the moment of the call: removed-before-call is checked, not assumed.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant};

use gnoblin_core_abi as abi;

use crate::guard::{PANICS, contained, set_code_error};
use crate::log;

/// The most events the mailbox holds before the oldest is dropped. A shell that never dispatches must not grow the
/// compositor's memory without bound.
pub const MAILBOX_CAP: usize = 65_536;

/// A generation's number: a sequence number the host never reuses.
pub type GenerationId = u64;
/// A registration's number: a sequence number the host never reuses.
pub type RegistrationId = u64;

/// Where a generation is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenerationState {
    /// Loaded and initialising, or waiting for its reload to commit: its registrations receive nothing yet.
    Staged,
    /// LiveProcess: its registrations receive events.
    Active,
    /// Gone: every call it makes is refused.
    Retired,
}

/// What a generation says about itself: the facts `ctx_info` answers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GenerationInfo {
    /// The file the generation was loaded from.
    pub module_path: String,
    /// The shell's private state directory, empty when it has none.
    pub state_dir: String,
    /// The language constant of the module (`LANG_NATIVE`, ...).
    pub language: u32,
    /// How many reloads preceded this load.
    pub reload_count: u32,
    /// Whether the host runs isolated in a child process.
    pub isolate: bool,
}

/// Why a call scoped to a generation was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CtxError {
    /// The host never issued this generation.
    Unknown(GenerationId),
    /// The generation has retired: the call did nothing.
    Retired(GenerationId),
    /// The call needs the generation's shell thread and was made from another.
    WrongThread(GenerationId),
}

impl CtxError {
    /// The ABI error code for this refusal.
    pub fn code(self) -> i32 {
        match self {
            Self::Unknown(_) => abi::E_STALE,
            Self::Retired(_) => abi::E_RETIRED,
            Self::WrongThread(_) => abi::E_THREAD,
        }
    }

    /// The text of the refusal for the call named `what`.
    pub fn message(self, what: &str) -> String {
        match self {
            Self::Unknown(id) => format!("{what}: no generation {id}"),
            Self::Retired(id) => format!("generation {id} has been retired; {what} did nothing"),
            Self::WrongThread(id) => format!("{what} must be called on the shell thread of generation {id}"),
        }
    }

    /// Records the refusal as this thread's last error and returns its code.
    pub fn report(self, what: &str) -> i32 {
        set_code_error(self.code(), self.message(what));
        self.code()
    }
}

/// Why a registration could not be removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoveError {
    /// It was removed already, or never existed.
    Gone,
    /// The call was made from a thread other than the shell thread of the registration's generation.
    WrongThread,
}

/// What a callback says about how it ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallbackOutcome {
    /// The callback ran (whatever it returned).
    Handled,
    /// The callback panicked and said so (a native shell's trampoline catches the unwind and reports it).
    Panicked,
}

/// A function to run once and give up: a posted piece of work, or a shutdown hook.
pub type Work = Box<dyn FnOnce() + Send>;
/// What gives up a registration's resources (a native shell's `drop_user`): called exactly once.
pub type Release = Box<dyn FnOnce() + Send>;

/// A callback that receives published events.
pub struct Listener<E> {
    /// Whether an event is for this listener. Called with the host's lock held: keep it to a comparison.
    pub matches: Box<dyn Fn(&E) -> bool + Send + Sync>,
    /// The callback. Called without the lock held; it may call back into the host.
    pub call: Arc<dyn Fn(&E) -> CallbackOutcome + Send + Sync>,
}

enum Recipient<E> {
    Listener(Listener<E>),
    /// A hook that runs once, when the generation retires. `None` once it has run.
    Shutdown(Option<Work>),
}

/// One thing a generation registered.
struct OwnedRegistration<E> {
    owner: GenerationId,
    target: Recipient<E>,
    release: Option<Release>,
    /// Removed while its callback was running: it is deleted, and its release runs, when the callback returns.
    dead: bool,
}

struct Generation {
    state: GenerationState,
    /// The thread that made the generation; every shell-thread call checks the caller against it.
    thread: ThreadId,
    /// Held for the life of the host, which is what `ctx_info` promises a shell about the strings it lends: a
    /// generation's record is never removed and never changed.
    info: Arc<GenerationInfo>,
}

/// Work posted to a shell thread.
struct Post {
    owner: GenerationId,
    work: Work,
    /// Runs instead of `work` when the generation retires first.
    discard: Release,
}

struct HostTables<E> {
    next_generation: u64,
    next_registration: u64,
    generations: BTreeMap<GenerationId, Generation>,
    registrations: BTreeMap<RegistrationId, OwnedRegistration<E>>,
    events: VecDeque<E>,
    posts: VecDeque<Post>,
    /// The registration whose callback is running now.
    in_flight: Option<RegistrationId>,
}

impl<E> HostTables<E> {
    fn owner_is_active(&self, owner: GenerationId) -> bool {
        self.generations
            .get(&owner)
            .is_some_and(|generation| generation.state == GenerationState::Active)
    }
}

/// The numbers the host keeps about itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostStats {
    /// Registrations alive, across every generation.
    pub registrations: u64,
    /// Callbacks, posts and hooks run.
    pub callbacks: u64,
    /// Generations retired.
    pub retired_generations: u64,
    /// Calls refused because the calling generation had retired.
    pub stale_calls: u64,
}

/// Every generation, registration and queued event of one shell host.
///
/// `E` is the event the compositor side publishes. A host is meant to live in a `static`: [`GenerationHost::new`] is `const`.
pub struct GenerationHost<E> {
    state: Mutex<HostTables<E>>,
    wake: Condvar,
    callbacks: AtomicU64,
    retired: AtomicU64,
    stale_calls: AtomicU64,
    observer: Option<&'static dyn LifecycleObserver>,
}

/// Told when a generation changes state, so state that belongs to the shell instance and not to a registration (the programs it
/// started, the input claims it holds) can follow the generation. Called on the thread that changed the state, with no lock of
/// the host held; a panic in it is contained.
pub trait LifecycleObserver: Send + Sync {
    /// A generation was made, staged.
    fn staged(&self, _id: GenerationId) {}
    /// A staged generation became active. `on_shell_thread` is whether the caller is the generation's own thread.
    fn activated(&self, _id: GenerationId, _on_shell_thread: bool) {}
    /// A generation retired, after every registration it owned was removed. `was` is its state before: a generation that was
    /// still staged was abandoned or failed its `init`.
    fn retired(&self, _id: GenerationId, _was: GenerationState, _on_shell_thread: bool) {}
    /// The generation's `init` is about to run, on the shell thread. The loader calls it for a native generation, and
    /// [`init_ended`](LifecycleObserver::init_ended) when `init` has returned or failed, so a host can time it.
    fn init_begun(&self, _id: GenerationId) {}
    /// The generation's `init` returned (or failed or panicked).
    fn init_ended(&self, _id: GenerationId) {}
    /// Whether a staged generation whose `init` returned may take over a reload: `Err` says why not, and the generation that is
    /// serving keeps serving. Asked by `NativeShell::reload` before the old generation ends; the first start has no generation to
    /// keep and does not ask.
    ///
    /// # Errors
    ///
    /// The reason the generation may not take over.
    fn vet(&self, _id: GenerationId) -> Result<(), String> {
        Ok(())
    }
}

impl<E> Default for GenerationHost<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E> GenerationHost<E> {
    /// An empty host.
    pub const fn new() -> Self {
        Self::with_observer_slot(None)
    }

    /// An empty host that tells `observer` when a generation is staged, activated and retired. The observer is part of the
    /// host's identity: it is fixed at construction, so no generation can exist that it did not see.
    pub const fn with_observer(observer: &'static dyn LifecycleObserver) -> Self {
        Self::with_observer_slot(Some(observer))
    }

    /// Tells the observer that `id`'s `init` is about to run. The loader calls it on the shell thread.
    pub fn init_begun(&self, id: GenerationId) {
        if let Some(observer) = self.observer {
            contained((), "lifecycle observer", || observer.init_begun(id));
        }
    }

    /// Tells the observer that `id`'s `init` has returned, failed or panicked.
    pub fn init_ended(&self, id: GenerationId) {
        if let Some(observer) = self.observer {
            contained((), "lifecycle observer", || observer.init_ended(id));
        }
    }

    /// Asks the observer whether the staged generation `id` may take over a reload. A host without an observer, and an observer that
    /// panics, allow it.
    ///
    /// # Errors
    ///
    /// The observer's reason.
    pub fn vet(&self, id: GenerationId) -> Result<(), String> {
        match self.observer {
            Some(observer) => contained(Ok(()), "lifecycle observer", || observer.vet(id)),
            None => Ok(()),
        }
    }

    const fn with_observer_slot(observer: Option<&'static dyn LifecycleObserver>) -> Self {
        Self {
            state: Mutex::new(HostTables {
                next_generation: 1,
                next_registration: 1,
                generations: BTreeMap::new(),
                registrations: BTreeMap::new(),
                events: VecDeque::new(),
                posts: VecDeque::new(),
                in_flight: None,
            }),
            wake: Condvar::new(),
            callbacks: AtomicU64::new(0),
            retired: AtomicU64::new(0),
            stale_calls: AtomicU64::new(0),
            observer,
        }
    }

    /// The state, recovering the lock if a panic poisoned it: the state is consistent between statements.
    fn lock(&self) -> MutexGuard<'_, HostTables<E>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs a release function, if there is one, containing a panic in it.
    fn run_release(release: Option<Release>, what: &str) {
        if let Some(release) = release {
            contained((), what, release);
        }
    }

    // ---- generations ---------------------------------------------------------------------------------------------

    /// Makes a generation for a module that is about to run, staged, owned by the calling thread (which becomes the shell
    /// thread). The generation's number is its sequence number.
    pub fn new_generation(&self, info: GenerationInfo) -> GenerationId {
        let id = {
            let mut state = self.lock();
            let id = state.next_generation;
            state.next_generation += 1;
            state.generations.insert(
                id,
                Generation {
                    state: GenerationState::Staged,
                    thread: thread::current().id(),
                    info: Arc::new(info),
                },
            );
            id
        };
        if let Some(observer) = self.observer {
            contained((), "lifecycle observer", || observer.staged(id));
        }
        id
    }

    /// The state of a generation, `None` for one the host never issued.
    pub fn state_of(&self, id: GenerationId) -> Option<GenerationState> {
        self.lock().generations.get(&id).map(|generation| generation.state)
    }

    /// What a generation says about itself, `None` for one the host never issued. The host keeps its own copy of the
    /// record for as long as it lives and never changes it, so a pointer into the strings stays valid after this
    /// handle is dropped.
    pub fn info(&self, id: GenerationId) -> Option<Arc<GenerationInfo>> {
        self.lock()
            .generations
            .get(&id)
            .map(|generation| Arc::clone(&generation.info))
    }

    /// Staged to active: from now its registrations receive events.
    ///
    /// # Errors
    ///
    /// The generation is unknown or is not staged.
    pub fn activate(&self, id: GenerationId) -> Result<(), String> {
        let on_shell_thread = {
            let mut state = self.lock();
            match state.generations.get_mut(&id) {
                Some(generation) if generation.state == GenerationState::Staged => {
                    generation.state = GenerationState::Active;
                    generation.thread == thread::current().id()
                }
                Some(generation) => return Err(format!("generation {id} is {:?}, not staged", generation.state)),
                None => return Err("no such context".to_owned()),
            }
        };
        if let Some(observer) = self.observer {
            contained((), "lifecycle observer", || observer.activated(id, on_shell_thread));
        }
        Ok(())
    }

    /// Checks that `id` names a generation the caller may use for a call named `what`. A retired generation counts as a
    /// stale call.
    ///
    /// # Errors
    ///
    /// The generation is unknown, retired, or (when `shell_thread`) the caller is not its shell thread.
    pub fn check(&self, id: GenerationId, shell_thread: bool) -> Result<(), CtxError> {
        let state = self.lock();
        let Some(generation) = state.generations.get(&id) else {
            return Err(CtxError::Unknown(id));
        };
        if generation.state == GenerationState::Retired {
            drop(state);
            self.stale_calls.fetch_add(1, Ordering::Relaxed);
            return Err(CtxError::Retired(id));
        }
        if shell_thread && generation.thread != thread::current().id() {
            return Err(CtxError::WrongThread(id));
        }
        Ok(())
    }

    /// How many registrations a generation holds (dead ones, awaiting the end of their callback, are not counted).
    pub fn registrations_of(&self, id: GenerationId) -> usize {
        self.lock()
            .registrations
            .values()
            .filter(|registration| registration.owner == id && !registration.dead)
            .count()
    }

    /// How many posts of a generation are waiting.
    pub fn posts_of(&self, id: GenerationId) -> usize {
        self.lock().posts.iter().filter(|post| post.owner == id).count()
    }

    /// Registrations that still exist for `id`, dead or not: what keeps a retired generation's code reachable.
    pub fn held_by(&self, id: GenerationId) -> usize {
        let state = self.lock();
        let registrations = state
            .registrations
            .values()
            .filter(|registration| registration.owner == id)
            .count();
        let posts = state.posts.iter().filter(|post| post.owner == id).count();
        registrations + posts
    }

    /// Runs every shutdown hook the generation registered, in registration order. A hook runs once.
    pub fn run_shutdown_hooks(&self, id: GenerationId) {
        loop {
            let next = {
                let mut state = self.lock();
                let found = state
                    .registrations
                    .iter()
                    .find(|(_, registration)| {
                        registration.owner == id
                            && !registration.dead
                            && matches!(registration.target, Recipient::Shutdown(Some(_)))
                    })
                    .map(|(key, _)| *key);
                found.and_then(|key| state.registrations.remove(&key))
            };
            let Some(registration) = next else { break };
            if let Recipient::Shutdown(Some(work)) = registration.target {
                contained((), "shutdown hook", work);
                self.callbacks.fetch_add(1, Ordering::Relaxed);
            }
            Self::run_release(registration.release, "release of a shutdown hook");
        }
    }

    /// Retires a generation: removes every registration it owns (running each release), drops its queued posts, and
    /// marks it so every later call from it is refused. Returns how many registrations were removed. Retiring twice does
    /// nothing the second time.
    ///
    /// A registration whose callback is running (retire called from inside it) is marked dead and removed when the
    /// callback returns.
    pub fn finish_retire(&self, id: GenerationId) -> usize {
        let (removed, posts, was, on_shell_thread) = {
            let mut state = self.lock();
            let Some(generation) = state.generations.get_mut(&id) else {
                return 0;
            };
            if generation.state == GenerationState::Retired {
                return 0;
            }
            let was = generation.state;
            let on_shell_thread = generation.thread == thread::current().id();
            generation.state = GenerationState::Retired;
            self.retired.fetch_add(1, Ordering::Relaxed);
            let in_flight = state.in_flight;
            let owned: Vec<RegistrationId> = state
                .registrations
                .iter()
                .filter(|(_, registration)| registration.owner == id)
                .map(|(key, _)| *key)
                .collect();
            let mut removed = Vec::new();
            for key in owned {
                if Some(key) == in_flight {
                    if let Some(registration) = state.registrations.get_mut(&key) {
                        registration.dead = true;
                    }
                } else if let Some(registration) = state.registrations.remove(&key) {
                    removed.push(registration);
                }
            }
            let (mine, rest): (VecDeque<Post>, VecDeque<Post>) = std::mem::take(&mut state.posts)
                .into_iter()
                .partition(|post| post.owner == id);
            state.posts = rest;
            (removed, mine, was, on_shell_thread)
        };
        let count = removed.len();
        for registration in removed {
            Self::run_release(registration.release, "release");
        }
        for post in posts {
            Self::run_release(Some(post.discard), "discard of a posted call");
        }
        if let Some(observer) = self.observer {
            contained((), "lifecycle observer", || observer.retired(id, was, on_shell_thread));
        }
        count
    }

    // ---- registrations -------------------------------------------------------------------------------------------

    /// Registers `listener` for `owner`, adopting `release`.
    ///
    /// # Errors
    ///
    /// The generation is unknown, retired, or the caller is not its shell thread. `release` is not adopted then: it is
    /// returned to the caller unrun inside the error's `Err` pair.
    pub fn register_listener(
        &self,
        owner: GenerationId,
        listener: Listener<E>,
        release: Option<Release>,
    ) -> Result<RegistrationId, (CtxError, Option<Release>)> {
        self.register(owner, Recipient::Listener(listener), release)
    }

    /// Registers a hook that runs once when `owner` retires, adopting `release`.
    ///
    /// # Errors
    ///
    /// As [`register_listener`](Self::register_listener).
    pub fn register_shutdown(
        &self,
        owner: GenerationId,
        hook: Work,
        release: Option<Release>,
    ) -> Result<RegistrationId, (CtxError, Option<Release>)> {
        self.register(owner, Recipient::Shutdown(Some(hook)), release)
    }

    fn register(
        &self,
        owner: GenerationId,
        target: Recipient<E>,
        release: Option<Release>,
    ) -> Result<RegistrationId, (CtxError, Option<Release>)> {
        if let Err(error) = self.check(owner, true) {
            return Err((error, release));
        }
        let mut state = self.lock();
        let id = state.next_registration;
        state.next_registration += 1;
        state.registrations.insert(
            id,
            OwnedRegistration {
                owner,
                target,
                release,
                dead: false,
            },
        );
        Ok(id)
    }

    /// Removes a registration, running its release. Removing the registration whose callback is running defers the release
    /// until the callback returns.
    ///
    /// # Errors
    ///
    /// The registration is gone, or the caller is not the shell thread of its generation.
    pub fn remove(&self, id: RegistrationId) -> Result<(), RemoveError> {
        let removed = {
            let mut state = self.lock();
            let Some(registration) = state.registrations.get(&id) else {
                return Err(RemoveError::Gone);
            };
            let owner_thread = state
                .generations
                .get(&registration.owner)
                .map(|generation| generation.thread);
            if owner_thread != Some(thread::current().id()) {
                return Err(RemoveError::WrongThread);
            }
            if state.in_flight == Some(id) {
                if let Some(registration) = state.registrations.get_mut(&id) {
                    registration.dead = true;
                }
                None
            } else {
                state.registrations.remove(&id)
            }
        };
        if let Some(registration) = removed {
            Self::run_release(registration.release, "release");
        }
        Ok(())
    }

    /// Queues `work` for the shell thread of `owner`; `discard` runs instead if the generation retires first.
    ///
    /// # Errors
    ///
    /// The generation is unknown or retired. Neither function ran or was adopted then: they are returned inside the pair.
    pub fn post(&self, owner: GenerationId, work: Work, discard: Release) -> Result<(), (CtxError, Work, Release)> {
        if let Err(error) = self.check(owner, false) {
            return Err((error, work, discard));
        }
        self.lock().posts.push_back(Post { owner, work, discard });
        self.wake.notify_all();
        Ok(())
    }

    // ---- the event queue and the dispatch loop -------------------------------------------------------------------

    /// Queues an event for the shell, waking its dispatch loop. The compositor side calls this from any thread.
    pub fn publish(&self, event: E) {
        {
            let mut state = self.lock();
            if state.events.len() >= MAILBOX_CAP {
                state.events.pop_front();
                log::warn!("the shell mailbox is full: dropped the oldest event");
            }
            state.events.push_back(event);
        }
        self.wake.notify_all();
    }

    /// How many events are waiting.
    pub fn queued_events(&self) -> usize {
        self.lock().events.len()
    }

    /// Runs the posts of `owner`, then delivers every queued event, waiting up to `timeout_ms` for the first. Returns the
    /// number of callbacks run.
    ///
    /// # Errors
    ///
    /// The generation is unknown, retired, or the caller is not its shell thread.
    pub fn dispatch(&self, owner: GenerationId, timeout_ms: u32) -> Result<usize, CtxError> {
        self.check(owner, true)?;
        let deadline = Instant::now() + Duration::from_millis(u64::from(timeout_ms));
        let (posts, events) = {
            let mut state = self.lock();
            loop {
                let active = state.owner_is_active(owner);
                let has_posts = state.posts.iter().any(|post| post.owner == owner);
                if has_posts || (active && !state.events.is_empty()) {
                    break;
                }
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Ok(0);
                }
                state = self
                    .wake
                    .wait_timeout(state, left)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
            }
            let active = state.owner_is_active(owner);
            let (mine, rest): (VecDeque<Post>, VecDeque<Post>) = std::mem::take(&mut state.posts)
                .into_iter()
                .partition(|post| post.owner == owner);
            state.posts = rest;
            let events: Vec<E> = if active {
                state.events.drain(..).collect()
            } else {
                Vec::new()
            };
            (mine, events)
        };
        let mut called = 0;
        for post in posts {
            // The post was taken off the queue just above, so this is the only run it gets; it consumes whatever it owns.
            contained((), "post", post.work);
            self.callbacks.fetch_add(1, Ordering::Relaxed);
            called += 1;
        }
        for event in &events {
            called += self.deliver(event);
        }
        Ok(called)
    }

    /// Delivers one event to every matching registration of every active generation, in registration order.
    fn deliver(&self, event: &E) -> usize {
        let matching: Vec<RegistrationId> = {
            let state = self.lock();
            state
                .registrations
                .iter()
                .filter(|(_, registration)| {
                    !registration.dead
                        && state.owner_is_active(registration.owner)
                        && matches!(&registration.target, Recipient::Listener(listener) if (listener.matches)(event))
                })
                .map(|(key, _)| *key)
                .collect()
        };
        let mut called = 0;
        for id in matching {
            let call = {
                let mut state = self.lock();
                let call = match state.registrations.get(&id) {
                    Some(registration) if !registration.dead && state.owner_is_active(registration.owner) => {
                        match &registration.target {
                            Recipient::Listener(listener) => Arc::clone(&listener.call),
                            Recipient::Shutdown(_) => continue,
                        }
                    }
                    _ => continue,
                };
                state.in_flight = Some(id);
                call
            };
            let delivery = contained(CallbackOutcome::Panicked, "callback", || call(event));
            self.callbacks.fetch_add(1, Ordering::Relaxed);
            called += 1;
            if delivery == CallbackOutcome::Panicked {
                PANICS.fetch_add(1, Ordering::Relaxed);
                log::error!("callback of registration {id} panicked; contained, the next event is still delivered");
            }
            let finished = {
                let mut state = self.lock();
                state.in_flight = None;
                match state.registrations.get(&id) {
                    Some(registration) if registration.dead => state.registrations.remove(&id),
                    _ => None,
                }
            };
            if let Some(registration) = finished {
                Self::run_release(registration.release, "release");
            }
        }
        called
    }

    // ---- numbers -------------------------------------------------------------------------------------------------

    /// The host's counters.
    pub fn stats(&self) -> HostStats {
        HostStats {
            registrations: self.lock().registrations.values().filter(|r| !r.dead).count() as u64,
            callbacks: self.callbacks.load(Ordering::Relaxed),
            retired_generations: self.retired.load(Ordering::Relaxed),
            stale_calls: self.stale_calls.load(Ordering::Relaxed),
        }
    }

    /// Every registration that exists, dead or not.
    pub fn live_registrations(&self) -> usize {
        self.lock().registrations.len()
    }
}
