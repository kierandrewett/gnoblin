# Lua runtime API

Use `gnoblin` runtime methods from an event callback to inspect or change the
running session. Lua and `gnoblinctl` use the same method names and argument
fields. Operation-based methods return an `Operation`; `gnoblinctl` waits for
the matching reply and prints its result.

`gnoblin.settings` returns an immutable snapshot of the active configuration
using public snake_case setting names. Read it as a property, for example
`gnoblin.settings.window_management.focus_mode`. It is available in native and
Shell-backed Lua runtimes after the initial configuration is committed.

The snapshot includes a `revision` that advances when committed configuration
values change. A no-op event or identical reload keeps the same revision. This
field belongs to the detached snapshot and is never written into the config.
`gnoblin.snapshot()` remains a compatibility function that returns a mutable
copy of the config view; prefer `gnoblin.settings` for reads.

`gnoblin.version()` returns an immutable `Version` record in native and
Shell-backed Lua runtimes. It reads the installed
`share/gnoblin/version.ini` metadata file. Set `GNOBLIN_VERSION_METADATA_FILE`
to use another file. If that path is unset or unreadable, Gnoblin checks the
executable's installation prefix and then the system XDG data directories. A
missing or invalid file does not raise an error; fields without a source value
are returned as `"unknown"`.

The `api` field comes from the native-control API constants when available at
build time. The `lua` field reports the linked Lua runtime version. Remote URLs
have userinfo, query, and fragment parts removed before returning.

The record contains string fields `gnoblin`, `gnome`, `mutter`, `lua`, `api`,
`git_remote`, `git_sha`, and `build_id`. The current installed identity file
does not include a separate `buildId`, so `build_id` is `"unknown"` unless an
identity file supplies it.

Native sessions provide immediate reads through `gnoblin.windows`,
`gnoblin.workspaces`, `gnoblin.monitors`, `gnoblin.layers`,
`gnoblin.input.devices()`, and `gnoblin.capabilities`. Callbacks run in
the compositor's main thread, so keep them short. See [Lua events](/config/lua-events)
for event names and callback behavior.

`gnoblin.focus.policy` returns a read-only snapshot of the supported focus
settings. Its `revision` identifies the committed settings snapshot.

The snapshot includes:

- `focus_mode` and `focus_new_windows`.
- `raise_on_click`, `auto_raise`, and `focus_change_on_pointer_rest`.
- `auto_raise_delay` and `revision`.

`gnoblin.focus.policy-changed` runs after a successful config commit only when
one of these effective focus settings changes. Its `policy` field contains the
committed snapshot, and the event's `revision` matches `policy.revision`.
Rejected or failed config changes do not emit the event. Native-control API
1.13 socket clients can subscribe to the same event.

Use `gnoblin.focus.history(filter?)` to read native window snapshots in recent
focus order. Filters accept `workspace_id`, `monitor_id`, and `limit`. Workspace
and monitor IDs match exactly. The limit is 1–256 and defaults to 50; filters
may return fewer records.

History includes minimized windows and removes windows after they close. At
startup, only the focused window seeds the recent order. Other open windows
stay in snapshot order until the runtime observes their focus. Shell-backed Lua
runtimes do not provide focus history.

The settings property becomes available after the initial config load commits.
Native compositor snapshots are seeded after the initial Lua config
evaluation. They are unavailable while Gnoblin first loads the config file, so
call these read methods from a runtime event callback after native startup.
Calling one before the snapshot is ready raises a Lua error.

```lua
gnoblin.events.on("gnoblin.window.focused", function(event)
    local window = gnoblin.windows.by_id(event.window_id)
    if window then
        print("Focused window: " .. window.title)
    end
end)
```

Operation-based Lua API calls queue one operation, including legacy list
queries. The handle exposes the completion state and result:

| Member                  | Type                                      | Meaning                                                                                                          |
| ----------------------- | ----------------------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| `id`                    | Positive integer                          | Request ID for this session.                                                                                     |
| `method`                | String                                    | Canonical API method name.                                                                                       |
| `status`                | `"pending"`, `"succeeded"`, or `"failed"` | Current operation state.                                                                                         |
| `value`                 | Result or `nil`                           | Completed value on success.                                                                                      |
| `error`                 | `Error`, string, or `nil`                 | Failure detail. Native API 1.11 uses the `Error` record; Shell-backed compatibility operations may use a string. |
| `on_complete(callback)` | Function to `Subscription`                | Call once with `(value, error)`.                                                                                 |

If the operation has already completed, Gnoblin schedules the callback for the
next main-loop turn. You can unsubscribe before it runs.

Callbacks registered while an operation is pending run when its completion is
dispatched. A callback can queue more operations; Gnoblin applies them after it
returns through the normal configuration and operation path. Gnoblin logs
callback errors and continues with other completion callbacks.

