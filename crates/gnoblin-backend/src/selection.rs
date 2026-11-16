//! The selection channel: the clipboard and the primary selection, shared between a nested session and its host.
//!
//! A nested session is a Wayland client of its host, so it can offer the host a selection and read the host's. The
//! contract has four calls, two in each direction, and no payload ever sits in this crate's memory:
//!
//! | Direction        | Call                                                                              |
//! | ---------------- | --------------------------------------------------------------------------------- |
//! | host to session  | [`Host::host_selection`](crate::Host::host_selection): the host's selection changed |
//! | session to host  | [`Backend::session_selection`](crate::Backend::session_selection): a client of the session set or cleared one |
//! | session reads    | [`Backend::read_host_selection`](crate::Backend::read_host_selection): a client of the session pastes what the host holds |
//! | host reads       | [`Host::session_selection_wanted`](crate::Host::session_selection_wanted): the host pastes what a client of the session holds |
//!
//! The first two carry only the list of mime types on offer. The bytes move when somebody pastes: a pipe is made, one
//! side is given to whoever holds the data, and [`transfer`] moves at most [`MAX_TRANSFER_BYTES`] from the pipe to the
//! reader. Nothing is read ahead, so a gigabyte copy that nobody pastes costs nothing.
//!
//! # What crosses, and when
//!
//! - **Only while the window has the host's keyboard.** The host sends its selection to the client that has keyboard
//!   focus, so the session sees the host's clipboard from the moment its window is focused until the moment it is
//!   not: leaving withdraws it. A selection a client of the session sets while the window is not focused is not offered
//!   to the host, now or when the window is focused later, so a background program cannot overwrite the person's
//!   clipboard. A selection offered to the host while focused stays the host's until something replaces it, which is
//!   what copying in the window and pasting elsewhere is. A copy reaches the host a moment after the program makes it,
//!   and a host may refuse one that arrives after the keyboard has moved (this repository's compositor does): it is
//!   then lost.
//! - **Only the types in [`bridged_mimes`].** Text and common images. File lists, application types and anything with
//!   a path or an identity in it do not cross.
//! - **Never a selection that carries [`PRIVATE_MIME`].** A password manager marks a copy as secret by offering that
//!   type; the whole selection stays where it is, in both directions.
//! - **At most [`MAX_TRANSFER_BYTES`] per paste**, and a paste that goes [`TRANSFER_STALL`] without a byte moving ends.
//!   The reader gets what moved before the limit, as a closed pipe, which is how a truncated paste looks to any client.
//! - **Not while the session is locked.** The compositor refuses both directions, as it does for Xwayland.

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rustix::event::{Nsecs, PollFd, PollFlags, Secs, Timespec, poll};

/// The most bytes one paste moves between the host and the session.
pub const MAX_TRANSFER_BYTES: u64 = 16 * 1024 * 1024;

/// How long a paste may go without a byte moving before it ends.
pub const TRANSFER_STALL: Duration = Duration::from_secs(10);

/// The type a password manager offers to say a copy is secret. A selection that offers it is not bridged at all.
pub const PRIVATE_MIME: &str = "x-kde-passwordManagerHint";

/// Which of the two selections a call is about.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SelectionTarget {
    /// The clipboard: copy and paste.
    Clipboard,
    /// The primary selection: select and middle-click.
    Primary,
}

/// Why a read of the host's selection was refused. The same words on every backend that has no host.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelectionRefusal {
    /// The backend has no host to share a selection with.
    Unsupported,
    /// The host holds no selection, or the window no longer has the host's keyboard, so there is nothing to read.
    NoOffer,
    /// The type asked for is not one of those that cross.
    MimeNotBridged,
}

impl std::fmt::Display for SelectionRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Unsupported => "this backend has no host selection to read",
            Self::NoOffer => "the host offers no selection to this window",
            Self::MimeNotBridged => "that type is not shared with the host",
        })
    }
}

impl std::error::Error for SelectionRefusal {}

/// The mime types of a selection that may cross, in the order they were offered.
///
/// The list is empty when the selection offers [`PRIVATE_MIME`]: it stays on its own side. A type is kept when it is
/// one of the text names (`text/plain`, with or without a charset, `UTF8_STRING`, `STRING`, `TEXT`), `text/html`, or
/// `image/png` and `image/jpeg`. The comparison ignores case and the spaces around a parameter.
#[must_use]
pub fn bridged_mimes(offered: &[String]) -> Vec<String> {
    if offered.iter().any(|mime| mime.eq_ignore_ascii_case(PRIVATE_MIME)) {
        return Vec::new();
    }
    offered.iter().filter(|mime| is_bridged(mime)).cloned().collect()
}

