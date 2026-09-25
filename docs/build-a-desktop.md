# Build a floating desktop

Gnoblin supplies the compositor, window management and interfaces that desktop
components can use. A shell supplies the visible controls. The shell can be one
program or a collection of smaller clients; it is independent of Gnoblin.
Bingux is one separate project built on these interfaces.

| Piece            | Gnoblin provides                                        | Your desktop chooses                           |
| ---------------- | ------------------------------------------------------- | ---------------------------------------------- |
| Windows          | Focus, move, resize, workspaces, rules and decorations  | Dock grouping, switcher order and menus        |
| Input            | Keybindings and input-source control                    | Launcher, search and popup design              |
| Surfaces         | Layer-shell placement, exclusive zones and effects      | Bar, dock, wallpaper and overlays              |
| Desktop services | Portals, permission policy and optional native features | Notification daemon and other visible controls |

![Calculator on a fresh Gnoblin devkit desktop with a Bingux bar and dock](images/gnoblin-example-desktop.png)

_Calculator in a clean Gnoblin devkit session. Bingux, a separate example
shell, supplies the bar, dock and wallpaper. The dock pins Files, Firefox and
Terminal from installed desktop entries._

The [configuration reference](/config/configure) describes the Lua
settings. The [CLI](gnoblinctl.md) is convenient for commands; the
[compositor bridge](compositor-bridge.md) supplies subscriptions and shortcuts
to a long-running shell. [Wayland protocols](wayland-protocols.md) serve native
clients, and [user scripts](user-scripts.md) react inside GNOME Shell.

## Assemble a small desktop

Install a bar, launcher, notification daemon and terminal. This example uses
Waybar, Fuzzel, Mako and Foot. They are independent applications; replace any
of them with a component you prefer. Put this in a **new**
`~/.config/gnoblin/init.lua`:

```lua
gnoblin.configure {
    cursor = {theme = "Adwaita-Hyprcursor", size = 28},
    autostart = {
        bar = {command = {"waybar"}},
        notifications = {command = {"mako"}},
    },
    shortcuts = {
        launcher = {binding = "<Super>d", command = {"fuzzel"}},
        terminal = {binding = "<Super>Return", command = {"foot"}},
    },
}
```

![Waybar and Files in a fresh Gnoblin session](images/gnoblin-build-a-desktop.png)

_Waybar above the stock Files app in a disposable Gnoblin profile._

The cursor setting requires the Adwaita Hyprcursor theme; install it using the
[cursor guide](/guides/cursors).

Capture this example from the checkout with
`scripts/capture-doc-examples.sh desktop`. It uses a disposable config and
profile; see the [capture script](devkit.md#documentation-captures) for setup.

If your existing `init.lua` loads files installed by a shell, keep those
`gnoblin.load(...)` lines and put your additions after them. Use an imported
shortcut's existing name when changing its command. [Load order and merging](/guides/files_and_load_order)
explain why the order matters.

Log in to Gnoblin, then run `gnoblinctl config path` to confirm the active
file. `gnoblinctl config reload` reports errors and applies a valid edit.
Autostart launches each named command once per login. Adding a new name during
a reload starts it, while disabling or removing one does not stop a running
process. See the [autostart guide](/guides/autostart) for details.

Waybar's Sway and Hyprland modules expect those compositors' own IPC and do
not gain that IPC merely by running under Gnoblin. Configure supported generic
modules or write a module using [window data](compositor-bridge.md#windows-and-controls).

## Give each visible function an owner

Only one notification daemon should own notifications. If your shell handles
them, leave `shell.notifications` disabled; if it does not, you can enable
Gnoblin's native service. The same choice applies to a window switcher and
keyboard-layout popup. See [native features](/guides/session_settings#native-features)
and [shortcut conflicts](/guides/shortcuts#avoid-conflicts).

A dock can use the foreign toplevel protocols for basic window handles or the
bridge for records, previews, activation and shortcut sessions. Keep window IDs
only for the lifetime of their windows. Refresh snapshots after windows close
or workspaces change. [Bridge examples](bridge-examples.md) show clients in
several languages.

## Choose the right interface

| Task                                     | Interface                                                  |
| ---------------------------------------- | ---------------------------------------------------------- |
| Change a window from a script            | `gnoblinctl window ...`                                    |
| Maintain a live switcher or dock         | Compositor bridge `windows` subscription                   |
| Place a bar or dock                      | `zwlr_layer_shell_v1`                                      |
| Capture output with a native client      | `zwlr_screencopy_manager_v1`, subject to its protocol gate |
| Apply per-app styling                    | `gnoblin.window_rule` in Lua                               |
| React to a workspace change inside Shell | GJS user script `api.on("workspace-changed", ...)`         |
| Supply an application titlebar           | Frame rule and optional renderer service                   |

The desktop portal has its own permission policy. Disabling a Wayland protocol
does not replace [portal permissions](/guides/permissions) for screen sharing or
remote control.

## Work on the desktop in a nested session

The [devkit](devkit.md) starts Gnoblin in a window. Use it to try a bar or rule
before logging out.

For screenshots and demos, give it fresh home and XDG config, data, cache,
state, and runtime directories. Launch only the applications that belong in
the example. A normal devkit invocation keeps your real home directory; see
the [isolation notes](devkit.md#isolation).

For packaging or a final session check, use a real Gnoblin login. A devkit
image proves what appeared in that nested run, not what is installed in a
login session.

## Continue from here

- [Window rules](/guides/window_rules), [effects](/guides/window_effects) and [frames](/guides/window_frames)
- [Shell integration](shell-integration.md), [window menus](/guides/window_menu) and [snapping](/guides/window_snapping)
- [Protocol catalog](wayland-protocols.md) and [compositor bridge](compositor-bridge.md)
- [Troubleshooting](troubleshooting.md) when a component does not appear
