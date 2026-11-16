//! What the dispatcher holds for one client: the frames its socket has not taken yet.
//!
//! The dispatcher serves every connection from one thread, and the lock card is pumped from that thread, so it must
//! never wait for a client's socket: a write to a full one stands the session still for as long as it waits. An
//! [`Outbox`] hands the socket what it takes at once (`MSG_DONTWAIT`), keeps the rest in order, and offers it again on
//! every tick of the dispatcher. A client that does not read costs its own connection and nothing else:
//!
//! - It is dropped when the socket has taken none of what waits for it for [`STALL_LIMIT`].
//! - It is dropped at once when more than [`MAX_OUTBOX`] bytes would wait for it.
//!
//! So the memory held for one client is at most [`MAX_OUTBOX`] and the one frame that crosses it, for at most
//! [`STALL_LIMIT`]. A client that does read, however slowly, loses no frame and sees them in the order they were queued.
//!
//! Each frame is a buffer of its own, encoded once and wiped when it is gone, because a frame can carry a secret (a
//! passphrase answered to the client that asked for it). A buffer is never grown or moved after the frame is in it, so no
//! copy of a frame is left behind in memory that was given back.

use std::collections::VecDeque;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use gnoblin_proto::{CodecError, Envelope, LENGTH_PREFIX_LEN, MAX_FRAME_LEN, write_frame};
use rustix::io::Errno;
use rustix::net::{self, SendFlags};
use zeroize::Zeroizing;

/// How long a client may take nothing of what waits for it before the connection is dropped.
pub(super) const STALL_LIMIT: Duration = Duration::from_secs(5);

/// The most bytes held for one client that is not taking them: two frames of the largest size. A client that is
/// reading one such frame is not behind for being sent the next; a client over this is not reading.
pub(super) const MAX_OUTBOX: usize = 2 * (MAX_FRAME_LEN + LENGTH_PREFIX_LEN);

/// Why a connection could not be served and is dropped.
#[derive(Debug, thiserror::Error)]
pub(super) enum Dropped {
    /// The socket took none of what waited for the client for [`STALL_LIMIT`].
    #[error("the client took nothing of what waited for it for {} seconds", STALL_LIMIT.as_secs())]
    Stalled,
    /// More than [`MAX_OUTBOX`] bytes would have waited for the client.
    #[error("more than {} bytes would wait for the client", MAX_OUTBOX)]
    Backlogged,
    /// A frame for the client did not encode.
    #[error("a frame for the client did not encode: {0}")]
    Unencodable(CodecError),
    /// The socket refused the write: the peer went away, or the kernel refused.
    #[error("the socket refused the write: {0}")]
    Socket(Errno),
}

impl Dropped {
    /// Whether the client stopped reading, which is worth saying, and not that it went away, which is routine.
    pub(super) fn is_a_stall(&self) -> bool {
        matches!(self, Self::Stalled | Self::Backlogged)
    }
}

/// The frames one client's socket has not taken yet.
pub(super) struct Outbox {
    /// The frames the socket has not fully taken, oldest first.
    frames: VecDeque<Zeroizing<Vec<u8>>>,
    /// How many bytes of the first frame the socket has taken.
    offset: usize,
    /// How many bytes the socket has not taken, over every frame.
    waiting: usize,
    /// When the socket last took a byte, or when the outbox last went from empty to waiting.
    progress: Instant,
}

impl Outbox {
    pub(super) fn new(now: Instant) -> Self {
        Self {
            frames: VecDeque::new(),
            offset: 0,
            waiting: 0,
            progress: now,
        }
    }

    /// How many bytes the socket has not taken yet.
    #[cfg(test)]
    pub(super) fn waiting(&self) -> usize {
        self.waiting
    }

    /// Queues `envelope` behind what already waits. It is refused when the client already has something waiting and
    /// this would take it past [`MAX_OUTBOX`]: a client with nothing waiting is sent one frame whatever its size.
    pub(super) fn push(&mut self, envelope: &Envelope, now: Instant) -> Result<(), Dropped> {
        let mut frame = Zeroizing::new(Vec::new());
        write_frame(&mut *frame, envelope).map_err(Dropped::Unencodable)?;
        if self.waiting == 0 {
            // The stall is counted from the moment the client had something to take.
            self.progress = now;
        } else if self.waiting + frame.len() > MAX_OUTBOX {
            return Err(Dropped::Backlogged);
        }
        self.waiting += frame.len();
        self.frames.push_back(frame);
        Ok(())
    }

