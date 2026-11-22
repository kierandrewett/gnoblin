//! The retire list: native code that cannot be unmapped, kept bounded and visible.
//!
//! A native shell generation that has retired is still mapped, and stays mapped: a callback, a thread
//! or a TLS destructor of the retired code may still run, and `dlclose` under any of them is the crash this design exists
//! to avoid. The price is one idle library copy per reload for the life of the process. That is acceptable only if it is
//! bounded and visible, so the list does three things:
//!
//! - it counts what it holds and adds up the size of the mappings ([`RetainedStats`], which `gnoblinctl status` shows);
//! - it logs every retirement with the running count (done by the loader as it pushes);
//! - past [`RetirePolicy::max_retained`] entries it refuses the next load ([`RetireList::check_room`]) with a text that
//!   tells the user to restart, because the only way to give the memory back is to end the process.
//!
//! A generation that can prove it is unreachable may be unloaded, but only when the policy allows it (default: no) and the
//! module says so (`NativeModule::try_unload` in the `native` module).

use std::fmt;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};

/// How many retired or failed libraries stay mapped before a reload is refused.
pub const DEFAULT_MAX_RETAINED: usize = 32;

/// What the host does about libraries it cannot unmap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetirePolicy {
    /// A reload is refused once this many libraries are retained. At least 1: a limit of 0 would refuse the first load.
    pub max_retained: usize,
    /// Whether `NativeModule::try_unload` may `dlclose` a generation that proves it is unreachable. Off by default.
    pub allow_unload: bool,
}

impl Default for RetirePolicy {
    fn default() -> Self {
        Self {
            max_retained: DEFAULT_MAX_RETAINED,
            allow_unload: false,
        }
    }
}

/// Why a library is on the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetainedWhy {
    /// Its generation ran and retired.
    Retired,
    /// Its generation was staged and then abandoned (a reload that did not commit).
    Abandoned,
    /// It was mapped and then refused: not a shell, a wrong ABI major, or a failing `init`.
    Failed,
}

/// One library the host keeps mapped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Retained {
    /// The `dlopen` handle.
    pub handle: usize,
    /// The module path as asked for.
    pub path: PathBuf,
    /// The label the shell gave its entry descriptor, empty when it never got that far.
    pub label: String,
    /// Why it is kept.
    pub why: RetainedWhy,
    /// The size of the mapped file, the most address space it holds.
    pub mapped_bytes: u64,
}

/// What `gnoblinctl status` shows about retention.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RetainedStats {
    /// Libraries kept mapped now.
    pub count: usize,
    /// The size of their files added up: an upper bound of the address space they hold (clean file pages).
    pub mapped_bytes: u64,
    /// The limit past which a reload is refused.
    pub max_retained: usize,
    /// Libraries given back with `try_unload` since the process started.
    pub unloaded: usize,
}

impl fmt::Display for RetainedStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} of {} retired shell libraries retained ({} mapped",
            self.count,
            self.max_retained,
            human_bytes(self.mapped_bytes)
        )?;
        if self.unloaded > 0 {
            write!(f, ", {} unloaded", self.unloaded)?;
        }
        write!(f, ")")
    }
}

/// `1.5 MiB`, `812 KiB`, `0 B`: enough to read a log line.
fn human_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    if bytes >= MIB {
        let tenths = bytes * 10 / MIB;
        format!("{}.{} MiB", tenths / 10, tenths % 10)
    } else if bytes >= KIB {
        format!("{} KiB", bytes / KIB)
    } else {
        format!("{bytes} B")
    }
}

/// The list is full: the next load is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoomError {
    /// Libraries retained.
    pub retained: usize,
    /// The limit.
    pub max: usize,
}

struct RetainedInner {
    policy: RetirePolicy,
    entries: Vec<Retained>,
    unloaded: usize,
}

/// The libraries a host keeps mapped.
pub struct RetireList {
    inner: Mutex<RetainedInner>,
}

impl Default for RetireList {
    fn default() -> Self {
        Self::new()
    }
}

