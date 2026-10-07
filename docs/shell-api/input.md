# Input and interaction

`gnoblin.input.devices()` returns read-only records from the latest native
input-device snapshot. It takes no arguments and is available in runtime event
callbacks after startup. The read raises a Lua error if its snapshot is
unavailable.

Input-device records expose these fields:

| Field                       | Type and values                                                 |
| --------------------------- | --------------------------------------------------------------- |
| `id`, `name`, `device_type` | Strings. The `input:N` ID remains stable for this session only. |
| `seat`                      | Optional string; absent when Mutter provides no seat name.      |
| `vendor_id`, `product_id`   | Optional integers.                                              |
| `capabilities`              | Array of capability names.                                      |
| `revision`                  | Integer state revision.                                         |

Subscribe to these events to track device changes in the native runtime:

- `gnoblin.input.device-added` contains `device`.
- `gnoblin.input.device-removed` contains `device_id` and the last device
  record in `last`.

`gnoblin.input.devices()` refreshes before either callback runs. Device IDs
last only for the current compositor session. Records do not expose device
paths or `enabled`.

The current Mutter backend does not expose a safe enabled-state getter, so
records omit `enabled`.

`gnoblin.input.sources()` returns configured XKB layouts and variants and
configured IBus engine IDs. XKB entries are checked against the installed XKB
registry. IBus records use the engine ID for both `name` and `short_name`.
Gnoblin does not need an IBus library to list or select them. The IBus service
must be running for selection; otherwise the operation fails with
`unavailable`.

The list comes only from `input_sources.sources` in Gnoblin's Lua config. If
that setting is absent, the list is empty and Gnoblin leaves Mutter's current
keymap in place. Gnoblin does not read the saved GNOME input-source list.

Mutter loads at most four layouts into one keymap at a time. Gnoblin switches
the active group of four when you select a listed source outside that group.

`gnoblin.input.current_source()` returns the current configured XKB source or
the current configured IBus engine, or nil when the active source is unknown
or is not configured in Gnoblin.

`gnoblin.input.select_source({type = "xkb", id = "us"})` requests a listed XKB
source and returns an operation handle. The operation completes after Mutter
confirms the keymap change. Use `{type = "ibus", id = "engine-id"}` to select a
listed IBus source. Gnoblin calls `org.freedesktop.IBus.SetGlobalEngine` over
the session bus and completes when IBus accepts the request. XKB selection
changes the seat's keyboard layout; IBus selection changes the session's
global input method.

Set `input_sources.per_window` to:

- `true` to remember and restore the last selected source for each focused window. A
  window without a saved source inherits the source active at its first focus.
- `false` or omit it (the default) to share the selected source across windows.

Subscribe to `gnoblin.input.source-changed` for confirmed current-source
changes and `gnoblin.input.sources-changed` when the configured source list
changes. The current-source event has `available = false` and no `source` when
Mutter's current keymap is external or unknown.

`gnoblin.input.orientation_lock()` returns an immutable `OrientationLock`
record with boolean `available` and `locked`, string `orientation`, `source`,
and integer `revision`. `source` is `system` when the system setting applies,
`config` when `input.orientation_lock` supplies a boolean, and `runtime` while
a runtime override is active.

The `orientation` field is `normal`, `bottom-up`, `left-up`, `right-up`, or
`undefined`. The last value means no orientation is available.

Call one of these to request a change:

- `gnoblin.input.set_orientation_lock(true)` locks the current orientation.
- `gnoblin.input.set_orientation_lock(false)` unlocks orientation.
- `gnoblin.input.set_orientation_lock("inherit")` clears the runtime override.

Each call returns an asynchronous `Operation<OrientationLock>` and does not
write the config file. After `inherit`, Gnoblin restores the configured boolean
if one is set or follows the system setting otherwise.

On config reload, Gnoblin applies a boolean `input.orientation_lock` value.
When omitted or set to `"inherit"`, it clears the override and follows the
system setting. Subscribe to `gnoblin.input.orientation-lock-changed` to
receive the updated record, plus these event fields:

- `name` identifies the event.
- `sequence` orders events.
- `time` is monotonic.

The socket method names and arguments are documented in the [compositor bridge](/compositor-bridge).

