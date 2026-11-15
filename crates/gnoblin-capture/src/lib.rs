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
//! # Failure modes: the capture embargo
//!
//! The list the plan asks for before isolated tests or code (`design/rust-compositor.md`, "Session lock" and "Renderer"
//! cards: "capture requested during locking", "model tests show no reachable state where a non-lock surface ... is
//! captured while not unlocked"). It is built from the rules the design states and checked against the code, not from
//! what happens to be tested:
//!
//! - the lock card and the lock's own list ([`gnoblin_lock`], entries 10 and 11), which give the states and who may see
//!   what in each;
//! - `design/capture-api.md`: "Locked session refuses" (every target kind, the check before any pixel is read, a View
//!   stream stopped and not resumed), "Secure treatment", and the consent and ownership rules for streams;
//! - `design/layers.md`, "Capture paths" and "Invariants": secure content is never drawn into any capture, whoever asks, and
//!   the lock surface is secure;
//! - `design/supersample.md`, "Captures": a capture is the composed frame, `mode * supersample` on each axis, and secure
//!   treatment is applied before any resampling.
//!
//! Each entry says what fails, then names the test that would catch it. A test in another crate says which. The paths
//! that read pixels are the table here ([`Table::capture`]), the still ([`still`]), the protocol handlers and the PipeWire
//! producer in `gnoblin-wayland` and `gnoblin-pipewire-export`, the IPC `capture.screen` in `gnoblin-compositor`, and the
//! compat adapters; an entry exists for each place a rule could be forgotten.
//!
//! ## The lock transition
//!
//! 1. **Capture while the lock is being established.** `Locking` is covered: a request that holds no grant and a stream
//!    that does are both refused: `capture_during_locking_is_refused` (`gnoblin-lock`),
//!    `no_session_copies_the_desktop_while_the_lock_is_not_released` (`tests/lock_cycle.rs`, the real lock driving the
//!    real table through a whole cycle) and `a_capture_while_the_lock_is_being_established_is_refused_by_name`
//!    (`gnoblin-compositor/tests/capture_screen.rs`, the `capture.screen` still).
//! 2. **A frame is produced after the lock.** The embargo is asked at every frame and the session stops, so there is no
//!    "paused": `a_frame_after_the_lock_is_refused_and_stops_the_session`.
//! 3. **A session opened while the lock is up gets a first frame.** Opening is not consent:
//!    `a_session_opened_under_the_lock_never_copies_a_frame`, `a_capture_during_the_lock_is_refused` and
//!    `a_gpu_capture_session_opened_under_the_lock_is_stopped_and_offers_nothing` (`gnoblin-wayland`, `tests/capture.rs` and
//!    `tests/dmabuf_capture.rs`).
//! 4. **Releasing the lock resumes a stream the user was not asked about again.** A stop is final:
//!    `a_stopped_session_never_resumes_after_the_unlock`, `a_view_stream_is_stopped_by_the_lock_and_never_resumes` and
//!    `no_session_copies_the_desktop_while_the_lock_is_not_released`.
//! 5. **A stream nobody reads outlives the lock.** A frame is where the lock is applied, and a stream nobody reads is
//!    never asked for one, so the lock is applied to every session on every loop turn:
//!    `the_lock_is_applied_to_a_session_that_is_never_asked_for_a_frame`,
//!    `the_lock_ends_a_view_stream_nobody_reads_and_leaves_an_output_stream_alone` and
//!    `the_lock_ends_a_window_stream_nobody_reads` (`gnoblin-pipewire-export`).
//! 6. **A granted stream is shown the desktop under the cover.** It sees the presented lock scene and only from `Locked`:
//!    `granted_stream_sees_only_the_lock_scene` (`gnoblin-lock`), `a_granted_output_stream_sees_the_lock_scene` and
//!    `a_granted_output_sees_the_lock_scene_only_and_only_while_locked`.
//! 7. **A granted window or View stream is fed the lock scene.** A window is desktop content and the scene is not a window:
//!    `a_granted_window_stream_is_refused_under_the_lock` and `a_granted_view_stream_is_refused_under_the_lock`.
//! 8. **A failed cover or a replacement locker hands a stopped stream back.** `CoverFailed` refuses even the scene, and a
//!    takeover needs a fresh cover: `a_failed_cover_and_a_takeover_never_hand_a_stopped_stream_back`,
//!    `cover_failure_keeps_the_cover` and `takeover_requires_a_fresh_cover` (`gnoblin-lock`).
//! 9. **A picture produced before the lock is delivered after it.** A buffer queued before the lock, a screencopy requested
//!    before it, and a View frame a reader had not yet taken are all dropped:
//!    `no_buffer_queued_before_the_lock_is_delivered_after_it_on_a_real_cast`,
//!    `the_lock_cuts_every_reader_of_a_view_and_none_is_served_a_picture_from_before_it` (`gnoblin-wayland`,
//!    `tests/view_capture_e2e.rs`), `a_gpu_screencopy_requested_before_the_lock_is_refused_after_it` and
//!    `a_lock_stops_a_gpu_capture_session_and_nothing_is_written_into_the_buffer` (`tests/dmabuf_capture.rs`).
//! 10. **A refusal is answered with the previous frame's pixels.** `a_refused_frame_never_repeats_the_previous_pixels`
//!     (`tests/capture.rs`) and `a_stream_started_before_the_lock_stops`.
//! 11. **A new cast starts under the lock.** The user consented to a cast of a desktop that is no longer the one shown:
//!     `a_view_cast_is_refused_while_the_session_is_locked`, `the_lock_ends_a_view_cast_that_is_running`
//!     (`gnoblin-compositor/tests/cast.rs`), `the_session_lock_ends_a_cast_of_a_view_and_refuses_a_new_one` and
//!     `the_session_lock_ends_a_cast_of_a_window_and_refuses_a_new_one` (`tests/view_capture_e2e.rs`).
//! 12. **A still is taken locked or embargoed.** The lock is asked before a pixel is read: the pure choice is
//!     `a_locked_session_refuses_even_when_the_embargo_allows_capture` and
//!     `an_embargoed_session_refuses_even_when_unlocked` ([`still`]); the end-to-end refusal by name is entry 1's
//!     `capture_screen` test.
//! 13. **A still reads pixels the lock covered after the caller looked.** The IPC handler checks a snapshot of the session,
//!     and the loop reads the pixels later; the loop asks the lock again where it reads:
//!     `a_guarded_screenshot_is_refused_when_the_loop_finds_the_session_covered` (`gnoblin-wayland`, `frame::tests`).
//! 14. **An adapter skips the embargo.** Every adapter member admits through the same gate, in every mode:
//!     `capture_is_refused_while_locked` and `capture_during_the_lock_transition_is_refused`
//!     (`gnoblin-compat-core/tests/failure_modes.rs`), `a_screenshot_is_refused_while_the_session_is_locked`
//!     (`gnoblin-compat-dbus-gnome-shell-screenshot`) and `locking_ends_every_session_and_refuses_a_new_one`
//!     (`gnoblin-compat-dbus-mutter-screencast`).
//!
//! ## Sessions, claims and frames
//!
//! 15. **A stream outlives its session.** `a_stream_that_outlives_its_session_copies_nothing`.
//! 16. **A session outlives the client that opened it,** so a crashed adapter leaves a stream running:
//!     `closing_an_owner_closes_only_its_sessions`, `a_session_ends_when_its_client_disconnects` (`tests/capture.rs`),
//!     `owner_disconnect_tears_the_stream_down` (`gnoblin-pipewire-export`) and
//!     `a_cast_ends_when_the_connection_that_held_it_goes_away` (`gnoblin-compositor/tests/cast.rs`).
//! 17. **A buffer that is still claimed is claimed again,** and the client reads a torn frame:
//!     `a_buffer_that_is_still_claimed_is_refused`.
//! 18. **A claim is replayed after it was superseded or spent.** `a_claim_from_an_earlier_tick_is_stale`,
//!     `a_stale_claim_from_an_earlier_tick_is_dropped_on_the_next_request` and `a_frame_is_copied_once`.
//! 19. **A session changes shape while a claim is out.** The claim names a buffer sized for the old shape, so copying the
//!     new shape into it would write past what the client made. `a_reformat_drops_a_claim_taken_for_the_old_shape`; the
//!     new shape reports all of itself as damage (`a_reformatted_session_copies_the_new_size_and_reports_all_of_it`) and a
//!     stopped session is not revived with damage (`a_reformat_leaves_a_stopped_session_with_no_damage`).
//! 20. **A client asks in a loop and spins the copy path.** One frame per tick per session: `a_session_produces_one_frame_per_tick`;
//!     and a consumer that stops reading never blocks the producer: `a_stalled_consumer_of_an_output_never_blocks_the_pump`.
//! 21. **A session identity is reused,** so a stale frame is taken for the new session's: `identities_are_never_reused`.
//! 22. **Damage lies.** A failed copy loses it, one that falls outside the buffer is reported, or one target's reaches
//!     another's: `a_failed_copy_keeps_the_damage_for_the_next_frame`, `damage_outside_the_region_is_clipped_away`,
//!     `the_second_frame_reports_only_the_changed_region` and `view_damage_does_not_reach_other_targets`.
//!
//! ## Targets
//!
//! 23. **A closed window or an unplugged output keeps being served.** `a_closed_window_stops_its_session`,
//!     `closing_a_target_closes_its_sessions`, `a_window_source_follows_its_window_and_ends_with_it` (`tests/capture.rs`),
//!     `a_window_stream_ends_when_the_window_is_gone` (`gnoblin-pipewire-export`),
//!     `a_cast_of_a_window_ends_cleanly_when_the_window_is_closed` and `destroying_the_view_ends_its_cast`
//!     (`tests/view_capture_e2e.rs`, `gnoblin-compositor/tests/cast.rs`).
//! 24. **A target with nothing inside the negotiated buffer reports a frame it never wrote.**
//!     `a_target_with_nothing_inside_the_buffer_stops_its_session`.
//! 25. **A frame carries pixels past the target's own rectangle,** so a window capture shows what lies around it:
//!     `a_frame_never_carries_pixels_past_the_targets_own_rectangle` (`gnoblin-pipewire-export`) and
//!     `a_capture_of_a_placed_window_is_its_placed_rectangle` (`tests/capture.rs`).
//! 26. **A target kind is read as another.** An output number that is a View's low half, or a View that is not in the scene:
//!     `a_view_is_never_read_as_an_output_and_an_output_never_as_a_view`,
//!     `a_cast_of_a_view_that_is_not_in_the_scene_is_refused_no_view` and
//!     `a_cast_of_an_output_that_is_not_there_is_refused` (`gnoblin-compositor/tests/cast.rs`).
//!
//! ## Consent and access
//!
//! 27. **A stream starts with no consent,** or for a window nobody asked the user about:
//!     `ungranted_session_is_refused_and_no_node_appears` (`gnoblin-pipewire-export`),
//!     `a_start_with_no_recorded_consent_is_refused_and_makes_no_node` and
//!     `a_window_grant_for_a_window_nobody_asked_the_user_about_is_refused_and_makes_no_node`
//!     (`gnoblin-compositor/tests/cast.rs`).
//! 28. **One caller uses another's session or node.** `only_the_owner_reads_the_node_id_and_starts_the_stream`,
//!     `a_second_caller_gets_its_own_node_and_never_another_callers` and
//!     `another_connection_can_neither_start_nor_end_a_cast_it_did_not_decide`; a grant that is revoked ends the stream
//!     (`revoked_grant_tears_the_stream_down`).
//! 29. **A sandboxed client binds a capture global:** `a_sandboxed_client_cannot_bind_the_capture_globals`
//!     (`gnoblin-wayland`, `tests/capture.rs`).
//!
//! ## Secure content
//!
//! 30. **A secure window's pixels reach a capture.** Output, window, GPU and software, a subsurface that hangs past it, a
//!     View over it, a View of it and a PNG of it: `a_window_the_shell_marks_secure_is_masked_in_its_capture`,
//!     `a_secure_window_masks_a_subsurface_hanging_past_it_in_a_capture_of_the_output`,
//!     `a_secure_window_is_masked_in_a_gpu_capture`, `a_view_of_a_secure_window_is_masked_in_a_capture_of_the_output`
//!     (`tests/capture.rs`, `tests/dmabuf_capture.rs`), `a_png_of_a_view_holds_none_of_a_secure_windows_pixels`,
//!     `a_view_role_session_never_receives_a_secure_windows_pixels`, `a_secure_gpu_window_never_reaches_a_capture_of_a_view`,
//!     `a_cast_of_a_window_never_carries_the_pixels_of_a_secure_window` (`tests/view_capture_e2e.rs`) and, below the
//!     protocols, `a_capture_masks_a_secure_layer_and_keeps_an_ordinary_overlay` (`gnoblin-backend`).
//! 31. **A secure node above a View's source leaks into one of its readers:**
//!     `a_node_above_the_views_source_that_is_secure_masks_every_reader_of_the_view`.
//! 32. **A mark arrives while a stream runs.** The stream ends before the mark is acknowledged, nothing of the window
//!     arrives after it, and a clear shows the window again only through a new frame:
//!     `a_cast_of_a_view_ends_when_a_window_in_it_is_marked_secure`,
//!     `a_mark_ends_the_cast_and_nothing_with_the_window_arrives_after_the_acknowledgement` (`tests/view_capture_e2e.rs`) and
//!     `a_shell_that_clears_the_secure_mark_unmasks_the_window` (`tests/capture.rs`).
//! 33. **The `omit` treatment is wrong.** It draws nothing for the node and shows the underlay, by design:
//!     `a_capture_omits_a_secure_layer_when_the_session_says_so` (`gnoblin-backend`) and
//!     `a_secure_window_is_omitted_from_a_gpu_capture_when_the_treatment_says_so` (`tests/dmabuf_capture.rs`).
//! 34. **Secure content is masked after resampling,** so an averaged pixel carries a trace of the layer's own:
//!     `a_supersampled_capture_masks_a_secure_layer_before_it_averages` (`gnoblin-backend`, at factors 2 and 3/2).
//! 35. **The lock surface is in a capture.** It is secure, so a capture shows the mask where it is, never its pixels:
//!     `a_lock_surface_is_drawn_over_the_cover` (`gnoblin-wayland`, `tests/session_lock.rs`).
//!
//! ## Capture sizes
//!
//! 36. **A capture advertises or follows the wrong size.** The composed frame is `mode * supersample`, a stream of a View
//!     or a window follows what is drawn and an output's keeps the size it started with:
//!     `the_capture_protocols_advertise_the_composed_size`,
//!     `the_composed_frame_is_hi_res_and_the_display_frame_is_its_exact_box_average` (`tests/supersample.rs`),
//!     `a_view_stream_follows_the_size_the_view_is_drawn_at`, `a_window_stream_follows_the_size_the_window_is_drawn_at` and
//!     `an_output_stream_keeps_the_size_it_started_with` (`gnoblin-pipewire-export`).
//!
//! ## Not covered, and why
//!
//! Each of these is a gap in what exists, not a rule with a test missing:
//!
//! - **`capture.still` (window and region targets).** `design/capture-api.md` says phase 1 is the only phase to build and
//!   that window stills must not ship until the secure-window question is answered. Nothing implements it, so no test
//!   exists and none is faked. When it lands, entries 1, 12 and 30 apply to it and it needs its own.
//! - **The privacy indicator for a still.** `design/capture-api.md` says whether `org.gnoblin.Privacy` reports a still at
//!   all is unverified, so the "signal before pixels are read" rule has nothing to be tested against.
//! - **DRM capture.** `Drm::capture` is `None` (`design/supersample.md`), so no output is screenshot-able on hardware at any
//!   factor. That refuses every capture; it does not leak one.
//! - **The magnifier.** `design/capture-api.md` says whether the output screenshot includes the magnifier's zoom is
//!   unverified. No test pins either answer.
//! - **A PipeWire frame a consumer already took.** The producer cannot recall it, so the secure-mark and lock guarantees
//!   do not reach it (the `gnoblin-pipewire-export` crate docs state this).
//!
//! The lock's own list cross-links here: see `gnoblin-lock`, "Failure modes first", entries 10 and 11.

/// The buffer a client negotiated: size, stride and pixel layout.
pub mod format;
/// Capture sessions: what each captures, who owns it and whether it is stopped.
pub mod session;
/// The [`CaptureSource`] seam that output capture and window capture share.
pub mod source;
/// Still pictures: one frame of an output, encoded as PNG.
pub mod still;
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
