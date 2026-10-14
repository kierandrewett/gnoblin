# Window Commands

List windows, inspect their rule-matching fields, and request supported window operations.

Window commands require a running session; window IDs remain valid for the lifetime of their window.

## Windows

```sh
gnoblinctl window list
gnoblinctl window minimize 42
gnoblinctl window unminimize 42
gnoblinctl window toggle-minimize 42
gnoblinctl window restore 42
gnoblinctl window maximize 42
gnoblinctl window close 42
gnoblinctl window kill 42
gnoblinctl window raise 42
gnoblinctl window lower 42
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

## Force-quit a frozen window

`kill` sends SIGKILL to the process that owns the window, so unsaved work is lost.

```sh
gnoblinctl window kill 42
```

A window whose client stops answering liveness pings reports `unresponsive` as true in
`window list --json`, and the compositor sends `gnoblin.window.changed`. The ping timeout is
`window_management.check_alive_timeout`.

Gnoblin shows no dialog of its own. A shell or a Lua script decides when to offer `kill`.

`window thumbnail` saves a PNG for a stable window ID. By default, it requests
an image up to 320 × 200 pixels. Set `--width` (1–480) and `--height` (1–320)
to change those bounds. The compositor may return a smaller image to preserve
the window's aspect ratio.

The command requires `--output PATH` and prints the saved path and dimensions.
Thumbnails are unavailable while the session is locked.

Filter by focused state, exact desktop app ID or a case-insensitive substring
of the title with `--focused`, `--app-id ID` or `--title TEXT`.

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
[compositor bridge](/bridge/input-and-focus#api-version-132-xdg-activation-focus-and-session-logout).