Native API 1.11 adds `gnoblin.operation.completed` for completed operations.
Its payload contains an operation ID, method, and success flag. Success has a
`value`; failure has an `Error` record with a stable code and human-readable
message.

During migration, Gnoblin also sends the legacy
`gnoblin.api.operation-completed` event with a request ID, result, and string
error. Shell-backed compatibility operations use the legacy shape.

Native `Error.code` values are:

| Code               | Meaning                                                          |
| ------------------ | ---------------------------------------------------------------- |
| `invalid_argument` | An operation argument or event-produced config value is invalid. |
| `not_found`        | The requested object or resource no longer exists.               |
| `unsupported`      | The current compositor or session cannot perform the operation.  |
| `denied`           | Session policy or security state rejects the operation.          |
| `busy`             | An exclusive input operation is already active.                  |
| `cancelled`        | The user cancelled the operation.                                |
| `timed_out`        | The operation reached its deadline.                              |
| `unavailable`      | The compositor or session service is not ready.                  |
| `internal`         | An unexpected implementation error occurred.                     |

Invalid arguments and calls made outside a runtime event callback fail before
an operation is queued. A queued request can still fail when Mutter or the
session handler applies it. If the config produced during that event fails
validation, Gnoblin rejects the event's queued operations and completes their
handles with the validation error.

## Workspaces

Use `{id = "code"}` to select a stable ID or `{number = 2}` to select the
current one-based position from 1 to 1024. A selector must contain exactly one
of these fields.

IDs declared in `gnoblin.configure.workspaces` persist across sessions. IDs
supplied to runtime `create` are temporary and last only for the session.
Gnoblin-generated `@session-N` IDs can be selected during that session but
should not be saved in configuration.
See the [workspace configuration reference](/config/configure/window_management)
and the [writing workspace recipe](/recipes/writing-workspace).

| Method                        | Arguments                                                          | Successful result                 |
| ----------------------------- | ------------------------------------------------------------------ | --------------------------------- |
| `workspace.list()`            | None                                                               | `{workspaces = {Workspace, ...}}` |
| `workspace.create(args)`      | `name` required; optional `id`, `activate`                         | New `Workspace` record            |
| `workspace.rename(args)`      | Exactly one of `id` or `number`, plus `name`                       | Updated `Workspace` record        |
| `workspace.remove(args)`      | Exactly one of `id` or `number`                                    | Removed workspace record          |
| `workspace.switch(args)`      | Exactly one of `id` or `number`                                    | Activated `Workspace` record      |
| `workspace.next()`            | None                                                               | Activated `Workspace` record      |
| `workspace.previous()`        | None                                                               | Activated `Workspace` record      |
| `workspace.move_active(args)` | `workspace` selector; optional `follow`                            | `{workspace, window, follow}`     |
| `workspace.move_window(args)` | `window` ID or `"active"`; `workspace` selector; optional `follow` | `{workspace, window, follow}`     |

Workspace mutations also have plural aliases. The plural collection reads
below return immediate native snapshots; `workspace.list()` remains an
operation-based compatibility method.

| Lua alias                      | Canonical method              |
| ------------------------------ | ----------------------------- |
| `workspaces.create(args)`      | `workspace.create(args)`      |
| `workspaces.rename(args)`      | `workspace.rename(args)`      |
| `workspaces.remove(args)`      | `workspace.remove(args)`      |
| `workspaces.activate(args)`    | `workspace.switch(args)`      |
| `workspaces.next()`            | `workspace.next()`            |
| `workspaces.previous()`        | `workspace.previous()`        |
| `workspaces.move_active(args)` | `workspace.move_active(args)` |
| `workspaces.move_window(args)` | `workspace.move_window(args)` |

`create` requires a nonempty name of up to 80 characters. Its optional ID must
match `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`; if omitted, Gnoblin generates a
session-only ID. `activate` defaults to `false`. Rename also requires a
nonempty name of up to 80 characters. `follow` defaults to `false`; when true,
moving a window also switches to the destination workspace.

A `Workspace` record has `id`, `number`, `name`, `active`, `windows`, and
`persistent` fields. Gnoblin does not remove a declared workspace, the active
workspace, or a workspace that still contains windows.

## Windows, layers, and monitors

### Immediate window snapshots

The native Mutter runtime exposes the latest window state directly:

| Lua call                        | Arguments                                                                   | Result                                |
| ------------------------------- | --------------------------------------------------------------------------- | ------------------------------------- |
| `gnoblin.windows.list(filter?)` | Optional `app_id`, `title`, `focused`, `workspace_id`, `monitor_id` filters | Read-only window snapshots            |
| `gnoblin.windows.focused()`     | None                                                                        | Read-only window snapshot or `nil`    |
| `gnoblin.windows.by_id(id)`     | Stable string window ID                                                     | Read-only window snapshot or `nil`    |
| `gnoblin.workspaces.list()`     | None                                                                        | Read-only workspace snapshots         |
| `gnoblin.workspaces.active()`   | None                                                                        | Read-only workspace snapshot or `nil` |
| `gnoblin.workspaces.by_id(id)`  | Stable string workspace ID                                                  | Read-only workspace snapshot or `nil` |

