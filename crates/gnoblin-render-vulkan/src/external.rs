#![allow(unsafe_code)]
//! Synchronisation with other processes through dmabuf fences.
//!
//! Two things cross the process boundary with a dmabuf: who is writing it, and when the renderer has finished with it. The
//! kernel keeps both as fences on the buffer, and since Linux 6.0 a `sync_file` can be taken from them and put into them
//! (`DMA_BUF_IOCTL_EXPORT_SYNC_FILE`, `DMA_BUF_IOCTL_IMPORT_SYNC_FILE`).
//!
//! - **Before sampling a client's buffer** the renderer waits for its writer. The linear path (`dmabuf.rs`) does it with the
//!   sync ioctl that brackets the CPU read. [`wait_for_writer`] is the same wait for a caller that has no mapping: it polls the
//!   buffer for readability, which is ready when its write fences have signalled, and it works on kernels before 6.0.
//! - **After writing a buffer others read** (a capture, a scan-out copy) the frame's fence goes into the buffer
//!   ([`import_sync_file`]), so the reader waits for it and the CPU does not. The linear path has already waited for the copy
//!   when it returns, so it needs no fence; a device that exports a binary semaphore as a `sync_file`
//!   (`VK_KHR_external_semaphore_fd`) passes it here instead of waiting.
//!
//! # Failure modes
//!
//! 1. **A kernel without the ioctls.** `ENOTTY` or `EINVAL` is returned as an error the caller answers with a CPU wait; it is
//!    never a crash: `tests::sync_files_are_refused_cleanly_on_a_file_that_is_not_a_dmabuf`.
//! 2. **The wait never ends.** [`wait_for_writer`] has a timeout and reports it.

use std::io;
use std::os::fd::{AsFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::time::Duration;

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::ioctl::{Opcode, Setter, Updater, opcode};

/// Reads the fences of the buffer's writers.
pub const SYNC_READ: u32 = 1 << 0;
/// Reads the fences of every access, writers and readers.
pub const SYNC_WRITE: u32 = 2;

/// The argument of both ioctls: which fences, and the `sync_file`'s descriptor (an output of the export, an input of the import).
#[repr(C)]
#[derive(Clone, Copy)]
struct SyncFileArgument {
    flags: u32,
    fd: i32,
}

const EXPORT_SYNC_FILE: Opcode = opcode::read_write::<SyncFileArgument>(b'b', 2);
const IMPORT_SYNC_FILE: Opcode = opcode::write::<SyncFileArgument>(b'b', 3);

/// A `sync_file` that signals when the writers of `dmabuf` (or all its users, with [`SYNC_WRITE`]) have finished.
///
/// # Errors
///
/// The kernel's error: `ENOTTY` for a descriptor that is not a dmabuf or a kernel before 6.0.
pub fn export_sync_file(dmabuf: BorrowedFd<'_>, access: u32) -> io::Result<OwnedFd> {
    let mut argument = SyncFileArgument { flags: access, fd: -1 };
    // SAFETY: the opcode and the argument's layout are the kernel's `dma_buf_export_sync_file`, which the kernel fills in.
    unsafe { rustix::ioctl::ioctl(dmabuf, Updater::<EXPORT_SYNC_FILE, _>::new(&mut argument)) }
        .map_err(io::Error::from)?;
    // SAFETY: on success the kernel made a new descriptor and handed its ownership to this process.
    Ok(unsafe { OwnedFd::from_raw_fd(argument.fd as RawFd) })
}

/// Makes `dmabuf` wait for `sync_file` before the accesses in `access` begin: its readers wait for a writer's fence with
/// [`SYNC_WRITE`], and its writers for a reader's with [`SYNC_READ`].
///
/// # Errors
///
/// The kernel's error, which the caller answers by waiting on the CPU.
pub fn import_sync_file(dmabuf: BorrowedFd<'_>, sync_file: BorrowedFd<'_>, access: u32) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    let argument = SyncFileArgument {
        flags: access,
        fd: sync_file.as_raw_fd(),
    };
    // SAFETY: the opcode and the argument's layout are the kernel's `dma_buf_import_sync_file`.
    unsafe { rustix::ioctl::ioctl(dmabuf, Setter::<IMPORT_SYNC_FILE, _>::new(argument)) }.map_err(io::Error::from)
}

/// Waits until the fences of the buffer's writers have signalled, by polling it for readability.
///
/// # Errors
///
/// `TimedOut` when they have not signalled after `timeout`.
pub fn wait_for_writer(dmabuf: impl AsFd, timeout: Duration) -> io::Result<()> {
    let mut fds = [PollFd::new(&dmabuf, PollFlags::IN)];
    let timeout = Timespec {
        tv_sec: timeout.as_secs() as _,
        tv_nsec: timeout.subsec_nanos() as _,
    };
    match poll(&mut fds, Some(&timeout)).map_err(io::Error::from)? {
        0 => Err(io::ErrorKind::TimedOut.into()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_files_are_refused_cleanly_on_a_file_that_is_not_a_dmabuf() {
        let file = std::fs::File::open("/dev/null").expect("a file");
        assert!(
            export_sync_file(file.as_fd(), SYNC_READ).is_err(),
            "not a dmabuf: an error, not a crash"
        );
        let other = std::fs::File::open("/dev/null").expect("a file");
        assert!(import_sync_file(file.as_fd(), other.as_fd(), SYNC_WRITE).is_err());
    }

    #[test]
    fn a_buffer_with_no_writer_is_ready_at_once() {
        // `/dev/null` polls readable immediately, as a dmabuf with no pending fence does.
        let file = std::fs::File::open("/dev/null").expect("a file");
        assert!(wait_for_writer(&file, Duration::from_millis(100)).is_ok());
    }
}
