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

`gnoblinctl lua` can also subscribe while a script runs. It stays open until
every subscription has ended. A one-time subscription ends after its first
event; keep a repeating subscription in a variable so the callback can call
`subscription:unsubscribe()`. Press Ctrl+C to stop listeners manually.

For desktop color-scheme change notifications, subscribe to
`gnoblin.appearance.color-scheme-changed`. The [appearance guide](/guides/theming)
explains how applications use the preference. Use
`gnoblin.appearance.color_scheme()` to read the current value.

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

Gnoblin forwards signals from these Mutter sources:

- Display and windows: `mutter.display`, `mutter.window`.
- Workspaces: `mutter.workspace-manager`, `mutter.workspace`.
- Compositor state: `mutter.backend`, `mutter.monitor-manager`,
  `mutter.cursor-tracker`.

It watches newly created windows and workspaces as they appear.

Gnoblin starts this signal watcher only when the config registers one of these
events or the `*` listener.

Gnoblin also exposes the Wayland pointer-window transition as
`mutter.wayland.pointer-window-changed`, with `app_id`, `wm_class`, and
`title`.

Touchpad swipe, pinch, and hold input is available as
`mutter.touchpad.gesture`. Mutter sends each phase through Gnoblin's native
compositor handler. That handler makes any synchronous input-claim decision and
queues the event for the supervised Lua runtime.

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

Lua callbacks receive the queued event asynchronously and cannot claim or
consume the Mutter input event. The compositor handles configured direct
actions and commands in the `any` context synchronously. During an active
session lock, it also handles `unlock-screen` bindings. Other contexts do not
have a Gnoblin UI handler.

The standalone native runtime also dispatches the stable
`gnoblin.input.gesture` event to Lua. Its payload includes:

- `gesture`, `phase`, `fingers`, `sequence`, and `time` on every event.
- `dx` and `dy` on swipe events; `scale` and `angle_delta` on pinch events.
- `input_time`, Mutter's original input timestamp.

Within the gesture stream, `sequence` increases and `time` uses monotonic-clock
microseconds. Device names and input tokens are omitted.

Socket clients receive the stable `gnoblin.input.gesture` event, not Mutter's
internal event. They need API 1.9 and request it with `op: "events"`. Frames
use `event` instead of Lua's `name` and include socket-stream sequence and
time. See the [compositor bridge](/compositor-bridge).

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
- `window-added` and `window-removed` include a window identity record in
  `argN`. It contains `window_id`, `app_id`, and `window_title`; `argN_type` is
  `MetaWindow`.
- `mutter.window.*` events include `window_id`, `app_id`, and `window_title`.

Values that cannot be represented as simple Lua event fields are omitted or
reduced to a type or name string.

### Gnoblin

The native `window.*` lifecycle events require the direct Mutter runtime.

Every Lua event record includes `name`. The table below lists event-specific
fields.

Window event records also include `revision`, `sequence`, and monotonic `time`.
Nested `window` and `last` records carry their own `revision`, matching the
event revision.

