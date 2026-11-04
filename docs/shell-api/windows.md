# Windows

## Read windows

The native Mutter runtime exposes the window state directly:

| Lua call                        | Arguments                                                                   | Result                   |
| ------------------------------- | --------------------------------------------------------------------------- | ------------------------ |
| `gnoblin.windows.list(filter?)` | Optional `app_id`, `title`, `focused`, `workspace_id`, `monitor_id` filters | `Window` objects         |
| `gnoblin.windows.focused()`     | None                                                                        | `Window` object or `nil` |
| `gnoblin.windows.by_id(id)`     | Stable string window ID                                                     | `Window` object or `nil` |

Window filter values:

- `title` takes a string and matches a case-insensitive substring.
- `focused` takes a boolean.
- `app_id`, `workspace_id`, and `monitor_id` take strings and match exactly.

Unknown filter names or values of the wrong type raise a Lua error. These reads
use a cached native snapshot and return immediately; they do not create an
`Operation`. Each object includes a state revision.

Gnoblin refreshes both collections in one coalesced main-loop publication. A
read made before that publication completes returns the last published
state. Lifecycle callbacks run after the corresponding snapshots are refreshed,
so handlers can read the new state.

These reads are available in Gnoblin's standalone session. They raise a
Lua error when the snapshot is unavailable.

## Live window objects

Every read of one window returns the same `Window` object, in event fields as
well as in the reads above. The object always shows the state from the
runtime's last update, so you can keep it and read it later. Its fields stay
read-only; change a window through its methods.

```lua
gnoblin.on("gnoblin.window.created", function(event)
    local window = event.window
    window:watch("title", function(_, title, old)
        print(old .. " -> " .. title)
    end)
    window:on("closed", function()
        print(window.title .. " closed")
    end)
end)
```

- `window.alive` is `false` once the window has closed. The object keeps its
  last state.
- `window:on(signal, fn)` calls `fn(window)` for `"focused"`, `"unfocused"` and
  `"closed"`, and `fn(window, changed)` for `"changed"`, where `changed` lists
  the fields that moved. Listening ends after `"closed"`.
- `window:watch(field, fn)` calls `fn(window, value, old)` when one field
  changes. The field must be one that `Window` declares; another name raises an
  error.

Both return a table whose `unsubscribe()` stops the callback. The global
events `gnoblin.window.created`, `closed`, `changed`, `attention-changed`,
`activation-denied`, `focused` and `unfocused` still fire as before.

## Window methods

Window and workspace objects expose methods that queue the corresponding
typed operation. Call them with colon syntax from a runtime event callback.
Each method returns an `Operation` handle.

| Window method                                   | Arguments                                 | Effect                                                       |
| ----------------------------------------------- | ----------------------------------------- | ------------------------------------------------------------ |
| `window:close()`                                | None                                      | Ask the application to close.                                |
| `window:kill()`                                 | None                                      | Kill the client process; use it when `unresponsive` is true. |
| `window:set_minimized(enabled)`                 | Boolean                                   | Minimize the window, or remove minimization.                 |
| `window:restore_or_minimize()`                  | None                                      | Unmaximize, restore the pre-snap frame, or minimize.         |
| `window:set_maximized(enabled)`                 | Boolean                                   | Set maximization.                                            |
| `window:set_fullscreen(enabled)`                | Boolean                                   | Set fullscreen.                                              |
| `window:set_above(enabled)`                     | Boolean                                   | Set the above state.                                         |
| `window:set_sticky(enabled)`                    | Boolean                                   | Set visibility across workspaces.                            |
| `window:move(x, y)`                             | Integers from −100000 to 100000           | Move in logical desktop pixels.                              |
| `window:resize(width, height)`                  | Integers from 1 to 32768                  | Resize in logical pixels.                                    |
| `window:move_to_workspace(workspace, options?)` | Workspace ID or number; option `follow`   | Move this window and optionally activate the destination.    |
| `window:move_to_monitor(monitor)`               | Monitor connector ID                      | Move this window to an active monitor.                       |
| `window:thumbnail(width, height)`               | Width from 1 to 480, height from 1 to 320 | Capture a bounded compositor-rendered PNG preview.           |
| `window:present()`                              | None                                      | Focus a window of a process the runtime started.             |

Record properties remain read-only. In the native runtime,
`Window:focus(context)` requests focus using the context from
`gnoblin.shortcut.activated`. The callback provides that value only for a
trusted shortcut key press. It expires after five seconds.
A successful focus operation completes with `{id = window.id}`.

