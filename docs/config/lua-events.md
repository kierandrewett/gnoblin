# Lua events

Register callbacks with `gnoblin.events.on(name, callback)` or
`gnoblin.events.once(name, callback)`. `gnoblin.on` remains an alias for
`gnoblin.events.on`. Both methods return a `Subscription`; call
`subscription:unsubscribe()` to remove a repeating callback. A `once`
subscription removes itself before its first callback runs.

For compositor-specific events, use `gnoblin.events.mutter.on(name, callback)`
or `gnoblin.events.mutter.once(name, callback)`. Pass the full event name,
which must start with `mutter.`. This namespace only accepts Mutter event
names; the events available depend on the running compositor build and are not
a stable cross-version API.

Event names identify their source: `gnome.shell.*` and `gnome.interface.*`
come from GNOME, `mutter.*` comes from Mutter, and `gnoblin.*` comes from
Gnoblin. The Lua runtime stays alive for the session. Reloading the config
replaces it and registers its callbacks again.

For the current GNOME light or dark appearance and a live border example, see
[Light and dark appearance](/guides/theming).

```lua
gnoblin.on("mutter.wayland.pointer-window-changed", function(event)
    local speed = event.app_id == "org.chromium.Chromium" and 0.3 or 1.0
    gnoblin.configure {input = {touchpad = {scroll_speed = speed}}}
end)
```

The Mutter pointer-window event changes when the pointer enters a different
Wayland surface, before later scroll events reach that surface. It does not
depend on keyboard focus or clicking the window. The event also fires when the
pointer leaves a client surface; then the window fields are empty strings.

## Event sources

### GNOME Shell

GNOME Shell events use the `gnome.shell.*` prefix.

| Event name                     | Fields                          | Dispatched when                                                               |
| ------------------------------ | ------------------------------- | ----------------------------------------------------------------------------- |
| `gnome.shell.focus.changed`    | `app_id`, `wm_class`, `title`   | The keyboard-focused window changes. This can differ from the pointer window. |
| `gnome.shell.window.created`   | `app_id`, `wm_class`, `title`   | A window is created.                                                          |
| `gnome.shell.window.unmanaged` | `app_id`, `wm_class`, `title`   | A window is removed.                                                          |
| `gnome.shell.input.<type>`     | Fields depend on the input type | A Clutter input event reaches the shell's captured-event handler.             |

Common input event types are:

- `motion`, with pointer coordinates `x` and `y`.
- `button_press` and `button_release`, with `button` and pointer coordinates.
- `scroll`, with pointer coordinates, `scroll_x`, `scroll_y`, and
  `scroll_direction`.
- `key_press` and `key_release`, with `key_symbol`.

Each input event also includes `type` and `time`. Events consumed before
reaching the captured-event handler are not included.

For compatibility, the earlier unqualified names remain available:
`pointer_window_changed`, `focus_changed`, `window_created`,
`window_unmanaged`, and `input.<type>`.
`pointer_window_changed` is emitted by Mutter's Wayland pointer tracking;
the other listed compatibility events come from the shell integration.

Gnoblin also reports the desktop's light or dark preference:

```lua
gnoblin.on("gnome.interface.color-scheme-changed", function(event)
    print(event.color_scheme) -- "default", "prefer-dark", or "prefer-light"
end)
```

The values are `default`, `prefer-dark`, and `prefer-light`. Gnoblin sends the
current value after config load and whenever it changes. GTK apps consume the
desktop preference automatically; some non-GTK apps opt into it. See the
[theming guide](/guides/theming) for GTK behavior and a live border example.

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

The source prefixes are `mutter.display`, `mutter.window`,
`mutter.workspace-manager`, `mutter.workspace`, `mutter.backend`,
`mutter.monitor-manager`, and `mutter.cursor-tracker`. Gnoblin forwards the
signals exposed by the running Mutter build. Newly created windows and
workspaces are watched as they appear.

Gnoblin starts this signal watcher only when the config registers one of these
events or the `*` listener.

Gnoblin also exposes the Wayland pointer-window transition as
`mutter.wayland.pointer-window-changed`, with `app_id`, `wm_class`, and
`title`. The older `pointer_window_changed` name remains available.

