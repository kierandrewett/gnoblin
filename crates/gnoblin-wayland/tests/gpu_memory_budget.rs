//! The memory budget against the real compositor and real `wayland-client` connections
//! (tests/gpu-memory-soak/DESIGN.md, section 6): what a client may hold, what the session may hold, and what happens at
//! the bound.
//!
//! Every test starts the session with bounds it chose, so the numbers are small and the tests are fast; the default
//! bounds are asserted where a test is about them. The ledger the session keeps is read from outside while the loop
//! runs (`Handle::ledger`), which is how a test sees a client's account without trusting what the client says.
//!
//! What is proven, in the order of the tests:
//!
//! - **A client past its own limit is ended and nobody else is** (`a_client_past_its_buffer_limit_...`): the protocol
//!   error is `wl_display.error.no_memory`, the session and a second client go on, and the refusal is counted.
//! - **The aggregate refuses the client that does not fit, and a leaver frees the room**
//!   (`clients_each_under_their_limit_...`).
//! - **A surface count is bounded per client**, at the default (`a_client_with_thousands_of_surfaces_...`) and at
//!   a limit a test sets exactly (`the_surface_limit_is_exact`).
//! - **Caches are given back before a client is refused** (`caches_are_given_back_...`).
//! - **A client that goes away mid-request releases everything** (`a_client_that_disconnects_...`), and after random
//!   churn the clients' accounts sum to the aggregate (`after_churn_the_accounts_sum_to_the_aggregate`).
//!
//! The lock is in `tests/session_lock.rs` (`budget_pressure_does_not_blank_a_lock_surface`), the eviction order and
//! the lock's View pictures in the unit tests of `src/gpu_memory.rs`, and the idle cost of the published numbers in
//! the `gnoblin` crate's `a_stock_shell_at_the_production_memory_period_does_no_work_when_nothing_happens`.

use gnoblin_wayland_testkit::session;

use std::thread;
use std::time::{Duration, Instant};

use gnoblin_wayland::{LedgerSnapshot, MemoryLimits};
use session::{Client, Handle, Session, TIMEOUT};

/// `wl_display.error.no_memory`.
const NO_MEMORY: u32 = 2;

const MIB: u64 = 1 << 20;

/// The most a test of per-client bounds asks of a bound it does not test: off.
const OFF: u64 = 0;

/// Waits up to ten seconds for the ledger to satisfy `done`, and returns what it held.
fn wait_ledger(handle: &Handle, what: &str, done: impl Fn(&LedgerSnapshot) -> bool) -> LedgerSnapshot {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let snapshot = handle.ledger.snapshot();
        if done(&snapshot) {
            return snapshot;
        }
        assert!(Instant::now() < deadline, "{what}; the ledger held {snapshot:#?}");
        thread::sleep(Duration::from_millis(5));
    }
}

/// Whether the ledger holds nothing for any client: what a session with every client gone must say.
fn is_empty(snapshot: &LedgerSnapshot) -> bool {
    snapshot.clients.is_empty() && snapshot.client_bytes == 0
}

/// A session that bounds a client's buffer bytes and nothing else.
fn limited(client_bytes: u64) -> Session {
    Session::start_limited(MemoryLimits {
        budget_bytes: OFF,
        client_bytes,
        client_surfaces: 0,
    })
}

/// The error a client is ended with: the object that carried it and the code.
fn ended_with(client: &mut Client) -> (String, u32) {
    client.protocol_error()
}

#[test]
fn a_client_past_its_buffer_limit_is_ended_with_no_memory_and_the_session_goes_on() {
    let session = limited(8 * MIB);
    session.run(|handle| {
        let mut hostile = Client::connect(handle.socket());
        let mut bystander = Client::connect(handle.socket());
        let _window = bystander.map_window("org.example.bystander", "bystander");
        // Eight buffers of 1 MiB are exactly the limit: all accepted, none refused.
        let held: Vec<_> = (0..8).map(|_| hostile.buffer(512, 512)).collect();
        hostile.roundtrip();
        let snapshot = wait_ledger(handle, "the eight buffers were never counted", |snapshot| {
            snapshot.clients.iter().any(|client| client.bytes == 8 * MIB)
        });
        assert_eq!(snapshot.refused, 0, "a client at its limit is not past it");

        // The ninth passes it: the client is ended with the error the protocol names, and its objects go with it.
        let _ninth = hostile.buffer(512, 512);
        assert_eq!(ended_with(&mut hostile), ("wl_display".to_owned(), NO_MEMORY));
        drop(held);
        let snapshot = wait_ledger(handle, "the ended client's buffers were never given back", |snapshot| {
            snapshot.clients.iter().all(|client| client.bytes < 8 * MIB)
        });
        assert_eq!(snapshot.refused, 1, "the refusal is counted once");

        // Nothing else was touched: the other client is served, can map more and its window is still managed.
        bystander.roundtrip();
        let _second = bystander.map_window("org.example.bystander2", "bystander two");
        bystander.roundtrip();
        assert_eq!(
            handle.windows().len(),
            2,
            "the session keeps both windows of the other client"
        );
    });
}

