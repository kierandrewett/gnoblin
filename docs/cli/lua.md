# Lua Console

Run Lua against the session API or execute a Lua file from the command line.

Interactive and file modes require a running Gnoblin session. Lua runs locally through `gnoblinctl`.

## Lua console

Run `gnoblinctl lua` inside a Gnoblin session to inspect and control the live
compositor with the Lua API. Enter one Lua statement or expression per line.
Expressions print their result; tables print as JSON. Type `:help` for the
console commands and `:quit` to exit.

The console runs the same API as a Lua configuration. Each name, argument order
and object is the same, so a line that works here works in a configuration.
Calls are positional, and optional arguments come last. A name that the API
does not have fails with `gnoblin.NAME is not part of the Lua API`.

Three things differ, because the console is one process that ends with your
script and holds no runtime:

- A write waits for the compositor and returns the completed value, not an
  `Operation`.
- A record is a copy of one moment. It has no `on` or `watch`; follow a change
  with `gnoblin.on(name, fn)`.
- A name that needs a runtime raises `NAME is not available in gnoblinctl lua
because the console has no runtime`. These are `gnoblin.timers`,
  `gnoblin.process`, `gnoblin.tween`, `gnoblin.idle.watch`,
  `gnoblin.input.grab`, `gnoblin.input.on`, `gnoblin.settings.override`,
  `gnoblin.load`, `gnoblin.array`, `gnoblin.snapshot`, `gnoblin.configure`,
  `gnoblin.window_rule`, `gnoblin.animation`, `Window:set_effect`,
  `Window:present` and the `on` and `watch` methods of every record.

`gnoblin.ease` and `gnoblin.lerp` work here. `gnoblin.config` holds only
`path()`, `default()` and `restore_default()`, as in a configuration, and
cannot be assigned to. Read settings with `gnoblin.settings`.

Read build and source identity with `gnoblin.version()`:

```lua
local version = gnoblin.version()
print(version.gnoblin, version.gnome, version.git_sha)
```

```lua
=gnoblin.find("Window", {focused = true})
=gnoblin.workspaces.list()
:quit
```

## Windows and focus

Window reads return read-only records with the same properties and typed
methods as the supervised runtime:

```lua
local window = gnoblin.windows.focused()
if window then
    print(window.id .. ": " .. window.title)
    print(window:set_minimized(true).id)
end
```

Window actions wait for compositor completion and return the completed value.
They do not return an asynchronous `Operation` handle.

Select objects with `gnoblin.find(kind, filter?)`:

- The kinds are `Window`, `Workspace`, `LayerSurface`, `Output`, `Shortcut`,
  `Launch` and `InputSource`. `Process` raises the no-runtime error.
- A table filter names scalar fields of the kind and keeps the records that
  equal every value.
- A function filter keeps the records for which it returns true.
- `gnoblin.windows.list()` and `gnoblin.layers.list()` take no filter.

```lua
local notes = gnoblin.find("Window", {app_id = "notes"})[1]
local wide = gnoblin.find("Window", function(window)
    return window.frame.width > 1000
end)
```

`window:set_effect` raises an error here, because an
[override](/shell-api/overrides) needs a configuration to hold it.