Window filter values:

- `title` takes a string and matches a case-insensitive substring.
- `focused` takes a boolean.
- `app_id`, `workspace_id`, and `monitor_id` take strings and match exactly.

Unknown filter names or values of the wrong type raise a Lua error. These reads
use a cached native snapshot and return immediately; they do not create an
`Operation`. Each record includes a state revision. Read the collection again
to get newer state.

Gnoblin refreshes both collections in one coalesced main-loop publication. A
read made before that publication completes returns the most recently published
state. Lifecycle callbacks run after the corresponding snapshots are refreshed,
so handlers can read the new state.

Subscribe to these window events to track changes:

- `gnoblin.window.created` and `gnoblin.window.closed`
- `gnoblin.window.changed` and `gnoblin.window.attention-changed`
- `gnoblin.window.focused` and `gnoblin.window.unfocused`

These reads are available only when Gnoblin runs with the native Mutter
runtime. They raise a Lua error when no native snapshot is available.
Shell-backed compatibility sessions do not populate these caches.

The native runtime exposes `gnoblin.monitors.list()` and
`gnoblin.monitors.primary()` as cached read-only snapshot methods. The primary
method returns a record or `nil`. Snapshots include active logical monitors;
cloned outputs use the first active connector alphabetically as the canonical
ID. Each record includes its state revision. Snapshots refresh before monitor
lifecycle callbacks run.

Subscribe to `gnoblin.monitor.added`, `gnoblin.monitor.changed`, and
`gnoblin.monitor.removed` to track output changes. The `changed` event's
`changed` array lists changed record properties: `id`, `index`, `x`, `y`,
`width`, `height`, `primary`, `scale`, `enabled`, `name`, `make`, `model`,
`serial`, `refresh_rate`, or `transform`. These immediate reads and events are
available only in the native Mutter runtime.

`gnoblin.layers.list(filter?)` returns read-only records from the latest native
layer-surface snapshot. Each record includes its state revision. Layer records
have no mutating methods; shell clients continue to own their surfaces.

The optional filter accepts exact string matches for `monitor_id`, `namespace`,
and `layer`. Unknown fields and non-string values raise a Lua error. The read
is available only in the native Mutter runtime and raises a Lua error while its
snapshot is unavailable.

Layer records expose these fields:

| Field                              | Type and values                                                                |
| ---------------------------------- | ------------------------------------------------------------------------------ |
| `id`                               | Stable string ID.                                                              |
| `title`, `namespace`, `monitor_id` | Optional strings.                                                              |
| `layer`                            | `"background"`, `"bottom"`, `"top"`, or `"overlay"`.                           |
| `keyboard_interactive`             | `"none"`, `"on_demand"`, or `"exclusive"`.                                     |
| `exclusive_zone`                   | Integer.                                                                       |
| `anchor`                           | Array containing zero or more of `"top"`, `"bottom"`, `"left"`, and `"right"`. |
| `geometry`                         | Rectangle.                                                                     |
| `mapped`                           | Boolean.                                                                       |
| `revision`                         | Integer state revision.                                                        |

Mutter currently omits `app_id` because layer-shell does not provide one.

`gnoblin.capabilities.list()` returns read-only native compositor and protocol
capability records. It takes no arguments and has no filters or mutators. The
read raises a Lua error if its snapshot is unavailable and is not populated in
Shell-backed compatibility sessions.

Each `Capability` has a string `id`, a string `description`, a boolean
`available`, an integer `revision`, and an optional string `reason`. The
current snapshot lists only capabilities advertised as available, so every
record has `available = true` and omits `reason`.

`gnoblin.input.devices()` returns read-only records from the latest native
input-device snapshot. It takes no arguments and is available in runtime event
callbacks after native startup. The read raises a Lua error if its snapshot is
unavailable and is not populated in Shell-backed compatibility sessions.

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

`gnoblin.input.sources()` returns the configured, available keyboard layouts
and variants. Native runtime records currently include XKB sources only.

The list comes from `input-sources.sources` when configured. Otherwise it uses
the desktop input-source setting. Mutter loads at most four layouts into one
keymap at a time. Gnoblin switches the active group of four when you select a
listed source outside that group.

`gnoblin.input.current_source()` returns the confirmed current source, or nil
when Mutter uses a keymap Gnoblin did not install or the active group is
unknown.

`gnoblin.input.select_source({type = "xkb", id = "us"})` requests a listed XKB
source and returns an operation handle. The operation completes only after
Mutter confirms the keymap change. Native IBus selection is not implemented;
selecting an IBus engine fails explicitly. Native selection changes the seat's
keyboard layout. Per-window source restoration remains in the Shell-backed
compatibility path, where XKB and IBus selection behavior is unchanged.

