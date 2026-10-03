# gnoblinctl

Run `gnoblinctl --version` to read the installed build identity without a
running session. It reports the Gnoblin and GNOME versions, Mutter and portal
backend versions, Lua and native API versions, build ID, Git remote, and commit.
`gnoblin --version` prints the same identity. Add `--json` to either command to
save the complete build identity as JSON.

In a Git checkout, the command reads the source manifests, current commit and
tracking remote (or `origin`). In a release tarball, it reads the embedded
source provenance. The installed command reads the identity recorded at build
time. A modified source tree is marked beside its commit; the commit alone does
not identify those local edits.

`gnoblinctl version` reads the build identity through the running compositor
API. It requires an active Gnoblin session.

[Configuration reference](/config/configure)

Control Gnoblin from a terminal or Lua configuration. Configure shell panels
with the external shell's own tools.

## Start here

Run commands from a terminal inside Gnoblin:

| Command                                        | Use it to                                                  |
| ---------------------------------------------- | ---------------------------------------------------------- |
| `gnoblinctl status`                            | Check the running session and lock-state availability      |
| `gnoblinctl logout`                            | End the session and return to the login manager            |
| `gnoblinctl session activity`                  | Read the latest idle-monitor sample                        |
| `gnoblinctl session lock`                      | Ask a subscribed external shell client to lock the session |
| `gnoblinctl window list`                       | Find open windows and their IDs                            |
| `gnoblinctl window match`                      | Show the values a window rule can match                    |
| `gnoblinctl window thumbnail ID --output PATH` | Save a window thumbnail as a PNG                           |
| `gnoblinctl layer list`                        | Find layer-surface namespaces                              |
| `gnoblinctl input devices`                     | List detected input devices and capabilities               |
| `gnoblinctl input orientation-lock [VALUE]`    | Read or set the orientation lock                           |
| `gnoblinctl workspace list`                    | Show workspace IDs, names, positions and windows           |
| `gnoblinctl config path`                       | Find the config file your session uses                     |
| `gnoblinctl config default`                    | Print the bundled default `init.lua`                       |
| `gnoblinctl config reload`                     | Apply supported edits and report restart-only changes      |
| `gnoblinctl shortcut list`                     | List shortcuts registered by the native compositor         |
| `gnoblinctl shortcut actions [GROUP]`          | List built-in shortcut actions, optionally by group        |
| `gnoblinctl shortcut capture`                  | Capture a key combination as a shortcut binding            |
| `gnoblinctl capabilities`                      | List compositor and protocol capabilities                  |
| `gnoblinctl focus history`                     | List recently focused windows                              |
| `gnoblinctl focus policy`                      | Show the committed focus policy                            |
| `gnoblinctl config show`                       | Show the committed settings snapshot                       |
| `gnoblinctl lua`                               | Open a local Lua console for the session API               |
| `gnoblinctl ping`                              | Check whether the compositor control socket responds       |

Run `gnoblinctl --help`, `gnoblinctl help window`, or a command's
`--help` for accepted arguments. A bare group lists its actions.

If a Lua configuration reload is already in progress, API commands retry that
temporary response until their `--timeout` expires. Other compositor errors are
reported immediately, and uncertain actions are not retried.

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

`gnoblin.focus.history(filter?)` returns the same read-only `Window` records in
most-recently-focused order. Its optional filter accepts `workspace_id`,
`monitor_id`, and `limit` (1–256):

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
unlocked:

