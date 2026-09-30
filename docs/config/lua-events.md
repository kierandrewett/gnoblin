# Lua events

Register repeating callbacks with `gnoblin.events.on`; use
`gnoblin.events.once` for one-time callbacks. Each returns a `Subscription`.

Remove a repeating callback with `subscription:unsubscribe()`. A `once`
subscription removes itself before invocation. `gnoblin.on` remains an alias
for `gnoblin.events.on`.

Subscribe to compositor signals with `gnoblin.events.mutter.on` or
`gnoblin.events.mutter.once`. Pass a full `mutter.*` event name. Available
signals depend on the running Mutter build and are not stable across versions.

Event names identify their source: `mutter.*` comes from Mutter and
`gnoblin.*` comes from Gnoblin. The supervised Lua runtime stays alive for the
session; reloading the config replaces its callbacks.

For desktop color-scheme change notifications, subscribe to
`gnoblin.appearance.color-scheme-changed`. The [appearance guide](/guides/theming)
explains how applications use the preference.

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

Workspace events include `workspace_index`. A `MetaWindow` argument is an
identity record with `window_id`, `app_id`, and `window_title`.

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

This event exposes input data to the active Lua config and to external
integrations connected to Mutter's `gnoblin-config-event` signal. Configured
direct actions and commands in the `any` context are handled by the native
compositor. During an active session lock, it also handles `unlock-screen`
bindings. Other contexts do not have a Gnoblin UI handler.

The standalone native runtime also dispatches the stable
`gnoblin.input.gesture` event to Lua. Its payload includes:

- `gesture`, `phase`, `fingers`, `sequence`, and `time` on every event.
- `dx` and `dy` on swipe events; `scale` and `angle_delta` on pinch events.
- `input_time`, Mutter's original input timestamp.

Within the gesture stream, `sequence` increases and `time` uses monotonic-clock
microseconds. Device names and input tokens are omitted.

Socket clients need API 1.9 and request this event with `op: "events"`. Their
frames use `event` instead of Lua's `name` and include socket-stream sequence
and time. See the [compositor bridge](/compositor-bridge).

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

The native `window.*` lifecycle events require the direct Mutter runtime.

Every Lua event record includes `name`. The table below lists event-specific
fields.

Window event records also include `revision`, `sequence`, and monotonic `time`.
Nested `window` and `last` records carry their own `revision`, matching the
event revision.

