//! What each client holds, and the bounds that stop a client or the session from holding without limit
//! (tests/gpu-memory-soak/DESIGN.md, section 6).
//!
//! The ledger is the one place the budget is decided. It counts three things per client (the bytes of its live
//! buffers, its live surfaces, and which buffers they are, so that a release names exactly what it gives back) and
//! keeps the session's aggregate beside them: the sum of the clients, the renderer's own footprint and the frames
//! capture keeps, as the loop last measured them. A request that would pass a bound is answered here, before
//! anything is mapped, imported or drawn.
//!
//! The ledger is shared (`Arc`) and locked because the request screen reads the state shared (`&State`) and a test or
//! an embedder reads it from another thread. The lock is held for a few additions, never across a call out.
//!
//! # Why the aggregate is kept apart from the per-client sum
//!
//! `client_bytes` is added to and subtracted from at every record and release, independently of the per-client
//! maps. They must agree at every instant; a release that names a buffer twice, or a client whose entry outlived its
//! buffers, shows as a difference, which is what `tests/gpu_memory_budget.rs` and the unit tests below assert.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, MutexGuard, PoisonError};

use smithay::reexports::wayland_server::backend::ClientId;

/// The default budget for everything the compositor holds, in bytes.
///
/// The incident this lane started from was a session that reached 31 GiB of GPU memory and then ran out. 6 GiB is
/// well under that and above what a desktop with a handful of 4K clients holds (a 3840 x 2160 buffer is 32 MiB, a
/// triple-buffered one 96 MiB, the output targets of three 4K outputs about 100 MiB); it fits a 8 GiB GPU with room
/// for the clients' own textures. A machine with more memory raises it, one with less lowers it
/// (`compositor.gpu-memory-budget-mib`).
pub const DEFAULT_BUDGET_BYTES: u64 = 6 << 30;

/// The default most one client may hold in live buffers, in bytes.
///
/// A third of the budget: one client, however it behaves, cannot take the session, and two of them together cannot
/// either. The largest buffer the compositor accepts (2^27 pixels) is 512 MiB, so a client can still hold several of
/// them. A browser or a video editor with dozens of large windows holds a few hundred MiB.
pub const DEFAULT_CLIENT_BYTES: u64 = 2 << 30;

/// The default most surfaces one client may hold at once.
///
/// A surface is a `wl_surface`: a toplevel, a popup, a subsurface or a cursor. A toolkit holds a few per window and a
/// browser with a hundred tabs a few thousand; each costs the compositor a record, so the cap bounds the records a
/// client can make, which the buffer bytes do not (a 1x1 surface costs one pixel and still costs its record).
pub const DEFAULT_CLIENT_SURFACES: u32 = 4096;

/// The bounds a session enforces. A zero value turns that bound off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryLimits {
    /// Everything the compositor holds: the clients' buffers, what the renderers hold and what capture keeps.
    pub budget_bytes: u64,
    /// The live buffer bytes of one client.
    pub client_bytes: u64,
    /// The live surfaces of one client.
    pub client_surfaces: u32,
}

impl Default for MemoryLimits {
    fn default() -> Self {
        Self {
            budget_bytes: DEFAULT_BUDGET_BYTES,
            client_bytes: DEFAULT_CLIENT_BYTES,
            client_surfaces: DEFAULT_CLIENT_SURFACES,
        }
    }
}

impl MemoryLimits {
    /// No bound at all, which is what the sessions before the budget existed had.
    #[must_use]
    pub const fn unlimited() -> Self {
        Self {
            budget_bytes: 0,
            client_bytes: 0,
            client_surfaces: 0,
        }
    }

    fn budget(&self) -> u64 {
        if self.budget_bytes == 0 {
            u64::MAX
        } else {
            self.budget_bytes
        }
    }

    fn per_client(&self) -> u64 {
        if self.client_bytes == 0 {
            u64::MAX
        } else {
            self.client_bytes
        }
    }

    fn surfaces(&self) -> u32 {
        if self.client_surfaces == 0 {
            u32::MAX
        } else {
            self.client_surfaces
        }
    }
}

