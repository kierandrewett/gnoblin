//! Serves a recorded tree stream to a [`TreeClient`](crate::TreeClient).
//!
//! A recording is the frames an observer received from the owner, as [`TreeClient::connect_recording`](
//! crate::TreeClient::connect_recording) wrote them: the attach reply, the snapshot and every change batch after it, in
//! order, each as it came off the socket. [`replay`] answers a client's attach with the attach reply and the snapshot,
//! and then holds the change batches back until [`Stepper::step`] releases the next one. A test therefore sees the
//! tree at every point the stream passed through, with the planner or the pointer run between the steps.
//!
//! The recording is of what a real compositor and the stock shell sent. Nothing here invents a node.

use std::io::{self, Write as _};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use gnoblin_proto::tree::tree::SnapshotPayload;
use gnoblin_proto::{LENGTH_PREFIX_LEN, MAX_FRAME_LEN, read_frame};

/// The frames of a recording, and where its snapshot ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recording {
    /// Every frame, as it was on the wire: its length prefix and its envelope.
    frames: Vec<Vec<u8>>,
    /// The index of the frame that completes the snapshot. The frames up to and including it answer the attach.
    snapshot_end: usize,
}

impl Recording {
    /// Splits `bytes` into frames and finds the end of the snapshot.
    ///
    /// # Errors
    ///
    /// The bytes end inside a frame, a frame is over the size limit or does not decode, or the stream holds no
    /// completed snapshot.
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let mut frames = Vec::new();
        let mut rest = bytes;
        while !rest.is_empty() {
            let prefix = rest
                .first_chunk::<LENGTH_PREFIX_LEN>()
                .ok_or("the recording ends inside a length prefix")?;
            let len = u32::from_be_bytes(*prefix) as usize;
            if len > MAX_FRAME_LEN {
                return Err(format!(
                    "a recorded frame of {len} bytes is over the {MAX_FRAME_LEN} byte limit"
                ));
            }
            let total = LENGTH_PREFIX_LEN + len;
            if rest.len() < total {
                return Err("the recording ends inside a frame".to_owned());
            }
            let (frame, tail) = rest.split_at(total);
            frames.push(frame.to_vec());
            rest = tail;
        }
        let mut snapshot_end = None;
        for (index, frame) in frames.iter().enumerate() {
            let envelope = read_frame(&mut frame.as_slice())
                .map_err(|error| format!("recorded frame {index} does not decode: {error}"))?
                .ok_or_else(|| format!("recorded frame {index} is empty"))?;
            if envelope.member == "snapshot" && envelope.payload_as::<SnapshotPayload>().is_some_and(|p| p.last) {
                snapshot_end = Some(index);
                break;
            }
        }
        let snapshot_end = snapshot_end.ok_or("the recording holds no completed snapshot")?;
        Ok(Self { frames, snapshot_end })
    }

    /// How many frames follow the snapshot: the steps [`replay`] holds back.
    #[must_use]
    pub fn steps(&self) -> usize {
        self.frames.len() - self.snapshot_end - 1
    }
}

/// Releases the frames after the snapshot, one at a time.
pub struct Stepper {
    release: Sender<()>,
    written: Receiver<()>,
    remaining: usize,
}

impl Stepper {
    /// Writes the next frame to the client and returns once it is in the socket, so the client's next
    /// [`poll`](crate::TreeClient::poll) reads it. `false` when the recording has no frame left.
    pub fn step(&mut self) -> bool {
        if self.remaining == 0 {
            return false;
        }
        self.remaining -= 1;
        self.release.send(()).is_ok() && self.written.recv().is_ok()
    }

    /// How many frames are still to come.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.remaining
    }
}

