# Input Handlers

Register handlers for supported local input events from Lua configuration.

Input handlers are registered from the loaded Lua configuration and are replaced on successful config reload.

## Input handlers

Register a handler from a config file with `gnoblin.input.on(match, callback)`.
For example, consume one mouse button from a particular device:

```lua
gnoblin.input.on({type = "button", device_id = "input:3", button = 8}, function(event)
    return "consume"
end)
```

Use a device ID returned by `gnoblin.input.devices()`; IDs last for the current
session. Omitting `device_id` matches every device of the selected event type.
The registration returns its handler ID. Reloading the config replaces the
registry.

The callback returns `"forward"` to continue normal routing or `"consume"` to
withhold delivery. Nil means forward for raw handlers. The table forms
`{action = "forward"}` and `{action = "consume"}` are also accepted. Remapping
and synthetic replacement events are not supported.

Use `keybindings.keyboard` or `keybindings.pointer` for named accelerator
callbacks. Those helpers use the same registry and consume an event when the
callback returns nil. See [Keybindings](/config/configure/keybindings).

Match selectors are:

- `type`: `"key"`, `"button"`, `"motion"`, `"scroll"`, `"touch"`, `"tablet"`, or
  `"gesture"`; required.
- `device_id`: session device ID; omitted by default.
- `keycode`: integer hardware keycode for `"key"` events; omitted by default.
- `button`: integer button number, 1–32, for button or tablet events.
- `accelerator`: a chord in the [chord syntax](/guides/shortcuts#chord-syntax);
  for `"button"` events a pointer chord such as `<Super>Button1`. Omitted by
  default. This selector applies only to key and button events.
- `modifiers`: exact integer modifier mask; omitted by default. Prefer
  `accelerator` for ordinary shortcuts.
- `allow_when_shortcuts_inhibited`: false by default. Set true to handle
  input even when the focused application inhibits compositor shortcuts.
- `phase`: optional event phase. Keyboard and button events use press/release;
  touch and gestures use begin/update/end/cancel. Tablet-pad buttons use
  press/release; rings, strips, and dials use update.

Callbacks receive `time` in milliseconds, integer `modifiers`, `device_id`,
`type`, `phase`, and `pointer = {x, y}` in compositor logical coordinates.
Keys include integer `keycode` and `symbol`; button events include `button`.
Scroll events include `dx`, `dy`, and `direction`: `"up"`, `"down"`, `"left"`,
`"right"`, or `"smooth"` for continuous scrolling.

Gesture events include `dx`, `dy`, and `fingers`; pinch events also include
`scale` and `angle_delta`. Tablet-pad events include `number`, `mode`, and
`value`; `kind` is `"button"`, `"ring"`, `"strip"`, or `"dial"`. Tablet tools arrive as pointer events, not tablet-pad events.

Touch events include the integer `slot` identifying their current contact.

A button press over a window can include a typed `window` and a [`grant`](/shell-api/grants)
for window operations. The window is absent over the desktop or a
layer surface. Check for both fields before starting a window operation.

Configuration changes and new input registrations are unavailable inside a raw
input callback. Request supported runtime operations instead.

A handler that throws or returns an invalid value decides nothing and stays active. The
handlers after it still decide, and an event that no handler decides is forwarded.

Gnoblin queues at most 256 events and waits at most 16 ms for the head decision.
A timeout, disconnected runtime, or queue overflow bypasses handlers until a successful config reload and shows an error overlay.
Late decisions cannot operate on retired input requests.

Press decisions also own their matching release. A release-only key or button
handler reserves the press and consumes both physical events; returning forward
from its release callback cannot undo that reservation. Prefer a press handler
when the callback must choose between consuming and forwarding a shortcut.

For touch and gesture streams, the begin decision applies through end or cancel.
Matched update callbacks can request operations, but cannot change delivery
halfway through the stream. Reload and runtime failure retain consumed streams
until their physical end so an application cannot receive an orphan release.

If the terminal event is lost, the next non-repeated press from that device and
control starts a new stream and retires the stale state.

### Start a program from Lua

Use [`gnoblin.process.spawn`](/shell-api/processes) to start a program from a
handler, read its output, or wait for its exit.