`device_type` is `"pointer"`, `"keyboard"`, `"extension"`, `"joystick"`,
`"tablet"`, `"touchpad"`, `"touchscreen"`, `"pen"`, `"eraser"`,
`"cursor"`, `"pad"`, or `"unknown"`. Capabilities are zero or more of
`"pointer"`, `"keyboard"`, `"touchpad"`, `"touch"`, `"tablet_tool"`,
`"tablet_pad"`, `"trackball"`, and `"trackpoint"`.

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
- `accelerator`: keyboard or modified Button1–Button3 accelerator; omitted by
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
`value`; `kind` is `"button"`, `"ring"`, `"strip"`, or `"dial"`. Tablet tools currently arrive as pointer events, not tablet-pad events.

Touch events include the integer `slot` identifying their current contact.

A button press over a window can include a typed `window` and trusted
`focus_context` for window operations. The window is absent over the desktop or a
layer surface. Check for both fields before starting a window operation.

Configuration changes and new input registrations are unavailable inside a raw
input callback. Request supported runtime operations instead.

Handler failures restore forwarding. Gnoblin queues at most 256 events and waits at most 16 ms
for the head decision. A timeout, disconnected runtime, or queue overflow
bypasses handlers until a successful config reload and shows an error overlay.
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

### Launch a command from Lua

`gnoblin.commands.run(argv)` accepts 1–64 UTF-8 strings, with a nonempty program
name and at most 64 KiB total. It returns an operation whose successful value
is `{accepted = true}`: the command has entered the launch queue. This does not
confirm that spawning succeeded or that the child exited successfully. Spawn
and exit failures are reported in Gnoblin's output.

### Run a command and read its output

`gnoblin.commands.capture(options)` runs a command and returns its standard
output. Use it to ask a prompt program for an answer.

| Field     | Type              | Default | Meaning                                                   |
| --------- | ----------------- | ------- | --------------------------------------------------------- |
| `argv`    | 1-64 strings      | none    | The program and its arguments. The first entry is not empty. |
| `stdin`   | string            | none    | Text sent to the command, up to 4096 bytes.               |
| `timeout` | integer, seconds  | `60`    | Time before Gnoblin stops the command. From 1 to 600.     |

The operation succeeds with `{exit_code = integer, stdout = string}` when the
command exits. A command stopped by a signal reports a negative `exit_code`
with the signal number. The operation fails when the command cannot start, runs
past `timeout`, prints more than 64 KiB, or prints text that is not UTF-8.

```lua
gnoblin.commands.capture {argv = {"date", "+%H:%M"}}:on_complete(function(result, err)
    if not err then
        print(result.stdout)
    end
end)
```

The command gets no standard input unless you set `stdin`, and Gnoblin discards
its standard error. Up to 4 captures run at the same time. The call fails while
the session is locked.

## Insert text into the focused Wayland client

Use `gnoblin.input.text_target(context)` from a shortcut event callback to
request a one-use text target. It returns an operation; on success, its value
is an opaque `TextTarget`:

```lua
gnoblin.events.on("gnoblin.shortcut.activated", function(event)
    if not event.focus_context then
        return
    end

    gnoblin.input.text_target(event.focus_context):on_complete(function(target, err)
        if err then
            return
        end
        target:insert_text("😀")
    end)
end)
```

The `gnoblinctl lua` console can use the same capability inside an event
callback. Save this as `insert-text.lua` and run it from a Gnoblin session:

```lua
gnoblin.events.once("gnoblin.shortcut.activated", function(event)
    if not event.focus_context then
        return
    end

    local target = gnoblin.input.text_target(event.focus_context)
    target:insert_text("hello from Gnoblin")
end)
```

```sh
gnoblinctl lua insert-text.lua
```

The console returns completed results synchronously. It keeps the target token
private and sends both requests over the connection that delivered the event.

The compositor consumes the `FocusContext` when it handles the request. It
creates a target only when the same Wayland surface and client still have
keyboard focus and have an active text-input-v3 session. The target expires
with its five-second `FocusContext` deadline. Locking the session, reloading
the runtime, losing focus, or closing the runtime revokes it. X11 clients are
not supported.

`TextTarget.window_id` is the focused window's stable ID when the surface
belongs to a window. `TextTarget.caret` is an optional rectangle in
compositor logical coordinates. These fields are for shell presentation; only
the opaque target authorizes insertion.

Call `target:insert_text(text)` once. It returns an operation and consumes the
target on the first attempt. Text must contain 1 to 256 bytes of valid UTF-8
without NUL or control characters.

