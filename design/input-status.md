# Input status

Checked against the code on `rust-compositor`. Each row cites the code or test that
backs it. Status words:

- `done`: implemented and covered by a test that runs without hardware.
- `partial`: some of the path exists; the rest is named below.
- `absent`: no code path.
- `needs hardware`: the remaining behaviour can only be checked on a real device.
  The row names the check to run.

## Summary

| #   | Claim                                    | Status                                   | What exists                                                                                                                             |
| --- | ---------------------------------------- | ---------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | Scroll reaches the pointer's window      | done                                     | Pixel and step scroll go to the client under the pointer. Nothing reaches a client when no window is under it.                          |
| 2   | Touch points reach the window under them | done (protocol), needs hardware (device) | `wl_touch` down, motion, up and frame reach the window under the finger, in window-local coordinates. No real touchscreen has run this. |
| 3   | Touch cancel ends every point            | done                                     | Cancel reaches every client holding a touch, on a session cover, and when a point's window closes.                                      |
| 4   | Touch shape and orientation              | absent                                   | `wl_touch.shape` and `wl_touch.orientation` are never sent. The backends report position only.                                          |
| 5   | Pointer lock and relative motion         | done                                     | A locked pointer reports relative motion, releases when focus moves, and releases on a session lock.                                    |
| 6   | Key bindings and key delivery            | done                                     | Bound keys are consumed. Keys carry their evdev code. A keymap change keeps the held key's target.                                      |
| 7   | Pointer gestures carrying data           | absent                                   | The gesture interfaces bind. No hold, pinch or swipe is reported by any backend.                                                        |
| 8   | Cursor drawing                           | absent                                   | A cursor shape request is recorded and never painted: a headless output has no pointer plane.                                           |

## 1. Scroll

What exists:

- A scroll over a window is sent to the pointer's client, in pixels and in steps.
  Test `a_scroll_over_a_window_reaches_the_pointer_in_pixels_and_steps`,
  `crates/gnoblin-wayland/tests/protocols.rs:992`.
- A scroll that moved nothing is not sent: `:1016`.
- A scroll with no window under the pointer reaches nobody: `:1042`.

Run: `cargo test -p gnoblin-wayland --test protocols scroll`.

## 2. Touch points

What exists:

- `touch_input` (`crates/gnoblin-wayland/src/state.rs:2764`). A
  down on a window asks for activation, as a pointer press does, then goes to the
  surface under the point. A point that lands on nothing reaches no client.
- A point keeps the surface it went down on: motion and up follow its slot, even when
  the finger leaves the window. The protocol ties a point to a surface only at down.
- `seat.add_touch()` is called when the seat is built (`state.rs:749`), so a client
  can bind `wl_touch` and the seat offers it.
- Each batch of `wl_touch` events is closed by a `wl_touch.frame`.
- Tests, `crates/gnoblin-wayland/tests/protocols.rs`:
    - `a_touch_on_a_window_reaches_it_at_window_local_coordinates` (`:1064`)
    - `a_touch_that_moves_off_the_window_stays_with_it` (`:1087`)
    - `a_touch_over_no_window_reaches_nobody` (`:1111`)
    - `a_touch_up_for_a_point_that_never_went_down_is_not_sent` (`:1129`)
    - `each_touch_point_keeps_its_own_id` (`:1176`)

What is missing:

- No real touchscreen has run this path. The tests inject points through the backend
  input hook. Check on a real device: start a touch on a window, move, lift, and read
  the client's events.

## 3. Touch cancel

What exists:

- `cancel_touches` in `state.rs` ends every point the seat holds. The protocol's
  cancel names no slot and goes to every client holding a touch.
- It runs on a `Cancel` phase, on a session cover (`cover_raised`), and when the
  window a point is on closes.
- Test `a_cancelled_touch_ends_its_point_without_an_up` (`:1151`). A cancel is not an
  up: the client sees no `wl_touch.up`.

Why it sends a motion first: Smithay's cancel acts only on a pending frame, which
a down, motion, up, shape or orientation event opens. Without a motion first, the
seat drops the cancel silently. `cancel_touches` sends each point a motion at its
last position, then cancels.

## 4. Touch shape and orientation

What is missing:

- `wl_touch.shape` and `wl_touch.orientation` are never sent. `shape` and `orientation`
  are not called from `crates/gnoblin-wayland/src`. The backends of this slice report
  position only, so there is no contact ellipse or angle to send.
- Decision needed: a backend that reports contact size would add these events. Until
  then, a client must not rely on them.

## 5. Pointer lock and relative motion

What exists:

- A locked pointer reports relative motion alongside absolute motion:
  `a_locked_pointer_reports_motion_as_relative_events` (`protocols.rs:894`).
- Focus moving away releases a pointer lock: `:941`.
- A session lock releases a pointer lock: `:1208`.

## 6. Key bindings and key delivery

What exists:

- A bound key is consumed and its release is not leaked to the client:
  `a_bound_key_is_consumed_and_its_release_is_not_leaked` (`protocols.rs:1406`).
- A key reaches the client with its evdev code from either source, physical or
  virtual: `:1437`.
- A keymap change under a held key keeps the key's target: `:1480`.
- Delivery order is `gnoblin-input`'s: grab, binding, client (`crates/gnoblin-wayland/src/lib.rs:106`).

## 7. Pointer gestures carrying data

What is missing:

- `zwp_pointer_gestures_v1` binds, and hold, pinch and swipe exist as interfaces.
  No backend reports a touchpad gesture, so none is delivered.
- `gnoblin-input`'s recogniser is the planned source of these events.

## 8. Cursor drawing

What is missing:

- A `wp_cursor_shape_v1` or `wl_cursor` request is recorded and never painted. A
  headless output has no pointer plane. Drawing it needs a cursor plane on the DRM
  backend.

## Refreshing this page

Re-check each row against its file and line when the code changes. A row moves to
`done` only with a test that runs without hardware, and to `needs hardware` only with
a named check on a device.