/// Starts a fake owner that replays `recording` to the client on the returned stream.
///
/// The attach is answered with the recorded attach reply and snapshot at once, so [`TreeClient::from_stream`](
/// crate::TreeClient::from_stream) returns with the tree as the snapshot had it. The rest comes through the
/// [`Stepper`]. The owner reads and ignores whatever else the client sends.
#[must_use]
pub fn replay(recording: &Recording) -> (UnixStream, Stepper) {
    let (client, mut server) = UnixStream::pair().expect("a socket pair");
    let frames = recording.frames.clone();
    let snapshot_end = recording.snapshot_end;
    let (release, released) = mpsc::channel::<()>();
    let (done, written) = mpsc::channel::<()>();
    let remaining = recording.steps();
    thread::spawn(move || {
        // The client's attach request comes first.
        if !matches!(read_frame(&mut server), Ok(Some(_))) {
            return;
        }
        let mut out = match server.try_clone() {
            Ok(out) => out,
            Err(_) => return,
        };
        for frame in &frames[..=snapshot_end] {
            if write_raw(&mut out, frame).is_err() {
                return;
            }
        }
        // Whatever the client sends later is read and dropped, so its writes never block.
        thread::spawn(move || while let Ok(Some(_)) = read_frame(&mut server) {});
        for frame in &frames[snapshot_end + 1..] {
            if released.recv().is_err() || write_raw(&mut out, frame).is_err() || done.send(()).is_err() {
                return;
            }
        }
    });
    (
        client,
        Stepper {
            release,
            written,
            remaining,
        },
    )
}

fn write_raw(out: &mut UnixStream, frame: &[u8]) -> io::Result<()> {
    out.write_all(frame)
}

#[cfg(test)]
mod tests {
    use gnoblin_proto::tree::tree::{AttachedPayload, ChangesPayload};
    use gnoblin_proto::{Envelope, write_frame};

    use super::*;
    use crate::TreeClient;

    fn wire(envelope: &Envelope) -> Vec<u8> {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, envelope).expect("encode");
        bytes
    }

    fn stream(changes: usize) -> Vec<u8> {
        let mut bytes = wire(&Envelope::reply_message(
            "tree",
            "attach",
            1,
            &AttachedPayload {
                epoch: 1,
                root: 1,
                serial: 1,
            },
        ));
        bytes.extend(wire(&Envelope::event_message(
            "tree",
            "snapshot",
            &SnapshotPayload {
                serial: 1,
                chunk: 0,
                last: true,
                nodes: Vec::new(),
            },
        )));
        for serial in 0..changes as u64 {
            bytes.extend(wire(&Envelope::event_message(
                "tree",
                "changes",
                &ChangesPayload {
                    serial: serial + 2,
                    changes: Vec::new(),
                },
            )));
        }
        bytes
    }

    #[test]
    fn a_recording_is_split_into_frames_with_the_snapshot_found() {
        let recording = Recording::parse(&stream(3)).expect("a valid recording");
        assert_eq!(recording.steps(), 3);
    }

    #[test]
    fn a_truncated_or_snapshotless_recording_is_refused() {
        let whole = stream(1);
        assert!(
            Recording::parse(&whole[..whole.len() - 1]).is_err(),
            "ends inside a frame"
        );
        assert!(Recording::parse(&whole[..2]).is_err(), "ends inside a prefix");
        let attach_only = wire(&Envelope::reply_message(
            "tree",
            "attach",
            1,
            &AttachedPayload {
                epoch: 1,
                root: 1,
                serial: 1,
            },
        ));
        assert!(Recording::parse(&attach_only).is_err(), "no snapshot");
        assert!(
            Recording::parse(&[0, 0, 0, 1, 0xff]).is_err(),
            "a frame that does not decode"
        );
    }

    #[test]
    fn the_client_attaches_and_the_change_batches_arrive_one_step_at_a_time() {
        let recording = Recording::parse(&stream(2)).expect("a valid recording");
        let (socket, mut stepper) = replay(&recording);
        let mut client = TreeClient::from_stream(socket).expect("attach over the replay");

        assert!(
            !client.poll().expect("nothing has arrived"),
            "the batches are held back"
        );
        assert_eq!(stepper.remaining(), 2);
        assert!(stepper.step());
        client.poll().expect("the first batch applies");
        assert!(stepper.step());
        client.poll().expect("the second batch applies");
        assert!(!stepper.step(), "the recording is used up");
        assert_eq!(stepper.remaining(), 0);
    }
}
