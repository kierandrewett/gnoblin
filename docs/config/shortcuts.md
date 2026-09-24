# shortcuts

[Configuration reference](/config/reference)

Use the named `shortcuts` table to launch a program when you press a key combination.
Use `keybindings` to change built-in actions such as closing a window.
Add the examples to `~/.config/gnoblin/init.lua`; both reload on save.

## Launch a command

Add this after any `gnoblin.load(...)` lines. It opens a terminal with Super+Enter:

```lua
gnoblin.configure {
    shortcuts = {
        my_terminal = {
            binding = "<Super>Return",
            command = {"ptyxis", "--new-window"},
        },
    },
}
```

Replace `ptyxis` with an installed terminal. Each command argument is a separate
string. Spaces inside a string stay in that argument.

The map key is the shortcut's name. Names use letters, numbers, `_` and `-`.
Up to 256 command shortcuts are allowed.

## Function form

For an existing config that uses declarations, the same shortcut is:

```lua
gnoblin.shortcut {
    name = "my_terminal",
    binding = "<Super>Return",
    command = {"ptyxis", "--new-window"},
}
```

Use the same name to change only the fields you supply. Set `enable = false`
with that name to disable it.

## Remove a shortcut

After loading the file that defines `my_terminal`, disable it directly:

```lua
gnoblin.configure.shortcuts.my_terminal.enable = false
```

The config is rebuilt on every reload. Disabling releases that entry's binding;
it does not stop a program already launched by the shortcut. Put this after
the file that defines it; [load order](/config/files_and_load_order#override-or-append)
matters. An unknown name raises a Lua error. The named map form,
`gnoblin.configure {shortcuts = {my_terminal = {enable = false}}}`, also works;
`gnoblin.remove_shortcut(name)` remains available for older configs.

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
Action names use underscores in Gnoblin config. For example, GSettings' key
`show-screenshot-ui` is `show_screenshot_ui` here.
To see available action names in the GNOME catalogue:

```sh
gsettings list-keys org.gnome.desktop.wm.keybindings
```

The values printed by `gsettings` are GNOME settings; Lua overrides are active
in Gnoblin and do not appear there.

Other groups are `shell`, `mutter` and `wayland`. See the
[keybinding groups](/config/reference#keybinding-groups) for their
GSettings schema names.

Use an empty list to disable an action, for example `close = {}`.
These overrides live in Gnoblin's native keybinding table and persist in the
Lua file. Removing an entry restores its built-in default on reload.

## Media keys

Volume, brightness and playback keys are ordinary Gnoblin command shortcuts
in the editable first-login `init.lua`. For example:

```lua
gnoblin.shortcut {
    name = "volume-up",
    binding = "XF86AudioRaiseVolume",
    command = {"wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%+"},
}
```

Change or disable them in Lua like any other named shortcut. Gnoblin does not
start GNOME Settings Daemon's media-key handler in its session.

## Inspect shortcuts loaded so far

Use `pairs(gnoblin.configure.shortcuts)` to loop over command shortcuts loaded
so far and edit them by name. The [example](/config/files_and_load_order#inspect-loaded-settings)
shows this. `gnoblin.snapshot()` copies the assembled config when you need a
stable value. Built-in actions in `keybindings` are separate. Lua runs before
the compositor registers keys, so neither view reports successful live grabs.

## Avoid conflicts

To override an imported shortcut, use the same `name`. Only supplied fields
change. Different names must use different bindings. See the
[override example](/recipes#add-a-shortcut-without-losing-the-others).

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
starts. The popup must implement the [input handoff protocol](/compositor-bridge).
Do not enable it for ordinary terminal or application launch commands.

See also [restore-or-minimise bindings](/config/window_state_shortcuts).
