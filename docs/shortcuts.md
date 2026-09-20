# Keyboard shortcuts

[Configuration reference](configuration-reference.md)

Use `gnoblin.shortcut` to launch a program when you press a key combination.
Use `keybindings` to change built-in actions such as closing a window.
Add the examples to `~/.config/gnoblin/init.lua`; both reload on save.

## Launch a command

Add this after any `gnoblin.load(...)` lines. It opens a terminal with Super+Enter:

```lua
gnoblin.shortcut {
    name = "my-terminal",
    binding = "<Super>Return",
    command = {"ptyxis", "--new-window"},
}
```

Replace `ptyxis` with an installed terminal. Each command argument is a separate
string. Spaces inside a string stay in that argument.

Names use letters, numbers, `_` and `-`. Up to 256 command shortcuts are allowed.
Removing one releases its binding; it does not stop a launched program.

## Key names

| Binding             | Keys                                         |
| ------------------- | -------------------------------------------- |
| `"<Super>Return"`   | Super + Enter                                |
| `"<Super><Shift>q"` | Super + Shift + Q                            |
| `"<Alt>F8"`         | Alt + F8                                     |
| `"Super"`           | Super press and release, without another key |

Super is usually the Windows-logo key. Put modifiers in angle brackets and
the main key after them, as in the examples above (GTK accelerator syntax). Held keys do not repeatedly launch commands.
Command shortcuts are inactive on the lock and login screens.

## Change a built-in action

```lua
gnoblin.configure {
    keybindings = {
        wm = {
            close = {"<Super>q"},
        },
    },
}
```

Here `wm` selects window-management actions and `close` names the action.
To see its available actions and current bindings:

```sh
gsettings list-recursively org.gnome.desktop.wm.keybindings
```

Other groups are `shell`, `mutter`, `wayland` and `media`. See the
[keybinding groups](configuration-reference.md#keybinding-groups) for their
GSettings schema names.

Use an empty list to disable an action, for example `close = {}`.
These overrides persist in GSettings: removing the Lua entry does not restore
the former binding. Set the desired value explicitly.

## Avoid conflicts

To override an imported shortcut, use the same `name`. Only supplied fields
change. Different names must use different bindings. See the
[override example](configuration-recipes.md#add-a-shortcut-without-losing-the-others).

An existing GNOME action can also own the key. Disable or rebind that action
first. Invalid or conflicting edits keep the previous working registrations.

## Commands and shell syntax

Commands run directly. `$HOME`, `~`, pipes and redirection are not expanded.
Use an absolute path, a program on PATH, or explicitly run a shell:

```lua
gnoblin.shortcut {
    name = "log-time",
    binding = "<Super><Shift>t",
    command = {"sh", "-c", "date >> \"$HOME/shortcut.log\""},
}
```

This appends the current time to `~/shortcut.log` when you press Super+Shift+T.

## Popups that capture typing

A shortcut can set `capture_input = true` to buffer typing while a popup
starts. The popup must implement the [input handoff protocol](compositor-bridge.md).
Do not enable it for ordinary terminal or application launch commands.

See also [restore-or-minimise bindings](window-state-shortcuts.md).