/// Why a request was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MemoryRefusal {
    /// The client's buffers would pass its own limit.
    #[error("the client holds {held} bytes of buffers and a buffer of {asked} more passes its limit of {limit}")]
    ClientBytes {
        /// What the client holds now.
        held: u64,
        /// What the request asks for.
        asked: u64,
        /// The limit.
        limit: u64,
    },
    /// The client already holds as many surfaces as it may.
    #[error("the client holds {held} surfaces, which is its limit")]
    ClientSurfaces {
        /// What the client holds now, which is the limit.
        held: u32,
    },
    /// The session's budget cannot hold the buffer, even after everything it may evict is given back.
    #[error(
        "the compositor holds {used} of its {budget} bytes and {asked} more do not fit, even after evicting what it \
         may ({reclaimable} bytes)"
    )]
    Budget {
        /// What the session holds now.
        used: u64,
        /// What the request asks for.
        asked: u64,
        /// The budget.
        budget: u64,
        /// What could be evicted and was not enough.
        reclaimable: u64,
    },
}

/// What a request that was not refused needs before it is served.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryAdmission {
    /// It fits as things are.
    Fits,
    /// It fits once this many bytes of caches are given back, which the loop does before it draws.
    Evict {
        /// The bytes that must be freed.
        needed: u64,
    },
}

/// What the loop last measured of the memory that is not a client's, and how much of it may be given back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct HeldMemory {
    /// The renderers' footprint over every output.
    pub renderer: u64,
    /// The frames capture keeps.
    pub capture: u64,
    /// The part of the two that may be evicted now: a frame cache nobody reads, View textures when no lock is up,
    /// blur scratch. The output targets, the staging buffers and the frame being scanned out are never in it.
    pub reclaimable: u64,
}

/// One client's account.
#[derive(Debug, Default)]
struct Account {
    /// The bytes of the client's live buffers.
    bytes: u64,
    /// Each live buffer by its protocol id, so a release gives back what that buffer took and a second release
    /// gives back nothing.
    buffers: HashMap<u32, u64>,
    /// The client's live surfaces by protocol id, for the same reason.
    surfaces: HashSet<u32>,
}

impl Account {
    fn is_empty(&self) -> bool {
        self.buffers.is_empty() && self.surfaces.is_empty()
    }
}

#[derive(Debug, Default)]
struct LedgerState {
    clients: HashMap<ClientId, Account>,
    /// The sum of the clients' bytes, kept apart from the per-client maps (see the module comment).
    client_bytes: u64,
    held: HeldMemory,
    /// What must be freed before the next frame, from the admissions that counted on eviction.
    need: u64,
    refused: u64,
    evicted: u64,
}

/// A client's numbers, for a reader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientMemory {
    /// The bytes of its live buffers.
    pub bytes: u64,
    /// Its live buffers.
    pub buffers: usize,
    /// Its live surfaces.
    pub surfaces: usize,
}

/// What the ledger holds, at one instant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LedgerSnapshot {
    /// The bounds in force.
    pub limits: MemoryLimits,
    /// Every client that holds a buffer or a surface.
    pub clients: Vec<ClientMemory>,
    /// The aggregate of the clients' bytes, as counted on its own.
    pub client_bytes: u64,
    /// The renderers' footprint over every output at the last measurement.
    pub renderer_bytes: u64,
    /// The frames capture kept at the last measurement.
    pub capture_bytes: u64,
    /// The part of those two that could be evicted at the last measurement.
    pub reclaimable_bytes: u64,
    /// The requests refused since the session began.
    pub refused: u64,
    /// The bytes evicted from caches since the session began.
    pub evicted: u64,
}

impl LedgerSnapshot {
    /// The sum of what every client holds, from the per-client entries: what `client_bytes` must equal.
    #[must_use]
    pub fn summed_client_bytes(&self) -> u64 {
        self.clients.iter().map(|client| client.bytes).sum()
    }

    /// Everything the budget counts.
    #[must_use]
    pub fn used(&self) -> u64 {
        self.client_bytes
            .saturating_add(self.renderer_bytes)
            .saturating_add(self.capture_bytes)
    }
}

/// The ledger. Shared by the state, the request screen and whoever reads the session from outside.
#[derive(Debug)]
pub struct Ledger {
    limits: MemoryLimits,
    inner: Mutex<LedgerState>,
}

