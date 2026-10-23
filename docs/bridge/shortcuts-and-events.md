# Shortcuts and Events

Subscribe to compositor events and manage client-owned shortcut bindings.

Event and shortcut operations are available at the minimum API versions stated below; connection-owned bindings end when their connection closes.

### API version 1.9: event subscriptions

Send a request with `op: "events"` and an `events` array to subscribe this
connection to selected native events. Event names must appear in `hello.events`.

Use the existing subscription operations for the `windows` and `monitors`
snapshot names. The greeting lists `generic-event-subscriptions` and
`input-gesture-events` in `hello.capabilities`.

The list may contain up to 64 unique event names. A valid request replaces the
connection's previous event list. Send an empty list to unsubscribe from these
events. Invalid lists leave the previous subscription in place.

```json
{
    "op": "events",
    "api_version": { "major": 1, "minor": 9 },
    "events": ["gnoblin.input.gesture", "gnoblin.window.focused"]
}
```

The server acknowledges a valid request with `event: "subscribed"` and the
accepted event names. Requests without API version 1.9 receive an error.
Subscriptions last until replaced or the connection closes.

The stable `gnoblin.input.gesture` event comes from Mutter's
`mutter.touchpad.gesture` source. `gesture` is `swipe`, `pinch`, or `hold`;
`phase` is `begin`, `update`, `end`, or `cancel`. Every event has `fingers`,
`sequence`, `time`, and Mutter's original `input_time`.

Swipe updates include `dx` and `dy`. Pinch updates include `scale` and
`angle_delta`.

`input_time` is Mutter's original timestamp. Socket `sequence` values increase
across native events and may have gaps when a connection filters other events.
`time` is monotonic-clock microseconds. Device names and private focus contexts
are not sent.

### API version 1.10: shortcut focus grants

Subscribe to `gnoblin.shortcut.activated` with an API 1.10 event request. The
event is sent only to subscribed connections and includes the configured
command shortcut name, `trigger: "press"`, and a connection-specific
`focus_context` token. Built-in action shortcuts are not included.

The token expires five seconds after the key press and is bound to the
connection that received it. It can authorize one focus request or, with API
1.12, one interactive move or resize request.

```json
{ "op": "events", "api_version": { "major": 1, "minor": 10 }, "events": ["gnoblin.shortcut.activated"] }
```

Use the received token with `window.focus` when the user selects a listed
window:

```json
{
    "op": "api",
    "id": "focus",
    "api_version": { "major": 1, "minor": 10 },
    "method": "window.focus",
    "arguments": { "id": "42", "focus_context": "TOKEN_FROM_SHORTCUT_EVENT" }
}
```

`window.focus` accepts exactly the string fields `id` and `focus_context`.
Every attempt with a valid connection token consumes it, including requests
with an invalid window ID or unknown argument. API 1.12 also accepts this token
for one interactive move or resize operation. The request fails if the token
expired, was already used, belongs to another connection, or was revoked.

Config reloads, session locks, subscription changes, and disconnects revoke
socket tokens. Native Lua `Window:focus(context)` uses its protected Lua
context instead of a socket token.

Any compositor-socket client can focus a listed window with an XDG Activation
token. The client must create the token after user input on one of its Wayland
surfaces, and the socket connection must belong to that same process.

Mutter verifies the input serial and source surface; Gnoblin consumes the token
once. Requests fail while the session is locked. Lua callbacks do not receive
raw activation tokens or input serials.

### API version 1.11: dynamic shortcut bindings

#### Register a binding

Use `shortcut.bind` to register a connection-owned global accelerator. The
`arguments` object accepts only `id` and `accelerator`.

- IDs contain 1 to 64 letters, digits, underscores, or hyphens.
- Accelerators use Mutter's GTK accelerator syntax and are limited to 128 bytes.
- Mutter rejects invalid accelerators and bindings already claimed by another
  owner.
- Bare `Super` is the overlay key and is not accepted by this method.
- Bindings activate on key press and ignore autorepeat.
- Each connection can register up to 32 bindings. The compositor accepts 128
  dynamic bindings in total.

```json
{
    "op": "api",
    "id": "bind-search",
    "api_version": { "major": 1, "minor": 11 },
    "method": "shortcut.bind",
    "arguments": { "id": "search", "accelerator": "<Super>space" }
}
```

Use `shortcut.unbind` with the same ID to release a binding. A client can remove
only its own IDs. Disconnecting the client releases all its bindings.

To keep a binding registered after its held session ends, use
`shortcut.session.end` with the binding ID and active session ID. This works
only for a session owned by the current connection. The ended event reports
reason `cancelled`.

Send the `session_id` from `gnoblin.shortcut.session.activated`:

```json
{
    "op": "api",
    "id": "end-switcher-session",
    "api_version": { "major": 1, "minor": 64 },
    "method": "shortcut.session.end",
    "arguments": { "id": "switcher", "session_id": 42 }
}
```

#### Receive activations

Subscribe to `gnoblin.shortcut.binding-activated` with API 1.11 to receive
activations for bindings owned by that connection. Each event includes the ID,
accelerator, `trigger: "press"`, and Mutter's input timestamp. It also includes
a monotonic timestamp and a connection-specific focus token. The token can
authorize one `window.focus`, `window.begin_move`, or `window.begin_resize`
request and expires after five seconds.