The [draw methods](/shell-api/windows#change-how-a-window-is-drawn) work here
too: `set_opacity`, `set_scale`, `set_rotation`, `set_offset` and
`reset_visuals` return the window's visual values. The Lua configuration owns
the change, so a config reload resets it.

Records here are copies, not [live objects](/shell-api/windows#live-window-objects),
and the console has no `gnoblin.tween()`. A tween's callbacks run in the Lua
configuration, which outlives a console script, so build tweens in your config.

`focus`, `begin_move`, and `begin_resize` require a [`Grant`](/shell-api/grants) from
an event callback. The ordinary REPL cannot create one or call these methods.

The same grant authorizes keyboard snapping. Inside the event callback, call
`gnoblin.windows.snap_context(event.grant)`, then
`context:commit(monitor, x, y, width, height)` to apply its frame. `monitor` is an
`Output` or a monitor ID.

In `gnoblinctl lua`, both calls wait for their results on the event connection.

`gnoblin.focus.history(filter?)` returns read-only `Window` records in
order of last focus. The optional filter accepts string workspace and
monitor IDs plus an integer `limit` from 1 to 256 (default 50):

```lua
for _, window in ipairs(gnoblin.focus.history { limit = 10 }) do
    print(window.id, window.title)
end
```

Read the committed focus behavior with `gnoblin.focus.policy()`:

```lua
local policy = gnoblin.focus.policy()
print(policy.focus_mode, policy.focus_new_windows, policy.revision)
```

## Settings and reload

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

## Privacy and capabilities

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

## Launch feedback

Read launch feedback with `gnoblin.launches.list()`. It returns read-only
`Launch` records. Each record carries its own `revision`, and `alive` is true
while its `state` is `pending`:

```lua
for _, launch in ipairs(gnoblin.launches.list()) do
    print(launch.application, launch.state, launch.revision)
end
```

The console can also begin and finish launch feedback. `begin` returns the
`Launch`. Call `launch:finish()` when the launch is cancelled or the
application has started:

```lua
local launch = gnoblin.launches.begin("launcher-42", "org.example.Editor", {
    timeout_ms = 3000,
})
print(launch.state)
launch:finish()
```

A console `Launch` is a snapshot. It has no `on` signal; use
`gnoblin.on("gnoblin.launch.changed", fn)` to follow a launch.

## Session

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

`gnoblin.input.grab` needs a configuration to receive the keys, so
`gnoblinctl lua` cannot use it. See [Keyboard grabs](/shell-api/input-grab).

`gnoblin.idle.watch(seconds)` needs a configuration to hold the watch, so
`gnoblinctl lua` cannot use it. Read the idle-monitor sample from a terminal
with `gnoblinctl session activity`. See [Idle watches](/shell-api/idle).

`gnoblin.session.lock()` asks a subscribed shell client to show its lock UI.
Its read-only result reports delivery, not that the compositor locked the
session. Check `gnoblin.session.status()` for the confirmed lock state.
`gnoblin.session.logout()` ends the session and returns `{accepted = true}`
before the compositor shuts down:

```lua
local request = gnoblin.session.lock()
print(request.dispatched, request.subscribers)
```

## Workspaces

Workspace snapshots are available through `gnoblin.workspaces.list()`,
`gnoblin.workspaces.active()`, and `gnoblin.workspaces.by_id(id)`. They return
read-only `Workspace` records. The console also exposes the workspace
operations available to configuration scripts:

```lua
local workspace = gnoblin.workspaces.by_id("web")
if workspace then
    workspace:activate()
end
```

Workspace actions wait for compositor completion and return the completed
value. The console supports `gnoblin.workspaces.create(name, options?)`,
`next()` and `previous()`. Records add `workspace:activate()`,
`workspace:rename(name)` and `workspace:remove()`. Window records add
`window:move_to_workspace(workspace, options?)`.

Creation accepts a nonempty name up to 80 characters. Options may hold `id` and
`activate`. An optional ID must start with an ASCII letter or digit and contain
up to 64 letters, digits, periods, underscores, or hyphens.

A `workspace` argument to `move_to_workspace` is a string ID or a positive
integer number. The `follow` option must be a boolean when supplied. The
compositor rejects removal of configured, active, or nonempty workspaces.

## Input, monitors and layers

Input devices and sources are read-only `InputDevice` and `InputSource`
records. Each record includes the revision of its snapshot. The current source
is `nil` when Gnoblin cannot identify one. An `InputSource` has `select()`; a
console record is a snapshot with no `on` signal:

```lua
for _, source in ipairs(gnoblin.input.sources()) do
    if source.current then
        print(source.name)
    end
end

local selected = gnoblin.input.current_source():select()
print(selected.name)
```

Source selection waits for Mutter or IBus to confirm the change and returns the
completed `InputSource` record.

Read displays with `gnoblin.outputs.list()`, `gnoblin.outputs.primary()` and
`gnoblin.outputs.by_id(id)`. They return read-only `Output` records, or `nil`
when no output matches.

```lua
for _, output in ipairs(gnoblin.outputs.list()) do
    print(output.id, output.width, output.height, output.scale)
end
```

A console script cannot hold an [override](/shell-api/overrides), because the
script ends before anything could revoke it. `gnoblin.settings.override` and
`window:set_effect` raise an error here. Make an override in a configuration.

Use `gnoblin.layers.list()` to read layer surfaces. Each result is a
read-only `LayerSurface` record. Select some of them with `gnoblin.find`, which
matches any scalar field exactly, such as `monitor_id`, `namespace` or `layer`
(`background`, `bottom`, `top` or `overlay`):

```lua
for _, surface in ipairs(gnoblin.find("LayerSurface", {layer = "top"})) do
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

## Animations

A `Window` or `LayerSurface` previews and inspects its own animations:

- `target:preview_animation(name, event?, autoplay?)` returns a read-only
  `AnimationPreview` record.
- The `seek`, `step`, `play` and `pause` methods of a preview return the
  completed updated record. `stop` returns its completion result.
- `target:inspect_animation(name, event?)` returns a read-only inspection
  record.

```lua
local window = gnoblin.windows.focused()
local preview = window:preview_animation("gnoblin-window-open", "open", false)
preview = preview:seek(0.5)
print(preview.progress)
preview:stop()

print(window:inspect_animation("gnoblin-window-open", "open").duration)
```

`gnoblin.shortcuts.configured()` and `gnoblin.shortcuts.actions(group?)` return
read-only `ShortcutState` and `ShortcutAction` records. `actions` accepts an optional `wm`, `mutter`, or `wayland` group:

```lua
for _, shortcut in ipairs(gnoblin.shortcuts.configured()) do
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
`gnoblin.location.authorization-requested`. The event carries `event.request`,
a `Request` of kind `location`, as it does in a configuration.

Answer it with `request:answer(allow, accuracy?)`, or deny it with
`request:cancel()`. The CLI can answer only a request received by an active
subscription, and sends the reply over that same event connection. Requests
expire after 25 seconds. This example denies the request; replace the decision
with a consent prompt before approving access:

```lua
local subscription
subscription = gnoblin.on("gnoblin.location.authorization-requested", function(event)
    local result = event.request:cancel()
    print(result.submitted)
    subscription:unsubscribe()
end)
```

`answer` takes `allow`, and an `accuracy` level that is no greater than the one
the app requested (it defaults to that level):

- The call returns a read-only result. `submitted` confirms Gnoblin accepted
  and queued the answer.
- `request.status` becomes `"answered"`. A second answer raises an error.
- System settings can still limit approved accuracy.
- The console makes a `Request` for location requests only.
  `request:finished()` and `request:on` raise the no-runtime error.

`gnoblinctl lua` can register, remove, and manage shortcut sessions:

- `bind` returns a `Shortcut` with `unbind()` and `end_session(session_id)`.
- `gnoblin.shortcuts.list()` returns the shortcuts that the script still binds.
- The console `Shortcut` has no `on` signal. Follow it with
  `gnoblin.on("gnoblin.shortcut.binding-activated", fn)`.

Bindings created by the console belong to its compositor connection, so the
process stays open while one of its bindings remains registered. Unbind the
shortcut or exit the process to release it. See the
[shortcut API reference](/shell-api/keyboard-shortcuts#shortcut-state-and-capture)
for the accepted binding options.

For example, save this as `shortcut.lua` and run `gnoblinctl lua ./shortcut.lua`:

```lua
local binding = gnoblin.shortcuts.bind("temporary-cli-shortcut", "<Alt>F12", {
    hold = "alt",
    mode = "modal",
})

local subscription
subscription = gnoblin.on("gnoblin.shortcut.session.activated", function(event)
    if event.id ~= binding.id then
        return
    end

    binding:end_session(event.session_id)
    binding:unbind()
    subscription:unsubscribe()
end)
```

The example ends the active modal session, removes its temporary binding, and
lets the CLI exit. If you stop the process first, its connection closes and
Gnoblin removes the binding.

A Lua event listener can use the `Grant` delivered with a trusted event to
focus a window or start an interactive move or resize. The grant is an opaque
value that works once for each operation, and the console sends the request on the event's connection.
`grant:narrow(scope, window?)` runs in the `gnoblinctl` process: the socket
carries no narrowing, and a grant here has no `expires_at_us`.

For example, save this as `focus.lua`, replacing the ID with one from
`gnoblinctl window list`:

```lua
local window = gnoblin.windows.by_id("replace-with-window-id")
assert(window, "window no longer exists")

gnoblin.on("gnoblin.shortcut.activated", function(event)
    if event.grant then
        window:focus(event.grant)
    end
end)
```

Run it with `gnoblinctl lua ./focus.lua` while a configured shortcut emits
`gnoblin.shortcut.activated`. Press Ctrl+C to stop the listener.

WM window-menu requests carry a `Grant` in `event.grant`. It has the scopes
`begin_move` and `begin_resize` for the window that raised the menu, and works once.

Call `window:begin_move(event.grant)` or `window:begin_resize(edge, event.grant)` on that window.
The console keeps its token private and sends the operation over the event's
connection. Application-menu requests have no grant.
