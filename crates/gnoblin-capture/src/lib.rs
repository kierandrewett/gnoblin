//! Frame export: capture sessions, the lock embargo, pacing and single-use frames.
//!
//! This is the Renderer card's "session owner" half of `design/rust-compositor.md`:
//! capture and remote-desktop sessions are compositor objects, and a session ends
//! when the client that opened it disconnects. The crate is pure policy — no
//! Smithay, no Wayland type, no clock, no device — so the rules it states can be
//! tested without a session, and a protocol handler cannot perform a frame without
//! them.
//!
//! # What it owns
//!
//! | Module | Owns |
//! | --- | --- |
//! | [`format`] | the buffer a client negotiated: size, stride and pixel layout |
//! | [`session`] | what a session captures, who owns it, and whether it is stopped |
//! | [`source`] | [`CaptureSource`], the one path output and window capture share |
//! | [`table`] | the rules a frame satisfies before it exposes a pixel, and the claims it hands out |
//!
//! # Why the rules live here rather than in the protocol modules
//!
//! The plan's failure modes for capture are all "a path that forgot a rule": a
//! stream that outgrows its session, a frame produced after the lock was applied, a
//! buffer reused while the client is still reading it, a closed window served for
//! ever. A protocol handler that copies pixels itself can forget any of them. Here it
//! cannot, because it never copies: it asks [`Table::request`] for a claim and
//! [`Table::capture`] resolves it, and the table applies every rule first.
//!
//! The embargo in particular is asked **at every frame** ([`Table::capture`] takes
//! the lock's answer as an argument), so a session opened before the lock is refused
//! by its next frame and is *stopped* rather than paused. Releasing the lock
//! therefore does not resume a stream the user was never asked about again.
//!
//! # Consent
//!
//! A session records the [`CaptureStream`] it holds. A protocol handler opens
//! [`CaptureStream::Requested`], because binding a global is not consent: such a
//! session is refused every frame while the lock is up. Only the portal path, which
//! has the user's answer, opens [`CaptureStream::Granted`], and only that stream may
//! continue through a lock — showing the lock scene, never the desktop underneath it.
//! That is D13's split: compat capture goes through the portal after consent, and the
//! native protocols are governed by the per-client access policy in
//! `gnoblin-wayland`.
//!
//! # PipeWire: what is missing, and why it is not stubbed here
//!
//! The plan gives this crate the PipeWire streams of capture and remote-desktop
//! sessions. They are **not** implemented, and there is no placeholder for them.
//! The decision, with the evidence behind it:
//!
//! - The prerequisites are present on this machine: `libpipewire-0.3` 1.4.11 and
//!   `libspa-0.2` development files, a running `pipewire` daemon, and the `pipewire`
//!   crate (0.10.1) in the local registry.
//! - The `pipewire` crate cannot be built with the pinned toolchain as it stands:
//!   `pipewire-sys` and `libspa-sys` run `bindgen` at build time, and bindgen 0.72.1
//!   driving the installed libclang 21 fails on glibc's `bits/floatn.h` with
//!   `__float128 is not supported on this target`, then panics on a size assertion.
//!   Both cached versions of the crate (0.9.2 and 0.10.1) fail identically. The build
//!   succeeds only with `LIBCLANG_PATH` pointed at an older libclang
//!   (`/usr/lib64/llvm20/lib64` here).
//! - A *tested* producer needs more than a build: a hermetic test needs its own
//!   `pipewire` instance (a private `PIPEWIRE_RUNTIME_DIR` and a daemon started by the
//!   test) plus a consumer stream in the same process to receive a frame and compare
//!   it, because nothing in this workspace can otherwise assert that a stream
//!   produces anything. Pointing a test at the developer's live session would make
//!   `cargo test` depend on and touch that session, which the plan forbids.
//!
//! The exact next step, in the order it has to happen: (1) make the dependency
//! buildable without a per-machine environment variable, by pinning a `bindgen` that
//! copes with the installed libclang or by moving to a `pipewire` release whose bindings
//! are pregenerated; (2) add a `Stream` producer behind [`CaptureSource`] — it drives
//! [`Table::request`] and [`Table::capture`] with the stream's buffer as the claim's
//! buffer, so no rule is duplicated and the embargo applies to it unchanged; (3) test
//! it with a private daemon and one consumer, asserting a frame's pixels and that
//! stopping the session stops the stream. Until all three exist, a PipeWire stream is
//! a missing feature rather than a function that returns nothing.
//!
//! # Failure modes first
//!
//! Each is a test in the module it belongs to; the plan's four are
//! `a_stream_that_outlives_its_session_copies_nothing`,
//! `a_frame_after_the_lock_is_refused_and_stops_the_session`,
//! `a_buffer_that_is_still_claimed_is_refused` and
//! `a_closed_window_stops_its_session`, all in [`table`].

pub mod format;
pub mod session;
pub mod source;
pub mod table;

pub use format::{Format, PixelFormat};
pub use session::{BufferId, Refusal, Session, SessionId, Stop, Target};
pub use source::{CaptureSource, Copied};
pub use table::{FrameClaim, FrameReport, Outcome, Table};