| Event name                                 | Fields                                                                             | Dispatched when                                                                                               |
| ------------------------------------------ | ---------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| `gnoblin.session.lock-state-changed`       | `state`, `revision`, `sequence`, `time`                                            | Mutter reports a lock-state transition in a native session.                                                   |
| `gnoblin.runtime.status-changed`           | `state`, `generation`, `sequence`, `time`                                          | The worker state or accepted runtime generation changes; matches `gnoblin.runtime.status()`.                  |
| `gnoblin.config.reloaded`                  | `path`, `revision`, `sequence`, `time`                                             | The candidate config commits and becomes active.                                                              |
| `gnoblin.config.reload-failed`             | `path`, `error`, `sequence`, `time`                                                | Candidate loading, validation, or apply fails while the active runtime stays available.                       |
| `gnoblin.workspace.created`                | `workspace`                                                                        | A runtime workspace is created.                                                                               |
| `gnoblin.workspace.renamed`                | `workspace`                                                                        | A workspace display name changes.                                                                             |
| `gnoblin.workspace.changed`                | `workspace`, `changed`                                                             | Its position, window count, or persistence state changes.                                                     |
| `gnoblin.workspace.removed`                | `workspace_id`, `last`                                                             | A temporary workspace is removed.                                                                             |
| `gnoblin.workspace.activated`              | `workspace`, optional `previous_id`                                                | The active workspace changes.                                                                                 |
| `gnoblin.workspace.window-moved`           | `window_id`, `from_id`, `to_id`                                                    | A window moves from one workspace to another.                                                                 |
| `gnoblin.monitor.added`                    | `monitor`                                                                          | An active logical monitor appears in the native runtime.                                                      |
| `gnoblin.monitor.changed`                  | `monitor`, `changed`                                                               | A listed monitor property changes; `changed` names the changed properties.                                    |
| `gnoblin.monitor.removed`                  | `monitor_id`, `last`                                                               | An active logical monitor is removed.                                                                         |
| `gnoblin.layer.created`                    | `layer`                                                                            | A layer-shell surface appears in the published snapshot.                                                      |
| `gnoblin.layer.changed`                    | `layer_id`, `layer`, `changed`                                                     | A published layer record changes, including mapped or unmapped transitions.                                   |
| `gnoblin.layer.removed`                    | `layer_id`, `last`                                                                 | A layer-shell surface disappears from the published snapshot.                                                 |
| `gnoblin.input.device-added`               | `device`                                                                           | An input device appears in the native runtime.                                                                |
| `gnoblin.input.device-removed`             | `device_id`, `last`                                                                | An input device is removed from the native runtime.                                                           |
| `gnoblin.input.sources-changed`            | `sources`                                                                          | The configured, available XKB source list changes.                                                            |
| `gnoblin.input.source-changed`             | `available`, optional `source`                                                     | Mutter confirms a different Gnoblin-owned keymap group, or the current source becomes unknown.                |
| `gnoblin.input.orientation-lock-changed`   | `available`, `locked`, `orientation`, `source`, `revision`, `sequence`, `time`     | Orientation-lock state or its source changes.                                                                 |
| `gnoblin.input.gesture`                    | `gesture`, `phase`, `fingers`, `sequence`, `time`, and gesture-specific fields     | A touchpad gesture phase reaches the standalone native Lua runtime.                                           |
| `gnoblin.launch.changed`                   | `launch`                                                                           | A native launch-feedback record is created or changes state.                                                  |
| `gnoblin.portal.grant-added`               | `grant`, `revision`, `sequence`, `time`                                            | A validated persistent portal grant is added or updated.                                                      |
| `gnoblin.portal.grant-removed`             | `grant_id`, `kind`, `revision`, `sequence`, `time`                                 | A persistent portal grant is revoked.                                                                         |
| `gnoblin.privacy.changed`                  | `state`, `revision`, `sequence`, `time`                                            | A monitored privacy activity changes; `state` is a `PrivacyState` snapshot.                                   |
| `gnoblin.location.authorization-requested` | `request_id`, `app_id`, `requested_accuracy`, `expires_at_us`, `sequence`, `time`  | GeoClue asks Gnoblin to authorize an application's location request.                                          |
| `gnoblin.capability.changed`               | `capability`, `revision`, `sequence`, `time`                                       | A native capability changes; `capability` is the updated `Capability` record.                                 |
| `gnoblin.appearance.color-scheme-changed`  | `color_scheme` (`default`, `prefer-dark`, `prefer-light`), `sequence`, `time`      | The desktop color-scheme preference changes; socket clients need API 1.34.                                    |
| `gnoblin.window.created`                   | `window`                                                                           | The native compositor runtime observes a new managed window.                                                  |
| `gnoblin.window.changed`                   | `window_id`, `changed`, `window`                                                   | A mapped window property changes in the native compositor runtime.                                            |
| `gnoblin.window.focused`                   | `window_id`, `window`                                                              | A window gains keyboard focus in the native compositor runtime.                                               |
| `gnoblin.window.unfocused`                 | `window_id`, `window`                                                              | A window loses keyboard focus in the native compositor runtime.                                               |
| `gnoblin.window.attention-changed`         | `window_id`, `window`, `demands_attention`                                         | Mutter's attention state changes.                                                                             |
| `gnoblin.window.activation-denied`         | `window_id`, `reason`                                                              | Strict focus policy denies an application's activation request; socket clients need API 1.69.                 |
| `gnoblin.window.closed`                    | `window_id`, `last`                                                                | The native compositor runtime removes a managed window.                                                       |
| `gnoblin.focus.policy-changed`             | `policy`, `revision`, `sequence`, `time`                                           | The effective focus policy changes after a successful config commit.                                          |
| `gnoblin.permission.changed`               | `policy`, `revision`, `sequence`, `time`                                           | The committed portal permission policy changes after a successful config commit.                              |
| `gnoblin.shortcut.activated`               | `shortcut`, `trigger`, `focus_context`                                             | A configured native command shortcut is activated by a trusted key press in the native runtime.               |
| `gnoblin.shortcut.binding-activated`       | `id`, `accelerator`, `trigger`, `first`, `modifiers`, `time`, `focus_context`      | A Lua-registered dynamic shortcut activates. Only the first activation can carry focus authority.             |
| `gnoblin.shortcut.binding-deactivated`     | `id`, `accelerator`, `input_time`                                                  | A press-triggered dynamic shortcut is physically released.                                                    |
| `gnoblin.shortcut.session.activated`       | `id`, `session_id`, `first`, `trigger`, `modifiers`, `time`                        | API 1.22. A held dynamic shortcut starts or repeats its modal session.                                        |
| `gnoblin.shortcut.session.key`             | `id`, `session_id`, key fields, optional `focus_context`                           | API 1.22. A modal key event; real, non-repeat input may carry one-use focus authority.                        |
| `gnoblin.shortcut.session.ended`           | `id`, `session_id`, `reason`, `time`                                               | API 1.22. A held shortcut session ends or is cancelled.                                                       |
| `gnoblin.osd.requested`                    | `monitor_id`, `output_names?`, `icon?`, `label?`, `sequence`, `time`               | API 1.27. Mutter requests an OSD; a shell decides whether and how to display it.                              |
| `gnoblin.animation.started`                | `animation`, `target`, `event`, `cancelled`                                        | A configured window or layer animation, workspace transition, or preview starts; socket API 1.18.             |
| `gnoblin.animation.finished`               | `animation`, `target`, `event`, `cancelled`                                        | A configured window or layer animation, workspace transition, or preview completes or stops; socket API 1.18. |
| `gnoblin.operation.completed`              | `operation_id`, `method`, `ok`, `value` or `error`, `revision`, `sequence`, `time` | Native API 1.11 completion event; `error` is an `Error` record.                                               |