Touchpad swipe, pinch, and hold input is available as
`mutter.touchpad.gesture`. Gnoblin dispatches one event for each phase before the
frontend handles that input event.

Each event includes its gesture type, phase, finger count, and timestamp. Swipe
updates include unaccelerated movement deltas. Pinch updates include scale and
angle changes. Accumulate the deltas in a callback to measure total movement.
Hold events have no movement fields.

```lua
gnoblin.on("mutter.touchpad.gesture", function(event)
    if event.gesture == "swipe" and event.phase == "update" then
        print(event.fingers, event.dx, event.dy)
    end
end)
```

This event exposes input data to the active Lua config and to frontend
integrations connected to Mutter's `gnoblin-config-event` signal. It does not
choose an action by itself. A frontend can use the phases to drive its own
workspace, overview, or window animations.

In the standalone native runtime, the same input also dispatches the stable
`gnoblin.input.gesture` event to Lua. The hybrid Shell keeps its existing
`mutter.touchpad.gesture` routing. The stable event payload includes:

- `gesture`, `phase`, `fingers`, `sequence`, and `time` on every event.
- `dx` and `dy` on swipe events; `scale` and `angle_delta` on pinch events.
- `input_time`, Mutter's original input timestamp.

`sequence` increases within the gesture stream. `time` is monotonic-clock
microseconds. Device names and input tokens are not included.
API 1.9 socket clients can request this event with `op: "events"`; socket
messages use `event` instead of Lua's `name` and include socket-stream sequence
and monotonic time metadata. See the [compositor bridge](/compositor-bridge).

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
- Object arguments appear as type names. Window arguments also include
  `argN_app_id`, `argN_wm_class`, and `argN_title`.
- `mutter.window.*` events include `window_app_id`, `window_wm_class`, and
  `window_title`.

Values that cannot be represented as simple Lua event fields are omitted or
reduced to a type or name string.

### Gnoblin

Gnoblin events use the `gnoblin.*` prefix.

The `workspace.*` fields in this table describe the native Lua runtime.
Shell-backed compatibility events retain their legacy workspace payloads,
described below. The native `window.*` lifecycle events require the direct
Mutter runtime.