The same context can authorize `begin_move` or `begin_resize`. It works once
across all three operations. A config reload invalidates contexts from the old
Lua runtime; a session lock also revokes them.

Mutter still applies its normal window activation policy.

Contexts are not provided for clicks, external-client shortcuts, release
bindings, synthetic input, input-method events, or repeated key presses. Direct
`Window:focus`, `Window:begin_move`, and `Window:begin_resize` calls without a
context remain denied.

For socket operation names and argument fields, see the [compositor bridge](/bridge/).

## Change how a window is drawn

These methods change only how the compositor draws a window. The frame,
workspace, focus and what the application sees stay the same. Each returns an
`Operation` whose value lists the window's visual state.

| Window method                  | Accepted values                       | Default  |
| ------------------------------ | ------------------------------------- | -------- |
| `window:set_opacity(opacity)`  | 0 (invisible) to 1 (opaque)           | 1        |
| `window:set_scale(scale)`      | 0.01 to 10, about the window's centre | 1        |
| `window:set_rotation(degrees)` | -3600 to 3600, about the centre       | 0        |
| `window:set_offset(x, y)`      | -100000 to 100000 logical pixels each | 0, 0     |
| `window:reset_visuals()`       | None                                  | Defaults |

```lua
window:set_offset(0, 400)
window:set_opacity(0.8)
```

- The window's `opacity`, `scale`, `rotation` and `offset` fields show the
  current values, so `window:watch("offset", fn)` sees each change.
- Pointer clicks follow the drawn position: after `set_offset`, a click where
  the window is drawn reaches it.
- A window rule's `opacity` multiplies with `set_opacity`.
- A config reload, or a stopped Lua worker, resets every window to the
  defaults.
- While the session is locked, these methods fail.
- An open, close, minimize or resize animation draws the window itself. A
  change made while one plays can be overwritten when it ends.

## Change what a window's rules give

`window:set_effect(name, value)` layers one [window rule](/config/window_rule#rule-fields) effect over one window and
returns an `Override`. The effect goes over every rule that matches the window and changes only what it sets. The
names are:

- `opacity`, `blur` and `blur_ignore_shadows`
- `shader` and `shader_uniforms`
- `frame` and `corners`

```lua
local blurred = window:set_effect("blur", 24)
local round = window:set_effect("corners", {radius = 14, border_width = 1})

blurred:revoke()
```

The effect ends when you revoke it, when the window closes and when a configuration reload commits. A newer effect of
the same name replaces an older one. The window `opacity` effect is the rule's opacity of the window content, and it
multiplies with `set_opacity`. See [overrides](/shell-api/overrides) for the ending reasons and the rules.

## Compositor requests

Subscribe to `gnoblin.window.menu-requested` to show a shell-owned window menu,
or `gnoblin.osd.requested` to show an OSD requested by Mutter. Lua callbacks
receive events with those names.

`gnoblin.window.menu-requested` includes:

- `window_id`: Gnoblin's stable window ID;
- `menu_type`: `wm` for a window manager menu or `app` for an application menu;
- `x` and `y`: global logical coordinates supplied by Mutter.

For `menu_type == "wm"`, Lua receives a non-serializable
`event.menu_context` userdata. In that callback, call
`event.menu_context:begin_move()` or
`event.menu_context:begin_resize(edge)` to start Mutter's keyboard grab on the
exact window that raised the menu. The context works once, expires after five
seconds, and is available only during that event callback. It is revoked on
session lock or runtime/config teardown. App-menu requests have no context.

`gnoblinctl lua` wraps this token as opaque `event.menu_context` userdata.
Call `event.menu_context:begin_move()` or
`event.menu_context:begin_resize(edge)` from the event callback. The console
hides the token and sends the operation over the event's connection. App-menu
events do not receive a `MenuContext`.

The token expires after five seconds and is consumed on every matching attempt,
including malformed arguments. The [compositor bridge](/bridge/)
documents the socket form. Gnoblin rechecks the window and lock state before
using a fresh Mutter timestamp.

`gnoblin.osd.requested` includes:

- `monitor_id`: the stable ID of the logical monitor;
- `output_names`: on current builds, a sorted, unique list of active physical
  connector names for that logical monitor. Older builds may omit it;
- `icon` and `label`: optional fields supplied by Mutter.

Mutter provides no OSD level or maximum. Gnoblin does not create the OSD; the
subscribed shell decides how to present the request.