Subscribe to `gnoblin.input.source-changed` for confirmed current-source
changes and `gnoblin.input.sources-changed` when the configured source list
changes. The current-source event has `available = false` and no `source` when
Mutter's current keymap is external or unknown.

`device_type` is `"pointer"`, `"keyboard"`, `"extension"`, `"joystick"`,
`"tablet"`, `"touchpad"`, `"touchscreen"`, `"pen"`, `"eraser"`,
`"cursor"`, `"pad"`, or `"unknown"`. Capabilities are zero or more of
`"pointer"`, `"keyboard"`, `"touchpad"`, `"touch"`, `"tablet_tool"`,
`"tablet_pad"`, `"trackball"`, and `"trackpoint"`.

### Snapshot record methods

Window and workspace snapshots expose methods that queue the corresponding
typed operation. Call them with colon syntax from a runtime event callback.
Each method returns an `Operation` handle.

| Window method                                  | Arguments                                     | Effect                                                    |
| ---------------------------------------------- | --------------------------------------------- | --------------------------------------------------------- |
| `window:close()`                               | None                                          | Ask the application to close.                             |
| `window:minimize()`                            | None                                          | Minimize the window.                                      |
| `window:toggle_minimize()`                     | None                                          | Minimize or restore the window.                           |
| `window:restore()`                             | None                                          | Remove minimization and maximization.                     |
| `window:set_maximized(enabled)`                | Boolean                                       | Set maximization.                                         |
| `window:set_fullscreen(enabled)`               | Boolean                                       | Set fullscreen.                                           |
| `window:set_above(enabled)`                    | Boolean                                       | Set the above state.                                      |
| `window:set_sticky(enabled)`                   | Boolean                                       | Set visibility across workspaces.                         |
| `window:move(position)`                        | `{x, y}` integer coordinates                  | Move in logical desktop pixels.                           |
| `window:resize(size)`                          | `{width, height}` integer dimensions          | Resize in logical pixels.                                 |
| `window:move_to_workspace(selector, options?)` | Workspace selector; optional `follow` boolean | Move this window and optionally activate the destination. |
| `window:move_to_monitor(target)`               | Monitor connector ID or `{id = ID}`           | Move this window to an active monitor.                    |

| Workspace method                        | Arguments                                                                   | Effect                              |
| --------------------------------------- | --------------------------------------------------------------------------- | ----------------------------------- |
| `workspace:activate()`                  | None                                                                        | Activate this workspace.            |
| `workspace:rename(name)`                | Nonempty name up to 80 characters                                           | Rename this workspace.              |
| `workspace:remove()`                    | None                                                                        | Remove this workspace when allowed. |
| `workspace:move_here(window, options?)` | Window snapshot, stable window ID, or `"active"`; optional `follow` boolean | Move the window to this workspace.  |

Record properties remain read-only. In the native runtime,
`Window:focus(context)` requests focus using the context from
`gnoblin.shortcut.activated`. The callback provides that value only for a
trusted shortcut key press. It expires after five seconds.

The same context can authorize `begin_move` or `begin_resize`. It works once
across all three operations. A config reload invalidates contexts from the old
Lua runtime; a session lock also revokes them.

Mutter still applies its normal window activation policy.

Contexts are not provided for clicks, Shell-backed shortcuts, release bindings,
synthetic input, input-method events, or repeated key presses. Direct
`gnoblin.window.focus`, `gnoblin.window.begin_move`, and
`gnoblin.window.begin_resize` calls without a context remain denied.

