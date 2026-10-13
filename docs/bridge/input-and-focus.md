# Input and Focus Operations

Use event-bound contexts to handle shell presentation requests, insert text, or arrange a window after trusted input.

These operations require the minimum API versions stated below and a live, unlocked session where noted.

### API 1.27: shell presentation requests

Subscribe to `gnoblin.window.menu-requested` and `gnoblin.osd.requested` to
handle requests that previously depended on shell-owned UI. Gnoblin reports
requests; the external shell owns their presentation.

The window-menu event contains:

- `window_id`: the stable Gnoblin window ID;
- `menu_type`: `wm` for a window manager menu or `app` for an application menu;
- `x` and `y`: global logical coordinates from Mutter.

The OSD event includes:

- `monitor_id`: the stable ID of the logical monitor;
- `output_names`: the sorted, unique names of active physical connectors for
  that logical monitor. Older API 1.27 builds may omit this field;
- `icon` and `label`: optional fields supplied by Mutter.

Mutter supplies no OSD level or maximum.

### API 1.30: WM menu actions

API 1.30 adds `menu_context` to WM requests from
`gnoblin.window.menu-requested`. App menu requests remain informational and
carry no authority.

The token belongs to the receiving connection. It expires after five seconds
and is bound to the live window that raised the menu. It cannot be reused as an
expiry timestamp or a window ID.

Use the token with the existing interactive-grab methods without supplying a
window ID:

```json
{
    "op": "api",
    "id": "menu-move",
    "api_version": { "major": 1, "minor": 30 },
    "method": "window.begin_move",
    "arguments": { "menu_context": "TOKEN_FROM_WM_MENU_EVENT" }
}
```

`window.begin_resize` accepts the same token and an `edge` from the eight
`ResizeEdge` values documented for API 1.12. Each token authorizes one
operation and is consumed even when its arguments are invalid.

A request rejected for an unsupported API version or a minor below 1.30 still
consumes its matching token; request a new WM menu event before retrying.

Before starting Mutter's keyboard grab with a fresh compositor timestamp,
Gnoblin resolves the original window again and checks that it is live and
movable or resizable. It also checks that the session is unlocked. Disconnect,
event-subscription replacement, session lock, runtime/config teardown, and use
revoke the token.

### API 1.28: text insertion and keyboard snapping

Subscribe to `gnoblin.shortcut.activated` and pass its one-use
`focus_context` to `input.text_target`:

```json
{
    "op": "api",
    "api_version": { "major": 1, "minor": 28 },
    "id": "text-target-1",
    "method": "input.text_target",
    "arguments": { "focus_context": "<token from the event>" }
}
```

The response contains an opaque `target`, and may include `window_id` and
`caret`. Pass the target to `input.insert_text` with the text:

```json
{
    "op": "api",
    "api_version": { "major": 1, "minor": 28 },
    "id": "insert-text-1",
    "method": "input.insert_text",
    "arguments": { "target": "<target from the response>", "text": "😀" }
}
```

Text must be 1–256 bytes of valid UTF-8, with no NUL or control characters.
Insertion requires the same active text-input-v3 client and focus epoch, an
unlocked session, and the allowed modifier state unchanged. Success returns
`inserted: true`. X11 clients are unsupported.

For keyboard-selected layouts, pass the shortcut event's `focus_context` to
`window.snap_context`. Gnoblin binds the context to the focused movable window.

```json
{
    "op": "api",
    "api_version": { "major": 1, "minor": 28 },
    "id": "snap-context-1",
    "method": "window.snap_context",
    "arguments": { "focus_context": "<token from the event>" }
}
```

The response includes an opaque, connection-bound `context`. It also reports
`window_id`, `monitor_id`, monitor geometry, and work-area geometry. The
context expires after five seconds; its expiry timestamp is not exposed. Use it
with a frame inside the selected monitor's work area:

```json
{
    "op": "api",
    "api_version": { "major": 1, "minor": 28 },
    "id": "snap-1",
    "method": "window.snap",
    "arguments": {
        "context": "<context from the response>",
        "monitor_id": "<active monitor ID>",
        "frame": { "x": 0, "y": 0, "width": 700, "height": 900 }
    }
}
```

