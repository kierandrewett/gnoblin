# Windows

> **Status:** not available yet: `:begin_move`, `:begin_resize`, `:narrow`, `:reset_visuals`, `:restore_or_minimize`, `:revoke`, `:set_above`, `:set_effect`, `:set_offset`, `:set_opacity`, `:set_rotation`, `:set_scale`, `:set_sticky`, `:thumbnail`, `gnoblin.osd.requested`, `gnoblin.shortcut.activated`, `gnoblin.window.menu`. The sections that use them describe the target, not current behaviour.

## Read windows

The native Mutter runtime exposes the window state directly:

| Lua call                          | Arguments                               | Result                   |
| --------------------------------- | --------------------------------------- | ------------------------ |
| `gnoblin.windows.list()`          | None                                    | `Window` objects         |
| `gnoblin.windows.focused()`       | None                                    | `Window` object or `nil` |
| `gnoblin.windows.by_id(id)`       | Integer window ID                       | `Window` object or `nil` |
| `gnoblin.find("Window", filter?)` | A table of field values, or a predicate | `Window` objects         |

`gnoblin.windows.list()` takes no filter. To select windows, call `gnoblin.find("Window", filter)`.

- A table filter keeps the windows whose fields equal every value you give. It accepts any scalar field of
  `Window`, such as `app_id`, `title`, `focused`, `x` or `width`.
- A predicate filter is a function that receives each window and returns `true` to keep it.

See [find](/shell-api/find).

```lua
local terminals = gnoblin.find("Window", {app_id = "org.gnome.Console"})
local drafts = gnoblin.find("Window", function(window)
    return window.title:find("draft", 1, true) ~= nil
end)
```

A filter that names a field `Window` does not have, or a field that is not a scalar such as `effects`, raises a Lua
error that lists the fields that work.

These reads use a cached native snapshot and return immediately. They do not create an `Operation`.

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
  changes. The field must be one that `Window` declares as observable, which
  means it changes while the window is alive. `watch` refuses a field that never
  moves and names the fields that work.
- `window.pid` is the process ID of the window's Wayland client, read from the client's socket credentials. It is `nil`
  for an X11 window and for a client whose credentials the compositor cannot read. It is never taken from the app ID
  or the title.
- `window.process` is the `id` of the live [`Process`](/shell-api/processes) that `gnoblin.process.spawn` started with
  that pid, or `nil`. Only a process the runtime started itself matches, and only while it is alive. Resolve it with
  `gnoblin.process.by_id`.
- The fields that never move while the window is alive are `id` and `pid`.

Both return a subscription, and `subscription:unsubscribe()` stops the callback. Every `on` and `watch` in the API returns this same shape. The global
events `gnoblin.window.created`, `closed`, `changed`, `attention-changed`,
`activation-denied`, `focused` and `unfocused` still fire as before.

## Window methods

Window and workspace objects expose methods that queue the corresponding
typed operation. Call them with colon syntax from a runtime event callback.
Each method returns an `Operation` handle.

| Window method                     | Arguments                                 | Effect                                                                                           |
| --------------------------------- | ----------------------------------------- | ------------------------------------------------------------------------------------------------ |
| `window:close()`                  | None                                      | Ask the application to close.                                                                    |
| `window:kill()`                   | None                                      | SIGKILL the window's process, refused if unknown, gone or changed, or the compositor's pid or 1. |
| `window:set_minimized(enabled)`   | Boolean                                   | Minimize the window, or remove minimization.                                                     |
| `window:restore_or_minimize()`    | None                                      | Unmaximize, restore the pre-snap frame, or minimize.                                             |
| `window:set_maximized(enabled)`   | Boolean                                   | Set maximization.                                                                                |
| `window:set_fullscreen(enabled)`  | Boolean                                   | Set fullscreen.                                                                                  |
| `window:set_above(enabled)`       | Boolean                                   | Set the above state.                                                                             |
| `window:set_sticky(enabled)`      | Boolean                                   | Set visibility across workspaces.                                                                |
| `window:move(x, y)`               | Integers from −100000 to 100000           | Move in logical desktop pixels.                                                                  |
| `window:resize(width, height)`    | Integers from 1 to 32768                  | Resize in logical pixels.                                                                        |
| `window:set_workspace(workspace)` | `Workspace`                               | Move this window, and its transient children, to the workspace.                                  |
| `window:set_output(output)`       | `Output`                                  | Move this window to an output, reshaping it into that output's work area.                        |
| `window:thumbnail(width, height)` | Width from 1 to 480, height from 1 to 320 | Capture a bounded compositor-rendered PNG preview.                                               |
| `window:present()`                | None                                      | Focus a window of a process the runtime started.                                                 |

Record properties remain read-only. In the native runtime,
`Window:focus(grant)` requests focus with the [`grant`](/shell-api/grants) of
an event such as `gnoblin.shortcut.activated`. The callback provides that value only for a
trusted key press, pointer binding or window menu request. It expires after five seconds.
A successful focus operation completes with `{id = window.id}`.

The same grant can authorize `begin_move` and `begin_resize`. Each of the three operations
works once for a gesture, so one key press can focus a window and then begin a move.
`grant:narrow(scope, window?)` hands out less. A config reload invalidates grants from
the old Lua runtime; a session lock also revokes them.

Mutter still applies its normal window activation policy.

Grants are not provided for external-client shortcuts, release
bindings, synthetic input, input-method events, or repeated key presses. Direct
`Window:focus`, `Window:begin_move`, and `Window:begin_resize` calls without a
grant remain denied.

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
  change made while one plays can be overwritten when it ends. On the Rust compositor a minimize
  fades the window's picture out over 200 ms and keeps it for restore, and a workspace switch slides
  the window; see [animations](/guides/animations).

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

Subscribe to `gnoblin.window.menu_requested` to show a shell-owned window menu,
or `gnoblin.osd.requested` to show an OSD requested by Mutter. Lua callbacks
receive events with those names.

`gnoblin.window.menu_requested` includes:

- `x`, `y`, `width` and `height`: the anchor rectangle, in global logical coordinates;
- `source`: `client`, `keyboard` or `extension`, saying what asked for the menu;
- `seat`: the seat that asked for it.

> **Status:** not available yet: `menu_type` and `event.grant`. The menu's grant sentences below describe the target.

For `menu_type == "wm"`, Lua receives a [`grant`](/shell-api/grants) in `event.grant`.
In that callback, call `window:begin_move(event.grant)` or
`window:begin_resize(edge, event.grant)` on the window that raised the menu
(`event.grant.window`) to start Mutter's keyboard grab on it.

- The grant works once, for one move or one resize.
- It expires after five seconds, and is available only during that event callback.
- A bad edge is a Lua error that spends nothing.
- It is revoked on session lock or runtime/config teardown.
- App-menu requests have no grant.

`gnoblinctl lua` wraps the socket token the same way: a `Grant` in `event.grant`,
used as `window:begin_move(event.grant)`. The console
hides the token and sends the operation over the event's connection. App-menu
events get no grant.

On the socket, the wire event carries a `menu_context` token in place of the `grant`. It expires after five seconds and is consumed on every matching attempt,
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