A separate shell process can subscribe through native-control API 1.10 and use
its connection-bound `focus_context` token; see the
[compositor bridge](/compositor-bridge#api-version-110-shortcut-focus-grants).
Socket tokens expire after five seconds and work only on the connection that
received them. Disconnect, event-filter replacement, config reload, or session
lock revokes them. A failed focus request consumes its context.

Any same-user process can connect to the mode-0600 socket, so same-user clients
are inside the trust boundary.

The singular `gnoblin.window.list()` and `gnoblin.workspace.list()` methods
remain operation based and return their existing record fields. Native window
records use the snake_case fields described in [Lua events](/config/lua-events)
and add a per-record revision.

Native workspace records rename the `windows` count to `window_count` and add
a revision. Window, workspace, monitor, layer, input-device, and capability
snapshots share a revision that advances when any of those states changes.

| Method                | Arguments                                                     | Successful result                        |
| --------------------- | ------------------------------------------------------------- | ---------------------------------------- |
| `window.list(args)`   | Optional `app_id`, `title`, `focused` filters                 | `{windows = {Window, ...}}`              |
| `window.match(args)`  | Optional string `window` ID; defaults to `"active"`           | Window identity and a `match` rule table |
| `window.action(args)` | `action`; optional `window`, geometry, `monitor`, `workspace` | Action result                            |
| `layer.list()`        | None                                                          | `{surfaces = {Surface, ...}}`            |
| `monitor.list()`      | None                                                          | `{monitors = {Monitor, ...}}`            |

In the native preview, each monitor has these fields:

| Field                       | Meaning                                                                                                            |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| `id`                        | Canonical active connector name. Cloned outputs use the first connector alphabetically.                            |
| `index`                     | Current zero-based Mutter order. It can change when outputs change.                                                |
| `x`, `y`, `width`, `height` | Logical-pixel monitor geometry.                                                                                    |
| `primary`                   | Whether this is the primary logical monitor.                                                                       |
| `scale`                     | Current monitor scale factor.                                                                                      |
| `enabled`                   | Always `true`; this list contains active logical monitors only.                                                    |
| `transform`                 | Logical monitor transform: `normal`, `90`, `180`, `270`, `flipped`, `flipped-90`, `flipped-180`, or `flipped-270`. |
| `name`                      | Optional display name supplied by Mutter.                                                                          |
| `make`, `model`, `serial`   | Optional physical display details supplied by Mutter.                                                              |
| `refresh_rate`              | Optional current refresh rate in Hz for the connector used as `id`.                                                |

Use `id` for typed moves and `index` only with the legacy `window.action` call.
When outputs are cloned, the refresh rate comes from the lexicographically
first active connector used as the monitor ID. The native result does not
include inactive physical outputs.

| Filter or field | Accepted value                 | Meaning                                             |
| --------------- | ------------------------------ | --------------------------------------------------- |
| `app_id`        | Application ID string          | Exact match in `window.list`.                       |
| `title`         | Text string                    | Case-insensitive substring filter in `window.list`. |
| `focused`       | Boolean; set `true` to filter  | Lists only the focused window when true.            |
| `window`        | `"active"` or stable window ID | Selects the `window.match` or action target.        |

`window.match` returns the stable window ID, desktop and GTK application IDs,
WM class, and a rule `match` table containing type, title, focus state, and the
rule app ID when available.

`window.action` accepts this action set:

```lua
WindowAction = "menu" | "interactive-move" | "interactive-resize" | "above"
             | "unabove" | "stick" | "unstick" | "focus" | "close"
             | "minimize" | "restore-or-minimize" | "restore" | "maximize"
             | "unmaximize" | "fullscreen" | "unfullscreen" | "move"
             | "resize" | "workspace" | "monitor"
```

| Action fields                  | Accepted value                   | Meaning                                                  |
| ------------------------------ | -------------------------------- | -------------------------------------------------------- |
| `x`, `y` for `move`            | Integers from −100000 to 100000  | Window position.                                         |
| `width`, `height` for `resize` | Integers from 1 to 32768         | Window size.                                             |
| `monitor`                      | Current zero-based index         | Destination monitor for the legacy `window.action` call. |
| `workspace`                    | `{id = ...}` or `{number = ...}` | Destination workspace for the `workspace` action.        |

Mutter rejects actions that the target window cannot perform in its current
state.

### Typed window operations

Typed operations take the stable string `id` returned by `window.list`. They
reject unknown argument fields. Each successful operation returns `{id}` as
its completed value. Window operations fail while the session is locked.

| Method                           | Arguments                                                                                             | Effect                                                                                                              |
| -------------------------------- | ----------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| `window.close(args)`             | `id`                                                                                                  | Requests a normal close; the application can show an unsaved-work prompt.                                           |
| `window.minimize(args)`          | `id`                                                                                                  | Minimizes the window.                                                                                               |
| `window.toggle_minimize(args)`   | `id`                                                                                                  | Restores a minimized window; otherwise minimizes it.                                                                |
| `window.restore(args)`           | `id`                                                                                                  | Removes minimization and maximization. It does not leave fullscreen.                                                |
| `window.set_maximized(args)`     | `id`, `enabled` boolean                                                                               | Sets maximization on or off.                                                                                        |
| `window.set_fullscreen(args)`    | `id`, `enabled` boolean                                                                               | Sets fullscreen on or off.                                                                                          |
| `window.set_above(args)`         | `id`, `enabled` boolean                                                                               | Sets always-on-top on or off.                                                                                       |
| `window.set_sticky(args)`        | `id`, `enabled` boolean                                                                               | Shows the window on all workspaces or only its own.                                                                 |
| `window.move(args)`              | `id`, integer `x`, integer `y` from −100000 to 100000                                                 | Sets the frame position in logical desktop pixels.                                                                  |
| `window.resize(args)`            | `id`, integer `width`, integer `height` from 1 to 32768                                               | Sets the outer frame size in logical pixels.                                                                        |
| `window.move_to_workspace(args)` | `id`, `workspace` selector; optional `follow` boolean                                                 | Moves the window; `follow = true` also activates that workspace.                                                    |
| `window.move_to_monitor(args)`   | `id`, `monitor` connector ID or `{id = connector ID}`                                                 | Moves the window to that active monitor.                                                                            |
| `window.focus(args)`             | Native Lua: `Window:focus(FocusContext)`; socket API 1.10: `id`, `focus_context`                      | Focuses a listed window using one trusted, single-use shortcut context. Direct calls without a context fail closed. |
| `window.begin_move(args)`        | Native Lua: `Window:begin_move(FocusContext)`; socket API 1.12: `id`, `focus_context`                 | Starts Mutter's keyboard move grab using the trusted shortcut timestamp.                                            |
| `window.begin_resize(args)`      | Native Lua: `Window:begin_resize(edge, FocusContext)`; socket API 1.12: `id`, `edge`, `focus_context` | Starts Mutter's keyboard resize grab at the selected edge.                                                          |

Resize edges are:

- `north`, `south`, `east`, or `west` to resize from a side.
- `north_east`, `north_west`, `south_east`, or `south_west` to resize from a corner.

Each interactive method needs the same one-use trusted context used by the
focus method. The context authorizes one operation. Mutter starts its keyboard
grab with the shortcut timestamp and current pointer sprite. Calls without a
trusted context fail closed.

The native `monitor.list()` result uses active connector names such as `DP-1`
or `eDP-1` as string IDs. For cloned outputs, it selects the first active
connector alphabetically.

A typed move fails with `not_found` if its connector ID is no longer listed.
The compatibility `window.action` monitor action still takes the current
numeric index.

Shell-backed sessions retain `window.action` for compatibility actions such as
`focus`, `menu`, `interactive-move`, and `interactive-resize`. The standalone
native compositor denies `window.action` with `action: "focus"`; use
`window.focus` with a verified context instead.

## Animations

| Method                    | Arguments                                        | Successful result                            |
| ------------------------- | ------------------------------------------------ | -------------------------------------------- |
| `animation.list()`        | None                                             | `{animations = {Animation, ...}}`            |
| `animation.surfaces()`    | None                                             | `{surfaces = {Surface, ...}}`                |
| `animation.inspect(args)` | `name`, `target`; optional `event`, `targetType` | Animation details and resolved specification |
| `animation.preview(args)` | Same as inspect; optional `autoplay`             | Preview session ID and specification         |
| `animation.seek(args)`    | `session`; `progress` from 0 to 1                | Session action result                        |
| `animation.step(args)`    | `session`; `milliseconds` from 1 to 60000        | Session action result                        |
| `animation.play(args)`    | `session`                                        | Session action result                        |
| `animation.pause(args)`   | `session`                                        | Session action result                        |
| `animation.stop(args)`    | `session`                                        | Session action result                        |

| Field        | Accepted value                                  | Meaning                                       |
| ------------ | ----------------------------------------------- | --------------------------------------------- |
| `targetType` | `window` (default), `layer`, or `namespace`     | Selects how to resolve `target`.              |
| `target`     | `"active"` or window ID; layer ID; or namespace | Identifies the preview target.                |
| `name`       | 1–80 ASCII letters, digits, `_` or `-`          | Selects an animation.                         |
| `event`      | Optional lowercase kebab-case event name        | Selects an event supported by that animation. |
| `autoplay`   | Boolean; default `false`                        | Starts a preview immediately when `true`.     |

See the [animation guide](/guides/animations) for supported animation events.

## Features, scripts, and input

| Method                  | Arguments    | Successful result                                               |
| ----------------------- | ------------ | --------------------------------------------------------------- |
| `feature.list()`        | None         | Feature records with `id`, `description`, and `enabled`         |
| `feature.show(args)`    | `id`         | `{id, enabled}`                                                 |
| `feature.enable(args)`  | `id`         | `{ok, id, enabled}`                                             |
| `feature.disable(args)` | `id`         | `{ok, id, enabled}`                                             |
| `script.list()`         | None         | `{scripts = {...}}`                                             |
| `input.list()`          | None         | Input-source records with `type`, `id`, `shortName`, and `name` |
| `input.current()`       | None         | Current input source record                                     |
| `input.select(args)`    | `type`, `id` | `{ok, type, id}`                                                |

Use IDs returned by the matching `list` method for feature and input
operations. Input `type` and `id` values depend on the sources available in
your session.

## Privacy and permissions

| Method                    | Arguments                | Successful result                                        |
| ------------------------- | ------------------------ | -------------------------------------------------------- |
| `privacy.get()`           | None                     | `{screenSharing, microphoneInUse, locationInUse}`        |
| `permissions.list()`      | None                     | Current permission policy                                |
| `permissions.policy()`    | None                     | Immutable policy with `default`, `rules`, and `revision` |
| `permissions.check(args)` | `capability`, `identity` | Permission decision with scope details                   |
| `grant.list()`            | None                     | `{grants = {Grant, ...}}`                                |
| `grant.revoke(args)`      | `kind`, `id`             | `{ok, id}`                                               |

Permission capabilities, identities, grant kinds, and scope fields use the
same values as [session permissions](/config/configure/permissions).

`permissions.policy()` returns the committed Gnoblin policy directly.
Native-control API 1.16 adds the matching socket method and
`gnoblin.permission.changed` event. The compatibility `permissions.list()`
response keeps its current wrapper and metadata.

Use `grant.revoke` with the `kind` and `id` from a listed portal grant.

Each grant record contains:

| Field           | Type and values                                                  |
| --------------- | ---------------------------------------------------------------- |
| `id`            | Opaque string returned by `grant.list()`.                        |
| `kind`          | `"screen-cast"` or `"remote-desktop"`.                           |
| `requester`     | Verified portal identity, such as `app-id:org.example.Recorder`. |
| `devices`       | Integer bitmask: keyboard `1`, pointer `2`, touchscreen `4`.     |
| `clipboard`     | Boolean.                                                         |
| `screenStreams` | Boolean indicating whether a screen-stream selection is stored.  |

Both methods return `Operation` handles. Grant listing and revocation run in
the portal backend, which validates the stored record before returning or
removing it. A missing grant, invalid record, or unavailable backend fails the
operation.

The native Lua runtime also provides `gnoblin.portals.grants(filter?)` for an
immediate snapshot. Pass `{kind = "screen-cast"}` or
`{kind = "remote-desktop"}` to filter the records. Each immutable
`PortalGrant` record contains:

| Field                | Type and values                                                 |
| -------------------- | --------------------------------------------------------------- |
| `id`                 | Opaque portal grant ID.                                         |
| `kind`               | `"screen-cast"` or `"remote-desktop"`.                          |
| `requester`          | Verified portal identity.                                       |
| `devices`            | Array of `"keyboard"`, `"pointer"`, or `"touchscreen"`.         |
| `clipboard`          | Boolean.                                                        |
| `has_screen_streams` | Boolean indicating whether a screen-stream selection is stored. |
| `created_at`         | Unix timestamp in milliseconds.                                 |
| `revision`           | Revision of the current native grant snapshot.                  |

Call `grant:revoke()` on a record from this snapshot. It returns an
`Operation` whose successful value contains `ok` and `id`. The portal backend
checks that the record still has the same creation time before revoking it.
Stale records fail.

New grants store their creation time. Older grants use file modification time
at whole-second precision; this is inferred metadata, not a verified consent
time. The collection is available after the native portal grant snapshot has
loaded.

Native-control API 1.15 adds the socket method `portals.grants`, which returns
the same records as a JSON array and accepts the same optional `kind` filter.
API 1.15 clients can subscribe to `gnoblin.portal.grant-added` and
`gnoblin.portal.grant-removed`; see the [compositor bridge](/compositor-bridge#api-version-115-portal-grant-snapshots-and-events).

## Launch feedback, shell, and configuration

Use `gnoblin.launches` to read launch feedback and track a launch request.
`list()` returns the latest cached native snapshot as immutable `Launch`
records. The native host seeds an empty snapshot during startup and refreshes
it before dispatching launch-change events. The method is unavailable before
native startup completes.

| Method                            | Arguments                                     | Result                                          |
| --------------------------------- | --------------------------------------------- | ----------------------------------------------- |
| `gnoblin.launches.list()`         | None                                          | `Launch[]` snapshot                             |
| `gnoblin.launches.begin(options)` | `token`, `application`; optional `timeout_ms` | An `Operation` whose value is a `Launch` record |
| `gnoblin.launches.end(token)`     | Launch token string                           | An `Operation` whose value is `{ok, token}`     |

`begin` uses the supplied token to identify this launch attempt. `timeout_ms`
defaults to 3000 and is an integer from 100 through 10000. It maps to the
native launch operation's `milliseconds` field. End a request with the same
token when it is cancelled or the application has started. The returned
`Operation` handle completes asynchronously, as described near the start of
this page.

The older `gnoblin.launch.status()`, `begin(args)`, and `end(args)` methods
remain available for compatibility. They keep the native field names and
argument-table shape described below.

| Compatibility method      | Arguments                                       | Successful result                                                |
| ------------------------- | ----------------------------------------------- | ---------------------------------------------------------------- |
| `launch.status()`         | None                                            | `{launches, revision}`                                           |
| `launch.begin(args)`      | `token`, `application`; optional `milliseconds` | `Launch` record                                                  |
| `launch.end(args)`        | `token`                                         | `{ok, token}`                                                    |
| `shell.ping()`            | None                                            | `{pong}`                                                         |
| `shell.version()`         | None                                            | `{version}`                                                      |
| `shell.status()`          | None                                            | Shell version, connection state, window count, focused window ID |
| `shell.reload()`          | None                                            | Reload acknowledgement                                           |
| `runtime.reload_config()` | None                                            | Configuration reload acknowledgement                             |

On the native compositor API, `launch.status()` returns current launch records
and their separate revision.

`launch.begin` accepts a token up to 128 characters and an application hint up
to 512 characters. `milliseconds` defaults to 3000 and is clamped to 100–10000
ms.

The native runtime retains at most 64 records. It evicts the oldest completed
record when needed and rejects a new launch if all retained records are pending.

Each immutable `Launch` record contains:

- `token` and `application`.
- `started_at`, as Unix time in milliseconds.
- `timeout_ms`, `state`, and `revision`.

The state begins as `pending`. Mutter reports `started` when a matching mapped
window appears or becomes focused. `launch.end` changes a pending record to
`ended`; the deadline changes it to `timed_out`.

Matching uses GTK application ID, WM class, or WM class instance. Values are
lowercased and a `.desktop` suffix is removed. The native API does not use
GNOME Shell application tracker names and cannot report failures from an
external process launcher. Subscribe to `gnoblin.launch.changed` for updates.
End a request with the same token when it is cancelled or the application has
started.

`shell.reload` reloads the GNOME Shell integration;
`runtime.reload_config` reloads Gnoblin configuration.

## Shortcut state and capture

`gnoblin.shortcuts.list()` returns named shortcuts configured for the native
compositor. Records do not include shortcuts owned by a shell integration.
Each read-only record contains:

- `name`, `binding`, `enabled`, `trigger`, and `revision`.
- Either `command` (an argument array) or `action` (a `group.key` identifier).
- `binding` as a string for one accelerator or an array for multiple bindings.

`enabled` is false for a configured built-in action with no bindings. Disabled
shortcut declarations and shortcuts that need shell input capture are not
registered by the native compositor and are not listed.

```lua
for _, shortcut in ipairs(gnoblin.shortcuts.list()) do
    print(shortcut.name, shortcut.binding)
end
```

Native-control API version 1.9 adds the read-only `shortcut.list` method. It
accepts no arguments and returns the same records as a JSON array. API version
1.10 adds the socket event `gnoblin.shortcut.activated` and the `window.focus`
method, which requires a one-use context from that event.

API 1.12 adds `window.begin_move` and `window.begin_resize`; both consume the
same connection-bound token. See the
[compositor bridge](/compositor-bridge#api-version-110-shortcut-focus-grants)
for the socket request format.

`gnoblin.shortcuts.actions(group?)` reads built-in actions from installed
GSettings schemas. Omit `group` to list all installed groups.

| Group     | Schema                                 |
| --------- | -------------------------------------- |
| `wm`      | `org.gnome.desktop.wm.keybindings`     |
| `mutter`  | `org.gnome.mutter.keybindings`         |
| `wayland` | `org.gnome.mutter.wayland.keybindings` |

An unknown group raises an error. Requesting a supported group whose schema is
not installed also raises an error. The `shell` group is not included. This
read API is separate from `gnoblin.configure.shortcuts`, which declares
shortcut configuration.

Each action record contains:

- `id`, `group`, and `key`.
- Optional `description`, when the schema provides one.
- `default_bindings`, the schema's exact accelerator string array.

These are schema defaults, not the current user override. An empty array means
that the action has no default accelerator. Actions are returned in `wm`,
`mutter`, `wayland` order, with keys sorted alphabetically within each group.

```lua
for _, action in ipairs(gnoblin.shortcuts.actions("wm")) do
    if action.id == "wm.close" then
        for _, binding in ipairs(action.default_bindings) do
            print(binding)
        end
    end
end
```

### Capture a shortcut

| Method                   | Arguments                                               | Successful result                                     |
| ------------------------ | ------------------------------------------------------- | ----------------------------------------------------- |
| `shortcut.capture(args)` | Optional `timeout` in seconds, from 1 to 60; default 30 | Pending operation; completion returns `{accelerator}` |

Call `gnoblin.shortcuts.capture()` from a runtime event callback. On success,
`value.accelerator` is the normalized accelerator string.

Press Escape to cancel. Capture fails if another capture is active, the session
is locked, Mutter has an active input-capture session, or a compositor stage
grab is active. If an input-capture session or stage grab starts during
capture, Gnoblin cancels the operation before forwarding keys to that owner.
The native runtime does not expose key events to Lua.

This method requires native-control API version 1.8 and advertises the
`shortcut-capture` capability. Check the socket handshake before calling it.

```lua
gnoblin.events.once("gnoblin.window.focused", function()
    local operation = gnoblin.shortcuts.capture({timeout = 10})
    operation:on_complete(function(value, err)
        if err then
            print("Shortcut capture failed: " .. err)
        else
            print(value.accelerator)
        end
    end)
end)
```

See [`gnoblinctl`](/gnoblinctl) for command-line forms of these operations.
