//! The pool buffers of one PipeWire stream, and the one place this crate writes through their mapping.
//!
//! The stream maps every buffer it offers (`MAP_BUFFERS`), so the producer can write the pixels it
//! queued. PipeWire documents no way to recall a buffer already queued to a stream, and a consumer
//! may dequeue one later, so zeroing the queued buffers is the producer's only lever on the pixels a
//! consumer could still take. Writing through the shared mapping is not a documented PipeWire
//! guarantee: it is the behaviour the daemon's memory gives today. A buffer the consumer dequeued
//! before the zeroing may be torn by it.

#![allow(unsafe_code)]

use pipewire::spa::sys::{spa_buffer, spa_data};
use pipewire::sys::pw_buffer;

/// The buffers a stream's pool holds, as PipeWire's `add_buffer` and `remove_buffer` callbacks named them.
///
/// A pointer is recorded at `add` and forgotten at `remove`. PipeWire keeps each buffer alive in
/// between. The stream lives on one loop thread, and the pointers are read only there.
#[derive(Default)]
pub(crate) struct StreamBuffers {
    buffers: Vec<*mut pw_buffer>,
}

impl StreamBuffers {
    /// Records a buffer PipeWire added. A null pointer is recorded too, so that [`StreamBuffers::zero`] refuses it
    /// rather than skipping a buffer it cannot reach.
    pub(crate) fn add(&mut self, buffer: *mut pw_buffer) {
        self.buffers.push(buffer);
    }

    /// Forgets a buffer PipeWire removed.
    pub(crate) fn remove(&mut self, buffer: *mut pw_buffer) {
        self.buffers.retain(|known| *known != buffer);
    }

    /// Zeroes the data of every recorded buffer, and answers whether all of it could be zeroed.
    ///
    /// The producer holds no buffer when this runs: `fill` queues its buffer before it returns, and the scrub
    /// runs between pumps, so every recorded buffer is one a consumer may take.
    ///
    /// A buffer that cannot be zeroed (a null pointer, no data plane, or a plane with no mapping) is
    /// skipped and makes the answer false, so the caller ends the stream rather than let a consumer
    /// take what the buffer still holds.
    pub(crate) fn zero(&self) -> bool {
        let mut zeroed = true;
        for buffer in &self.buffers {
            // SAFETY: `add` recorded this pointer from PipeWire's callback and `remove` has not forgotten it,
            // so PipeWire keeps the buffer alive; the pool is read only on the stream's loop thread.
            let Some(planes) = (unsafe { planes_of(*buffer) }) else {
                zeroed = false;
                continue;
            };
            for plane in planes {
                let Ok(length) = usize::try_from(plane.maxsize) else {
                    zeroed = false;
                    continue;
                };
                if plane.data.is_null() {
                    zeroed = false;
                    continue;
                }
                // SAFETY: a non-null plane's `maxsize` bytes are the mapping PipeWire gave this process for it,
                // and nothing else holds a reference to them on this thread while the write runs.
                unsafe { std::slice::from_raw_parts_mut(plane.data.cast::<u8>(), length) }.fill(0);
            }
        }
        zeroed
    }
}

/// The data planes of a pool buffer. `None` when the pointer is null or the buffer has no planes.
///
/// # Safety
///
/// `buffer` is null, or a pool buffer PipeWire has not removed, and no other reference to its planes exists.
unsafe fn planes_of<'a>(buffer: *mut pw_buffer) -> Option<&'a mut [spa_data]> {
    if buffer.is_null() {
        return None;
    }
    // SAFETY: `buffer` is live, so its `buffer` field names the `spa_buffer` PipeWire built with it.
    let spa: *mut spa_buffer = unsafe { (*buffer).buffer };
    // SAFETY: `spa` is null or the live `spa_buffer` named above.
    let spa = unsafe { spa.as_mut() }?;
    if spa.n_datas == 0 || spa.datas.is_null() {
        return None;
    }
    let count = usize::try_from(spa.n_datas).ok()?;
    // SAFETY: `datas` names `n_datas` `spa_data` entries that the same `spa_buffer` owns.
    Some(unsafe { std::slice::from_raw_parts_mut(spa.datas, count) })
}

