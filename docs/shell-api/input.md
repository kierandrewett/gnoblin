# Input and interaction

`gnoblin.input.devices()` returns read-only records from the native
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

Each listed source is an `InputSource` object, one for each type and ID. Every
read of one source returns the same object, and the compositor's events update
it. `source:select()` requests that source and returns an operation handle.

- For an XKB source, the operation completes after Mutter confirms the keymap
  change. It changes the seat's keyboard layout.
- For an IBus engine, Gnoblin calls `org.freedesktop.IBus.SetGlobalEngine` over
  the session bus and completes when IBus accepts the request. It changes the
  session's global input method.

An `InputSource` has `alive`, which is false once the source has left the list.
`source:watch("current", fn)` follows its selection. `source:on(signal, fn)`
takes these signals:

- `changed`, with the list of changed fields.
- `selected`, when the source becomes the current one.
- `closed`, when the source leaves the configured list.

```lua
for _, source in ipairs(gnoblin.input.sources()) do
    source:on("selected", function() print("now typing", source.name) end)
end
```

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

Change the lock with an [override](/shell-api/overrides):

- `gnoblin.settings.override("input.orientation_lock", true)` locks the current orientation.
- `gnoblin.settings.override("input.orientation_lock", false)` unlocks orientation.
- `override:revoke()` ends the override.

The call returns an `Override` and does not write the config file. After the
override ends, Gnoblin restores the configured boolean if one is set or follows
the system setting otherwise. A newer override replaces an older one.

On config reload, Gnoblin applies a boolean `input.orientation_lock` value and
ends every override. When the value is omitted or set to `"inherit"`, Gnoblin
follows the system setting. Subscribe to `gnoblin.input.orientation-lock-changed` to
receive the updated record, plus these event fields:

- `name` identifies the event.
- `sequence` orders events.
- `time` is monotonic.

The socket method names and arguments are documented in the [compositor bridge](/bridge/).

`device_type` is `"pointer"`, `"keyboard"`, `"extension"`, `"joystick"`,
`"tablet"`, `"touchpad"`, `"touchscreen"`, `"pen"`, `"eraser"`,
`"cursor"`, `"pad"`, or `"unknown"`. Capabilities are zero or more of
`"pointer"`, `"keyboard"`, `"touchpad"`, `"touch"`, `"tablet_tool"`,
`"tablet_pad"`, `"trackball"`, and `"trackpoint"`.

## Input handlers

Handle local pointer, keyboard and gesture events with `gnoblin.input.on(match, callback)`. See
[input handlers](/shell-api/input-handlers). To start a program from a handler, use
[`gnoblin.process.spawn`](/shell-api/processes).

## Insert text into the focused Wayland client

Use `gnoblin.input.text_target(grant)` from a shortcut event callback to
request a one-use text target. It returns an operation; on success, its value
is an opaque `TextTarget`:

```lua
gnoblin.on("gnoblin.shortcut.activated", function(event)
    if not event.grant then
        return
    end

    gnoblin.input.text_target(event.grant):on_complete(function(target, err)
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
gnoblin.once("gnoblin.shortcut.activated", function(event)
    if not event.grant then
        return
    end

    local target = gnoblin.input.text_target(event.grant)
    target:insert_text("hello from Gnoblin")
end)
```

```sh
gnoblinctl lua insert-text.lua
```

The console returns completed results synchronously. It keeps the target token
private and sends both requests over the connection that delivered the event.

The compositor spends the `text_target` scope of the [grant](/shell-api/grants) when it
handles the request. It creates a target only when the same Wayland surface and client
still have keyboard focus and have an active text-input-v3 session.

The target expires with the five-second deadline of the gesture. Locking the session, reloading
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

On release, Mutter checks the last accepted offer against the actual pointer
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
[compositor bridge](/bridge/) for the socket contract.

## Tablet-pad help

Subscribe to `gnoblin.input.pad-help-requested` to show help for a tablet pad
whose configured button action is `"help"`. Lua listeners receive the event
directly. See [Lua events](/events/) for callback behavior.

The event contains:

- `device`: the current `gnoblin.input.devices()` record for the pad;
- `monitor_id` and `output_names`: optional stable target-monitor identity;
- `buttons`: zero-based button numbers with optional localized action labels
  and optional `mode_group` values;
- `mode_groups`: group numbers, each group's `mode_count`, and its
  `switch_buttons`;
- `features`: zero-based rings, strips, and dials with optional labels for
  clockwise, counterclockwise, up, or down actions;
- `edition_mode`: whether the request is for editing; button help
  sends `false`.

The event provides labels when available, but no image or popup. Draw and
dismiss the overlay in your shell. Each help press sends a new event; without a
listener, no overlay appears.

| Lua method                         | Arguments | Successful result                                          |
| ---------------------------------- | --------- | ---------------------------------------------------------- |
| `gnoblin.input.devices()`          | None      | Physical input-device records                              |
| `gnoblin.input.sources()`          | None      | Configured `InputSource` objects                           |
| `gnoblin.input.current_source()`   | None      | Current `InputSource`, or `nil`                            |
| `source:select()`                  | None      | Operation returning the selected source                    |
| `gnoblin.input.orientation_lock()` | None      | Immutable `OrientationLock` record                         |
| `gnoblin.input.grab(options?)`     | `timeout` | A live `Grab`, see [Keyboard grabs](/shell-api/input-grab) |

The socket operation `input.select` takes a source's `type` and `id`. In Lua it
is `source:select()`.
