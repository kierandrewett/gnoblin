# Lua runtime API

Use `gnoblin` runtime methods from an event callback to inspect or change the
running session. `gnoblinctl` calls the same registered Lua API methods, so Lua
automation and the command-line client use the same public API.

Runtime calls enqueue an operation and return a positive integer request ID.
Gnoblin later dispatches `gnoblin.api.operation-completed` with the same ID.
Callbacks run in the compositor's main thread; keep them short. See [Lua
events](/config/lua-events) for event names and callback behavior.

```lua
local pending = {}

gnoblin.on("gnoblin.workspace.activated", function(event)
    if event.id == "review" then
        pending[gnoblin.workspace.list()] = "list workspaces"
    end
end)

gnoblin.on("gnoblin.api.operation-completed", function(event)
    local purpose = pending[event.request_id]
    if not purpose then return end
    pending[event.request_id] = nil

    if event.ok then
        print(purpose .. ": completed")
    else
        print(purpose .. ": " .. event.error)
    end
end)
```

The completion event contains `request_id`, `method`, and `ok`. Successful
operations include `result`; failed operations include an `error` message.
Store the returned request ID when an action's result matters, then handle only
the completion with that ID. A method can also fail immediately if its Lua
arguments are invalid or if it is called outside a runtime event callback.

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

`create` requires a nonempty name of up to 80 characters. Its optional ID must
match `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`; if omitted, Gnoblin generates a
session-only ID. `activate` defaults to `false`. Rename also requires a
nonempty name of up to 80 characters. `follow` defaults to `false`; when true,
moving a window also switches to the destination workspace.

A `Workspace` record has `id`, `number`, `name`, `active`, `windows`, and
`persistent` fields. Gnoblin does not remove a declared workspace, the active
workspace, or a workspace that still contains windows.

## Windows, layers, and monitors

| Method                | Arguments                                                     | Successful result                        |
| --------------------- | ------------------------------------------------------------- | ---------------------------------------- |
| `window.list(args)`   | Optional `app_id`, `title`, `focused` filters                 | `{windows = {Window, ...}}`              |
| `window.match(args)`  | Optional string `window` ID; defaults to `"active"`           | Window identity and a `match` rule table |
| `window.action(args)` | `action`; optional `window`, geometry, `monitor`, `workspace` | Action result                            |
| `layer.list()`        | None                                                          | `{surfaces = {Surface, ...}}`            |
| `monitor.list()`      | None                                                          | `{monitors = {Monitor, ...}}`            |

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

| Action fields                  | Accepted value                   | Meaning                                           |
| ------------------------------ | -------------------------------- | ------------------------------------------------- |
| `x`, `y` for `move`            | Integers from −100000 to 100000  | Window position.                                  |
| `width`, `height` for `resize` | Integers from 1 to 32768         | Window size.                                      |
| `monitor`                      | Index from `monitor.list()`      | Destination monitor.                              |
| `workspace`                    | `{id = ...}` or `{number = ...}` | Destination workspace for the `workspace` action. |

Mutter rejects actions that the target window cannot perform in its current
state.

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

| Method                    | Arguments                | Successful result                                 |
| ------------------------- | ------------------------ | ------------------------------------------------- |
| `privacy.get()`           | None                     | `{screenSharing, microphoneInUse, locationInUse}` |
| `permissions.list()`      | None                     | Current permission policy                         |
| `permissions.check(args)` | `capability`, `identity` | Permission decision with scope details            |
| `grant.list()`            | None                     | `{grants = {Grant, ...}}`                         |
| `grant.revoke(args)`      | `kind`, `id`             | `{ok, id}`                                        |

Permission capabilities, identities, grant kinds, and scope fields use the
same values as [session permissions](/config/configure/permissions).
`grant.revoke` revokes a listed portal grant; get its `kind` and `id` from
`grant.list()`.

## Launch feedback, shell, and configuration

| Method                    | Arguments                                       | Successful result                                                |
| ------------------------- | ----------------------------------------------- | ---------------------------------------------------------------- |
| `launch.status()`         | None                                            | `{launches = {...}}`                                             |
| `launch.begin(args)`      | `token`, `application`; optional `milliseconds` | `{ok, token}`                                                    |
| `launch.end(args)`        | `token`                                         | `{ok, token}`                                                    |
| `shell.ping()`            | None                                            | `{pong}`                                                         |
| `shell.version()`         | None                                            | `{version}`                                                      |
| `shell.status()`          | None                                            | Shell version, connection state, window count, focused window ID |
| `shell.reload()`          | None                                            | Reload acknowledgement                                           |
| `runtime.reload_config()` | None                                            | Configuration reload acknowledgement                             |

`launch.begin` accepts a token of up to 128 characters and an application name
of up to 512 characters. Its `milliseconds` value defaults to 3000 and is
clamped to 100–10000 ms. Pass the same token to `launch.end` when the
application starts or the launch request is cancelled.
`shell.reload` reloads the GNOME Shell integration;
`runtime.reload_config` reloads Gnoblin configuration.

## Shortcut capture

| Method                   | Arguments                          | Successful result |
| ------------------------ | ---------------------------------- | ----------------- |
| `shortcut.capture(args)` | `timeout` in seconds, from 1 to 60 | `{accelerator}`   |

The capture asks for one keyboard shortcut and returns its accelerator string.
It fails if another capture is active, keyboard input is already grabbed, or
the session is locked.

See [`gnoblinctl`](/gnoblinctl) for command-line forms of these operations.