`window.snap` checks the monitor and work-area bounds before moving the window.
Success returns `committed: true`, `window_id`, and `monitor_id`.

### API 1.38: restore a snapped window

Call `window.restore_or_minimize` with a stable window ID:

```json
{
    "op": "api",
    "api_version": { "major": 1, "minor": 38 },
    "id": "restore-window-1",
    "method": "window.restore_or_minimize",
    "arguments": { "id": "42" }
}
```

Mutter unmaximizes a maximized window, restores its saved pre-snap frame when
available, or minimizes it. The response includes `id` and `action`, whose
value is `unmaximize`, `restore`, or `minimize`. The operation fails while the
session is locked or if the target window is no longer available. Every
supported socket client uses the shared Lua operation, which requires the Lua
supervisor.

Targets and contexts belong to the connection that created them:

- Other connections cannot use or consume them.
- The owner loses a token on its first commit attempt, even if another
  argument is invalid.
- Gnoblin revokes tokens when the owner disconnects, replaces its event
  subscription, locks the session, or reloads the config.
- The compositor retains at most 128 active text targets and 128 active snap
  contexts. Creation fails at the limit until a token expires or is revoked.

### Pointer snap offers

Subscribe before a move begins to receive drag state:

- `gnoblin.window.drag.started` starts a drag record.
- `gnoblin.window.drag.updated` refreshes its pointer and monitor state.
- `gnoblin.window.drag.ended` reports its result.

Started and updated events include an unpredictable `drag_token` for the
receiving connection. Use it on that same connection to call
`window.snap.offer` with the event's `id` as `drag_id`.

An offer contains 1 to 128 targets. Each target has a unique `id` and integer
logical-coordinate `hit` and `frame` rectangles. Both rectangles must fit in
the current work area.

Targets may also set `maximize`, `required_modifiers`, or
`forbidden_modifiers`. Modifier filters currently accept only `control`.

The token is bound to the connection, live drag, and runtime generation. The
first accepted Lua or socket offer owns the drag; only its owner can replace
the target list. Disconnect or replacing the event subscription revokes a
socket token and clears its offer.

Mutter rechecks the release pointer, modifiers, monitor, and work area before
applying a match. Trusted keyboard `SnapContext` is available to Lua only.

The socket exposes these reads at the listed API versions:

| API version | Socket method             | Lua read                            | Arguments                                                  |
| ----------- | ------------------------- | ----------------------------------- | ---------------------------------------------------------- |
| 1.19        | `version`                 | `gnoblin.version()`                 | None                                                       |
| 1.19        | `capabilities.list`       | `gnoblin.capabilities.list()`       | None                                                       |
| 1.19        | `focus.history`           | `gnoblin.focus.history(filter)`     | `workspace_id`, `monitor_id`, `limit`                      |
| 1.19        | `settings`                | `gnoblin.settings`                  | None                                                       |
| 1.19        | `focus.policy`            | `gnoblin.focus.policy`              | None                                                       |
| 1.37        | `windows.list`            | `gnoblin.windows.list(filter)`      | `app_id`, `title`, `focused`, `workspace_id`, `monitor_id` |
| 1.37        | `workspaces.list`         | `gnoblin.workspaces.list()`         | None                                                       |
| 1.37        | `monitors.list`           | `gnoblin.monitors.list()`           | None                                                       |
| 1.74        | `monitors.privacy_screen` | `gnoblin.monitors.privacy_screen()` | None                                                       |
| 1.37        | `layers.list`             | `gnoblin.layers.list(filter)`       | `monitor_id`, `namespace`, `layer`                         |
| 1.37        | `launches.list`           | `gnoblin.launches.list()`           | None                                                       |
| 1.39        | `launches.snapshot`       | `gnoblin.launches.snapshot()`       | None                                                       |
| 1.40        | `shortcuts.list`          | `gnoblin.shortcuts.list()`          | None                                                       |
| 1.41        | `shortcuts.actions`       | `gnoblin.shortcuts.actions(group?)` | Optional `group`: `wm`, `mutter`, or `wayland`             |
| 1.42        | `permissions.list`        | `gnoblin.permissions.list()`        | None                                                       |
| 1.43        | `permissions.check`       | `gnoblin.permissions.check(args)`   | `capability`, `identity`                                   |
| 1.44        | `permissions.policy`      | `gnoblin.permissions.policy()`      | None                                                       |
| 1.45        | `portals.grants`          | `gnoblin.portals.grants(filter?)`   | Optional `kind`: `screen-cast` or `remote-desktop`         |
| 1.24        | `session.activity`        | `gnoblin.session.activity()`        | None                                                       |
| 1.29        | `session.status`          | `gnoblin.session.status()`          | None                                                       |
| 1.70        | `appearance.color_scheme` | `gnoblin.appearance.color_scheme()` | None                                                       |

