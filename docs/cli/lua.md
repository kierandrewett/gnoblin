# Lua Console

Run Lua against the session API or execute a Lua file from the command line.

Interactive and file modes require a running Gnoblin session. Lua runs locally through `gnoblinctl`.

## Lua console

Run `gnoblinctl lua` inside a Gnoblin session to inspect and control the live
compositor with the Lua API. Enter one Lua statement or expression per line.
Expressions print their result; tables print as JSON. Type `:help` for the
console commands and `:quit` to exit.

Read build and source identity with `gnoblin.version()`:

```lua
local version = gnoblin.version()
print(version.gnoblin, version.gnome, version.git_sha)
```

```lua
=gnoblin.windows.list { focused = true }
=gnoblin.workspaces.list()
:quit
```

Window reads return read-only records with the same properties and typed
methods as the supervised runtime:

```lua
local window = gnoblin.windows.focused()
if window then
    print(window.id .. ": " .. window.title)
    print(window:minimize().id)
end
```

Window actions wait for compositor completion and return the completed value.
They do not return an asynchronous `Operation` handle.

`focus`, `begin_move`, and `begin_resize` require a trusted `FocusContext` from
an event callback. The ordinary REPL cannot create one or call these methods.

The same context authorizes keyboard snapping. Inside the event callback, call
`gnoblin.windows.snap_context(event.focus_context)`, then
`context:commit({monitor_id, frame})` to apply its frame.

In `gnoblinctl lua`, both calls wait for their results on the event connection.

`gnoblin.focus.history(filter?)` returns read-only `Window` records in
most-recently-focused order. The optional filter accepts string workspace and
monitor IDs plus an integer `limit` from 1 to 256 (default 50):

```lua
for _, window in ipairs(gnoblin.focus.history { limit = 10 }) do
    print(window.id, window.title)
end
```

Read the committed focus behavior through the read-only `FocusPolicy` property:

```lua
local policy = gnoblin.focus.policy
print(policy.focus_mode, policy.focus_new_windows, policy.revision)
```

`gnoblin.settings` returns the committed settings snapshot as a deeply
read-only value. Read nested options directly from the snapshot:

```lua
local settings = gnoblin.settings
print(settings.window_management.focus_mode, settings.revision)
```

Reload the Lua configuration from the console when it changes. The operation
returns a deeply read-only result; inspect it to see what was applied:

```lua
local result = gnoblin.runtime.reload_config()
print(result.action, result.runtime_generation)
```

`gnoblin.privacy.state()` returns a deeply read-only `PrivacyState` record.
Check `available` before reading an activity field; unavailable sources omit
their activity value:

```lua
local privacy = gnoblin.privacy.state()
if privacy.available.screen_sharing then
    print(privacy.screen_sharing and "Sharing" or "Not sharing")
end
```

The console can ask Mutter to stop tracked screen-sharing or recording
sessions. The result reports how many stop requests were issued; it does not
confirm that the sessions have closed:

```lua
local sharing = gnoblin.privacy.stop_sharing()
local recording = gnoblin.privacy.stop_recording()
print(sharing.requested, recording.requested)
```

`gnoblin.capabilities.list()` returns read-only `Capability` records with the
capability ID, description, availability, revision, and an optional reason:

```lua
for _, capability in ipairs(gnoblin.capabilities.list()) do
    print(capability.id, capability.available, capability.reason)
end
```

Read launch feedback with `gnoblin.launches.list()`. Use
`gnoblin.launches.snapshot()` when you also need the collection revision. Both
calls return deeply read-only launch records:

```lua
local snapshot = gnoblin.launches.snapshot()
for _, launch in ipairs(snapshot.launches) do
    print(launch.application, launch.state, snapshot.revision)
end
```

The console can also begin and finish launch feedback. Call `finish()` with the
same token when the launch is cancelled or the application has started:

```lua
local token = "launcher-42"
local launch = gnoblin.launches.begin {
    token = token,
    application = "org.example.Editor",
    timeout_ms = 3000,
}
print(launch.state)
gnoblin.launches.finish(token)
```

`gnoblin.permissions.policy()` returns the committed permission policy as a
deeply read-only `PermissionPolicy` record:

```lua
local policy = gnoblin.permissions.policy()
print(policy.default, policy.revision)
for _, rule in ipairs(policy.rules) do
    print(rule.name, rule.level)
end
```

Use `gnoblin.permissions.list()` to also read the supported capabilities,
permission levels, and configuration path. Its nested values are deeply
read-only.

Check how the committed policy evaluates a capability for an application
identity. The console accepts either two strings or a table:

```lua
local decision = gnoblin.permissions.check {
    capability = "remote-desktop",
    identity = "app-id:org.example.Remote",
}
print(decision.level, decision.rule, decision.revision)
```

