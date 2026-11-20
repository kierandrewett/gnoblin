//! What a shell's shader costs, measured without ever waiting for the GPU.
//!
//! The watchdog needs to know how long a shader pass takes, and the GPU runs behind the CPU, so a draw call's own time says
//! nothing. Waiting for the GPU after each draw (`glFinish`) would answer, and would also stall the compositor on every frame
//! a shader is on screen. Nothing here waits: a measurement is started when the draw is issued and read on a later poll, and
//! a measurement that is not ready yet is left for the next one.
//!
//! There are two ways to measure, chosen once from what the context offers:
//!
//! - **Timer queries** (`GL_EXT_disjoint_timer_query`): the GPU times the draw itself. Every draw is timed. A query's result
//!   is read when the driver says it is available, a frame or two later, and every result read since the last check is
//!   thrown away when the driver reports the timing as disjoint (a clock change or a power event makes them meaningless).
//!   The renderer is always a GLES one, so the desktop `ARB_timer_query` is not reachable here.
//! - **Fences**, for a context without them: a fence is placed after the draw and polled (`glGetSynciv`, which does not
//!   block) at the end of the frame and at the start of the next. Only a sample is measured: the first
//!   [`SAMPLE_FIRST`] draws of a source, which is when a bad one shows itself, and then one in [`SAMPLE_EVERY`]. A poll sees
//!   the fence signalled or not, so what is learned is a bound: the cost is at least the age of the last poll that found the
//!   fence unsignalled, and at most the age of the first that found it signalled. The lower bound is what is reported when
//!   there is one, because it is never wrong in the direction that matters: a shader that is idle between frames is not
//!   charged for the wait for the next frame, and one that takes 300 ms is still seen to take about that, to the length
//!   of a frame.
//!
//! The logic is generic over [`Gpu`], the few calls it makes, so a test drives it with a fake whose answers it controls.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gnoblin_render::ShaderKey;

/// How many of a new source's first draws are measured with fences.
pub(crate) const SAMPLE_FIRST: u64 = 3;

/// After the first draws, one draw in this many is measured with a fence.
pub(crate) const SAMPLE_EVERY: u64 = 30;

/// The most measurements waiting for a result. When a shell keeps a driver from answering, the oldest are dropped.
const MAX_PENDING: usize = 32;

/// How long a measurement is kept before it is given up on: a query that never answers is dropped without a result, and a
/// fence that is still unsignalled is reported with its lower bound, which is then at least this.
const EXPIRE: Duration = Duration::from_secs(2);

/// How a context measures a draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TimerMode {
    /// The GPU times each draw with a timer query.
    Queries,
    /// A sample of the draws is bracketed by a fence that is polled.
    Fences,
}

/// What a query's result is, when asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum QueryState {
    /// The driver has not finished the query.
    Pending,
    /// The GPU time of the draw.
    Done(Duration),
}

/// The GPU calls a measurement makes.
pub(crate) trait Gpu {
    type Query: Copy;
    type Fence: Copy;
    /// Starts a timer query around the draw that follows, or `None` when one cannot be made.
    fn begin_query(&mut self) -> Option<Self::Query>;
    /// Ends the query [`Self::begin_query`] started.
    fn end_query(&mut self);
    /// The query's result when it is available, without waiting.
    fn query_state(&mut self, query: Self::Query) -> QueryState;
    /// Whether the driver timed anything disjointly since it was last asked. Asking clears it.
    fn take_disjoint(&mut self) -> bool;
    fn delete_query(&mut self, query: Self::Query);
    /// A fence after the commands issued so far, or `None` when one cannot be made.
    fn fence(&mut self) -> Option<Self::Fence>;
    /// Whether the fence has signalled, without waiting.
    fn fence_signalled(&mut self, fence: Self::Fence) -> bool;
    fn delete_fence(&mut self, fence: Self::Fence);
}

/// What was started for one draw, to be handed to [`ShaderClock::end`] once the draw is issued.
#[derive(Debug)]
pub(crate) enum Begun<Q> {
    /// Nothing is measured for this draw.
    No,
    Query(Q),
    Fence,
}

enum InFlight<G: Gpu> {
    Query {
        key: ShaderKey,
        frame: u64,
        query: G::Query,
        issued: Instant,
    },
    Fence {
        key: ShaderKey,
        frame: u64,
        fence: G::Fence,
        issued: Instant,
        /// The age at the last poll that found the fence unsignalled, which is a lower bound on the draw's cost.
        waited: Option<Duration>,
    },
}