`session.status` returns:

- `state: "running"` when the compositor answers the request.
- `lock_available`, plus `lock_state` when that value is true.
- `revision`, which identifies the lock-state snapshot and matches the
  `revision` on the corresponding lock-state event.

Mutter answers this socket read directly from its current state, so the read
does not require the Lua supervisor. It reports compositor availability, not
Lua supervisor health. The socket cannot report a final state after the
compositor stops; a failed connection means the compositor is unavailable.

API 1.20 adds `runtime.reload_config()` with no arguments. In a standalone
native session, reload validates and applies the complete Lua configuration
while the session is running. Native settings, policies, autostart entries,
and Lua callbacks are updated as one transaction.

If loading or applying the candidate fails, Gnoblin keeps the previous
configuration active. The native open-animation matcher uses updated rules for
windows mapped after reload.

Reload does not replay open animations for windows already mapped.

The result contains `ok: true`, `action: "config reload"`, and
`runtime_generation`. This is the generation the new runtime will receive if
Gnoblin activates the new config.

The Lua operation succeeds after Gnoblin loads and validates the new config.
It does not mean the config is active yet. Subscribe to
`gnoblin.config.reloaded` to know when the new config is active. If the
session stops first, Gnoblin discards the new config and does not send this
event.

The socket returns success after the new config is active. If the session stops
before then, the connection may close before the server sends an error. Treat
that disconnect as a failed request.

API 1.20 clients can subscribe to config reload events:

- `gnoblin.config.reloaded` includes `path`, `revision`, `sequence`, and
  monotonic-clock `time`.
- `gnoblin.config.reload-failed` includes `path`, an error string, `sequence`,
  and monotonic-clock `time`.

Lua listeners receive the same event fields.
If a reload is already in progress, a second request returns an error. It does
not emit `gnoblin.config.reload-failed` because Gnoblin has not loaded or
validated a second config.

#### API 1.68: modal key focus contexts

- Lua listeners may receive a `FocusContext` userdata. API 1.68 socket clients
  may receive `focus_context`, a one-use token bound to their connection.
- Gnoblin issues a context only for real, non-repeated key presses and
  releases. Synthetic and input-method events are excluded.
- A context expires after five seconds and authorizes one focus-sensitive
  compositor operation.
- Gnoblin still delivers the key event if no context is available.

If the Lua runtime stops while a socket client owns the active session, Gnoblin
ends it with `runtime_stopped`.

Modal and bare-Super sessions capture those key events for the shell instead
of delivering them to the focused application. A session ends when its held
modifier is released, after ten seconds, on lock, or when the binding is
unbound or its owner disconnects.

```json
{
    "op": "api",
    "api_version": { "major": 1, "minor": 22 },
    "id": "bind-switcher",
    "method": "shortcut.bind",
    "arguments": {
        "id": "switcher",
        "accelerator": "<Alt>Tab",
        "hold": "alt",
        "mode": "modal"
    }
}
```

The shell should subscribe before binding if it needs session events. Bare
Super capture is intended for shells that open their overlay after the key is
released.

### API version 1.13: focus policy events

Subscribe to `gnoblin.focus.policy-changed` with an API 1.13 event request.
Gnoblin sends the event after a successful config commit when the effective
focus policy changes. Failed or rejected config changes do not emit it.