#[cfg(test)]
impl StreamBuffers {
    /// A copy of the bytes of every data plane the pool maps, for the tests that check what a buffer holds.
    pub(crate) fn planes(&self) -> Vec<Vec<u8>> {
        let mut copies = Vec::new();
        for buffer in &self.buffers {
            // SAFETY: as in `zero`.
            let Some(planes) = (unsafe { planes_of(*buffer) }) else {
                continue;
            };
            for plane in planes.iter() {
                let Ok(length) = usize::try_from(plane.maxsize) else {
                    continue;
                };
                if plane.data.is_null() {
                    continue;
                }
                // SAFETY: as in `zero`.
                copies.push(unsafe { std::slice::from_raw_parts(plane.data.cast::<u8>(), length) }.to_vec());
            }
        }
        copies
    }
}

/// A buffer built in test memory, with one plane over `bytes` when `mapped` and with no mapping otherwise.
#[cfg(test)]
pub(crate) struct Fake {
    buffer: pw_buffer,
    spa: spa_buffer,
    plane: spa_data,
    bytes: Vec<u8>,
}

#[cfg(test)]
impl Fake {
    /// Boxed so the pointers the fake sets between its own fields stay where they are once it is in a pool.
    pub(crate) fn boxed(length: usize, mapped: bool) -> Box<Self> {
        // SAFETY: the C structs hold only pointers and integers, for which all zeros is a valid value; the `Vec` is set in place.
        let mut fake = Box::new(Self {
            buffer: unsafe { std::mem::zeroed() },
            spa: unsafe { std::mem::zeroed() },
            plane: unsafe { std::mem::zeroed() },
            bytes: vec![0xff; length],
        });
        fake.plane.maxsize = u32::try_from(length).expect("a small test plane");
        fake.plane.data = if mapped {
            fake.bytes.as_mut_ptr().cast()
        } else {
            std::ptr::null_mut()
        };
        fake.spa.n_datas = 1;
        fake.spa.datas = std::ptr::addr_of_mut!(fake.plane);
        fake.buffer.buffer = std::ptr::addr_of_mut!(fake.spa);
        fake
    }

    /// The pointer the pool records for this buffer.
    pub(crate) fn pointer(&mut self) -> *mut pw_buffer {
        std::ptr::addr_of_mut!(self.buffer)
    }

    /// The bytes of the plane, as the producer's mapping holds them.
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// A `pw_buffer` with no `spa_buffer` at all: nothing in it can be mapped, so nothing can be zeroed.
#[cfg(test)]
pub(crate) fn without_buffer() -> Box<pw_buffer> {
    // SAFETY: every field of a `pw_buffer` is a pointer or an integer, and all zeros is a valid value of each.
    Box::new(unsafe { std::mem::zeroed() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_clears_every_mapped_plane_and_refuses_an_unmapped_one() {
        let mut mapped = Fake::boxed(8, true);
        let mut unmapped = Fake::boxed(8, false);
        let mut pool = StreamBuffers::default();
        pool.add(mapped.pointer());
        pool.add(unmapped.pointer());

        assert!(!pool.zero(), "a plane with no mapping cannot be zeroed");
        assert_eq!(mapped.bytes(), &[0u8; 8], "the mapped plane is zeroed");
        assert_eq!(pool.planes(), vec![vec![0u8; 8]], "only the mapped plane is mapped");
    }

    #[test]
    fn zero_refuses_a_null_pointer_and_a_buffer_with_no_planes() {
        let mut pool = StreamBuffers::default();
        assert!(pool.zero(), "an empty pool has nothing to zero");

        pool.add(std::ptr::null_mut());
        assert!(!pool.zero(), "a null pointer cannot be zeroed");

        let mut bare = without_buffer();
        let mut bare_pool = StreamBuffers::default();
        bare_pool.add(&mut *bare);
        assert!(!bare_pool.zero(), "a buffer with no spa_buffer cannot be zeroed");
    }

    #[test]
    fn remove_forgets_the_buffer() {
        let mut fake = Fake::boxed(4, true);
        let pointer = fake.pointer();
        let mut pool = StreamBuffers::default();
        pool.add(pointer);
        pool.remove(pointer);
        assert!(pool.planes().is_empty());
        assert!(pool.zero(), "a removed buffer is not zeroed, so it cannot be refused");
    }
}