impl RetireList {
    /// An empty list under the default policy.
    pub const fn new() -> Self {
        Self {
            inner: Mutex::new(RetainedInner {
                policy: RetirePolicy {
                    max_retained: DEFAULT_MAX_RETAINED,
                    allow_unload: false,
                },
                entries: Vec::new(),
                unloaded: 0,
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, RetainedInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The policy in force.
    pub fn policy(&self) -> RetirePolicy {
        self.lock().policy
    }

    /// Replaces the policy. A `max_retained` of 0 is raised to 1.
    pub fn set_policy(&self, policy: RetirePolicy) {
        self.lock().policy = RetirePolicy {
            max_retained: policy.max_retained.max(1),
            ..policy
        };
    }

    /// Whether another library may be mapped.
    ///
    /// # Errors
    ///
    /// [`RoomError`] when the list already holds [`RetirePolicy::max_retained`] libraries.
    pub fn check_room(&self) -> Result<(), RoomError> {
        let inner = self.lock();
        if inner.entries.len() >= inner.policy.max_retained {
            return Err(RoomError {
                retained: inner.entries.len(),
                max: inner.policy.max_retained,
            });
        }
        Ok(())
    }

    /// Keeps a library mapped for good (or until `try_unload`) and returns the numbers after adding it.
    pub fn retain(&self, entry: Retained) -> RetainedStats {
        let mut inner = self.lock();
        inner.entries.push(entry);
        Self::stats_of(&inner)
    }

    /// Takes a library off the list because it is about to be unloaded, counting it as unloaded. `None` when the handle is
    /// not on the list.
    pub fn release(&self, handle: usize) -> Option<Retained> {
        let mut inner = self.lock();
        let at = inner.entries.iter().position(|entry| entry.handle == handle)?;
        inner.unloaded += 1;
        Some(inner.entries.remove(at))
    }

    /// The numbers `gnoblinctl status` shows.
    pub fn stats(&self) -> RetainedStats {
        Self::stats_of(&self.lock())
    }

    fn stats_of(inner: &RetainedInner) -> RetainedStats {
        RetainedStats {
            count: inner.entries.len(),
            mapped_bytes: inner.entries.iter().map(|entry| entry.mapped_bytes).sum(),
            max_retained: inner.policy.max_retained,
            unloaded: inner.unloaded,
        }
    }

    /// A copy of the list, oldest first, for a diagnostic listing.
    pub fn entries(&self) -> Vec<Retained> {
        self.lock().entries.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(handle: usize, bytes: u64) -> Retained {
        Retained {
            handle,
            path: PathBuf::from(format!("/shells/{handle}.so")),
            label: format!("shell{handle}"),
            why: RetainedWhy::Retired,
            mapped_bytes: bytes,
        }
    }

    #[test]
    fn the_default_policy_keeps_32_and_never_unloads() {
        let policy = RetirePolicy::default();
        assert_eq!(policy.max_retained, 32);
        assert!(!policy.allow_unload);
        assert_eq!(RetireList::new().policy(), policy);
    }

    #[test]
    fn the_count_and_the_mapped_size_are_added_up_as_libraries_are_retained() {
        let list = RetireList::new();
        assert_eq!(list.stats().count, 0);
        list.retain(entry(1, 1000));
        let stats = list.retain(entry(2, 2500));
        assert_eq!((stats.count, stats.mapped_bytes, stats.max_retained), (2, 3500, 32));
        assert_eq!(list.stats(), stats);
        assert_eq!(list.entries().len(), 2);
    }

    #[test]
    fn a_load_is_refused_once_the_limit_is_reached_and_not_before() {
        let list = RetireList::new();
        list.set_policy(RetirePolicy {
            max_retained: 3,
            allow_unload: false,
        });
        for handle in 1..=3 {
            assert_eq!(list.check_room(), Ok(()), "room for load {handle}");
            list.retain(entry(handle, 10));
        }
        assert_eq!(list.check_room(), Err(RoomError { retained: 3, max: 3 }));
    }

    #[test]
    fn a_limit_of_zero_is_raised_so_the_first_load_is_never_refused() {
        let list = RetireList::new();
        list.set_policy(RetirePolicy {
            max_retained: 0,
            allow_unload: false,
        });
        assert_eq!(list.policy().max_retained, 1);
        assert_eq!(list.check_room(), Ok(()));
    }

    #[test]
    fn releasing_a_library_makes_room_and_is_counted() {
        let list = RetireList::new();
        list.set_policy(RetirePolicy {
            max_retained: 1,
            allow_unload: true,
        });
        list.retain(entry(7, 4096));
        assert!(list.check_room().is_err());
        assert_eq!(list.release(99), None, "a handle that is not retained");
        let released = list.release(7).expect("retained");
        assert_eq!(released.mapped_bytes, 4096);
        assert_eq!(list.check_room(), Ok(()));
        let stats = list.stats();
        assert_eq!((stats.count, stats.mapped_bytes, stats.unloaded), (0, 0, 1));
    }

    #[test]
    fn the_status_line_names_the_count_the_limit_and_the_size() {
        let list = RetireList::new();
        list.retain(entry(1, 3 * 1024 * 1024 / 2));
        list.retain(entry(2, 812 * 1024));
        assert_eq!(
            list.stats().to_string(),
            "2 of 32 retired shell libraries retained (2.2 MiB mapped)"
        );
        list.release(1);
        assert_eq!(
            list.stats().to_string(),
            "1 of 32 retired shell libraries retained (812 KiB mapped, 1 unloaded)"
        );
        assert_eq!(human_bytes(0), "0 B");
    }
}