Started animation events include `cancelled: false`. Finished events set it to
`true` when playback is interrupted and `false` when it reaches its end.

For `workspace-switch`, `target` and `to_workspace` are the destination
workspace ID. `from_workspace` is the workspace being left.

The `direction` field can be `left`, `right`, `up`, `down`, `up-left`,
`up-right`, `down-left`, or `down-right`. Diagonal values mean the transition
moves on both axes. Other animation events do not include workspace fields.

`gnoblin.runtime.status-changed` mirrors the `state` and `generation` returned
by `gnoblin.runtime.status()`.

External clients can observe `restarting`. A Lua worker cannot run callbacks
while suspended. Its replacement receives `running` after restoring state.
When the supervisor reports that it will not restart the worker, Mutter emits a
final `unavailable` event while it remains alive.

Events are not replayed. Subscribe before reading the status snapshot, and
read it again after reconnecting. The compositor can close the socket without
a final event if it exits first.

Structured event values are read-only snapshots. Call methods on a lifecycle
event's `window` record to act on that window:

```lua
gnoblin.on("gnoblin.window.changed", function(event)
    if event.window.demands_attention then
        event.window:set_above(true)
    end
end)
```

### Reloads and appearance

When reload requests overlap, inspect their `Operation` results. Gnoblin emits
no separate event.