#[test]
fn a_destroyed_buffer_gives_its_bytes_back_to_the_client() {
    let session = limited(8 * MIB);
    session.run(|handle| {
        let mut client = Client::connect(handle.socket());
        // A client that creates and destroys buffers for ever, as a toolkit does every frame, stays far under a
        // limit that only its live buffers count against.
        for _ in 0..64 {
            let buffer = client.buffer(512, 512);
            client.roundtrip();
            buffer.destroy();
            client.roundtrip();
        }
        let snapshot = wait_ledger(handle, "the destroyed buffers were never given back", |snapshot| {
            snapshot.clients.iter().all(|client| client.bytes == 0)
        });
        assert_eq!(snapshot.refused, 0);
        assert_eq!(snapshot.client_bytes, 0);
    });
}

#[test]
fn clients_each_under_their_limit_are_refused_at_the_aggregate_and_a_leaver_frees_the_room() {
    // 24 MiB each is under the client limit of 32, two of them with the output's own 3 MiB target and its 3 MiB
    // capture frame fit in 64, and a third does not fit even with the capture frame given back.
    let session = Session::start_limited(MemoryLimits {
        budget_bytes: 64 * MIB,
        client_bytes: 32 * MIB,
        client_surfaces: 0,
    });
    session.run(|handle| {
        let big = |client: &Client| client.buffer(3072, 2048);
        let mut first = Client::connect(handle.socket());
        let mut second = Client::connect(handle.socket());
        let _a = big(&first);
        first.roundtrip();
        let _b = big(&second);
        second.roundtrip();
        let snapshot = wait_ledger(handle, "two clients of 24 MiB were never counted", |snapshot| {
            snapshot.client_bytes == 48 * MIB
        });
        assert_eq!(snapshot.refused, 0, "two fit");

        let mut third = Client::connect(handle.socket());
        let _c = big(&third);
        assert_eq!(ended_with(&mut third), ("wl_display".to_owned(), NO_MEMORY));
        let snapshot = wait_ledger(handle, "the refusal was never counted", |snapshot| {
            snapshot.refused == 1
        });
        assert_eq!(snapshot.client_bytes, 48 * MIB, "the refused client holds nothing");

        // The first two are served: the aggregate ended one connection, not the session.
        first.roundtrip();
        second.roundtrip();

        // One of them leaves; its room goes back to the budget and a new client of the same size is accepted.
        drop(first);
        wait_ledger(handle, "the leaver's buffer was never given back", |snapshot| {
            snapshot.client_bytes == 24 * MIB
        });
        let mut fourth = Client::connect(handle.socket());
        let _d = big(&fourth);
        fourth.roundtrip();
        let snapshot = wait_ledger(handle, "the new client was never counted", |snapshot| {
            snapshot.client_bytes == 48 * MIB
        });
        assert_eq!(snapshot.refused, 1, "the new client was not refused");
    });
}

#[test]
fn a_client_with_thousands_of_surfaces_is_ended_at_the_default_surface_limit() {
    // The session's default bounds: no limits are set by the test.
    let session = Session::start();
    session.run(|handle| {
        let limit = handle.ledger.limits().client_surfaces as usize;
        assert!(
            limit > 256,
            "the per-client limit is above the session's window cap, so both are real: {limit}"
        );
        let mut hostile = Client::connect(handle.socket());
        // Surfaces, not toplevels: nothing here is managed, so the session's window cap of 256 cannot be what stops
        // the client.
        let surfaces: Vec<_> = (0..limit + 1000)
            .map(|_| hostile.compositor.create_surface(&hostile.qh, ()))
            .collect();
        assert_eq!(ended_with(&mut hostile), ("wl_display".to_owned(), NO_MEMORY));
        let snapshot = handle.ledger.snapshot();
        assert_eq!(snapshot.refused, 1);
        drop(surfaces);
        drop(hostile);
        wait_ledger(handle, "the ended client's surfaces were never given back", is_empty);

        // Another client is served and its surfaces count from zero.
        let mut other = Client::connect(handle.socket());
        let _window = other.map_window("org.example.other", "other");
        other.roundtrip();
        assert_eq!(handle.windows().len(), 1);
    });
}

