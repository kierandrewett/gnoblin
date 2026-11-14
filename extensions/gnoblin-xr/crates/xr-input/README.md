# xr-input

The controller input model for `gnoblin-xr`. It has no OpenXR types, no socket and no clock, so every rule runs under
`cargo test` with no headset.

## API

- `ControllerState { ray, select, secondary, scroll }`: one sample per frame. `select` and `secondary` are `0` to `1`
  (a click reports 0 or 1, a trigger its pull), `scroll` is the stick as `(x right, y up)`.
- `Controller::update(&state, dt) -> Vec<PointerEvent>`: the events the sample causes, in order: the select edge, the
  secondary edge, then the scroll. `any_down()` says whether a button is down.
- `PointerEvent::Button { button, pressed }` and `PointerEvent::Scroll { dx, dy }`.
- `pointing_ray(aim, head)`: the ray from the aim pose, else the head's gaze, else nothing.
- `Arbiter::sample(&hands, claims, now) -> ControllerState`: the right hand as the pointer's sample under the shell's
  claims. `now` is the frame's display time in nanoseconds and the arbiter's only clock.

## Rules

- Select is `BTN_LEFT` (`0x110`) and secondary is `BTN_RIGHT` (`0x111`).
- A button presses when its value reaches `PRESS_THRESHOLD` (0.75) and releases when it falls to `RELEASE_THRESHOLD`
  (0.25). Between them it keeps its state, so a half pull neither presses nor chatters. A NaN changes nothing.
- A stick whose length is within `SCROLL_DEADZONE` (0.15) does nothing. Past it the length is rescaled so the speed
  starts at zero and reaches `SCROLL_RATE` (400 axis units a second) at full deflection, and the direction is kept.
  Up on the stick is up the page, a negative vertical amount, as on a wheel.
- `dt` is clamped to `0..=MAX_STEP` (0.1 s), so a stall is one step, a first sample (`dt` 0) scrolls nothing, and a
  clock that went backwards or a NaN scrolls nothing.
- The arbiter holds every press of the trigger, the grip, the face button and the stick for `CLAIM_HOLD` (150 ms). A
  claim in that time takes the press, so the window never hears of it; with none, the press is forwarded at the
  deadline at the ray the hand had when it went down, and a tap that ended inside the hold is a press then a release a
  frame apart. A press the window already has when a later claim arrives is released.

## Test

```sh
cd extensions/gnoblin-xr
cargo test -p xr-input
```
