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

## Rules

- Select is `BTN_LEFT` (`0x110`) and secondary is `BTN_RIGHT` (`0x111`).
- A button presses when its value reaches `PRESS_THRESHOLD` (0.75) and releases when it falls to `RELEASE_THRESHOLD`
  (0.25). Between them it keeps its state, so a half pull neither presses nor chatters. A NaN changes nothing.
- A stick whose length is within `SCROLL_DEADZONE` (0.15) does nothing. Past it the length is rescaled so the speed
  starts at zero and reaches `SCROLL_RATE` (400 axis units a second) at full deflection, and the direction is kept.
  Up on the stick is up the page, a negative vertical amount, as on a wheel.
- `dt` is clamped to `0..=MAX_STEP` (0.1 s), so a stall is one step, a first sample (`dt` 0) scrolls nothing, and a
  clock that went backwards or a NaN scrolls nothing.

## Test

```sh
cd extensions/gnoblin-xr
cargo test -p xr-input
```
