# Mutter Events

Subscribe to forwarded GObject signals from Mutter objects.

Available signals depend on the running Mutter build and can change between Mutter versions.

### Mutter

Mutter GObject signals use the object's source prefix followed by the signal
name. Gnoblin watches the current display, windows and workspaces, workspace
manager, backend, monitor manager, and cursor tracker. Examples include:

```lua
gnoblin.on("mutter.display.restacked", function(event)
    print(event.name)
end)

gnoblin.on("mutter.window.position-changed", function(event)
    print(event.window_title)
end)
```

Gnoblin forwards signals from these Mutter sources:

- Display and windows: `mutter.display`, `mutter.window`.
- Workspaces: `mutter.workspace-manager`, `mutter.workspace`.
- Compositor state: `mutter.backend`, `mutter.monitor-manager`,
  `mutter.cursor-tracker`.

It watches newly created windows and workspaces as they appear.

Gnoblin starts a signal watcher when the supervised Lua config or a compositor
socket client subscribes to that signal.

Gnoblin also exposes the Wayland pointer-window transition as
`mutter.wayland.pointer-window-changed`, with `app_id`, `wm_class`, and
`title`.

Touchpad swipe, pinch, and hold input is available as
`mutter.touchpad.gesture`. Mutter sends each phase through Gnoblin's native
compositor handler. That handler makes any synchronous input-claim decision and
queues the event for the supervised Lua runtime.

Each event includes its gesture type, phase, finger count, sequence, and
timestamp. Swipe updates include unaccelerated movement deltas. Pinch updates
include scale and angle changes. Accumulate the deltas in a callback to measure
total movement. Hold events have no movement fields.

```lua
gnoblin.on("mutter.touchpad.gesture", function(event)
    if event.gesture == "swipe" and event.phase == "update" then
        print(event.fingers, event.dx, event.dy)
    end
end)
```

Lua callbacks receive the queued event asynchronously and cannot claim or
consume the Mutter input event. The compositor handles configured direct
actions and commands in the `any` context synchronously. During an active
session lock, it also handles `locked` bindings. Other contexts do not
have a Gnoblin UI handler.

The standalone native runtime also dispatches the stable
`gnoblin.input.gesture` event to Lua. Its payload includes:

- `gesture`, `phase`, `fingers`, `sequence`, and `time` on every event.
- `dx` and `dy` on swipe events; `scale` and `angle_delta` on pinch events.
- `input_time`, Mutter's original input timestamp.

The compositor assigns `sequence` before dispatch, so it continues increasing
across Lua config reloads and runtime-worker recovery within one compositor
session. `time` uses monotonic-clock microseconds. Device names and input tokens
are omitted.

Compositor socket clients can subscribe to stable `gnoblin.*` events and raw
`mutter.*` signals. Their request format and event framing are documented in
the [compositor bridge](/bridge/). Raw Mutter signal names and
payloads can change with the pinned Mutter version.

```lua
gnoblin.events.on("gnoblin.input.gesture", function(event)
    if event.gesture == "swipe" and event.phase == "update" then
        print(event.sequence, event.time, event.dx, event.dy)
    end
end)
```

Every signal event includes `source` and `signal`.

- Scalar signal arguments appear as `arg0`, `arg1`, and so on. Their GObject
  types appear in `arg0_type`, `arg1_type`, and so on.
- Workspace signals include `workspace_index`.
- Signals that pass a `MetaWindow`, including `mutter.display.window-created`,
  include a window identity record in `argN`. It contains `window_id`, `app_id`,
  and `window_title`; `argN_type` is `MetaWindow`.
- `mutter.window.*` events include `window_id`, `app_id`, and `window_title`.

Values that cannot be represented as simple Lua event fields are omitted or
reduced to a type or name string.