```lua
local status = gnoblin.session.status()
print(status.state)
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
exit the process to release it. See the [shortcut API reference](./config/runtime-api.md#shortcut-state-and-capture)
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

## Windows

```sh
gnoblinctl window list
gnoblinctl window minimize 42
gnoblinctl window unminimize 42
gnoblinctl window toggle-minimize 42
gnoblinctl window restore 42
gnoblinctl window maximize 42
gnoblinctl window close 42
gnoblinctl window thumbnail 42 --output window.png
```

Each ID from `window list` lasts for that window's lifetime, not across
logins. Commands that accept `active` use the focused window's ID and fail
without sending a request when no window is focused. `toggle-minimize` requires
an explicit ID.
`unminimize` removes minimization while preserving maximized state; `restore`
returns the window to its normal size.

`gnoblinctl window focus` is rejected because the command cannot create the
one-use trusted context required to focus a window. A Lua script can act on a
trusted shortcut event by subscribing to `gnoblin.shortcut.activated` and
passing `event.focus_context` to the typed `Window` methods. The compositor
still validates and consumes the context. Interactive move or resize requires
pointer interaction or a trusted shortcut context.

To see the exact identity and title used by `gnoblin.window_rule`, run:

```sh
gnoblinctl window match
gnoblinctl window match 42 --json
```

The result shows the GTK application ID, WM class, and `rule_app_id` used by
window rules. Gnoblin uses the GTK ID when available and otherwise uses the WM
class.

The `match` object contains `type`, `app_id`, `title`, and `focused`. Use its
raw `app_id` and `title` values in a rule. The `APP ID` column in `window list`
shows a desktop-entry ID, which can differ.

`restore` removes minimisation and maximization. `restore-or-minimize`
unmaximizes a maximized window, restores its saved pre-snap frame, or minimizes
it when neither state applies.

`toggle-minimize` restores a minimized window or minimizes any other window.
Use `unmaximize` and `unfullscreen` to clear those states directly. `close`
requests a normal close, including unsaved-work prompts.

`window thumbnail` saves a PNG for a stable window ID. By default, it requests
an image up to 320 × 200 pixels. Set `--width` (1–480) and `--height` (1–320)
to change those bounds. The compositor may return a smaller image to preserve
the window's aspect ratio.

The command requires `--output PATH` and prints the saved path and dimensions.
Thumbnails are unavailable while the session is locked.

Filter by focused state, exact desktop app ID or a case-insensitive substring
of the title with `--focused`, `--app-id ID` or `--title TEXT`.

## Layer surfaces

List current layer-shell surfaces and their namespaces with:

```sh
gnoblinctl layer list
```

The command reads the API 1.37 `layers.list` snapshot and keeps the `layers`
object in JSON output. Records include a state revision.

Use a surface's `namespace` as the `layer` value in a window rule. The
`animation surfaces` command reports the same surfaces for animation previews.

## Input devices

List input devices detected by the compositor with:

```sh
gnoblinctl input devices
```

Each record reports the device name and type, seat, available capabilities,
and vendor or product IDs when the compositor provides them. Device IDs last
only for the current compositor session. The command returns a one-time
snapshot through the shared Lua read introduced in API 1.46. All supported
socket client versions use this read, which requires the Lua supervisor.

API 1.4 socket clients can also subscribe to device add and removal events.
`gnoblinctl input list` reads the API 1.6 `input.sources` snapshot through the
shared Lua read introduced in API 1.46. It prints the configured XKB sources.

`gnoblinctl input current` reads `input.current_source` through the same Lua
route. Its JSON record includes `available` and, when known, `source`.

`gnoblinctl input select TYPE ID` calls `input.select`. It waits for
Mutter to confirm the layout change.

`gnoblinctl input orientation-lock` reads the current orientation-lock
record through `input.orientation_lock`. Pass `on` or `off` to set the lock;
pass `inherit` to clear the runtime override and restore the configured value
or follow the system setting. This command does not write the config file.

```sh
gnoblinctl input orientation-lock
gnoblinctl input orientation-lock on
gnoblinctl input orientation-lock inherit
```

The Lua-only aliases `gnoblin.input.list()` and `gnoblin.input.current()` are
not socket methods. Socket clients use the names above.

## Animations

Use animation previews to inspect registered curves for compatible window and
layer-shell targets. A preview starts paused and changes only the target's
visual transform; it does not minimize or close the target.

```sh
gnoblinctl animation list
gnoblinctl animation get gnome-open
gnoblinctl animation surfaces
gnoblinctl animation inspect gnome-open --window active
session=$(gnoblinctl animation preview gnome-open --window active --format table | sed -n 's/^session: //p')
gnoblinctl animation seek "$session" 50
gnoblinctl animation step "$session" 16
gnoblinctl animation play "$session"
gnoblinctl animation pause "$session"
gnoblinctl animation stop "$session"
```

Omitting `--window` uses the active window. For layer-shell surfaces, choose
`--layer ID` or `--namespace NAME`; a namespace must resolve to exactly one
visible surface. `seek` accepts an integer percentage from 0 to 100; `step` advances
by milliseconds.

Seeking to 100% keeps the last frame visible until `stop`,
which restores the target's original visual state. `inspect` accepts `--event EVENT` to inspect a particular event variant. `preview --autoplay` starts playback immediately.

`animation surfaces` prints layer surface IDs, namespaces, and titles. Layer
surfaces are not included in `window list`.

The Lua console exposes the same snapshot through
`gnoblin.animations.surfaces()`. Its `surfaces` array and nested records are
read-only:

```lua
for _, surface in ipairs(gnoblin.animations.surfaces().surfaces) do
    print(surface.id, surface.namespace, surface.title)