| Event name                                | Fields                                                                             | Dispatched when                                                                                     |
| ----------------------------------------- | ---------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- |
| `gnoblin.session.lock-state-changed`      | `state`, `sequence`, `time`                                                        | Mutter reports a lock-state transition in a native session.                                         |
| `gnoblin.config.reloaded`                 | `path`, `revision`                                                                 | The candidate config commits and becomes active.                                                    |
| `gnoblin.config.reload-failed`            | `path`, `error`                                                                    | Candidate loading, validation, or apply fails while the active runtime stays available.             |
| `gnoblin.workspace.created`               | `workspace`                                                                        | A runtime workspace is created.                                                                     |
| `gnoblin.workspace.renamed`               | `workspace`                                                                        | A workspace display name changes.                                                                   |
| `gnoblin.workspace.changed`               | `workspace`, `changed`                                                             | Its position, window count, or persistence state changes.                                           |
| `gnoblin.workspace.removed`               | `workspace_id`, `last`                                                             | A temporary workspace is removed.                                                                   |
| `gnoblin.workspace.activated`             | `workspace`, optional `previous_id`                                                | The active workspace changes.                                                                       |
| `gnoblin.workspace.window-moved`          | `window_id`, `from_id`, `to_id`                                                    | A window moves from one workspace to another.                                                       |
| `gnoblin.monitor.added`                   | `monitor`                                                                          | An active logical monitor appears in the native runtime.                                            |
| `gnoblin.monitor.changed`                 | `monitor`, `changed`                                                               | A listed monitor property changes; `changed` names the changed properties.                          |
| `gnoblin.monitor.removed`                 | `monitor_id`, `last`                                                               | An active logical monitor is removed.                                                               |
| `gnoblin.input.device-added`              | `device`                                                                           | An input device appears in the native runtime.                                                      |
| `gnoblin.input.device-removed`            | `device_id`, `last`                                                                | An input device is removed from the native runtime.                                                 |
| `gnoblin.input.sources-changed`           | `sources`                                                                          | The configured, available XKB source list changes.                                                  |
| `gnoblin.input.source-changed`            | `available`, optional `source`                                                     | Mutter confirms a different Gnoblin-owned keymap group, or the current source becomes unknown.      |
| `gnoblin.input.gesture`                   | `gesture`, `phase`, `fingers`, `sequence`, `time`, and gesture-specific fields     | A touchpad gesture phase reaches the standalone native Lua runtime.                                 |
| `gnoblin.launch.changed`                  | `launch`                                                                           | A native launch-feedback record is created or changes state.                                        |
| `gnoblin.portal.grant-added`              | `grant`, `revision`, `sequence`, `time`                                            | A validated persistent portal grant is added or updated.                                            |
| `gnoblin.portal.grant-removed`            | `grant_id`, `kind`, `revision`, `sequence`, `time`                                 | A persistent portal grant is revoked.                                                               |
| `gnoblin.privacy.changed`                 | `state`, `revision`, `sequence`, `time`                                            | A monitored privacy activity changes; `state` is a `PrivacyState` snapshot.                         |
| `gnoblin.capability.changed`              | `capability`, `revision`, `sequence`, `time`                                       | The availability of `microphone-monitor` changes; `capability` is the updated `Capability` record.  |
| `gnoblin.appearance.color-scheme-changed` | `color_scheme` (`default`, `prefer-dark`, `prefer-light`), `sequence`, `time`      | The desktop color-scheme preference changes; socket clients need API 1.34.                          |
| `gnoblin.window.created`                  | `window`                                                                           | The native compositor runtime observes a new managed window.                                        |
| `gnoblin.window.changed`                  | `window_id`, `changed`, `window`                                                   | A mapped window property changes in the native compositor runtime.                                  |
| `gnoblin.window.focused`                  | `window_id`, `window`                                                              | A window gains keyboard focus in the native compositor runtime.                                     |
| `gnoblin.window.unfocused`                | `window_id`, `window`                                                              | A window loses keyboard focus in the native compositor runtime.                                     |
| `gnoblin.window.attention-changed`        | `window_id`, `window`, `demands_attention`                                         | Mutter's attention state changes.                                                                   |
| `gnoblin.window.closed`                   | `window_id`, `last`                                                                | The native compositor runtime removes a managed window.                                             |
| `gnoblin.focus.policy-changed`            | `policy`, `revision`, `sequence`, `time`                                           | The effective focus policy changes after a successful config commit.                                |
| `gnoblin.permission.changed`              | `policy`, `revision`, `sequence`, `time`                                           | The committed portal permission policy changes after a successful config commit.                    |
| `gnoblin.shortcut.activated`              | `shortcut`, `trigger`, `focus_context`                                             | A configured native command shortcut is activated by a trusted key press in the native runtime.     |
| `gnoblin.animation.started`               | `animation`, `target`, `event`                                                     | A configured lifecycle animation or preview begins playback; socket subscription requires API 1.18. |
| `gnoblin.animation.finished`              | `animation`, `target`, `event`, `cancelled`                                        | A configured lifecycle animation or preview completes or is interrupted; API 1.18.                  |
| `gnoblin.operation.completed`             | `operation_id`, `method`, `ok`, `value` or `error`, `revision`, `sequence`, `time` | Native API 1.11 completion event; `error` is an `Error` record.                                     |
| `gnoblin.api.operation-completed`         | `request_id`, `method`, `ok`, `result` or string `error`                           | Legacy completion event retained during migration.                                                  |

Overlapping reload requests report only through their operation results.

The standalone runtime watches `color-scheme` in
`org.gnome.desktop.interface`. It emits this event after the value changes, but
does not report the current value on startup or subscription.

```lua
gnoblin.on("gnoblin.appearance.color-scheme-changed", function(event)
    print("Desktop color scheme: " .. event.color_scheme)
end)
```

Mutter reports these session lock states in `gnoblin.session.lock-state-changed`:

- `unlocked`: no active session lock.
- `covering`: the compositor has started the lock transition.
- `locked`: Mutter confirmed the lock transition.
- `failsafe`: the lock client failed and Mutter retained its failsafe state.

Only `locked` confirms the lock transition. A `session.lock()` operation result
reports request dispatch and does not confirm delivery or lock state.

Native-control API 1.17 adds `gnoblin.privacy.changed`. Its `state` snapshot
reports availability for screen sharing, microphone, camera, and location.
Activity appears only for available sources. Native sessions currently monitor
screen sharing.

When PipeWire monitoring connects or disconnects, API 1.33 emits
`gnoblin.capability.changed` after updating the capability snapshot. An
unavailable record includes one of these reasons:

- `remote_desktop_disabled`: Mutter was built without remote-desktop support.
- `pipewire_unavailable`: the monitor is disconnected or could not connect.

Lua callbacks receive `focus_context` as protected userdata for
`Window:focus(context)`.

Native-control API 1.10 socket clients can subscribe to
`gnoblin.shortcut.activated`. Each connection receives a separate, single-use
token for `window.focus`. Socket tokens never appear in Lua payloads or reach
other connections.

In Lua, `event` identifies the animation definition. Socket frames reserve
`event` for the protocol event name and use `animation_event` for the
definition.

A socket token expires five seconds after the shortcut press. It is revoked
when its connection closes, its event subscription changes, the session locks,
or the config reloads.

