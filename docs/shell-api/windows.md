# Windows

## Immediate window snapshots

The native Mutter runtime exposes the latest window state directly:

| Lua call                        | Arguments                                                                   | Result                                |
| ------------------------------- | --------------------------------------------------------------------------- | ------------------------------------- |
| `gnoblin.windows.list(filter?)` | Optional `app_id`, `title`, `focused`, `workspace_id`, `monitor_id` filters | Read-only window snapshots            |
| `gnoblin.windows.focused()`     | None                                                                        | Read-only window snapshot or `nil`    |
| `gnoblin.windows.by_id(id)`     | Stable string window ID                                                     | Read-only window snapshot or `nil`    |

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
- `gnoblin.window.changed`, `gnoblin.window.attention-changed`, and
  `gnoblin.window.activation-denied`
- `gnoblin.window.focused` and `gnoblin.window.unfocused`

These reads are available in Gnoblin's standalone session. They raise a
Lua error when the snapshot is unavailable.

## Snapshot record methods

Window and workspace snapshots expose methods that queue the corresponding
typed operation. Call them with colon syntax from a runtime event callback.
Each method returns an `Operation` handle.

| Window method                                  | Arguments                                         | Effect                                                    |
| ---------------------------------------------- | ------------------------------------------------- | --------------------------------------------------------- |
| `window:close()`                               | None                                              | Ask the application to close.                             |
| `window:minimize()`                            | None                                              | Minimize the window.                                      |
| `window:toggle_minimize()`                     | None                                              | Minimize or restore the window.                           |
| `window:unminimize()`                          | None                                              | Remove minimization without changing maximization.        |
| `window:restore()`                             | None                                              | Remove minimization and maximization.                     |
| `window:restore_or_minimize()`                  | None                                              | Unmaximize, restore the pre-snap frame, or minimize.      |
| `window:set_maximized(enabled)`                | Boolean                                           | Set maximization.                                         |
| `window:set_fullscreen(enabled)`               | Boolean                                           | Set fullscreen.                                           |
| `window:set_above(enabled)`                    | Boolean                                           | Set the above state.                                      |
| `window:set_sticky(enabled)`                   | Boolean                                           | Set visibility across workspaces.                         |
| `window:move(position)`                        | `{x, y}`; integers from −100000 to 100000 each     | Move in logical desktop pixels.                           |
| `window:resize(size)`                          | `{width, height}`; integers from 1 to 32768        | Resize in logical pixels.                                 |
| `window:move_to_workspace(selector, options?)` | Workspace selector; optional `follow` boolean     | Move this window and optionally activate the destination. |
| `window:move_to_monitor(target)`               | Monitor connector ID or `{id = ID}`               | Move this window to an active monitor.                    |
| `window:thumbnail(size)`                       | `{width = integer 1–480, height = integer 1–320}` | Capture a bounded compositor-rendered PNG preview.        |

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

For socket operation names and argument fields, see the [compositor bridge](/compositor-bridge).


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
including malformed arguments. The [compositor bridge](/compositor-bridge)
documents the socket form. Gnoblin rechecks the window and lock state before
using a fresh Mutter timestamp.

`gnoblin.osd.requested` includes:

- `monitor_id`: the stable ID of the logical monitor;
- `output_names`: on current builds, a sorted, unique list of active physical
  connector names for that logical monitor. Older builds may omit it;
- `icon` and `label`: optional fields supplied by Mutter.

Mutter provides no OSD level or maximum. Gnoblin does not create the OSD; the
subscribed shell decides how to present the request.