/// Whether one mime type crosses.
#[must_use]
pub fn is_bridged(mime: &str) -> bool {
    let normal: String = mime
        .chars()
        .filter(|character| !character.is_whitespace())
        .map(|character| character.to_ascii_lowercase())
        .collect();
    matches!(
        normal.as_str(),
        "text/plain"
            | "text/plain;charset=utf-8"
            | "text/plain;charset=us-ascii"
            | "utf8_string"
            | "string"
            | "text"
            | "text/html"
            | "text/html;charset=utf-8"
            | "image/png"
            | "image/jpeg"
    )
}

/// How a transfer ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TransferEnd {
    /// The writer closed its end: everything it had was moved.
    Complete,
    /// [`MAX_TRANSFER_BYTES`] moved and the writer had more: the reader sees a closed pipe.
    Truncated,
    /// Nothing moved for [`TRANSFER_STALL`], from either side.
    Stalled,
    /// The reader went away or a descriptor failed.
    Failed,
}

/// What a transfer did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Transferred {
    /// How many bytes moved.
    pub bytes: u64,
    /// Why it stopped.
    pub end: TransferEnd,
}

/// Moves what `from` produces into `to` on a thread of its own, within the limits of this module, and returns the
/// thread. Both descriptors are closed when it ends, so the reader of `to` sees the end of the data however it ended.
pub fn transfer(from: OwnedFd, to: OwnedFd) -> JoinHandle<Transferred> {
    std::thread::spawn(move || copy_capped(from, to, MAX_TRANSFER_BYTES, TRANSFER_STALL))
}

/// [`transfer`] on the calling thread, with explicit limits.
#[must_use]
pub fn copy_capped(from: OwnedFd, to: OwnedFd, cap: u64, stall: Duration) -> Transferred {
    let mut source = File::from(from);
    let mut sink = File::from(to);
    let mut buffer = [0u8; 4096];
    let mut bytes = 0u64;
    let end = loop {
        if bytes >= cap {
            // The cap is exact: one more readable byte means the writer had more than the limit.
            break match wait(source.as_fd(), PollFlags::IN, stall) {
                Ready::Yes => match source.read(&mut buffer[..1]) {
                    Ok(0) => TransferEnd::Complete,
                    Ok(_) => TransferEnd::Truncated,
                    Err(_) => TransferEnd::Failed,
                },
                Ready::Timeout => TransferEnd::Stalled,
                Ready::Error => TransferEnd::Failed,
            };
        }
        match wait(source.as_fd(), PollFlags::IN, stall) {
            Ready::Yes => {}
            Ready::Timeout => break TransferEnd::Stalled,
            Ready::Error => break TransferEnd::Failed,
        }
        let room = usize::try_from((cap - bytes).min(buffer.len() as u64)).unwrap_or(buffer.len());
        let count = match source.read(&mut buffer[..room]) {
            Ok(0) => break TransferEnd::Complete,
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break TransferEnd::Failed,
        };
        // A pipe is writable when a page is free, so a write of at most a page after the poll does not block.
        match wait(sink.as_fd(), PollFlags::OUT, stall) {
            Ready::Yes => {}
            Ready::Timeout => break TransferEnd::Stalled,
            Ready::Error => break TransferEnd::Failed,
        }
        if sink.write_all(&buffer[..count]).is_err() {
            break TransferEnd::Failed;
        }
        bytes += count as u64;
    };
    Transferred { bytes, end }
}

/// The result of waiting on one descriptor.
enum Ready {
    Yes,
    Timeout,
    Error,
}