Native socket bindings are separate from Lua event registrations:

- API 1.11 adds `shortcut.bind`, `shortcut.unbind`, and
  `gnoblin.shortcut.binding-activated` for connection-owned bindings.
- API 1.22 adds held and modal sessions with
  `gnoblin.shortcut.session.activated`, `gnoblin.shortcut.session.key`, and
  `gnoblin.shortcut.session.ended`.

See the [shortcut session reference](/compositor-bridge#api-version-122-held-and-modal-shortcut-sessions)
for accepted options and event fields.

Native API 1.11 adds a stable operation-completion event. It includes:

- `operation_id`, `method`, and `ok`.
- `value` on success, or an `Error` record with `code` and `message` on failure.

The legacy `gnoblin.api.operation-completed` event remains available with
`request_id`, `result`, and a string error.

A wildcard listener receives both event names during migration. Filter by
`event.name` or handle one form to avoid processing each completion twice.

In Lua, `operation_id` matches the returned operation handle's `id`.

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

When started with `--gnoblin-config PATH`, Mutter sends window, workspace,
monitor, input, and launch-feedback events directly to Lua. Native `sequence`
numbers increase across that stream. Gesture events use their own sequence and
have no state revision.

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

Create, rename, and activation events include a `workspace` record. Workspace
change events include a `changed` array listing `number`, `window_count`, or
`persistent`. The `window_count` value matches immediate workspace reads.

A removal carries `workspace_id` and `last`, the final record before removal.
The moved event has no workspace record; it reports the stable window ID and
the source and destination workspace IDs.

The attention event also carries `demands_attention` at the top level. It can
follow a focus request that policy did not activate; it reports Mutter's
attention state and does not identify why the window requested attention.

Window records include the fields available in standalone compositor snapshots:

- Identity: `id`, `title`, `app_id`, `gtk_app_id`, `wm_class`, `rule_app_id`, and optional `role`.
- Location: `workspace_id`, `workspace_number`, `monitor_id`, `monitor_index`, `parent`.
- State: `maximized`, `fullscreen`, `minimized`, `focused`, `above`, `sticky`, `modal`, and `demands_attention`.
- Capabilities: `closable`, `minimizable`, `maximizable`, `movable`, and `resizable`.
- Type: `type`, an integer `MetaWindowType` value.
- Geometry: `frame` and the optional `monitor` origin.
- Interaction: `last_user_time`.
- Record metadata: `revision`, the state revision for this record.

A field is omitted when the native record does not provide it. `monitor_id` is
the active connector name for the current logical monitor. Cloned outputs use
the lexicographically first active connector. `monitor_index` is the current
Mutter order and can change when outputs change.

The `type` value follows Mutter's `MetaWindowType` enum. It describes the
client surface rather than shell presentation.

`modal` is true for type value `4`; `changed` includes `modal` when it changes.

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

A closed event's `last` field contains the final available window record.

`removed` reports the workspace's last record before removal. Its `number` is
the position immediately before removal.

When `workspace.create` uses `activate = true`, Gnoblin emits `created` first
with `active = false`, then emits `activated` after switching workspaces.

A Lua API call returns an `Operation` handle with its current status, result,
failure reason, and a completion callback. See the [Lua runtime API](/config/runtime-api)
for the handle fields and callback behavior.

## Listen to every forwarded event

Pass `"*"` to receive every event source currently forwarded by Gnoblin:

```lua
gnoblin.on("*", function(event)
    print(event.name)
end)
```

Registering a name does not create an event source; Mutter or Gnoblin must
dispatch it. Signal availability follows the Mutter version used to build
Gnoblin.

`gnoblin.listeners` maps each registered event name to its callback list.
Inspect it to see which handlers earlier config files have added. The list is
for inspection; remove handlers through their `Subscription`:

```lua
for name, callbacks in pairs(gnoblin.listeners) do
    print(name, #callbacks)
end
```

Callbacks run synchronously on the supervised Lua runtime's event loop,
separate from Mutter's compositor main thread. Keep them short, especially
handlers for high-frequency `*.input.*` events. Operations requested from a
callback are dispatched after it returns.

`gnoblin.configure` changes follow the usual live-setting rules. Startup-only
settings still need a new session. If any listener fails, Gnoblin logs the
error and continues with the remaining listeners. It rolls back configuration
changes from that event, but still dispatches operations queued by its
callbacks.

If the resulting config fails validation, Gnoblin restores the last valid
config and fails those queued operations. For settings applied by the
compositor, Gnoblin applies the updated document after the callback returns.

## Type definition

Event names are open-ended: Gnoblin accepts a nonempty UTF-8 string of up to
128 bytes with no NUL byte, but only names dispatched by Gnoblin or Mutter
produce callbacks. The event fields depend on the event name; all
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