Insertion succeeds only while the same surface, client, focus epoch, and active
text input remain current. The session must be unlocked. Mutter commits the
text through its focused input-method path.

Modifiers held when the shortcut activated may remain held. Adding another
modifier invalidates the target. Ctrl, Alt, Shift, Lock, Meta, and Hyper state
prevents target creation. Gnoblin keeps at most 128 active text targets per
compositor; creation fails while the limit is full.


## Pointer and keyboard snapping

Lua pointer callbacks receive a read-only `event.drag` record on the started
and updated events. It contains window and drag IDs, settings revision, pointer
coordinates, modifiers, monitor and work-area rectangles, the window frame,
and maximized state.

Call `event.drag:offer_targets(targets)` during the callback. An offer can
contain 1 to 128 targets. Each target has:

- a unique `id`;
- pointer `hit` and destination `frame` rectangles;
- optional `maximize`, `required_modifiers`, and `forbidden_modifiers` fields.

Rectangles use integer logical coordinates and `{x, y, width, height}` fields.
Both must fit in the current work area. Modifier arrays accept only `control`,
which cannot be both required and forbidden.

On release, Mutter checks the latest accepted offer against the actual pointer
and modifiers. A match applies its frame; otherwise Mutter completes the
normal move or tile-preview behavior. The shell owns snap guides and other
presentation.

Drag records expire on release, lock, config reload, window loss, or compositor
teardown. An old runtime cannot replace an offer after reload.

Socket clients can offer pointer snap targets through the compositor bridge.
Lua callbacks use `event.drag:offer_targets()` as described above.

### Keyboard snapping

For keyboard-selected layouts, call `gnoblin.windows.snap_context(context)` in
a trusted shortcut callback. It consumes the one-use focus context and returns
an `Operation<SnapContext>` for the focused window; callers cannot provide a
window ID.

Commit once with a monitor ID and frame rectangle. Gnoblin checks the window,
monitor, lock state, runtime generation, and work-area bounds again. The
operation resolves to `{window_id, monitor_id, committed = true}`. The context
expires with its source focus context and is revoked by lock or config reload.

The context is available only in a trusted shortcut callback. It expires
after five seconds and is revoked by session lock or config reload. See the
[compositor bridge](/compositor-bridge) for the socket contract.


## Tablet-pad help

Subscribe to `gnoblin.input.pad-help-requested` to show help for a tablet pad
whose configured button action is `"help"`. Lua listeners receive the event
directly. See [Lua events](/config/lua-events) for callback behavior.

The event contains:

- `device`: the current `gnoblin.input.devices()` record for the pad;
- `monitor_id` and `output_names`: optional stable target-monitor identity;
- `buttons`: zero-based button numbers with optional localized action labels
  and optional `mode_group` values;
- `mode_groups`: group numbers, each group's `mode_count`, and its
  `switch_buttons`;
- `features`: zero-based rings, strips, and dials with optional labels for
  clockwise, counterclockwise, up, or down actions;
- `edition_mode`: whether the request is for editing; button help currently
  sends `false`.

The event provides labels when available, but no image or popup. Draw and
dismiss the overlay in your shell. Each help press sends a new event; without a
listener, no overlay appears.

| Lua method                                  | Arguments                       | Successful result                       |
| ------------------------------------------- | ------------------------------- | --------------------------------------- |
| `gnoblin.input.devices()`                   | None                            | Physical input-device records           |
| `gnoblin.input.sources()`                   | None                            | Configured XKB input-source records     |
| `gnoblin.input.current_source()`            | None                            | Current XKB source, or `nil`            |
| `gnoblin.input.select_source({type, id})`   | Source selector                 | Operation returning the selected source |
| `gnoblin.input.orientation_lock()`          | None                            | Immutable `OrientationLock` record      |
| `gnoblin.input.set_orientation_lock(value)` | `true`, `false`, or `"inherit"` | `Operation<OrientationLock>`            |

Pass a source's `type` and `id` from `gnoblin.input.sources()` to
`gnoblin.input.select_source()`.

Lua also retains two read aliases: `gnoblin.input.list()` matches
`gnoblin.input.sources()`, and `gnoblin.input.current()` matches
`gnoblin.input.current_source()`. The socket operation `input.select` is
exposed in Lua as `gnoblin.input.select_source()`.

These aliases are available in supervised configuration and `gnoblinctl lua`.
The CLI maps them to the canonical socket reads. Socket clients call the
canonical method names directly.
