# Sessions and Windows

Read session activity and request supported window or session actions through the bridge.

Each operation section states its minimum API version. Session lock and trusted operations depend on live session state.

## Session locking

### API version 1.21

API 1.21 adds:

- `session.lock`
- `gnoblin.session.lock-requested`
- `gnoblin.session.lock-state-changed`

#### Request a lock

`session.lock` takes no arguments. It sends the lock request to clients
subscribed to `gnoblin.session.lock-requested`. It fails if compositor locking
is unavailable or no client is subscribed.

A successful response contains `dispatched: true` and `subscribers`. This is the
number of connected clients targeted for the request. Queue limits can close a
slow connection before it handles the event. These fields are not a delivery
acknowledgement or evidence that the session is locked.

#### Track lock state

Subscribe to `gnoblin.session.lock-state-changed` for Mutter's lock state. Each
event includes `state`, `revision`, `sequence`, and monotonic-clock `time`.
`revision` matches the lock-state snapshot revision. The possible states are:

- `unlocked`: no active session lock.
- `covering`: the compositor has started the lock transition.
- `locked`: Mutter confirmed the lock transition.
- `failsafe`: the lock client failed and Mutter retained its failsafe state.

Only `locked` confirms the lock transition. There is no socket unlock method.

## Window thumbnails

### API version 1.23

API 1.23 adds the asynchronous `window.thumbnail` method and the
`window-thumbnails` capability. Pass a stable window ID and positive integer
dimensions, capped at 480 by 320.

The result contains `window_id`, the actual dimensions, and `data`, a base64
PNG. Mutter preserves aspect ratio and limits the encoded PNG to 512 KiB.

- Requests fail while locked.
- Each client may have one request in progress, with four across the session.
- Gnoblin drops a result if the requester disconnects or the window closes.

The preview is rendered from the Mutter window actor. Mutter 51 provides no
protected-content metadata or capture-redaction guarantee; Gnoblin does not
promise to hide DRM or other protected content. The socket is restricted to
the current user, and same-user processes are inside this session trust
boundary. Keep previews within that boundary.

```json
{
    "op": "api",
    "id": "thumbnail-1",
    "api_version": { "major": 1, "minor": 23 },
    "method": "window.thumbnail",
    "arguments": { "id": "42", "width": 480, "height": 320 }
}
```

The server replies with a request ID and operation ID. Subscribe to
`gnoblin.operation.completed` at API 1.11 to receive the asynchronous result.

## Session activity

### API version 1.24

API 1.24 adds the `session-activity` capability, the `session.activity` read,
and `gnoblin.session.activity-changed`.

The read returns a snapshot with these fields:

- `available`: whether idle monitoring works in this session.
- `idle`: whether the fixed idle threshold has elapsed.
- `threshold_ms`: the fixed threshold, 120,000 milliseconds.
- `idle_for_ms`: elapsed idle time, or zero when not idle.
- `revision`: the snapshot revision.

Idle inhibitors and desktop timeout preferences do not change the threshold.
When monitoring is unavailable, `idle` is false and `idle_for_ms` is zero.
During idle, reads advance the duration from the last native sample using
monotonic time.

Subscribe to the event for state transitions. Its `idle_for_ms` is sampled
when availability or idle state changes and does not update continuously. The
event also includes `threshold_ms`, `revision`, `sequence`, and monotonic-clock
`time`.

```json
{
    "op": "events",
    "api_version": { "major": 1, "minor": 24 },
    "events": ["gnoblin.session.activity-changed"]
}
```

The shell lock client renders and authenticates its own UI. Mutter remains
authoritative for lock state.

```json
{"op":"events","api_version":{"major":1,"minor":21},"events":["gnoblin.session.lock-requested","gnoblin.session.lock-state-changed"]}
{"op":"api","api_version":{"major":1,"minor":21},"id":"lock-1","method":"session.lock","arguments":{}}
```

The first event lets a shell client handle the request. A successful method
response means at least one subscribed connection was targeted. Use the state
event to learn what Mutter actually did.

## Read snapshots

Send an empty `arguments` object for reads without filters. `focus.history`
uses the same filters as the Lua method: string `workspace_id` and
`monitor_id`, plus integer `limit` from 1 to 256 (default 50).

The response contains the snapshot directly in `result`. List reads return JSON
arrays, including an empty array when there are no records.

Snapshot reads are available after native control seeds their state. API 1.37
collection reads also require a connected Lua supervisor.

`windows.list` returns the same filtered, read-only window records as the Lua
API.

`launches.snapshot` keeps the collection revision available when the array is
empty.

For example, request the committed settings snapshot with:

```json
{
    "op": "api",
    "id": "settings",
    "api_version": { "major": 1, "minor": 19 },
    "method": "settings",
    "arguments": {}
}
```