#[test]
fn the_surface_limit_is_exact() {
    let session = Session::start_limited(MemoryLimits {
        budget_bytes: OFF,
        client_bytes: OFF,
        client_surfaces: 50,
    });
    session.run(|handle| {
        let mut client = Client::connect(handle.socket());
        let mut kept: Vec<_> = (0..50)
            .map(|_| client.compositor.create_surface(&client.qh, ()))
            .collect();
        client.roundtrip();
        let snapshot = wait_ledger(handle, "fifty surfaces were never counted", |snapshot| {
            snapshot.clients.iter().any(|account| account.surfaces == 50)
        });
        assert_eq!(snapshot.refused, 0, "fifty is the limit, not past it");

        // A destroyed surface is room again: destroy one and make one.
        kept.pop().expect("a surface").destroy();
        client.roundtrip();
        kept.push(client.compositor.create_surface(&client.qh, ()));
        client.roundtrip();
        assert_eq!(
            handle.ledger.snapshot().refused,
            0,
            "a destroyed surface is room for another"
        );

        let _past = client.compositor.create_surface(&client.qh, ());
        assert_eq!(ended_with(&mut client), ("wl_display".to_owned(), NO_MEMORY));
    });
}

#[test]
fn caches_are_given_back_before_a_client_is_refused() {
    // The output is 1024 x 768: its target is 3 MiB and the frame capture keeps is 3 MiB. Two clients of 16 MiB are
    // 38 MiB with them, which is 2 MiB past the budget, and the capture frame (which nothing reads) covers it.
    // A third client is past the budget by 13 MiB and nothing is left to give.
    let session = Session::start_limited(MemoryLimits {
        budget_bytes: 36 * MIB,
        client_bytes: OFF,
        client_surfaces: 0,
    });
    session.run(|handle| {
        let base = wait_ledger(handle, "the loop never measured a drawn frame", |snapshot| {
            snapshot.capture_bytes > 0 && snapshot.renderer_bytes > 0
        });
        assert_eq!(base.evicted, 0, "a quiet session gives nothing back");
        assert_eq!(base.refused, 0);

        let mut first = Client::connect(handle.socket());
        let mut second = Client::connect(handle.socket());
        let _a = first.buffer(2048, 2048);
        first.roundtrip();
        let _b = second.buffer(2048, 2048);
        second.roundtrip();

        let after = wait_ledger(handle, "the capture frame was never given back", |snapshot| {
            snapshot.evicted > 0 && snapshot.capture_bytes == 0
        });
        assert_eq!(after.refused, 0, "the second client fit once the cache was given back");
        assert_eq!(after.client_bytes, 32 * MIB);
        assert!(after.used() <= 36 * MIB, "the session is within its budget: {after:#?}");
        assert_eq!(
            after.evicted, base.capture_bytes,
            "what was given back is the capture frame and nothing else"
        );

        // Nothing is left to evict, so the third is refused, and the first two go on.
        let mut third = Client::connect(handle.socket());
        let _c = third.buffer(2048, 2048);
        assert_eq!(ended_with(&mut third), ("wl_display".to_owned(), NO_MEMORY));
        first.roundtrip();
        second.roundtrip();
        let end = wait_ledger(handle, "the refusal was never counted", |snapshot| {
            snapshot.refused == 1
        });
        assert_eq!(end.client_bytes, 32 * MIB);
    });
}

#[test]
fn a_client_that_disconnects_under_pressure_gives_everything_back() {
    let session = Session::start_limited(MemoryLimits {
        budget_bytes: 64 * MIB,
        client_bytes: 32 * MIB,
        client_surfaces: 200,
    });
    session.run(|handle| {
        let mut steady = Client::connect(handle.socket());
        let _held = steady.buffer(1024, 1024);
        steady.roundtrip();
        let baseline = wait_ledger(handle, "the steady client was never counted", |snapshot| {
            snapshot.client_bytes == 4 * MIB
        });

        // A client that is part-way through making objects when its connection goes: some of its requests were read,
        // some are still in the socket. Whatever the server saw, nothing may stay counted afterwards.
        for round in 0..5 {
            let leaver = Client::connect(handle.socket());
            let _buffers: Vec<_> = (0..4).map(|_| leaver.buffer(1024, 1024)).collect();
            let _surfaces: Vec<_> = (0..100 + round)
                .map(|_| leaver.compositor.create_surface(&leaver.qh, ()))
                .collect();
            leaver.connection.flush().expect("the requests reach the socket");
            drop(leaver);
            let snapshot = wait_ledger(handle, "the leaver's account was never emptied", |snapshot| {
                snapshot.clients.len() == 1 && snapshot.client_bytes == baseline.client_bytes
            });
            assert_eq!(
                snapshot.clients[0].surfaces, 0,
                "only the steady client's account is left"
            );
        }
        steady.roundtrip();
    });
}