end
```

Inspect the resolved animation for a visible target with
`gnoblin.animations.inspect {name, target, event?, target_type?}`. The result
and its nested tables are read-only:

```lua
local inspection = gnoblin.animations.inspect {
    name = "gnome-open",
    target = "active",
}
print(inspection.event, inspection.duration)
```

`animation list` marks entries that current preview targets can run with
`previewable`.

`animation get NAME` prints the configured record for an exact name. It returns
`null` when no animation matches. Use `--json` in scripts.

Workspace, shadow, and layer-companion animations run on internal compositor
actors or effects, so the current CLI cannot preview them against a window or
layer surface.

See the [animation guide](/guides/animations) for custom curves, events and
GNOME-style presets.

## Move and resize

```sh
gnoblinctl window move 42 100 80
gnoblinctl window resize 42 900 600
gnoblinctl window workspace 42 2
gnoblinctl window monitor 42 DP-1
```

The move example places the window at `(100, 80)` on the desktop. Resize sets
its outer size to 900 × 600 logical pixels, including the frame. Apps can
constrain the result, and geometry operations reject states such as fullscreen
or non-resizable windows.

For a stable window ID, `window monitor` takes a connector ID from
`gnoblinctl monitor list`. With the `active` window target, pass the current
numeric `index` from that list. The CLI resolves the focused window and maps
the index to a connector ID before sending the typed move request.

## Workspaces and monitors

```sh
gnoblinctl workspace list
gnoblinctl workspace create --name "Build"
gnoblinctl workspace create --id build --name "Build" --activate
gnoblinctl workspace rename --id build --name "Compile"
gnoblinctl workspace remove --id build
gnoblinctl workspace switch --number 2
gnoblinctl workspace switch --id code
gnoblinctl workspace next
gnoblinctl workspace previous
gnoblinctl workspace move-active --id code --follow
gnoblinctl workspace move-active --number 2
gnoblinctl monitor list
```

Workspace numbers are one-based positions and may change when dynamic
workspaces are removed. Configured IDs follow their `MetaWorkspace` when
reordered; generated IDs such as `@session-1` last for the session. Names are
display labels. Use `workspace list` to see each ID, number, name, active state,
and window count.

Monitor IDs are active connector names such as `DP-1` and `eDP-1`. Use the
exact ID from `gnoblinctl monitor list`. Each entry also has a current `index`
that can change when outputs are added or removed. Cloned outputs use the
lexicographically first active connector as their ID.

`gnoblinctl monitor list` reads the API 1.37 `monitors.list` snapshot. Its JSON
output keeps the `monitors` object, and each record includes its state revision.

The standalone compositor supports workspace list, create,
rename, remove, switch, next, previous, and window moves by ID or number.
Its list includes configured IDs and names, generated session IDs, active state,
and window counts. To remove a temporary workspace, first switch away from it
and move or close its windows. Edit the config to remove a configured workspace.

`workspace create` requires `--name`:

- `--id ID` assigns an ID for this session.
- `--activate` switches to the new workspace.
- Without `--id`, Gnoblin assigns a session-only ID such as `@session-1`.

Runtime-created workspaces are temporary. `workspace rename` and
`workspace remove` require exactly one of `--id` or `--number`. Gnoblin refuses
to remove a persistent, active, or occupied workspace.

Use `--number NUMBER` to select a workspace by its current one-based position.
The positional form `workspace switch NUMBER` remains a legacy shorthand;
prefer the explicit selector.
`workspace move-active` moves the focused window and switches workspaces only
when `--follow` is supplied. `window workspace` accepts an ID or a number:

```sh
gnoblinctl window workspace 42 --id code
gnoblinctl window workspace 42 --number 2
```

## Capture a shortcut

Run `gnoblinctl shortcut capture`, then press the key combination. The command
prints its GTK accelerator. Press Escape to cancel. Bare Super prints the
special `Super` binding.

The default timeout is 30 seconds. Set `--timeout` to an integer from 1 to 60
seconds to change it. If a session lock, input-capture session, or stage grab
starts during capture, the command reports cancellation and releases keyboard
input to that owner.

## List configured shortcuts

Run `gnoblinctl shortcut list` to list shortcuts registered by the native
compositor. External clients' shortcuts and disabled declarations are not
included. Each record includes its name, binding, enabled state, trigger,
revision, and either a command or built-in action. A binding with multiple
accelerators is shown as a JSON array in the table.
The command reads the shared Lua snapshot through API 1.40.

Use `--json` for structured output in a terminal or pipe:

```sh
gnoblinctl shortcut list --json
```

## List built-in shortcut actions

Run `gnoblinctl shortcut actions` to list actions that can be assigned in Lua
configuration. Pass `wm`, `mutter`, or `wayland` to show one group:

```sh
gnoblinctl shortcut actions wm
```

Use `--json` to preserve the complete action records in a pipe.
The command reads the shared Lua API through `shortcuts.actions` (API 1.41).

## Window actions

Most actions without extra arguments accept an optional window ID; they use
`active` if omitted. Typed actions resolve `active` to a stable ID before the
operation. `toggle-minimize` and geometry actions require the ID and numbers
shown.

| Action                                   | Arguments after action                           | Effect                                                |
| ---------------------------------------- | ------------------------------------------------ | ----------------------------------------------------- |
| `menu`                                   | `[ID]`                                           | Rejected; requires a trusted shell input context      |
| `interactive-move`, `interactive-resize` | `[ID]`                                           | Rejected; requires a trusted shell input context      |
| `above`, `unabove`                       | `[ID]`                                           | Set or clear always-on-top                            |
| `stick`, `unstick`                       | `[ID]`                                           | Show on all workspaces or only its own                |
| `focus`                                  | `[ID]`                                           | Rejected; focusing requires a one-use trusted context |
| `close`, `minimize`                      | `[ID]`                                           | Request close or minimize                             |
| `restore-or-minimize`                    | `[ID]`                                           | Unmaximize, restore a saved snap frame, or minimize   |
| `toggle-minimize`                        | `ID`                                             | Restore if minimized; otherwise minimize              |
| `restore`, `maximize`, `unmaximize`      | `[ID]`                                           | Change minimization or maximization                   |
| `fullscreen`, `unfullscreen`             | `[ID]`                                           | Enter or leave fullscreen                             |
| `move`                                   | `ID X Y`                                         | Set frame position; each coordinate: −100000–100000   |
| `resize`                                 | `ID WIDTH HEIGHT`                                | Set frame size; each dimension: 1–32768               |
| `workspace`                              | `ID [WORKSPACE]`, `--number NUMBER` or `--id ID` | Move to an existing workspace                         |
| `monitor`                                | `WINDOW MONITOR`                                 | Connector ID for a window ID; index for `active`      |

`gnoblinctl` cannot create the one-use input context required by `menu` and
interactive move or resize. These commands fail locally without sending a
request. Start them from a shell input binding that receives the trusted
context.

The CLI uses typed native methods for actions with a typed equivalent, including
when `active` is resolved to a stable ID. Menu and interactive actions remain
separate because they need compositor input context. Shell clients can focus a
clicked window with an XDG Activation token through the
[compositor bridge](/compositor-bridge#api-version-132-xdg-activation-focus-and-session-logout).

## Session and policy commands

| Command                                                           | Use                                                                                      |
| ----------------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| `status`                                                          | Read live session state and lock availability (API 1.29+)                                |
| `logout`                                                          | End the session and return to the login manager (API 1.32+)                              |
| `session activity`                                                | Read the latest idle-monitor sample (API 1.24+)                                          |
| `session lock`                                                    | Ask a subscribed shell client to lock the session (API 1.21+)                            |
| `ping`                                                            | Check whether the compositor control socket responds                                     |
| `version`                                                         | Read the running compositor build identity                                               |
| `config path`, `config default`, `config show`, `config reload`   | Find, print, inspect, or reload the active configuration                                 |
| `reload`                                                          | Alias for `config reload`                                                                |
| `capabilities`                                                    | List compositor and protocol capabilities                                                |
| `focus history [--workspace-id ID] [--monitor-id ID] [--limit N]` | List recent windows in focus order; limit is 1–256, default 50 (API 1.19+)               |
| `focus policy`                                                    | Read the committed focus behavior (API 1.19+)                                            |
| `input list`, `input current`                                     | Inspect configured and selected keyboard sources                                         |
| `input select TYPE ID`                                            | Select an exact source from `input list`                                                 |
| `privacy`                                                         | Read the privacy activity sources available in this session                              |
| `privacy stop-sharing`                                            | Ask Mutter to stop tracked screen-sharing sessions (API 1.31+)                           |
| `privacy stop-recording`                                          | Ask Mutter to stop tracked recording sessions (API 1.31+)                                |
| `permissions list`                                                | Read portal rules and capabilities through the Lua runtime (API 1.42+)                   |
| `permissions policy`                                              | Read the committed policy and its revision through Lua (API 1.44+)                       |
| `permissions check CAPABILITY IDENTITY`                           | Explain a decision for `app-id:…` or `host-exe:…` (API 1.43+)                            |
| `grant list`, `grant revoke KIND ID`                              | List or revoke persistent portal grants; kind is `screen-cast` or `remote-desktop`       |
| `launch status`                                                   | List pending launch feedback and its revision (API 1.39+)                                |
| `launch begin TOKEN APP [MILLISECONDS]`, `launch end TOKEN`       | Start or end busy-cursor feedback; duration defaults to 3000 ms, clamped to 100–10000 ms |
| `shortcut capture`                                                | Briefly grab the keyboard and print a GTK accelerator or `Super` binding                 |

`session activity` reports the latest native idle-monitor sample. If the
session has no idle monitor, the response reports that activity data is
unavailable. `session lock` requests a lock from a subscribed external shell
client. A successful request means the client received it; use the session
lock state to confirm that the screen is locked.

`permissions list` requires a connected Lua supervisor. Older socket clients
that request API 1.41 or earlier retain the native compatibility route.
`permissions check` also requires the supervisor; clients requesting API 1.42
or earlier retain its native compatibility route. `permissions policy`
requires the supervisor at API 1.44; older API clients retain the native route.

The privacy stop commands ask Mutter to stop matching tracked sessions. Their
`requested` count reports how many stop calls were issued; it does not confirm
that the sessions have closed. They do not revoke saved portal grants.

In a standalone native session, `gnoblinctl config reload` applies changes to:

- `animations`
- `input`
- `permissions`
- `touchpad-gestures`
- `window-rules`
- `workspaces`

Other setting changes are rejected without replacing the active runtime; start
a new session to apply them. The native open-animation matcher uses updated
rules for windows mapped after reload. Reload does not replay open animations
for windows already mapped.

For example, to inspect a portal decision and change keyboard source:

```sh
gnoblinctl permissions check screen-cast app-id:org.example.Recorder
gnoblinctl permissions policy --json
gnoblinctl input list --json
gnoblinctl input select xkb us
```

Use a capability from `permissions list` and a source from `input list`.
See [permission policy](/guides/permissions) and [launch feedback](launch-feedback.md).
Launch feedback does not start an application.

Run `gnoblinctl privacy` to see one status per source. Each status is active,
inactive or unavailable. Unavailable sources have no activity value. Camera
activity follows running PipeWire nodes marked with the `Camera` media role.

Location activity follows GeoClue's service state and `InUse` property. If it
is unavailable, check that GeoClue allows the `gnoblin` agent ID in its
`[agent]` whitelist; preserve the other IDs when editing the list.
Add `--json` to print the `PrivacyState` record. It contains a revision, an
availability flag for each source, and an activity value only when that source
is available.

The command uses the shared Lua read in API 1.47 and requires the Lua
supervisor.

| Source         | Availability field            | Optional activity field |
| -------------- | ----------------------------- | ----------------------- |
| Screen sharing | `available.screen_sharing`    | `screen_sharing`        |
| Microphone     | `available.microphone_in_use` | `microphone_in_use`     |
| Camera         | `available.camera_in_use`     | `camera_in_use`         |
| Location       | `available.location_in_use`   | `location_in_use`       |

`grant list` waits for the portal backend and prints its validated persistent
grants. Use a listed grant's exact `kind` and opaque `id` with `grant revoke`.
See the [runtime API reference](/config/runtime-api#privacy-and-permissions)
for the fields in each grant record.
Human-readable output prints each full grant ID on its own line, even when it
exceeds the terminal width. Use `--json` for machine-readable output.

See the [shortcuts guide](/guides/shortcuts#key-names) for configuration
examples.

## Output for scripts

```sh
gnoblinctl window list --json
gnoblinctl input list --format table
```

Structured results use tables in a terminal and JSON in a pipe.
`--json` forces JSON. Options work before or after the command.

For example, `gnoblinctl window list --focused --json` returns this shape.
The CLI keeps the `windows` object for compatibility. Each record comes from
the API 1.37 window snapshot and uses the Lua API's snake_case fields:

```json
{
    "windows": [
        {
            "id": "42",
            "revision": 123,
            "title": "Notes",
            "app_id": "org.example.Editor.desktop",
            "focused": true,
            "minimized": false,
            "workspace_id": "workspace-1",
            "workspace_number": 1,
            "monitor_id": "DP-1",
            "frame": { "x": 100, "y": 80, "width": 900, "height": 600 }
        }
    ]
}
```

Each record's `revision` identifies the compositor state represented by that
record. Optional fields are omitted when Mutter does not provide them. Use
`jq -r '.windows[].id'` to print the listed IDs.

The snapshot includes window identity, focus and state, workspace and monitor
location, geometry, and operation capabilities. See the [runtime API
reference](/config/runtime-api#immediate-window-snapshots) for the fields and
their availability.

In the standalone session, `app_id` comes from the GTK app ID or WM class.

With `jq` installed, print just the focused window ID:

```sh
gnoblinctl window list --focused --json | jq -r '.windows[].id'
```

A successful `gnoblinctl window minimize 42 --json` returns the affected
stable ID:

```json
{ "id": "42" }
```

`gnoblinctl workspace list` reads the API 1.37 `workspaces.list` snapshot.
Its JSON output keeps the `workspaces` object and the `windows` count field;
each record also includes the snapshot revision:

```json
{
    "workspaces": [
        {
            "id": "code",
            "number": 1,
            "name": "Code",
            "active": true,
            "windows": 2,
            "revision": 123
        },
        {
            "id": "web",
            "number": 2,
            "name": "Web",
            "active": false,
            "windows": 0,
            "revision": 123
        }
    ]
}
```

| Exit code | Meaning                          |
| --------- | -------------------------------- |
| 0         | Success                          |
| 1         | Runtime failure; error on stderr |
| 2         | Invalid arguments                |

`--timeout SECONDS` accepts 1–60; default is 5.
Uncertain actions are not retried automatically.

A reply with `pending: true` means accepted, not finished.
List state again to confirm the result. Window changes are rejected while locked.

## Command-line completion

```sh
# Bash: ~/.bashrc
eval "$(gnoblinctl completion bash)"

# Zsh: ~/.zshrc, after compinit
eval "$(gnoblinctl completion zsh)"

# Fish
gnoblinctl completion fish > ~/.config/fish/completions/gnoblinctl.fish
```

## Connection problems

The installed CLI uses GLib, GIO and JSON-GLib. It sends commands through the
[compositor bridge](compositor-bridge.md). Python is needed to build from
source, but not to run `gnoblinctl`.

The socket defaults to `$XDG_RUNTIME_DIR/gnoblin/compositor-v1.sock`.
Override it with `--socket PATH` or `GNOBLIN_COMPOSITOR_SOCKET`.

The bridge is built into current Gnoblin source builds. External shell
integrations run as separate processes and use the bridge directly. Check
`gnoblinctl ping`, the running Gnoblin version, and the session log. `ping`
checks only whether the compositor control socket responds; it does not check
external shell clients. See [CLI development](cli-development.md) for the
transport contract.