| Event name                         | Fields                                                                             | Dispatched when                                                                                      |
| ---------------------------------- | ---------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| `gnoblin.config.reloaded`          | `path`                                                                             | Config and its Lua modules load and apply successfully.                                              |
| `gnoblin.config.reload-failed`     | `path`, `error`                                                                    | A config reload fails while a previously loaded event subscription is still active.                  |
| `gnoblin.workspace.created`        | `workspace`                                                                        | A runtime workspace is created.                                                                      |
| `gnoblin.workspace.renamed`        | `workspace`                                                                        | A workspace display name changes.                                                                    |
| `gnoblin.workspace.changed`        | `workspace`, `changed`                                                             | Its position, window count, or persistence state changes.                                            |
| `gnoblin.workspace.removed`        | `workspace_id`, `last`                                                             | A temporary workspace is removed.                                                                    |
| `gnoblin.workspace.activated`      | `workspace`, optional `previous_id`                                                | The active workspace changes.                                                                        |
| `gnoblin.workspace.window-moved`   | `window_id`, `from_id`, `to_id`                                                    | A window moves from one workspace to another.                                                        |
| `gnoblin.monitor.added`            | `monitor`                                                                          | An active logical monitor appears in the native runtime.                                             |
| `gnoblin.monitor.changed`          | `monitor`, `changed`                                                               | A listed monitor property changes; `changed` names the changed properties.                           |
| `gnoblin.monitor.removed`          | `monitor_id`, `last`                                                               | An active logical monitor is removed.                                                                |
| `gnoblin.input.device-added`       | `device`                                                                           | An input device appears in the native runtime.                                                       |
| `gnoblin.input.device-removed`     | `device_id`, `last`                                                                | An input device is removed from the native runtime.                                                  |
| `gnoblin.input.sources-changed`    | `sources`                                                                          | The configured, available XKB source list changes.                                                   |
| `gnoblin.input.source-changed`     | `available`, optional `source`                                                     | Mutter confirms a different Gnoblin-owned keymap group, or the current source becomes unknown.       |
| `gnoblin.input.gesture`            | `gesture`, `phase`, `fingers`, `sequence`, `time`, and gesture-specific fields     | A touchpad gesture phase reaches the standalone native Lua runtime.                                  |
| `gnoblin.launch.changed`           | `launch`                                                                           | A native launch-feedback record is created or changes state.                                         |
| `gnoblin.portal.grant-added`       | `grant`, `revision`, `sequence`, `time`                                            | A validated persistent portal grant is added or updated.                                             |
| `gnoblin.portal.grant-removed`     | `grant_id`, `kind`, `revision`, `sequence`, `time`                                 | A persistent portal grant is revoked.                                                                |
| `gnoblin.window.created`           | `window`                                                                           | The native compositor runtime observes a new managed window.                                         |
| `gnoblin.window.changed`           | `window_id`, `changed`, `window`                                                   | A mapped window property changes in the native compositor runtime.                                   |
| `gnoblin.window.focused`           | `window_id`, `window`                                                              | A window gains keyboard focus in the native compositor runtime.                                      |
| `gnoblin.window.unfocused`         | `window_id`, `window`                                                              | A window loses keyboard focus in the native compositor runtime.                                      |
| `gnoblin.window.attention-changed` | `window_id`, `window`, `demands_attention`                                         | Mutter's attention state changes.                                                                    |
| `gnoblin.window.closed`            | `window_id`, `last`                                                                | The native compositor runtime removes a managed window.                                              |
| `gnoblin.focus.policy-changed`     | `policy`, `revision`, `sequence`, `time`                                           | The effective focus policy changes after a successful config commit.                                 |
| `gnoblin.permission.changed`       | `policy`, `revision`, `sequence`, `time`                                           | The committed portal permission policy changes after a successful config commit.                     |
| `gnoblin.shortcut.activated`       | `shortcut`, `trigger`, `focus_context`                                             | A configured native command shortcut is activated by a trusted key press in the native runtime.      |
| `gnoblin.operation.completed`      | `operation_id`, `method`, `ok`, `value` or `error`, `revision`, `sequence`, `time` | Native API 1.11 completion event; `error` is an `Error` record.                                      |
| `gnoblin.api.operation-completed`  | `request_id`, `method`, `ok`, `result` or string `error`                           | Legacy completion event retained during migration.                                                   |
| `gnoblin.feature.changed`          | `feature`, `enabled`                                                               | A Gnoblin feature changes after initial state setup.                                                 |
| `gnoblin.scripts.loaded`           | `scripts`                                                                          | The user script load pass completes; `scripts` is a comma-separated list of loaded script filenames. |
| `gnoblin.scripts.load_failed`      | `script`, `error`                                                                  | A user script cannot be imported or throws while loading.                                            |

`gnoblin.focus.policy-changed` runs after a successful config commit when one
of the effective focus settings changes. Its `policy` is the committed
snapshot; `revision` matches `policy.revision`. A failed or rejected config
does not emit this event. Native-control API 1.13 socket clients can subscribe
to the same event.

The native Lua runtime receives `focus_context` as protected userdata for
`Window:focus(context)`. Native-control API 1.10 socket clients can also opt in
to `gnoblin.shortcut.activated`. Each subscribed connection receives its own
random token and can use it once with `window.focus`. Socket tokens are never
included in Lua event payloads or sent to another connection.

A socket token expires five seconds after the shortcut press. It is revoked
when its connection closes, its event subscription changes, the session locks,
or the config reloads.

Native socket API 1.11 adds connection-owned dynamic bindings through
`shortcut.bind` and `shortcut.unbind`. Socket clients that bind a shortcut can
subscribe to `gnoblin.shortcut.binding-activated`; the event is sent only to
the connection that owns the binding and includes a one-use `focus_context`.
These socket bindings activate on press and do not provide held-modifier,
modal, or type-ahead input handling. They are not Lua event registrations.

Native API 1.11 adds a stable operation-completion event. It includes:

- `operation_id`, `method`, and `ok`.
- `value` on success, or an `Error` record with `code` and `message` on failure.

The older `gnoblin.api.operation-completed` event remains available with
`request_id`, `result`, and a string error. A wildcard listener receives both
event names during migration. Filter by `event.name` or handle only one form to
avoid processing a completion twice.

Native API 1.15 adds two portal grant lifecycle events. Gnoblin updates the
snapshot cache before dispatching either event. Both include snapshot revision,
event sequence, and monotonic time.

