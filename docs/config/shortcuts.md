# shortcuts

[Configuration reference](/config/reference)

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

## Remove a shortcut

`remove_shortcut` excludes a named `gnoblin.shortcut` entry added earlier in
the same config load. Use it when a shell's config supplies a shortcut you do
not want:

```lua
gnoblin.remove_shortcut("my-terminal")
```

The config is rebuilt on every reload. This releases that entry's binding;
it does not change GNOME's built-in keybindings or shortcuts belonging to
other programs. An unknown name does nothing. Put the removal after the file
that adds the shortcut;
[load order](/config/files_and_load_order#override-or-append) matters.

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
Lua file. Removing an entry restores its built-in default on reload. Media keys
handled by GNOME Settings Daemon are outside this table.

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