The `reply` record returns the settings snapshot directly in its `result`
field. Use `capabilities.list`, `focus.history`, and `windows.list` for array
results.

## Window, workspace, and monitor snapshots

API 1.1 `windows` sends an initial window snapshot, then window and workspace
events. Each event includes `revision`, `sequence`, and monotonic `time` in
microseconds. Socket events use `event` where Lua records use `name`.

The initial snapshot establishes the window baseline. Existing windows do
not emit `created` when a client subscribes. Workspace state is not part of
that snapshot; request `workspace.list` for a workspace baseline. When focus
moves, `unfocused` arrives before `focused`. A focus-only change does not also
emit `changed`.

Workspace lifecycle events carry a `workspace` record, except removal, which
includes `workspace_id` and the final `last` record. `workspace.changed` also
includes a `changed` array containing `number`, `window_count`, or
`persistent`. A window move reports `window_id`, `from_id`, and `to_id`.

API 1.1 `monitors` sends the active logical monitor snapshot and lifecycle
events. Added and changed events include a `monitor` record. Changed events
also include a `changed` list; removed events include `monitor_id` and the
final `last` record.

The `changed` list can contain `id`, `index`, `x`, `y`, `width`, `height`,
`primary`, `scale`, `enabled`, `name`, `make`, `model`, `serial`,
`refresh_rate`, `transform`, or `work_area`.

## Layer surfaces

`layer.list` returns layer-shell surfaces with stable IDs, layer placement,
anchors, exclusive zone, keyboard-interactivity mode, geometry, and optional
title and namespace. Records include a revision and do not have application
IDs. The method accepts exact-string filters `monitor_id`, `namespace`, and
`layer`.

Request API 1.2 or newer. Every supported client version uses the shared Lua
snapshot for this read; the Lua supervisor must be connected.

```json
{ "op": "api", "id": "layers", "api_version": { "major": 1, "minor": 2 }, "method": "layer.list", "arguments": {} }
```

## Shell UI sessions

### API version 1.75

The `ui-sessions` capability lets separate shell processes share small state
snapshots and send commands. The shell decides how to present state and how
to respond to commands. The socket greeting advertises the capability when
this transport is available.

Send `watch` once per connection. A new watcher receives the current state for
each published name. Publish and command messages use names made of 1-128
ASCII letters, digits, periods, underscores, or hyphens, and object payloads:

```json
{"op":"ui-session","action":"watch"}
{"op":"ui-session","action":"state","name":"bingux.search","state":{"open":true,"query":"terminal"}}
{"op":"ui-session","action":"command","name":"bingux.search","command":{"action":"close"}}
```

Gnoblin sends `ui-state` events when state changes and `ui-command` events for
commands. Each published name has one owning connection. Other connections
cannot replace its state.

Commands are not stored and may target a name before it publishes state. When
the owner disconnects, Gnoblin releases the name and sends watchers a
`ui-state` event with `state: null`.

Gnoblin allows at most 64 active names. Each state or command payload is
limited to 8 KiB. Connections that publish state or commands must watch first.
Requests may include an optional string `id` to receive a reply or error.

A visible shell overlay can keep its companion layer surfaces above or below
it. Include `visible`, `surface`, `companions`, and `companionsAbove` in its
state, using exact layer-shell namespaces:

```json
{
    "op": "ui-session",
    "action": "state",
    "name": "desktop.editor",
    "state": {
        "visible": true,
        "surface": "desktop-editor",
        "companions": ["desktop-panel", "desktop-inspector"],
        "companionsAbove": true
    }
}
```

`visible` is a boolean. `surface` is one layer-shell namespace from 1 to 128
UTF-8 bytes. `companions` is an array of 1 to 32 namespace strings, each 1 to
128 UTF-8 bytes. Repeated names count once; the array cannot include `surface`.

`companionsAbove: true` places companion surfaces above the overlay. `false` or
omission places them below. Other state fields belong to the shell.

Ordering applies to mapped top or overlay layer surfaces on the same monitor
and layer level. It preserves their relative stacking order without changing
keyboard focus.

A top-layer surface stays on the top layer; ordering does not promote it to the
overlay layer. Application windows, transient windows, and background or
bottom layers cannot be companions.

Each visible declaration must use namespaces distinct from other visible
declarations. If declarations share any namespace, all declarations using that
namespace are ignored. Self references and invalid ordering fields are ignored;
state is still forwarded.

Set `visible: false` when the overlay closes. Gnoblin removes its ordering
constraints when it closes or its owner disconnects. Ordinary raising then
applies, and the constraints are suspended while the session is locked.

When showing a lazy panel and inspector together, publish the inspector first
to prevent the panel from covering it on its first map.