- `gnoblin.portal.grant-added` carries the new grant fields as a plain event
  table.
- `gnoblin.portal.grant-removed` carries the opaque grant ID and portal kind.

Native API 1.16 adds `gnoblin.permission.changed` after a successful config
commit changes the portal policy. The event carries the committed policy and
standard revision, sequence, and monotonic-time metadata.

The policy contains a default level, ordered rules, and the committed settings
revision. Its revision matches the event's top-level revision. Rejected or
unchanged policies do not emit this event.

The compositor socket and its directory are accessible only to the current
user. Any process running as that user can subscribe, so treat same-user socket
clients as trusted shell components. See the
[compositor bridge](/compositor-bridge#api-version-110-shortcut-focus-grants)
for the request format.

```lua
gnoblin.on("gnoblin.feature.changed", function(event)
    print(event.feature .. " enabled: " .. tostring(event.enabled))
end)
```

Shell-backed compatibility workspace events expose the workspace record's
`id`, current `number`, `name`, `active`, `windows`, and `persistent` fields
as the event payload itself. Native events wrap the record under `workspace`
(or `last` on removal).

Mutter started with `--gnoblin-config PATH` dispatches native window,
workspace, monitor, input, and launch-feedback events directly to Lua. Native
lifecycle events have `name`, `revision`, `sequence`, and `time` fields.
Sequence numbers increase across native lifecycle events. `time` is
monotonic-clock microseconds. Gesture events use their own sequence and do not
include a state revision.

`gnoblin.launch.changed` wraps the record under `launch`. It contains:

- `token` and `application`.
- `started_at`, as Unix time in milliseconds.
- `timeout_ms`, `state`, and `revision`.

States are `pending`, `started`, `ended`, and `timed_out`. Mutter reports a
launch when a matching mapped window appears or becomes focused. It cannot
report failures from an external process launcher. The revision is scoped to
launch records and does not advance the compositor `state_revision`.

`gnoblin.operation.completed` reports shortcut capture results:

- Success returns the accelerator in `value.accelerator`.
- Cancellation, timeout, a locked session, or an input grab returns an `Error` record.

The legacy `gnoblin.api.operation-completed` event uses
`result.accelerator` and a string error. The capture hook consumes key events
while active. It cancels and releases the hook if the session locks or another
input grab starts.

Monitor records describe active logical monitors. Their `id` is the canonical
connector name. A monitor change lists the changed record properties in
`changed`: `id`, `index`, `x`, `y`, `width`, `height`, `primary`, `scale`,
`enabled`, `name`, `make`, `model`, `serial`, `refresh_rate`, or `transform`.

Native workspace events use a `workspace` record for create, rename, and
activation. A change event includes a `changed` array containing `number`,
`window_count`, or `persistent`. Workspace records use `window_count` for the
number of windows, matching immediate native workspace reads.

A removal carries `workspace_id` and `last`, the final record before removal.
The moved event has no workspace record; it reports the stable window ID and
the source and destination workspace IDs.

The attention event also carries `demands_attention` at the top level. It can
follow a focus request that policy did not activate; it reports Mutter's
attention state and does not identify why the window requested attention.

Window records include the fields available in the native preview:

- Identity: `id`, `title`, `app_id`, `gtk_app_id`, `wm_class`, `rule_app_id`, and optional `role`.
- Location: `workspace_id`, `workspace_number`, `monitor_id`, `monitor_index`, `parent`.
- State: `maximized`, `fullscreen`, `minimized`, `focused`, `above`, `sticky`, and `demands_attention`.
- Capabilities: `closable`, `minimizable`, `maximizable`, `movable`, and `resizable`.
- Type: `type`, an integer `MetaWindowType` value.
- Geometry: `frame` and the optional `monitor` origin.
- Interaction: `last_user_time`.

A field is omitted when the native record does not provide it. `monitor_id` is
the active connector name for the current logical monitor. Cloned outputs use
the lexicographically first active connector. `monitor_index` is the current
Mutter order and can change when outputs change.

The raw `type` value follows Mutter's `MetaWindowType` enum. It describes the
client surface type; it does not determine how a shell presents the window.

| Value | Type           | Meaning                               |
| ----- | -------------- | ------------------------------------- |
| 0     | normal         | Standard application window.          |
| 1     | desktop        | Desktop background surface.           |
| 2     | dock           | Dock or panel surface.                |
| 3     | dialog         | Dialog window.                        |
| 4     | modal dialog   | Dialog that blocks its parent window. |
| 5     | toolbar        | Toolbar surface.                      |
| 6     | menu           | Menu surface.                         |
| 7     | utility        | Utility window.                       |
| 8     | splash screen  | Application startup surface.          |
| 9     | dropdown menu  | Dropdown menu surface.                |
| 10    | popup menu     | Popup menu surface.                   |
| 11    | tooltip        | Tooltip surface.                      |
| 12    | notification   | Notification surface.                 |
| 13    | combo box      | Combo-box popup surface.              |
| 14    | drag-and-drop  | Drag-and-drop surface.                |
| 15    | override other | Other override-redirect surface.      |

The `changed` array lists mapped properties that changed. Focus transitions and
attention changes use separate events; `demands_attention` is omitted from the
`changed` array.

When focus moves between windows that remain managed, the native runtime
dispatches `unfocused` before `focused` within that state revision.

A closed event's `last` field contains the final available window record. The
Shell-backed compatibility runtime continues to expose its GNOME Shell window
events.

`removed` reports the workspace's last record before removal. Its `number` is
the position immediately before removal.

When `workspace.create` uses `activate = true`, Gnoblin emits `created` first
with `active = false`, then emits `activated` after switching workspaces.

A Lua API call returns an `Operation` handle with its current status, result,
failure reason, and a completion callback. See the [Lua runtime API](/config/runtime-api)
for the handle fields and callback behavior.

Native API 1.11 dispatches the `gnoblin.operation.completed` event. It includes:

- `operation_id` and `method` for the completed operation.
- `ok` and either `value` or an `Error` record with `code` and `message`.

The operation ID matches the handle's `id`. The legacy
`gnoblin.api.operation-completed` event remains during migration and uses
`request_id`, `result`, and a string `error`. Runtime actions requested from an
event callback run after the callback returns.

## Listen to every forwarded event

Pass `"*"` to receive every event source currently forwarded by Gnoblin:

```lua
gnoblin.on("*", function(event)
    print(event.name)
end)
```

Every callback receives one event table. `event.name` contains the dispatched
name. Registering a name does not create an event source: Mutter, GNOME Shell,
or Gnoblin must dispatch it. Signal availability follows the Mutter and GNOME
Shell versions used to build Gnoblin.

`gnoblin.listeners` maps each registered event name to its callback list.
Inspect it to see which handlers earlier config files have added. The list is
for inspection; remove handlers through their `Subscription`:

```lua
for name, callbacks in pairs(gnoblin.listeners) do
    print(name, #callbacks)
end
```

Register callbacks with `gnoblin.events.on` or `gnoblin.events.once`; use
`gnoblin.on` only when supporting older config files. The listener table is for
inspection.

Callbacks run synchronously in the compositor's main thread. Keep them short,
especially handlers for high-frequency `*.input.*` events.

`gnoblin.configure` changes follow the usual live-setting rules. Startup-only
settings still need a new session. If any listener fails, Gnoblin logs the
error and continues with the remaining listeners. It rolls back configuration
changes from that event, but still dispatches operations queued by its
callbacks.

If the resulting config fails validation, Gnoblin restores the last valid
config and fails those queued operations. For settings applied by the shell,
Gnoblin applies the updated document after the callback returns.

## Type definition

Event names are open-ended: Gnoblin accepts a nonempty UTF-8 string of up to
128 bytes with no NUL byte, but only names dispatched by Gnoblin, Mutter or
the shell produce callbacks. The event fields depend on the event name; all
callback values are Lua tables.

```lua
gnoblin.on(event_name, function(event)
    -- event_name: string | "*"
    -- event = {
    --     name = string,
    --     source = string?, signal = string?,
    --     app_id = string?, wm_class = string?, title = string?,
    --     type = string?, time = number?, x = number?, y = number?,
    --     button = integer?, key_symbol = string?,
    --     scroll_x = number?, scroll_y = number?, scroll_direction = string?,
    --     ... -- additional fields depend on the event source
    -- }
end)
```
