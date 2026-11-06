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
//! # PipeWire: what is proven, what is missing, and where it goes
//!
//! No file in `src/` speaks PipeWire yet, and there is no placeholder for it. The
//! decision and its evidence are in `design/pipewire-capture-spike.md`:
//!
//! - The round trip is proven by a spike. `tests/pipewire_stream.rs` starts a private
//!   `pipewire` daemon, runs a producer and a consumer stream in one process and
//!   checks every frame's bytes. The spike uses the `pipewire` crate directly, not
//!   this crate's code. `pipewire` is a dev-dependency, so no shipped binary links it.
//! - The `pipewire` crate did not build on the first workstation it was tried on:
//!   `pipewire-sys` and `libspa-sys` run `bindgen`, which failed on glibc's
//!   `bits/floatn.h` with the installed libclang 21 and passed only with
//!   `LIBCLANG_PATH` pointing at an older libclang. The spike itself ran on an
//!   Agent37 instance.
//! - The real producer belongs in an engine-side crate, not here. This crate holds
//!   policy and no mechanism. A producer there implements [`CaptureSource`] and drives
//!   [`Table::request`] and [`Table::capture`], so every rule in this crate applies to
//!   it unchanged.
//! - Shipping needs a packaging decision, because a release binary would load the
//!   system libpipewire at run time.
//!
//! Until the producer exists, a PipeWire stream is a missing feature, not a function
//! that returns nothing.
//!
//! # Failure modes first
//!
//! Each is a test in the module it belongs to; the plan's four are
//! `a_stream_that_outlives_its_session_copies_nothing`,
//! `a_frame_after_the_lock_is_refused_and_stops_the_session`,
//! `a_buffer_that_is_still_claimed_is_refused` and
//! `a_closed_window_stops_its_session`, all in [`table`].

/// The buffer a client negotiated: size, stride and pixel layout.
pub mod format;
/// Capture sessions: what each captures, who owns it and whether it is stopped.
pub mod session;
/// The [`CaptureSource`] seam that output capture and window capture share.
pub mod source;
/// The session table, which applies every rule before a frame exposes pixels.
pub mod table;

/// The buffer format and pixel layout types from [`format`].
pub use format::{Format, PixelFormat};
/// Session identities, the buffer id, the refusal and stop reasons, and the session value.
pub use session::{BufferId, Refusal, Session, SessionId, Stop, Target};
/// The source seam and the copy result a source reports, from [`source`].
pub use source::{CaptureSource, Copied};
/// The table, its claims and outcomes, and the per-frame report, from [`table`].
pub use table::{CaptureOutcome, FrameClaim, FrameReport, Table};