/// A small deterministic generator: the churn below is the same on every run, so a failure can be repeated.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, below: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        usize::try_from(self.0 >> 33).unwrap_or(0) % below
    }
}

/// A client and what the test knows it holds.
struct Churned {
    client: Client,
    buffers: Vec<(wayland_client::protocol::wl_buffer::WlBuffer, u64)>,
    surfaces: Vec<wayland_client::protocol::wl_surface::WlSurface>,
}

impl Churned {
    fn bytes(&self) -> u64 {
        self.buffers.iter().map(|(_, bytes)| bytes).sum()
    }
}

#[test]
fn after_churn_the_accounts_sum_to_the_aggregate() {
    // No bound is reached: the point is that what is counted is exactly what the clients hold, however they got there.
    let session = Session::start_limited(MemoryLimits::unlimited());
    session.run(|handle| {
        let mut random = Lcg(0x5eed);
        let mut clients: Vec<Churned> = Vec::new();
        for step in 0..600 {
            match random.next(8) {
                0 if clients.len() < 6 => clients.push(Churned {
                    client: Client::connect(handle.socket()),
                    buffers: Vec::new(),
                    surfaces: Vec::new(),
                }),
                1 if !clients.is_empty() => {
                    // A client leaves with whatever it holds.
                    let index = random.next(clients.len());
                    clients.swap_remove(index);
                }
                2..=3 if !clients.is_empty() => {
                    let index = random.next(clients.len());
                    let (width, height) = (16 + random.next(240), 16 + random.next(240));
                    let held = &mut clients[index];
                    let bytes = (width * height * 4) as u64;
                    held.buffers
                        .push((held.client.buffer(width as i32, height as i32), bytes));
                }
                4 if !clients.is_empty() => {
                    let index = random.next(clients.len());
                    let held = &mut clients[index];
                    if !held.buffers.is_empty() {
                        let (buffer, _) = held.buffers.swap_remove(random.next(held.buffers.len()));
                        buffer.destroy();
                    }
                }
                5..=6 if !clients.is_empty() => {
                    let index = random.next(clients.len());
                    let held = &mut clients[index];
                    held.surfaces
                        .push(held.client.compositor.create_surface(&held.client.qh, ()));
                }
                7 if !clients.is_empty() => {
                    let index = random.next(clients.len());
                    let held = &mut clients[index];
                    if !held.surfaces.is_empty() {
                        held.surfaces.swap_remove(random.next(held.surfaces.len())).destroy();
                    }
                }
                _ => {}
            }
            // Every so often every client is brought up to date and the ledger is compared with what the test knows.
            if step % 50 == 49 {
                for held in &mut clients {
                    held.client.roundtrip();
                }
                let mut expected: Vec<(u64, usize, usize)> = clients
                    .iter()
                    .filter(|held| !held.buffers.is_empty() || !held.surfaces.is_empty())
                    .map(|held| (held.bytes(), held.buffers.len(), held.surfaces.len()))
                    .collect();
                expected.sort_unstable();
                let total: u64 = expected.iter().map(|(bytes, _, _)| bytes).sum();
                let snapshot = wait_ledger(handle, "the ledger never matched what the clients hold", |snapshot| {
                    let mut seen: Vec<_> = snapshot
                        .clients
                        .iter()
                        .map(|client| (client.bytes, client.buffers, client.surfaces))
                        .collect();
                    seen.sort_unstable();
                    seen == expected
                });
                assert_eq!(
                    snapshot.client_bytes, total,
                    "the aggregate is the sum of what the clients hold"
                );
                assert_eq!(snapshot.client_bytes, snapshot.summed_client_bytes());
            }
        }
        drop(clients);
        let snapshot = wait_ledger(handle, "the ledger kept an account after every client left", is_empty);
        assert_eq!(snapshot.refused, 0);
    });
}
