//! Whether a socket has something to read, and waiting for any of several to have.
//!
//! A read with a timeout waits out the whole timeout when nothing has arrived, and that wait is a cost per link per
//! turn for a shell that holds several connections. Asking the kernel first, with a zero timeout, costs a system call
//! and nothing more, so a read is made only when it will return at once; and one `poll(2)` over every connection lets a
//! quiet loop sleep until any of them has bytes or its next deadline comes.

use std::io;
use std::os::fd::{AsFd, BorrowedFd};
use std::time::Duration;

use rustix::event::{Nsecs, PollFd, PollFlags, Secs, Timespec, poll};

/// What counts as something to read: bytes, a hang-up and an error all make a read return at once.
fn ready() -> PollFlags {
    PollFlags::IN | PollFlags::HUP | PollFlags::ERR
}

/// The longest wait `poll(2)` is given: a deadline past it is cut here, and the caller asks again.
const LONGEST: Duration = Duration::from_secs(3600);

/// Whether a read of `fd` would return at once: it holds bytes, or its peer is gone. A failure of the check itself says
/// yes, so the read that follows reports the real error.
pub fn readable_now(fd: &impl AsFd) -> bool {
    let mut fds = [PollFd::new(fd, PollFlags::IN)];
    let now = Timespec { tv_sec: 0, tv_nsec: 0 };
    loop {
        match poll(&mut fds, Some(&now)) {
            Ok(_) => return fds[0].revents().intersects(ready()),
            Err(rustix::io::Errno::INTR) => {}
            Err(_) => return true,
        }
    }
}

/// Blocks until one of `fds` is readable or `timeout` has passed, and says whether one is. A signal ends the wait early
/// with `false`, so a caller that checks a stop flag between waits sees it.
///
/// # Errors
///
/// The kernel refused the wait, which is a descriptor that is not open.
pub fn wait_readable(fds: &[BorrowedFd<'_>], timeout: Duration) -> io::Result<bool> {
    let timeout = timeout.min(LONGEST);
    let wait = Timespec {
        tv_sec: Secs::try_from(timeout.as_secs()).unwrap_or(Secs::MAX),
        tv_nsec: Nsecs::from(timeout.subsec_nanos()),
    };
    let mut polled: Vec<PollFd<'_>> = fds.iter().map(|fd| PollFd::new(fd, PollFlags::IN)).collect();
    match poll(&mut polled, Some(&wait)) {
        Ok(_) => Ok(polled.iter().any(|fd| fd.revents().intersects(ready()))),
        Err(rustix::io::Errno::INTR) => Ok(false),
        Err(error) => Err(io::Error::from(error)),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::os::fd::AsFd;
    use std::os::unix::net::UnixStream;
    use std::time::{Duration, Instant};

    use super::{readable_now, wait_readable};

    #[test]
    fn a_quiet_socket_is_not_readable_and_a_written_one_is() {
        let (near, mut far) = UnixStream::pair().expect("a socket pair");
        assert!(!readable_now(&near));
        far.write_all(b"x").expect("write");
        assert!(readable_now(&near));
    }

    #[test]
    fn a_closed_peer_is_readable_so_the_read_can_report_it() {
        let (near, far) = UnixStream::pair().expect("a socket pair");
        drop(far);
        assert!(readable_now(&near));
    }

    #[test]
    fn a_wait_ends_at_its_deadline_when_nothing_arrives_and_at_once_when_something_has() {
        let (near, mut far) = UnixStream::pair().expect("a socket pair");
        let started = Instant::now();
        assert!(!wait_readable(&[near.as_fd()], Duration::from_millis(30)).expect("a wait"));
        assert!(
            started.elapsed() >= Duration::from_millis(30),
            "it waited for the deadline"
        );
        far.write_all(b"x").expect("write");
        let started = Instant::now();
        assert!(wait_readable(&[near.as_fd()], Duration::from_secs(5)).expect("a wait"));
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "it did not wait for the deadline"
        );
    }

    #[test]
    fn any_one_of_several_sockets_ends_the_wait() {
        let (quiet, _quiet_far) = UnixStream::pair().expect("a socket pair");
        let (busy, mut busy_far) = UnixStream::pair().expect("a socket pair");
        busy_far.write_all(b"x").expect("write");
        assert!(wait_readable(&[quiet.as_fd(), busy.as_fd()], Duration::from_secs(5)).expect("a wait"));
        assert!(!wait_readable(&[], Duration::from_millis(1)).expect("a wait on nothing"));
    }
}