    /// Hands `socket` what it takes at once and keeps the rest. It never waits: a socket that is full leaves the frames
    /// where they are, and the connection is dropped only when the client has taken nothing for [`STALL_LIMIT`] as of
    /// `now`.
    pub(super) fn drain(&mut self, socket: &UnixStream, now: Instant) -> Result<(), Dropped> {
        while let Some(front) = self.frames.front() {
            match net::send(socket, &front[self.offset..], SendFlags::DONTWAIT | SendFlags::NOSIGNAL) {
                Ok(0) => return Err(Dropped::Socket(Errno::PIPE)),
                Ok(taken) => {
                    self.progress = now;
                    self.waiting -= taken;
                    self.offset += taken;
                    if self.offset == front.len() {
                        // Dropped, so wiped.
                        self.frames.pop_front();
                        self.offset = 0;
                    }
                }
                Err(Errno::INTR) => {}
                Err(Errno::AGAIN) => {
                    return if now.saturating_duration_since(self.progress) >= STALL_LIMIT {
                        Err(Dropped::Stalled)
                    } else {
                        Ok(())
                    };
                }
                Err(error) => return Err(Dropped::Socket(error)),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::os::unix::net::UnixStream;

    use gnoblin_proto::{Kind, read_frame};

    use super::*;

    /// An event whose frame is about `size` bytes, and which says which one it is.
    fn frame_of(number: u32, size: usize) -> Envelope {
        Envelope::event("test", format!("frame-{number}"), vec![7; size])
    }

    fn pair() -> (UnixStream, UnixStream) {
        let (ours, theirs) = UnixStream::pair().expect("a socket pair");
        theirs.set_nonblocking(true).expect("a reader that never waits");
        (ours, theirs)
    }

    /// Reads whatever the peer has, without waiting, and returns the members of the frames that are whole.
    fn read_whole_frames(theirs: &mut UnixStream, taken: &mut Vec<u8>) -> Vec<String> {
        let mut chunk = vec![0u8; 64 * 1024];
        loop {
            match theirs.read(&mut chunk) {
                Ok(0) => break,
                Ok(count) => taken.extend_from_slice(&chunk[..count]),
                Err(_) => break,
            }
        }
        let mut members = Vec::new();
        let mut cursor = std::io::Cursor::new(&taken[..]);
        let mut consumed = 0;
        while let Ok(Some(envelope)) = read_frame(&mut cursor) {
            assert!(matches!(envelope.kind, Some(Kind::Event(_))));
            members.push(envelope.member.clone());
            consumed = usize::try_from(cursor.position()).expect("a position");
        }
        taken.drain(..consumed);
        members
    }

    /// Reads as a client does, and gives the outbox back what the socket has room for, until `count` frames have
    /// arrived or ten seconds have passed. Returns the members of the frames, in the order they arrived.
    fn read_all(
        outbox: &mut Outbox,
        ours: &UnixStream,
        theirs: &mut UnixStream,
        now: Instant,
        count: usize,
    ) -> Vec<String> {
        let (mut seen, mut taken) = (Vec::new(), Vec::new());
        let deadline = Instant::now() + Duration::from_secs(10);
        while seen.len() < count {
            assert!(Instant::now() < deadline, "{} of {count} frames arrived", seen.len());
            seen.extend(read_whole_frames(theirs, &mut taken));
            outbox
                .drain(ours, now)
                .expect("the socket takes what the client has read");
        }
        assert!(taken.is_empty(), "no byte of a frame was left over");
        seen
    }

    #[test]
    fn a_client_that_reads_late_gets_every_frame_whole_and_in_order() {
        let (ours, mut theirs) = pair();
        let now = Instant::now();
        let mut outbox = Outbox::new(now);
        // About 4 MB: far more than a socket holds, so the outbox keeps most of it.
        let count = 200;
        for number in 0..count {
            outbox.push(&frame_of(number, 20_000), now).expect("queued");
            outbox
                .drain(&ours, now)
                .expect("a socket that is merely full is not a failure");
        }
        assert!(
            outbox.waiting() > 1_000_000,
            "the socket did not take it all: {}",
            outbox.waiting()
        );

        let seen = read_all(&mut outbox, &ours, &mut theirs, now, count as usize);
        let expected: Vec<String> = (0..count).map(|number| format!("frame-{number}")).collect();
        assert_eq!(seen, expected, "every frame, once, in the order they were queued");
        assert_eq!(outbox.waiting(), 0);
    }

    #[test]
    fn a_client_that_takes_nothing_is_dropped_once_the_stall_limit_has_passed_and_not_before() {
        let (ours, _theirs) = pair();
        let start = Instant::now();
        let mut outbox = Outbox::new(start);
        outbox.push(&frame_of(0, 2_000_000), start).expect("queued");
        outbox.drain(&ours, start).expect("the socket took what it could");
        let waiting = outbox.waiting();
        assert!(waiting > 0, "a frame of 2 MB does not fit in a socket");

        let almost = start + STALL_LIMIT - Duration::from_millis(1);
        outbox
            .drain(&ours, almost)
            .expect("the client has not been stalled for the whole limit yet");
        let error = outbox
            .drain(&ours, start + STALL_LIMIT)
            .expect_err("the client took nothing for the whole limit");
        assert!(matches!(error, Dropped::Stalled), "{error}");
        assert!(error.is_a_stall());
        assert_eq!(outbox.waiting(), waiting, "nothing was taken in between");
    }

    #[test]
    fn a_client_that_reads_a_little_now_and_then_is_not_stalled() {
        let (ours, mut theirs) = pair();
        let start = Instant::now();
        let mut outbox = Outbox::new(start);
        outbox.push(&frame_of(0, 2_000_000), start).expect("queued");
        outbox.drain(&ours, start).expect("queued and offered");

        // The client reads some just before the limit: the limit counts from the last byte it took.
        let later = start + STALL_LIMIT - Duration::from_millis(1);
        let mut taken = Vec::new();
        read_whole_frames(&mut theirs, &mut taken);
        outbox.drain(&ours, later).expect("the socket took more");
        let still = later + STALL_LIMIT - Duration::from_millis(1);
        outbox
            .drain(&ours, still)
            .expect("a client that took bytes a moment ago is not stalled");
    }

    #[test]
    fn a_client_with_more_than_the_bound_waiting_is_dropped_at_once_and_the_memory_is_bounded() {
        let (ours, _theirs) = pair();
        let now = Instant::now();
        let mut outbox = Outbox::new(now);
        let size = 1_000_000;
        let mut queued = 0;
        let error = loop {
            match outbox.push(&frame_of(queued, size), now) {
                Ok(()) => {
                    outbox.drain(&ours, now).expect("the socket is full, not broken");
                    queued += 1;
                    assert!(queued < 100, "the outbox never refused");
                }
                Err(error) => break error,
            }
        };
        assert!(matches!(error, Dropped::Backlogged), "{error}");
        assert!(error.is_a_stall());
        assert!(
            outbox.waiting() <= MAX_OUTBOX,
            "{} bytes waited, the bound is {MAX_OUTBOX}",
            outbox.waiting()
        );
        assert!(
            outbox.waiting() + size > MAX_OUTBOX,
            "the frame that was refused is the one that crossed the bound"
        );
    }

    #[test]
    fn a_client_with_nothing_waiting_is_sent_a_frame_of_the_largest_size() {
        // The client reads as a process does, blocking, on a thread of its own.
        let (ours, mut theirs) = UnixStream::pair().expect("a socket pair");
        let reader = std::thread::spawn(move || {
            (0..2)
                .map(|_| {
                    read_frame(&mut theirs)
                        .expect("a frame decodes")
                        .expect("the stream is still open")
                        .member
                        .clone()
                })
                .collect::<Vec<_>>()
        });
        let now = Instant::now();
        let mut outbox = Outbox::new(now);
        let size = MAX_FRAME_LEN - 4096;
        outbox
            .push(&frame_of(0, size), now)
            .expect("one frame of any size is queued");

        // A second large frame behind it is still within the bound: a client that is reading the first is not behind.
        outbox
            .push(&frame_of(1, size), now)
            .expect("two frames of the largest size are within the bound");

        let deadline = Instant::now() + Duration::from_secs(30);
        while outbox.waiting() > 0 {
            assert!(Instant::now() < deadline, "{} bytes were never taken", outbox.waiting());
            outbox
                .drain(&ours, Instant::now())
                .expect("a client that reads is never stalled");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(reader.join().expect("the reader ends"), ["frame-0", "frame-1"]);
    }

    #[test]
    fn a_peer_that_went_away_is_a_socket_failure_and_not_a_stall() {
        let (ours, theirs) = pair();
        drop(theirs);
        let now = Instant::now();
        let mut outbox = Outbox::new(now);
        outbox.push(&frame_of(0, 100), now).expect("queued");
        let error = outbox.drain(&ours, now).expect_err("nobody reads");
        assert!(matches!(error, Dropped::Socket(_)), "{error}");
        assert!(!error.is_a_stall(), "a client that went away is routine");
    }

    #[test]
    fn a_frame_that_cannot_be_encoded_drops_the_connection_and_queues_nothing() {
        let now = Instant::now();
        let mut outbox = Outbox::new(now);
        let error = outbox
            .push(&frame_of(0, MAX_FRAME_LEN + 1), now)
            .expect_err("a frame over the limit does not encode");
        assert!(matches!(error, Dropped::Unencodable(_)), "{error}");
        assert_eq!(outbox.waiting(), 0);
    }
}