`gnoblin.appearance.color-scheme-changed` fires after the desktop color-scheme
preference changes. It does not report the initial value. The preference is
`color-scheme` in `org.gnome.desktop.interface`. Read the current value with
`gnoblin.appearance.color_scheme()`; it returns `nil` if the schema or key is
unavailable.

```lua
gnoblin.on("gnoblin.appearance.color-scheme-changed", function(event)
    print("Desktop color scheme: " .. event.color_scheme)
end)
```

### Session state

`gnoblin.session.lock-state-changed` reports these session lock states:

- `unlocked`: no active session lock.
- `covering`: the compositor has started the lock transition.
- `locked`: Mutter confirmed the lock transition.
- `failsafe`: the lock client failed and Mutter retained its failsafe state.

Only `locked` confirms the lock transition. A `session.lock()` operation result
reports request dispatch and does not confirm delivery or lock state.

`gnoblin.privacy.changed` reports available screen-sharing, microphone, camera,
and location activity in `state`. Activity appears only for available sources.
The native runtime monitors screen sharing, PipeWire microphone and camera
activity, and GeoClue location activity. GeoClue authorization requests arrive
through `gnoblin.location.authorization-requested`; answer them with
`gnoblin.location.authorize_app`. See the [runtime API reference](runtime-api.md#privacy-and-permissions)
for accuracy levels, deadlines, and the authorization rules.

`gnoblin.capability.changed` fires when a monitored native capability becomes
available or unavailable. Gnoblin updates the capability snapshot first.
Unavailable monitor capabilities report one of these reasons:

- `remote_desktop_disabled`: Mutter was built without remote-desktop support.
- `pipewire_unavailable`: the monitor is disconnected or could not connect.

Pass `focus_context` to `Window:focus(context)` to use the authority granted to
that event. Lua cannot inspect or create the protected value.

### Shortcuts and operations

Socket clients can subscribe to `gnoblin.shortcut.activated`. Each connection
receives a separate, single-use token for `window.focus`. The token expires
five seconds after the shortcut press. It is revoked when its connection closes,
its event subscription changes, the session locks, or the config reloads. Tokens
never appear in Lua payloads or reach other connections.

Lua listeners may receive `event.focus_context` on real, non-repeated
`gnoblin.shortcut.session.key` events. Repeated events do not carry authority.
Synthetic and input-method events are excluded from modal key events.

The userdata expires after five seconds and authorizes one focus-sensitive
compositor operation. The event fields are `keyval`, `keycode`, `modifiers`,
`phase`, and `time`. Gnoblin still delivers a captured key event if it cannot
issue a context.

Socket clients receive `focus_context` as a connection-bound token starting
with API 1.68. Only the binding owner receives it.

In Lua animation events, `event` identifies the animation definition. Socket
frames use `event` for the protocol event name and `animation_event` for the
definition.

Event order depends on the binding's `trigger`:

- `"press"` activates on key-down and deactivates on key-up.
- `"release"` activates on key-up and has no later deactivation event.

Lua callbacks and socket clients can subscribe to both events. Socket clients
can also bind dynamic shortcuts and request held or modal shortcut sessions.

See the [shortcut session reference](/compositor-bridge#api-version-122-held-and-modal-shortcut-sessions)
for accepted options and event fields.

Use `gnoblin.operation.completed` to observe asynchronous operations:

- `operation_id`, `method`, and `ok`.
- `value` on success, or an `Error` record with `code` and `message` on failure.

In Lua, `operation_id` matches the returned operation handle's `id`.

### Portal events

Gnoblin updates the portal-grant snapshot before dispatching either grant
event. Events include a revision, sequence, and monotonic time.

- `gnoblin.portal.grant-added` carries a read-only `PortalGrant` snapshot.
- `gnoblin.portal.grant-removed` carries the opaque grant ID and portal kind.

`gnoblin.permission.changed` fires after a successful config commit changes the
portal policy. It carries the committed policy and event metadata.

The policy contains a default level, ordered rules, and the committed settings
revision. Its revision matches the event's top-level revision. Rejected or
unchanged policies do not emit this event.

The compositor socket and its directory are accessible only to the current
user. Any process running as that user can subscribe. Treat same-user socket
clients as trusted shell components. See the
[compositor bridge](/compositor-bridge#api-version-110-shortcut-focus-grants)
for the request format.

### Launch events

With `--gnoblin-config PATH`, Mutter sends window, workspace, monitor, input,
and launch-feedback events directly to Lua. `sequence` increases across this
event stream. Gesture events use their own sequence and have no state revision.

`gnoblin.launch.changed` wraps the record under `launch`. It contains:

- `token` and `application`.
- `started_at`, as Unix time in milliseconds.
- `timeout_ms`, `state`, and `revision`.

States are `pending`, `started`, `ended`, and `timed_out`. Mutter reports a
launch when a matching mapped window appears or becomes focused. It cannot
report failures from an external process launcher. The revision is scoped to
launch records and does not advance the compositor `state_revision`.

Shortcut capture also completes through `gnoblin.operation.completed`:

- Success returns the accelerator in `value.accelerator`.
- Cancellation, timeout, a locked session, or an input grab returns an `Error` record.

The capture hook consumes key events while active, then cancels and releases
the hook if the session locks or another input grab starts.

### Monitor and workspace records

Monitor records describe active logical monitors. The `id` is the canonical
connector name. A monitor event's `changed` field lists updated properties:

- Position: `index`, `x`, `y`, `width`, `height`.
- Display: `primary`, `scale`, `enabled`, `refresh_rate`, `transform`.
- Identity: `id`, `name`, `make`, `model`, `serial`.

Create, rename, and activation events include a `workspace` record. Workspace
change events include a `changed` array listing `number`, `window_count`, or
`persistent`. The `window_count` value matches immediate workspace reads.

A removal carries `workspace_id` and `last`, the final record before removal.
The moved event reports the stable window ID and source and destination
workspace IDs. It has no workspace record.

The attention event also carries `demands_attention` at the top level. It can
follow a focus request that policy did not activate; it reports Mutter's
attention state and does not identify why the window requested attention.

Strict focus policy also emits `gnoblin.window.activation-denied` when Mutter
rejects an application's activation request. The event's `reason` is one of:

- `missing_context`: the request has no activation context.
- `invalid_context`: its token is not valid.
- `stale_context`: its startup context is too old.

Socket clients need native-control API 1.69 to subscribe to this event. It
contains no activation token or input serial.

### Window records

Window records contain the fields available from the compositor:

- Identity: `id`, `title`, `app_id`, `gtk_app_id`, `wm_class`, `rule_app_id`, and optional `role`.
- Location: `workspace_id`, `workspace_number`, `monitor_id`, `monitor_index`, `parent`.
- State: `maximized`, `fullscreen`, `minimized`, `focused`, `above`, `sticky`, `modal`, and `demands_attention`.
- Capabilities: `closable`, `minimizable`, `maximizable`, `movable`, and `resizable`.
- Type: `type`, an integer `MetaWindowType` value.
- Geometry: `frame` and the optional `monitor` origin.
- Interaction: `last_user_time`.
- Record metadata: `revision`, the state revision for this record.

A field is omitted when the compositor does not provide it. `monitor_id` is
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

Layer lifecycle events follow the latest native layer snapshot. Each carries
the shared state `revision`, `sequence`, and monotonic `time`. The stable ID is
in `layer.id` on `created`; `changed` and `removed` also carry `layer_id`.

- `created` includes the record when its layer role first appears in the
  snapshot. It can initially be unmapped.
- `changed` includes the updated `layer` and a `changed` array. The array names
  fields that changed, except for per-record `revision`. Mapped transitions are
  changes.
- `removed` includes the final record as `last` and is sent when the layer
  disappears from the snapshot.

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