```json
{ "op": "events", "api_version": { "major": 1, "minor": 11 }, "events": ["gnoblin.shortcut.binding-activated"] }
```

API 1.11 adds `gnoblin.operation.completed`, which carries an operation ID and
either a success value or a structured error. The legacy event remains for
older clients. Subscribe with `op: "events"`; clients tracking input-source or
shortcut-capture APIs also receive completion notifications. Match them by
operation ID.

Dynamic bindings are connection-owned and disappear when the client
disconnects. The API 1.11 binding activates on press. API 1.22 adds held and
modal shortcut sessions, described below. It does not buffer typing for later
delivery to a popup.

### API version 1.12: interactive window grabs

API 1.12 adds `window.begin_move` and `window.begin_resize`. Both require a
connection-bound `focus_context` token from a trusted shortcut activation and
consume it on every attempt, including invalid arguments. API 1.68 modal key
events also provide these tokens.

Use `window.begin_move` to start Mutter's keyboard move grab for a listed
window:

```json
{
    "op": "api",
    "id": "move",
    "api_version": { "major": 1, "minor": 12 },
    "method": "window.begin_move",
    "arguments": { "id": "42", "focus_context": "TOKEN_FROM_SHORTCUT_EVENT" }
}
```

Use `window.begin_resize` with one of `north`, `south`, `east`, `west`,
`north_east`, `north_west`, `south_east`, or `south_west`:

```json
{
    "op": "api",
    "id": "resize",
    "api_version": { "major": 1, "minor": 12 },
    "method": "window.begin_resize",
    "arguments": { "id": "42", "edge": "south_east", "focus_context": "TOKEN_FROM_SHORTCUT_EVENT" }
}
```

The greeting advertises `window-interactive-grabs` when these methods are
available. Mutter starts the keyboard grab with the trusted shortcut timestamp
and current pointer sprite. Calls fail if the token is expired, already used,
revoked, or belongs to another connection; if the session is locked; or if the
window cannot be moved or resized.

### API version 1.22: held and modal shortcut sessions

API 1.22 extends `shortcut.bind` with four optional fields:

- `trigger` is `press` (the default) or `release`.
- `hold` is `none` (the default), `super`, `control`, or `alt`. A held binding
  emits a shortcut-session activation and remains active until the held
  modifier is released.
- `mode` is `passive` (the default) or `modal`. Modal mode requires a non-`none`
  `hold` value and captures keyboard input while the held modifier remains
  down.
- `capture_input` is a boolean, defaulting to `false`. It is supported only
  for an explicit `accelerator: "Super"` binding. Bare Super must use
  `trigger: "release"` and `hold: "none"`.

Subscribe to these API 1.22 events to track a held binding:

- `gnoblin.shortcut.session.activated` starts or repeats the session.
- `gnoblin.shortcut.session.key` carries `keyval`, `keycode`, `modifiers`,
  `phase`, and `time`. The phase is `press` or `release`.
- `gnoblin.shortcut.session.ended` reports why the session stopped.

## Client-owned shortcuts and controls

Use `shortcut.bind` and `shortcut.unbind` to register a connection-owned
accelerator. The native bridge reports activation through
`gnoblin.shortcut.binding-activated`. It does not provide modal grabs, held
modifier sessions, a switcher fallback, or buffered typing. See
[API version 1.11](#api-version-111-dynamic-shortcuts) for the request shape.

Window, workspace, input, privacy, animation, and
launch methods are listed in the API-version tables above. They use
`op: "api"` with a version, request ID, method name, and arguments object.

### API version 1.71: layer lifecycle events

Request API 1.71 or newer to subscribe. The `hello` message advertises the
event names and the `layer-lifecycle-events` capability.

```json
{
    "op": "events",
    "api_version": { "major": 1, "minor": 71 },
    "events": ["gnoblin.layer.created", "gnoblin.layer.changed", "gnoblin.layer.removed"]
}
```

Every event includes state `revision`, `sequence`, and monotonic `time`. Layer
records use the fields from [`layer.list`](#api-version-12-layer-surfaces).

- `created` includes `layer`. Its stable ID is `layer.id`; it may be unmapped.
- `changed` includes `layer_id`, the updated `layer`, and a `changed` array of
  fields. Mapping and unmapping are changes; per-record `revision` is excluded.
- `removed` includes `layer_id` and the final record as `last`.

Lua receives the same events, with its layer snapshot refreshed before each
callback.

## Built-in and configured shortcuts

API 1.5 `shortcut.actions` lists built-in window-manager and Mutter actions.
Its optional `group` accepts `wm` for window-manager actions, `mutter` for
compositor actions, or `wayland` for Mutter’s Wayland actions. Omit `group` to
list all supported groups. Results include an ID, group, key, default bindings,
and a description when available. Defaults come from Gnoblin’s pinned Mutter
metadata and do not include user overrides.

API 1.9 `shortcut.list` returns native configured shortcuts. Each record
contains:

- `name`, `enabled`, `trigger`, and `revision`;
- `binding` as a string for one accelerator or an array for several;
- a command argument array or a built-in action ID.

Disabled declarations and external-client shortcuts are not included. Use
`shortcuts.list` for new clients.