/// The measurements in flight and the counts that choose which draws are sampled.
pub(crate) struct ShaderClock<G: Gpu> {
    mode: TimerMode,
    pending: Vec<InFlight<G>>,
    /// The frame being painted, counted by [`Self::next_frame`], so the draws of one frame are told from another's.
    frame: u64,
    /// How many measurements were started, for a test to tell a schedule from what the driver answered.
    #[cfg(test)]
    issued: u64,
    /// Draws seen per source, for the fence sample. Only sources the cache holds have an entry: [`Self::forget`].
    draws: HashMap<ShaderKey, u64>,
}

impl<G: Gpu> ShaderClock<G> {
    pub(crate) fn new(mode: TimerMode) -> Self {
        Self {
            mode,
            pending: Vec::new(),
            frame: 0,
            #[cfg(test)]
            issued: 0,
            draws: HashMap::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn mode(&self) -> TimerMode {
        self.mode
    }

    /// Starts a frame: the draws measured from here on are one frame's.
    pub(crate) fn next_frame(&mut self) {
        self.frame += 1;
    }

    /// How many measurements were started.
    #[cfg(test)]
    pub(crate) fn issued(&self) -> u64 {
        self.issued
    }

    /// Measures from now on in `mode`. The measurements in flight are dropped.
    #[cfg(test)]
    pub(crate) fn set_mode(&mut self, gpu: &mut G, mode: TimerMode) {
        self.clear(gpu);
        self.mode = mode;
    }

    /// Decides whether the draw of `key` that is about to be issued is measured, and starts the measurement.
    pub(crate) fn begin(&mut self, gpu: &mut G, key: ShaderKey) -> Begun<G::Query> {
        match self.mode {
            TimerMode::Queries => gpu.begin_query().map_or(Begun::No, Begun::Query),
            TimerMode::Fences => {
                let draws = self.draws.entry(key).or_insert(0);
                *draws += 1;
                if *draws <= SAMPLE_FIRST || draws.is_multiple_of(SAMPLE_EVERY) {
                    Begun::Fence
                } else {
                    Begun::No
                }
            }
        }
    }

    /// Finishes the start made by [`Self::begin`], after the draw is issued.
    pub(crate) fn end(&mut self, gpu: &mut G, started: Begun<G::Query>, key: ShaderKey, now: Instant) {
        let entry = match started {
            Begun::No => return,
            Begun::Query(query) => {
                gpu.end_query();
                InFlight::Query {
                    key,
                    frame: self.frame,
                    query,
                    issued: now,
                }
            }
            Begun::Fence => match gpu.fence() {
                Some(fence) => InFlight::Fence {
                    key,
                    frame: self.frame,
                    fence,
                    issued: now,
                    waited: None,
                },
                None => return,
            },
        };
        #[cfg(test)]
        {
            self.issued += 1;
        }
        self.pending.push(entry);
        if self.pending.len() > MAX_PENDING {
            let oldest = self.pending.remove(0);
            Self::delete(gpu, oldest);
        }
    }

    /// Looks at every measurement in flight, without waiting, and returns what finished: for each source, what its draws
    /// cost in the worst frame that finished. The draws of one source in one frame add up (a node with the source twice
    /// pays for both); frames do not add up, because the watchdog counts frames over its budget.
    pub(crate) fn poll(&mut self, gpu: &mut G, now: Instant) -> Vec<(ShaderKey, Duration)> {
        let mut done: Vec<(ShaderKey, u64, Duration)> = Vec::new();
        let mut read_queries: Vec<(ShaderKey, u64, Duration)> = Vec::new();
        for entry in std::mem::take(&mut self.pending) {
            match entry {
                InFlight::Query {
                    key,
                    frame,
                    query,
                    issued,
                } => match gpu.query_state(query) {
                    QueryState::Done(elapsed) => {
                        gpu.delete_query(query);
                        read_queries.push((key, frame, elapsed));
                    }
                    QueryState::Pending if now.saturating_duration_since(issued) < EXPIRE => {
                        self.pending.push(InFlight::Query {
                            key,
                            frame,
                            query,
                            issued,
                        });
                    }
                    QueryState::Pending => gpu.delete_query(query),
                },
                InFlight::Fence {
                    key,
                    frame,
                    fence,
                    issued,
                    waited,
                } => {
                    let age = now.saturating_duration_since(issued);
                    if gpu.fence_signalled(fence) {
                        gpu.delete_fence(fence);
                        done.push((key, frame, waited.unwrap_or(age)));
                    } else if age >= EXPIRE {
                        gpu.delete_fence(fence);
                        done.push((key, frame, age));
                    } else {
                        self.pending.push(InFlight::Fence {
                            key,
                            frame,
                            fence,
                            issued,
                            waited: Some(age),
                        });
                    }
                }
            }
        }
        // Reading the flag clears it, so it is read once for everything read in this poll.
        if !read_queries.is_empty() && !gpu.take_disjoint() {
            done.extend(read_queries);
        }
        worst_frames(&done)
    }

    /// Drops the count of a source the cache no longer holds.
    pub(crate) fn forget(&mut self, key: ShaderKey) {
        self.draws.remove(&key);
    }

    /// Drops every measurement and count: the programs they belong to are gone, and after a lost context so are the queries.
    pub(crate) fn clear(&mut self, gpu: &mut G) {
        for entry in std::mem::take(&mut self.pending) {
            Self::delete(gpu, entry);
        }
        self.draws.clear();
    }

    /// How many measurements are waiting for a result.
    #[cfg(test)]
    pub(crate) fn pending(&self) -> usize {
        self.pending.len()
    }

    fn delete(gpu: &mut G, entry: InFlight<G>) {
        match entry {
            InFlight::Query { query, .. } => gpu.delete_query(query),
            InFlight::Fence { fence, .. } => gpu.delete_fence(fence),
        }
    }
}

/// For each source, the largest total over a frame, with the sources in the order they first appear.
fn worst_frames(done: &[(ShaderKey, u64, Duration)]) -> Vec<(ShaderKey, Duration)> {
    let mut frames: Vec<(ShaderKey, u64, Duration)> = Vec::new();
    for (key, frame, elapsed) in done {
        match frames
            .iter_mut()
            .find(|(held, held_frame, _)| held == key && held_frame == frame)
        {
            Some((_, _, total)) => *total = total.saturating_add(*elapsed),
            None => frames.push((*key, *frame, *elapsed)),
        }
    }
    let mut worst: Vec<(ShaderKey, Duration)> = Vec::new();
    for (key, _, total) in frames {
        match worst.iter_mut().find(|(held, _)| *held == key) {
            Some((_, most)) => *most = (*most).max(total),
            None => worst.push((key, total)),
        }
    }
    worst
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A GPU whose answers a test sets.
    #[derive(Default)]
    struct Fake {
        next: u32,
        /// Queries and fences the fake has made and not been told to delete.
        live: Vec<u32>,
        /// What each query answers.
        answers: HashMap<u32, QueryState>,
        /// Which fences have signalled.
        signalled: Vec<u32>,
        disjoint: bool,
        queries_supported: bool,
        /// Calls made, in order.
        calls: Vec<&'static str>,
    }

    impl Gpu for Fake {
        type Query = u32;
        type Fence = u32;
        fn begin_query(&mut self) -> Option<u32> {
            self.calls.push("begin_query");
            if !self.queries_supported {
                return None;
            }
            self.next += 1;
            self.live.push(self.next);
            Some(self.next)
        }
        fn end_query(&mut self) {
            self.calls.push("end_query");
        }
        fn query_state(&mut self, query: u32) -> QueryState {
            self.calls.push("query_state");
            self.answers.get(&query).copied().unwrap_or(QueryState::Pending)
        }
        fn take_disjoint(&mut self) -> bool {
            self.calls.push("take_disjoint");
            std::mem::take(&mut self.disjoint)
        }
        fn delete_query(&mut self, query: u32) {
            self.live.retain(|held| *held != query);
        }
        fn fence(&mut self) -> Option<u32> {
            self.calls.push("fence");
            self.next += 1;
            self.live.push(self.next);
            Some(self.next)
        }
        fn fence_signalled(&mut self, fence: u32) -> bool {
            self.calls.push("fence_signalled");
            self.signalled.contains(&fence)
        }
        fn delete_fence(&mut self, fence: u32) {
            self.live.retain(|held| *held != fence);
        }
    }

    fn key(n: u32) -> ShaderKey {
        ShaderKey::of(&format!("fn effect{n}() {{}}"))
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// One draw in a frame of its own.
    fn draw(clock: &mut ShaderClock<Fake>, gpu: &mut Fake, key: ShaderKey, now: Instant) {
        clock.next_frame();
        draw_in_frame(clock, gpu, key, now);
    }

    /// One more draw in the frame already started.
    fn draw_in_frame(clock: &mut ShaderClock<Fake>, gpu: &mut Fake, key: ShaderKey, now: Instant) {
        let started = clock.begin(gpu, key);
        clock.end(gpu, started, key, now);
    }

    #[test]
    fn with_timer_queries_every_draw_is_timed_and_a_result_arrives_when_the_driver_has_it() {
        let mut gpu = Fake {
            queries_supported: true,
            ..Fake::default()
        };
        let mut clock = ShaderClock::new(TimerMode::Queries);
        let t0 = Instant::now();
        for _ in 0..4 {
            draw(&mut clock, &mut gpu, key(1), t0);
        }
        assert_eq!(clock.pending(), 4, "every draw is timed, not a sample");
        assert!(
            clock.poll(&mut gpu, t0).is_empty(),
            "nothing is ready, and the poll did not wait for it"
        );
        gpu.answers.insert(1, QueryState::Done(ms(7)));
        gpu.answers.insert(3, QueryState::Done(ms(9)));
        assert_eq!(
            clock.poll(&mut gpu, t0),
            vec![(key(1), ms(9))],
            "two frames' results come back as the worse of them"
        );
        assert_eq!(clock.pending(), 2, "the two that were not ready are kept");
        assert_eq!(gpu.live.len(), 2, "and the finished queries were deleted");
    }

    #[test]
    fn a_disjoint_timing_discards_the_results_read_with_it() {
        let mut gpu = Fake {
            queries_supported: true,
            ..Fake::default()
        };
        let mut clock = ShaderClock::new(TimerMode::Queries);
        let t0 = Instant::now();
        draw(&mut clock, &mut gpu, key(1), t0);
        gpu.answers.insert(1, QueryState::Done(ms(900)));
        gpu.disjoint = true;
        assert!(
            clock.poll(&mut gpu, t0).is_empty(),
            "a timing the driver calls disjoint is not a cost"
        );
        assert!(gpu.live.is_empty(), "and the query is gone");
        draw(&mut clock, &mut gpu, key(1), t0);
        gpu.answers.insert(2, QueryState::Done(ms(4)));
        assert_eq!(
            clock.poll(&mut gpu, t0),
            vec![(key(1), ms(4))],
            "the next one is read as usual"
        );
    }

    #[test]
    fn a_query_that_never_answers_is_dropped_without_a_result() {
        let mut gpu = Fake {
            queries_supported: true,
            ..Fake::default()
        };
        let mut clock = ShaderClock::new(TimerMode::Queries);
        let t0 = Instant::now();
        draw(&mut clock, &mut gpu, key(1), t0);
        assert!(clock.poll(&mut gpu, t0 + EXPIRE + ms(1)).is_empty());
        assert_eq!(clock.pending(), 0);
        assert!(gpu.live.is_empty(), "the query was deleted, not leaked");
    }

    #[test]
    fn a_context_that_cannot_make_a_query_measures_nothing_and_does_not_fail() {
        let mut gpu = Fake::default();
        let mut clock = ShaderClock::new(TimerMode::Queries);
        draw(&mut clock, &mut gpu, key(1), Instant::now());
        assert_eq!(clock.pending(), 0);
    }

    #[test]
    fn without_timer_queries_the_first_draws_and_then_one_in_n_are_fenced() {
        let mut gpu = Fake::default();
        let mut clock = ShaderClock::new(TimerMode::Fences);
        let t0 = Instant::now();
        let mut fenced = Vec::new();
        for frame in 1..=100u64 {
            let before = gpu.calls.iter().filter(|call| **call == "fence").count();
            draw(&mut clock, &mut gpu, key(1), t0);
            if gpu.calls.iter().filter(|call| **call == "fence").count() > before {
                fenced.push(frame);
            }
        }
        assert_eq!(
            fenced,
            vec![1, 2, 3, 30, 60, 90],
            "the first {SAMPLE_FIRST}, then every {SAMPLE_EVERY}th"
        );
        assert!(
            !gpu.calls.contains(&"begin_query"),
            "a context without timer queries never starts one"
        );
        let mut other = ShaderClock::<Fake>::new(TimerMode::Fences);
        let before = gpu.calls.iter().filter(|call| **call == "fence").count();
        draw(&mut other, &mut gpu, key(2), t0);
        assert_eq!(
            gpu.calls.iter().filter(|call| **call == "fence").count(),
            before + 1,
            "a new source is sampled from its first draw, whatever else was drawn"
        );
    }

    #[test]
    fn a_fence_found_signalled_at_the_first_poll_reports_its_age() {
        let mut gpu = Fake::default();
        let mut clock = ShaderClock::new(TimerMode::Fences);
        let t0 = Instant::now();
        draw(&mut clock, &mut gpu, key(1), t0);
        gpu.signalled.push(1);
        assert_eq!(clock.poll(&mut gpu, t0 + ms(3)), vec![(key(1), ms(3))]);
        assert!(gpu.live.is_empty());
    }

    #[test]
    fn a_fence_that_is_late_reports_the_last_age_it_was_seen_unsignalled_not_the_wait_for_the_next_frame() {
        let mut gpu = Fake::default();
        let mut clock = ShaderClock::new(TimerMode::Fences);
        let t0 = Instant::now();
        draw(&mut clock, &mut gpu, key(1), t0);
        assert!(
            clock.poll(&mut gpu, t0 + ms(2)).is_empty(),
            "not signalled at the end of the frame"
        );
        assert!(clock.poll(&mut gpu, t0 + ms(18)).is_empty(), "nor a frame later");
        gpu.signalled.push(1);
        // The next frame came a second later (nothing was animating): the fence signalled some time in that gap, and the
        // shader is not charged for the gap.
        assert_eq!(clock.poll(&mut gpu, t0 + ms(1018)), vec![(key(1), ms(18))]);
    }

    #[test]
    fn a_runaway_fence_is_seen_to_take_as_long_as_the_polls_found_it_running() {
        let mut gpu = Fake::default();
        let mut clock = ShaderClock::new(TimerMode::Fences);
        let t0 = Instant::now();
        draw(&mut clock, &mut gpu, key(1), t0);
        for step in 1..=19u64 {
            assert!(clock.poll(&mut gpu, t0 + ms(16 * step)).is_empty());
        }
        gpu.signalled.push(1);
        let done = clock.poll(&mut gpu, t0 + ms(16 * 20));
        assert_eq!(
            done,
            vec![(key(1), ms(16 * 19))],
            "about 300 ms, to the length of a frame"
        );
        assert!(
            done[0].1 > Duration::from_millis(50),
            "which is over the watchdog's budget"
        );
    }

    #[test]
    fn a_fence_that_never_signals_is_reported_and_deleted_when_it_expires() {
        let mut gpu = Fake::default();
        let mut clock = ShaderClock::new(TimerMode::Fences);
        let t0 = Instant::now();
        draw(&mut clock, &mut gpu, key(1), t0);
        assert_eq!(clock.poll(&mut gpu, t0 + EXPIRE), vec![(key(1), EXPIRE)]);
        assert!(gpu.live.is_empty());
        assert_eq!(clock.pending(), 0);
    }

    #[test]
    fn the_measurements_in_flight_are_bounded_and_clear_gives_every_one_back() {
        let mut gpu = Fake::default();
        let mut clock = ShaderClock::new(TimerMode::Fences);
        let t0 = Instant::now();
        for n in 0..100 {
            draw(&mut clock, &mut gpu, key(n), t0);
        }
        assert_eq!(clock.pending(), MAX_PENDING);
        assert_eq!(gpu.live.len(), MAX_PENDING, "what was dropped was deleted");
        clock.clear(&mut gpu);
        assert_eq!(clock.pending(), 0);
        assert!(gpu.live.is_empty());
        assert!(clock.draws.is_empty(), "and the counts are gone with them");
    }

    #[test]
    fn a_forgotten_source_is_sampled_from_its_first_draw_again() {
        let mut gpu = Fake::default();
        let mut clock = ShaderClock::new(TimerMode::Fences);
        let t0 = Instant::now();
        for _ in 0..10 {
            draw(&mut clock, &mut gpu, key(1), t0);
        }
        assert_eq!(clock.pending(), 3);
        clock.forget(key(1));
        draw(&mut clock, &mut gpu, key(1), t0);
        assert_eq!(clock.pending(), 4);
    }

    #[test]
    fn the_draws_of_one_frame_add_up_and_the_frames_of_one_poll_do_not() {
        let mut gpu = Fake {
            queries_supported: true,
            ..Fake::default()
        };
        let mut clock = ShaderClock::new(TimerMode::Queries);
        let t0 = Instant::now();
        // Frame 1 draws the source twice (6 ms and 5 ms); frame 2 once (9 ms); another source once (2 ms).
        clock.next_frame();
        draw_in_frame(&mut clock, &mut gpu, key(1), t0);
        draw_in_frame(&mut clock, &mut gpu, key(1), t0);
        clock.next_frame();
        draw_in_frame(&mut clock, &mut gpu, key(1), t0);
        draw_in_frame(&mut clock, &mut gpu, key(2), t0);
        for (query, cost) in [(1, 6), (2, 5), (3, 9), (4, 2)] {
            gpu.answers.insert(query, QueryState::Done(ms(cost)));
        }
        assert_eq!(
            clock.poll(&mut gpu, t0),
            vec![(key(1), ms(11)), (key(2), ms(2))],
            "11 ms is the frame with two draws, which is worse than the frame with one"
        );
    }
}