impl Ledger {
    /// A ledger that enforces `limits`.
    #[must_use]
    pub fn new(limits: MemoryLimits) -> Self {
        Self {
            limits,
            inner: Mutex::new(LedgerState::default()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, LedgerState> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The bounds in force.
    #[must_use]
    pub fn limits(&self) -> MemoryLimits {
        self.limits
    }

    /// Whether a buffer of `bytes` may be created by `client`, and what that needs.
    ///
    /// Nothing is recorded: the buffer is recorded by [`Ledger::record_buffer`] once the object exists, so a request
    /// the protocol layer rejects afterwards leaves no entry behind.
    ///
    /// # Errors
    ///
    /// [`MemoryRefusal::ClientBytes`] when the client's own limit would pass, and [`MemoryRefusal::Budget`] when the session's
    /// budget cannot hold the buffer even after every cache that may be evicted is given back.
    pub fn admit_buffer(&self, client: &ClientId, bytes: u64) -> Result<MemoryAdmission, MemoryRefusal> {
        let mut inner = self.lock();
        let held = inner.clients.get(client).map_or(0, |account| account.bytes);
        let limit = self.limits.per_client();
        if held.saturating_add(bytes) > limit {
            inner.refused = inner.refused.saturating_add(1);
            return Err(MemoryRefusal::ClientBytes {
                held,
                asked: bytes,
                limit,
            });
        }
        self.admit_bytes(&mut inner, bytes)
    }

    /// Whether the session may hold `bytes` more that no client asked for: the target of an output whose mode or
    /// supersample factor grows. The same answer as for a buffer, without a client's own limit.
    ///
    /// # Errors
    ///
    /// [`MemoryRefusal::Budget`] when the budget cannot hold them even after every cache that may be evicted is given back.
    pub fn admit_growth(&self, bytes: u64) -> Result<MemoryAdmission, MemoryRefusal> {
        let mut inner = self.lock();
        self.admit_bytes(&mut inner, bytes)
    }

    /// The aggregate half of an admission: what the session holds, plus `bytes`, against the budget.
    fn admit_bytes(&self, inner: &mut LedgerState, bytes: u64) -> Result<MemoryAdmission, MemoryRefusal> {
        let used = inner
            .client_bytes
            .saturating_add(inner.held.renderer)
            .saturating_add(inner.held.capture);
        let after = used.saturating_add(bytes);
        let budget = self.limits.budget();
        if after <= budget {
            return Ok(MemoryAdmission::Fits);
        }
        let needed = after - budget;
        if inner.held.reclaimable >= needed {
            inner.need = inner.need.max(needed);
            return Ok(MemoryAdmission::Evict { needed });
        }
        inner.refused = inner.refused.saturating_add(1);
        Err(MemoryRefusal::Budget {
            used,
            asked: bytes,
            budget,
            reclaimable: inner.held.reclaimable,
        })
    }

    /// Records a buffer the client now has. A buffer already recorded under the id is replaced, not added.
    pub fn record_buffer(&self, client: &ClientId, id: u32, bytes: u64) {
        let mut inner = self.lock();
        let account = inner.clients.entry(client.clone()).or_default();
        let before = account.buffers.insert(id, bytes).unwrap_or(0);
        account.bytes = account.bytes.saturating_sub(before).saturating_add(bytes);
        inner.client_bytes = inner.client_bytes.saturating_sub(before).saturating_add(bytes);
    }

    /// Gives back what the buffer took. A buffer that was never recorded, or already released, gives back nothing.
    pub fn release_buffer(&self, client: &ClientId, id: u32) {
        let mut inner = self.lock();
        let Some(account) = inner.clients.get_mut(client) else {
            return;
        };
        let Some(bytes) = account.buffers.remove(&id) else {
            return;
        };
        account.bytes = account.bytes.saturating_sub(bytes);
        let gone = account.is_empty();
        inner.client_bytes = inner.client_bytes.saturating_sub(bytes);
        if gone {
            inner.clients.remove(client);
        }
    }

    /// Whether `client` may make another surface.
    ///
    /// # Errors
    ///
    /// [`MemoryRefusal::ClientSurfaces`] when it holds as many as it may.
    pub fn admit_surface(&self, client: &ClientId) -> Result<(), MemoryRefusal> {
        let mut inner = self.lock();
        let held = inner.clients.get(client).map_or(0, |account| account.surfaces.len());
        let held = u32::try_from(held).unwrap_or(u32::MAX);
        if held >= self.limits.surfaces() {
            inner.refused = inner.refused.saturating_add(1);
            return Err(MemoryRefusal::ClientSurfaces { held });
        }
        Ok(())
    }

    /// Records a surface the client now has.
    pub fn record_surface(&self, client: &ClientId, id: u32) {
        self.lock()
            .clients
            .entry(client.clone())
            .or_default()
            .surfaces
            .insert(id);
    }

    /// Gives a surface back.
    pub fn release_surface(&self, client: &ClientId, id: u32) {
        let mut inner = self.lock();
        let Some(account) = inner.clients.get_mut(client) else {
            return;
        };
        account.surfaces.remove(&id);
        if account.is_empty() {
            inner.clients.remove(client);
        }
    }

    /// Stores what the loop measured of the memory that is not a client's.
    pub(crate) fn observe(&self, held: HeldMemory) {
        self.lock().held = held;
    }

    /// Whether `bytes` more would still be within the budget, as last measured.
    pub(crate) fn has_room(&self, bytes: u64) -> bool {
        let inner = self.lock();
        let used = inner
            .client_bytes
            .saturating_add(inner.held.renderer)
            .saturating_add(inner.held.capture);
        used.saturating_add(bytes) <= self.limits.budget()
    }

    /// Whether an admission counted on caches being given back and the loop has not done it yet.
    pub(crate) fn needs_room(&self) -> bool {
        self.lock().need > 0
    }

    /// The bytes the admissions since the last call counted on being freed, and forgets them.
    pub(crate) fn take_need(&self) -> Option<u64> {
        let mut inner = self.lock();
        (inner.need > 0).then(|| std::mem::take(&mut inner.need))
    }

    /// Counts bytes given back from caches.
    pub(crate) fn note_evicted(&self, bytes: u64) {
        let mut inner = self.lock();
        inner.evicted = inner.evicted.saturating_add(bytes);
    }

    /// The numbers the screen nodes publish: the clients' bytes, the refusals and the bytes evicted.
    #[must_use]
    pub(crate) fn published(&self) -> (u64, u64, u64) {
        let inner = self.lock();
        (inner.client_bytes, inner.refused, inner.evicted)
    }

    /// Everything the ledger holds, copied.
    #[must_use]
    pub fn snapshot(&self) -> LedgerSnapshot {
        let inner = self.lock();
        LedgerSnapshot {
            limits: self.limits,
            clients: inner
                .clients
                .values()
                .map(|account| ClientMemory {
                    bytes: account.bytes,
                    buffers: account.buffers.len(),
                    surfaces: account.surfaces.len(),
                })
                .collect(),
            client_bytes: inner.client_bytes,
            renderer_bytes: inner.held.renderer,
            capture_bytes: inner.held.capture,
            reclaimable_bytes: inner.held.reclaimable,
            refused: inner.refused,
            evicted: inner.evicted,
        }
    }
}

/// Client identities for the tests of this module and of `gpu_memory`: a `ClientId` only exists for a connection to a
/// display, and the ledger only hashes and compares it.
#[cfg(test)]
pub(crate) mod test_support {
    use std::os::unix::net::UnixStream;
    use std::sync::Arc;

    use smithay::reexports::wayland_server::Display;
    use smithay::reexports::wayland_server::backend::{ClientData, ClientId};

    struct Data;
    impl ClientData for Data {}

    /// `count` distinct client ids.
    pub(crate) fn clients(count: usize) -> Vec<ClientId> {
        let display = Display::<()>::new().expect("a display");
        let mut handle = display.handle();
        let ids = (0..count)
            .map(|_| {
                let (server, client) = UnixStream::pair().expect("a socket pair");
                // The peer end is leaked on purpose: the id only has to stay distinct for the test.
                std::mem::forget(client);
                handle.insert_client(server, Arc::new(Data)).expect("a client").id()
            })
            .collect();
        // A dropped display would end the clients; the ids are only compared, so it is kept for the process.
        std::mem::forget(display);
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::test_support::clients;

    fn limits() -> MemoryLimits {
        MemoryLimits {
            budget_bytes: 1000,
            client_bytes: 400,
            client_surfaces: 3,
        }
    }

    #[test]
    fn a_client_past_its_own_limit_is_refused_and_another_is_not() {
        let ids = clients(2);
        let ledger = Ledger::new(limits());
        assert_eq!(ledger.admit_buffer(&ids[0], 300), Ok(MemoryAdmission::Fits));
        ledger.record_buffer(&ids[0], 1, 300);
        assert!(matches!(
            ledger.admit_buffer(&ids[0], 101),
            Err(MemoryRefusal::ClientBytes {
                held: 300,
                asked: 101,
                limit: 400
            })
        ));
        assert_eq!(ledger.admit_buffer(&ids[0], 100), Ok(MemoryAdmission::Fits));
        assert_eq!(ledger.admit_buffer(&ids[1], 400), Ok(MemoryAdmission::Fits));
        assert_eq!(ledger.snapshot().refused, 1);
    }

    #[test]
    fn the_aggregate_refuses_what_no_eviction_can_make_room_for() {
        let ids = clients(3);
        let ledger = Ledger::new(limits());
        for id in ids.iter().take(2) {
            ledger.record_buffer(id, 1, 400);
        }
        // 800 held, 200 left: a buffer of 300 does not fit and there is nothing to evict.
        assert!(matches!(
            ledger.admit_buffer(&ids[2], 300),
            Err(MemoryRefusal::Budget {
                used: 800,
                asked: 300,
                budget: 1000,
                reclaimable: 0
            })
        ));
        // Releasing one client's buffer frees the room for the next.
        ledger.release_buffer(&ids[0], 1);
        assert_eq!(ledger.admit_buffer(&ids[2], 300), Ok(MemoryAdmission::Fits));
    }

    #[test]
    fn what_can_be_evicted_is_counted_before_a_request_is_refused() {
        let ids = clients(2);
        let ledger = Ledger::new(limits());
        ledger.record_buffer(&ids[0], 1, 400);
        ledger.observe(HeldMemory {
            renderer: 300,
            capture: 200,
            reclaimable: 250,
        });
        // 900 held. A buffer of 300 needs 200 back and 250 can be given.
        assert_eq!(
            ledger.admit_buffer(&ids[1], 300),
            Ok(MemoryAdmission::Evict { needed: 200 })
        );
        assert_eq!(ledger.take_need(), Some(200));
        assert_eq!(ledger.take_need(), None, "the need is taken once");
        // A buffer of 400 needs 300 back and only 250 can be given.
        assert!(matches!(
            ledger.admit_buffer(&ids[1], 400),
            Err(MemoryRefusal::Budget { .. })
        ));
    }

    #[test]
    fn surfaces_are_bounded_per_client_and_released_by_id() {
        let ids = clients(2);
        let ledger = Ledger::new(limits());
        for id in 0..3 {
            assert_eq!(ledger.admit_surface(&ids[0]), Ok(()));
            ledger.record_surface(&ids[0], id);
        }
        assert_eq!(
            ledger.admit_surface(&ids[0]),
            Err(MemoryRefusal::ClientSurfaces { held: 3 })
        );
        assert_eq!(
            ledger.admit_surface(&ids[1]),
            Ok(()),
            "another client has its own count"
        );
        ledger.release_surface(&ids[0], 1);
        ledger.release_surface(&ids[0], 1);
        assert_eq!(ledger.admit_surface(&ids[0]), Ok(()), "one released, once");
    }

    #[test]
    fn a_second_release_gives_back_nothing_and_an_empty_client_leaves_no_entry() {
        let ids = clients(1);
        let ledger = Ledger::new(MemoryLimits::default());
        ledger.record_buffer(&ids[0], 7, 100);
        ledger.record_buffer(&ids[0], 8, 50);
        ledger.release_buffer(&ids[0], 7);
        ledger.release_buffer(&ids[0], 7);
        let snapshot = ledger.snapshot();
        assert_eq!((snapshot.client_bytes, snapshot.summed_client_bytes()), (50, 50));
        ledger.release_buffer(&ids[0], 8);
        let snapshot = ledger.snapshot();
        assert!(snapshot.clients.is_empty());
        assert_eq!(snapshot.client_bytes, 0);
    }

    #[test]
    fn a_buffer_recorded_again_under_its_id_replaces_the_old_one() {
        let ids = clients(1);
        let ledger = Ledger::new(MemoryLimits::default());
        ledger.record_buffer(&ids[0], 1, 100);
        ledger.record_buffer(&ids[0], 1, 30);
        let snapshot = ledger.snapshot();
        assert_eq!((snapshot.client_bytes, snapshot.summed_client_bytes()), (30, 30));
    }

    #[test]
    fn a_zero_limit_turns_that_bound_off() {
        let ids = clients(1);
        let ledger = Ledger::new(MemoryLimits::unlimited());
        assert_eq!(ledger.admit_buffer(&ids[0], u64::MAX / 2), Ok(MemoryAdmission::Fits));
        for id in 0..10_000 {
            assert_eq!(ledger.admit_surface(&ids[0]), Ok(()));
            ledger.record_surface(&ids[0], id);
        }
    }
}
