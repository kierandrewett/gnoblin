# Build a floating desktop

Gnoblin supplies the compositor, window management and interfaces that desktop
components can use. A shell supplies the visible controls. The shell can be one
program or a collection of smaller clients; it is independent of Gnoblin.
Bingux is one separate project built on these interfaces.

| Piece | Gnoblin provides | Your desktop chooses |
| --- | --- | --- |
| Windows | Focus, move, resize, workspaces, rules and decorations | Dock grouping, switcher order and menus |
| Input | Keybindings and input-source control | Launcher, search and popup design |
| Surfaces | Layer-shell placement, exclusive zones and effects | Bar, dock, wallpaper and overlays |
| Desktop services | Portals, permission policy and optional native features | Notification daemon and other visible controls |

The [configuration reference](configuration-reference.md) describes the Lua
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
gnoblin.autostart {name = "bar", command = {"waybar"}}
gnoblin.autostart {name = "notifications", command = {"mako"}}

gnoblin.shortcut {
    name = "launcher",
    binding = "<Super>d",
    command = {"fuzzel"},
}
gnoblin.shortcut {
    name = "terminal",
    binding = "<Super>Return",
    command = {"foot"},
}
```

If your existing `init.lua` loads files installed by a shell, keep those
`gnoblin.load(...)` lines and put your additions after them. Use an imported
shortcut's existing name when changing its command. [Load order and merging](configuration-loading.md)
explain why the order matters.

Log in to Gnoblin, then run `gnoblinctl config path` to confirm the active
file. `gnoblinctl config reload` reports errors and applies a valid edit.
Autostart runs each named command once per login; adding a new name during a
reload starts it, while removing one does not stop a running process.

Waybar's Sway and Hyprland modules expect those compositors' own IPC and do
not gain that IPC merely by running under Gnoblin. Configure supported generic
modules or write a module using [window data](compositor-bridge.md#windows-and-controls).

## Give each visible function an owner

Only one notification daemon should own notifications. If your shell handles
them, leave `shell.notifications` disabled; if it does not, you can enable
Gnoblin's native service. The same choice applies to a window switcher and
keyboard-layout popup. See [native features](session-settings.md#native-features)
and [shortcut conflicts](shortcuts.md#avoid-conflicts).

A dock can use the foreign toplevel protocols for basic window handles or the
bridge for records, previews, activation and shortcut sessions. Keep window IDs
only for the lifetime of their windows. Refresh snapshots after windows close
or workspaces change. [Bridge examples](bridge-examples.md) show clients in
several languages.

## Choose the right interface

| Task | Interface |
| --- | --- |
| Change a window from a script | `gnoblinctl window ...` |
| Maintain a live switcher or dock | Compositor bridge `windows` subscription |
| Place a bar or dock | `zwlr_layer_shell_v1` |
| Capture output with a native client | `zwlr_screencopy_manager_v1`, subject to its protocol gate |
| Apply per-app styling | `gnoblin.window_rule` in Lua |
| React to a workspace change inside Shell | GJS user script `api.on("workspace-changed", ...)` |
| Supply an application titlebar | Frame rule and optional renderer service |

The desktop portal has its own permission policy. Disabling a Wayland protocol
does not replace [portal permissions](permissions.md) for screen sharing or
remote control.

## Work on the desktop in a nested session

The [devkit](devkit.md) starts Gnoblin in a window. It is useful for trying a
bar or rule before logging out. When making screenshots or demos, give it a
fresh home and XDG config, data, cache, state and runtime directories, and run
only the applications that belong in the example. A normal devkit invocation
keeps your real home directory; see its [isolation notes](devkit.md#isolation).

For packaging or a final session check, use a real Gnoblin login. A devkit
image proves what appeared in that nested run, not what is installed in a
login session.

## Continue from here

- [Window rules](window-rules.md), [effects](window-effects.md) and [frames](window-frames.md)
- [Shell integration](shell-integration.md), [window menus](window-menu.md) and [snapping](window-snapping.md)
- [Protocol catalog](wayland-protocols.md) and [compositor bridge](compositor-bridge.md)
- [Troubleshooting](troubleshooting.md) when a component does not appear