```json
{ "op": "events", "api_version": { "major": 1, "minor": 13 }, "events": ["gnoblin.focus.policy-changed"] }
```

The event contains `policy`, `revision`, `sequence`, and monotonic `time`.
`policy` is the committed focus-policy snapshot. `revision` matches
`policy.revision`; both identify the committed settings snapshot.

### API version 1.32: XDG Activation focus and session logout

API 1.32 accepts `activation_token` as an alternative to `focus_context` for
`window.focus`. The token must come from an XDG Activation request made by the
process that owns the compositor socket connection.

Gnoblin resolves the stable target ID, asks Mutter to validate the token's
source surface and input serial, and consumes the token before focusing the
window. Tokens cannot be replayed. Requests do not accept a caller-supplied
timestamp or serial.

```json
{
    "op": "api",
    "id": "focus",
    "api_version": { "major": 1, "minor": 32 },
    "method": "window.focus",
    "arguments": { "id": "42", "activation_token": "TOKEN_FROM_XDG_ACTIVATION" }
}
```

`session.logout` takes an empty `arguments` object and returns an operation
descriptor. On success, Gnoblin shuts down the compositor and its session
wrapper stops the session's user services. The desktop returns to the login
manager.

A client subscribed to `gnoblin.operation.completed` can see the operation
completion before shutdown, but the socket may close before the reply or
completion is delivered. Do not retry after a disconnect; the session may
already be closing.

```json
{
    "op": "api",
    "id": "logout",
    "api_version": { "major": 1, "minor": 32 },
    "method": "session.logout",
    "arguments": {}
}
```

### API version 1.35: snake_case window fields

Window snapshots and window-change event records now include snake_case fields
that match Lua records. New clients should use:

- `app_id`, `workspace_id`, and `monitor_id`;
- `last_user_time`;
- `frame`, which aliases the previous `geometry` field.

API 1.x retains camelCase fields for compatibility. Socket window lifecycle
events include both naming styles in `window` and `last` records, and list both
spellings in `changed`. Lua callbacks use snake_case names only.

### API version 1.69: focus denial events

Subscribe to `gnoblin.window.activation-denied` to learn when strict focus
policy rejects an application's activation request. The event includes a
stable `window_id` and a `reason`:

- `missing_context`: the request has no activation context.
- `invalid_context`: its token is not valid.
- `stale_context`: its startup context is too old.

It does not include activation tokens or input serials.

### API version 1.73: tablet-pad help requests

Subscribe to the help request from a tablet-pad button action with API 1.73:

```json
{
    "op": "events",
    "api_version": { "major": 1, "minor": 73 },
    "events": ["gnoblin.input.pad-help-requested"]
}
```

The event contains:

- `device`: the current input-device record;
- optional `monitor_id` and `output_names`;
- zero-based `buttons` with optional localized labels;
- `mode_groups` with mode counts and switch-button numbers;
- `features` for rings, strips, and dials, with optional labels;
- `edition_mode`, which is false for tablet-button help.

Lua callbacks receive the same event. The shell owns the overlay and can
dismiss it at any time. Gnoblin retains no OSD actor, so later help presses
produce new events.

## Input devices and XKB sources

`input.devices` returns `{devices, revision}`. Each record has its own
`revision`; device IDs last for the compositor session. API 1.4 adds device
added and removed events after the initial snapshot. Removed events include
`device_id` and the final `last` record. Device paths and enabled state are not
exposed.

API 1.6 adds three XKB input-source methods:

- `input.sources` lists configured layouts and variants.
- `input.current_source` returns `available: false` when Gnoblin did not
  install the active keymap.
- `input.select` takes the exact `type` and `id` from a source record. It
  completes asynchronously after Mutter confirms the keymap change.

Native IBus selection is unsupported. Mutter loads at most four layouts into
one keymap.

Source events include `revision`, `sequence`, and monotonic `time`.
`gnoblin.input.sources-changed` carries the updated list;
`gnoblin.input.source-changed` carries availability and, when known, the
confirmed source.