/// Waits until `fd` is ready for `flags`, hung up, or `limit` passes.
fn wait(fd: std::os::fd::BorrowedFd<'_>, flags: PollFlags, limit: Duration) -> Ready {
    let deadline = Instant::now() + limit;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ready::Timeout;
        }
        let timeout = Timespec {
            tv_sec: Secs::try_from(left.as_secs()).unwrap_or(Secs::MAX),
            tv_nsec: Nsecs::from(left.subsec_nanos()),
        };
        let mut fds = [PollFd::new(&fd, flags)];
        match poll(&mut fds, Some(&timeout)) {
            Ok(0) => return Ready::Timeout,
            Ok(_) => {
                let seen = fds[0].revents();
                if seen.contains(PollFlags::NVAL) {
                    return Ready::Error;
                }
                // A hang-up or an error is ready too: the read that follows says which.
                return Ready::Yes;
            }
            Err(rustix::io::Errno::INTR) => {}
            Err(_) => return Ready::Error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pipe() -> (OwnedFd, OwnedFd) {
        rustix::pipe::pipe().expect("a pipe")
    }

    fn strings(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn text_and_images_cross_and_file_lists_do_not() {
        let offered = strings(&[
            "text/plain;charset=utf-8",
            "UTF8_STRING",
            "text/uri-list",
            "application/x-gnome-copied-files",
            "image/png",
            "TEXT",
        ]);
        assert_eq!(
            bridged_mimes(&offered),
            strings(&["text/plain;charset=utf-8", "UTF8_STRING", "image/png", "TEXT"])
        );
    }

    #[test]
    fn a_secret_copy_crosses_whole_or_not_at_all() {
        let offered = strings(&["text/plain", PRIVATE_MIME]);
        assert!(
            bridged_mimes(&offered).is_empty(),
            "the hint keeps the whole selection back"
        );
        assert!(bridged_mimes(&strings(&["text/plain", "X-KDE-PasswordManagerHint"])).is_empty());
    }

    #[test]
    fn a_type_is_compared_without_case_or_spaces() {
        assert!(is_bridged("Text/Plain; charset=UTF-8"));
        assert!(!is_bridged("text/plain;charset=latin1"));
    }

    #[test]
    fn a_transfer_moves_what_the_writer_wrote() {
        let (host_read, host_write) = pipe();
        let (client_read, client_write) = pipe();
        let mut writer = File::from(host_write);
        writer.write_all(b"hello from the host").expect("write");
        drop(writer);
        let done = copy_capped(host_read, client_write, 1024, Duration::from_secs(5));
        assert_eq!(
            done,
            Transferred {
                bytes: 19,
                end: TransferEnd::Complete
            }
        );
        let mut got = String::new();
        File::from(client_read).read_to_string(&mut got).expect("read");
        assert_eq!(got, "hello from the host");
    }

    #[test]
    fn a_transfer_past_the_cap_is_cut_and_says_so() {
        let (host_read, host_write) = pipe();
        let (client_read, client_write) = pipe();
        let mut writer = File::from(host_write);
        writer.write_all(&[7u8; 3000]).expect("write");
        drop(writer);
        let done = copy_capped(host_read, client_write, 1000, Duration::from_secs(5));
        assert_eq!(
            done,
            Transferred {
                bytes: 1000,
                end: TransferEnd::Truncated
            }
        );
        let mut got = Vec::new();
        File::from(client_read).read_to_end(&mut got).expect("read");
        assert_eq!(got.len(), 1000, "the reader sees exactly the cap and then the end");
    }

    #[test]
    fn a_payload_of_exactly_the_cap_is_complete() {
        let (host_read, host_write) = pipe();
        let (client_read, client_write) = pipe();
        let mut writer = File::from(host_write);
        writer.write_all(&[1u8; 1000]).expect("write");
        drop(writer);
        let done = copy_capped(host_read, client_write, 1000, Duration::from_secs(5));
        assert_eq!(done.end, TransferEnd::Complete);
        let mut got = Vec::new();
        File::from(client_read).read_to_end(&mut got).expect("read");
        assert_eq!(got.len(), 1000);
    }

    #[test]
    fn a_writer_that_goes_quiet_ends_the_transfer() {
        let (host_read, _host_write) = pipe();
        let (_client_read, client_write) = pipe();
        let done = copy_capped(host_read, client_write, 1000, Duration::from_millis(50));
        assert_eq!(
            done,
            Transferred {
                bytes: 0,
                end: TransferEnd::Stalled
            }
        );
    }

    #[test]
    fn a_reader_that_never_reads_ends_the_transfer() {
        let (host_read, host_write) = pipe();
        let (_client_read, client_write) = pipe();
        // More than a pipe holds, so the sink fills and the write side stalls.
        let writer = std::thread::spawn(move || {
            let mut writer = File::from(host_write);
            let _ = writer.write_all(&vec![3u8; 1 << 20]);
        });
        let done = copy_capped(host_read, client_write, 4 << 20, Duration::from_millis(100));
        assert_eq!(done.end, TransferEnd::Stalled);
        assert!(
            done.bytes > 0 && done.bytes < 1 << 20,
            "some moved before the pipe filled"
        );
        drop(writer);
    }
}