`gnoblin.session.status()` returns a read-only `SessionStatus`. When lock state
is unavailable, `lock_state` is omitted; that does not mean the session is
unlocked. `revision` identifies the current lock-state snapshot and matches
the revision on its corresponding lock-state event:

```lua
local status = gnoblin.session.status()
print(status.state, status.revision)
if status.lock_available then
    print(status.lock_state)
end
```

`gnoblin.session.activity()` returns the latest idle-monitor sample as a
read-only `SessionActivity` record:

```lua
local activity = gnoblin.session.activity()
if activity.available then
    print(activity.idle, activity.idle_for_ms, activity.threshold_ms)
end
```

`gnoblin.session.lock()` asks a subscribed shell client to show its lock UI.
Its read-only result reports delivery, not that the compositor locked the
session. Check `gnoblin.session.status()` for the confirmed lock state.
`gnoblin.session.logout()` ends the session and returns `{accepted = true}`
before the compositor shuts down:

```lua
local request = gnoblin.session.lock()
print(request.dispatched, request.subscribers)
```

Workspace snapshots are available through `gnoblin.workspaces.list()`,
`gnoblin.workspaces.active()`, and `gnoblin.workspaces.by_id(id)`. They return
read-only `Workspace` records. The console also exposes the workspace
operations available to configuration scripts:

```lua
local workspace = gnoblin.workspaces.by_id("web")
if workspace then
    workspace:activate()
    workspace:move_here("42", { follow = true })
end
```

Workspace actions wait for compositor completion and return the completed
value. The console supports these namespace methods:

- `create {name, id?, activate?}`
- `rename {id|number, name}`, `activate {id|number}`, and `remove {id|number}`
- `next()` and `previous()`
- `move_active {workspace, follow?}` and `move_window {window, workspace, follow?}`

A workspace selector is a table with exactly one of `id` or positive `number`.
Creation accepts a nonempty name up to 80 characters. An optional ID must
start with an ASCII letter or digit and contain up to 64 letters, digits,
periods, underscores, or hyphens. `activate` and `follow` must be booleans
when supplied. A window is `"active"` or a stable window ID.

`move_active` and `move_window` return a read-only `WorkspaceMove` with the
resulting workspace, window ID, and `follow` value. The compositor rejects
removal of configured, active, or nonempty workspaces.

Input devices and sources are read-only `InputDevice` and `InputSource`
records. Each record includes the revision of its snapshot. The current source
is `nil` when Gnoblin cannot identify one. Select a listed source by passing
its `type` and `id`:

```lua
for _, source in ipairs(gnoblin.input.sources()) do
    if source.current then
        print(source.name)
    end
end

local selected = gnoblin.input.select_source { type = "xkb", id = "us" }
print(selected.name)
```

Source selection waits for Mutter or IBus to confirm the change and returns the
completed `InputSource` record.

Monitor snapshots are available through `gnoblin.monitors.list()` and
`gnoblin.monitors.primary()`. They return read-only `Monitor` records, or
`nil` when the session has no primary monitor:

```lua
for _, monitor in ipairs(gnoblin.monitors.list()) do
    print(monitor.id, monitor.width, monitor.height, monitor.scale)
end
```

Use `gnoblin.layers.list(filter?)` to read layer surfaces. Each result is a
read-only `LayerSurface` record. `gnoblin.layer.list()` is the compatibility
spelling for the same snapshot.

The optional filter matches these fields exactly:

- `monitor_id`: the monitor's stable connector ID.
- `namespace`: the layer surface namespace.
- `layer`: `background`, `bottom`, `top`, or `overlay`.

```lua
for _, surface in ipairs(gnoblin.layers.list { layer = "top" }) do
    print(surface.namespace, surface.geometry.width)
end
```

Read the effective animation and shadow policy for a layer namespace with
`gnoblin.layers.animation_policy(namespace)`. It returns a deeply read-only
`LayerAnimationPolicy` record:

```lua
local policy = gnoblin.layers.animation_policy("bingux-panel")
print(policy.enter.animation, policy.exit.animation, policy.window_shadow)
```

Animation previews return a read-only `AnimationPreview` record. Its
`seek`, `step`, `play`, and `pause` methods return the completed updated record;
`stop` returns its completion result:

```lua
local preview = gnoblin.animations.preview {
    name = "gnoblin-window-open",
    event = "open",
    target_type = "window",
    target = "active",
}
preview = preview:seek(0.5)
print(preview.progress)
preview:stop()
```

Portal grants are read-only `PortalGrant` records. Filter by kind and revoke a
grant from the snapshot:

```lua
for _, grant in ipairs(gnoblin.portals.grants { kind = "remote-desktop" }) do
    print(grant.requester, table.concat(grant.devices, ", "))
    grant:revoke()
end
```

Revocation uses the record's creation time, so a stale record cannot revoke a
new grant that reuses its ID. The console waits for each revoke to complete.
Both `PortalGrant:revoke()` and `gnoblin.grant.revoke(args)` return deeply
read-only results.
Scripts can also use the shared `gnoblin.grant.revoke` call directly; pass the
timestamp from the snapshot to keep the same stale-record check:

```lua
local grants = gnoblin.portals.grants { kind = "remote-desktop" }
local grant = grants[1]
if grant then
    gnoblin.grant.revoke {
        kind = grant.kind,
        id = grant.id,
        created_at = grant.created_at,
    }
end
```

Shortcut snapshots return read-only `ShortcutState` and `ShortcutAction`
records. `gnoblin.shortcuts.actions(group?)` accepts an optional `wm`,
`mutter`, or `wayland` group:

```lua
for _, shortcut in ipairs(gnoblin.shortcuts.list()) do
    print(shortcut.name, shortcut.binding, shortcut.enabled)
end

for _, action in ipairs(gnoblin.shortcuts.actions("wm")) do
    print(action.id, action.default_bindings[1])
end
```

Capture a shortcut from the terminal console. The call waits for the key press
and returns the normalized accelerator. The timeout defaults to 30 seconds and
can be set from 1 to 60 seconds; Escape cancels capture.

```lua
local captured = gnoblin.shortcuts.capture({timeout = 10})
print(captured.accelerator)
```

To run a Lua file with the same API, pass its path:

```sh
gnoblinctl lua ./inspect.lua
```

The Lua code runs in the local `gnoblinctl` process. Session calls go through
the compositor's typed API, which validates methods and arguments. `gnoblinctl`
never sends Lua source to the compositor.

The console can subscribe to runtime events. `gnoblinctl lua` stays open while
any subscription is active. A `once` subscription ends after its event; call
`subscription:unsubscribe()` to stop a repeating listener. Press Ctrl+C to
stop a script that continues listening.

For worker health, read `gnoblin.runtime.status()` after subscribing to
`gnoblin.runtime.status-changed`. The event reports recovery and accepted
configuration-generation changes while the compositor socket is connected; see
the [runtime API guide](/shell-api/) for its fields and reconnect
behavior.

To handle a location request, subscribe to
`gnoblin.location.authorization-requested`. The CLI can answer only a request
received by an active subscription, and sends the reply over that same event
connection. Requests expire after 25 seconds. This example denies the request;
replace the decision with a consent prompt before approving access:

```lua
local subscription
subscription = gnoblin.events.on("gnoblin.location.authorization-requested", function(event)
    local result = gnoblin.location.authorize_app {
        request_id = event.request_id,
        allow = false,
        accuracy = 0,
    }
    print(result.submitted)
    subscription:unsubscribe()
end)
```

Approval must choose an accuracy level no greater than the one requested by the
app. The call returns a read-only result; `submitted` confirms Gnoblin accepted
and queued the answer. System settings can still limit approved accuracy.

`gnoblinctl lua` can register, remove, and manage shortcut sessions. Bindings
created by the console belong to its compositor connection, so the process
stays open while one of its bindings remains registered. Unbind the shortcut or
exit the process to release it. See the [shortcut API reference](/shell-api/keyboard-shortcuts#shortcut-state-and-capture)
for the accepted binding options.

For example, save this as `shortcut.lua` and run `gnoblinctl lua ./shortcut.lua`:

```lua
local binding = gnoblin.shortcuts.bind {
    id = "temporary-cli-shortcut",
    accelerator = "<Alt>F12",
    hold = "alt",
    mode = "modal",
}

local subscription
subscription = gnoblin.events.on("gnoblin.shortcut.session.activated", function(event)
    if event.id ~= binding.id then
        return
    end

    gnoblin.shortcuts.end_session {
        id = event.id,
        session_id = event.session_id,
    }
    gnoblin.shortcuts.unbind { id = binding.id }
    subscription:unsubscribe()
end)
```

The example ends the active modal session, removes its temporary binding, and
lets the CLI exit. If you stop the process first, its connection closes and
Gnoblin removes the binding.

A Lua event listener can use a `FocusContext` delivered with a trusted event to
focus a window or start an interactive move or resize. The context is an opaque,
one-use value and the console sends the request on the event's connection.

For example, save this as `focus.lua`, replacing the ID with one from
`gnoblinctl window list`:

```lua
local window = gnoblin.windows.by_id("replace-with-window-id")
assert(window, "window no longer exists")

gnoblin.events.on("gnoblin.shortcut.activated", function(event)
    if event.focus_context then
        window:focus(event.focus_context)
    end
end)
```

Run it with `gnoblinctl lua ./focus.lua` while a configured shortcut emits
`gnoblin.shortcut.activated`. Press Ctrl+C to stop the listener.

WM window-menu requests carry an opaque `MenuContext` in
`event.menu_context`. Call `:begin_move()` or `:begin_resize(edge)` on that
value to use the one-time authority for the window that raised the menu. The
console keeps its token private and sends the operation over the event's
connection. Application-menu requests have no `MenuContext`.
