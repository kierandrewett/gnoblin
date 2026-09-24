# Shortcuts

[Configuration reference](/config/configure)

Use `gnoblin.configure {shortcuts = {...}}` to launch a program with a key combination, including a media key. Use `keybindings` to change built-in actions such as closing a window. Add the examples to `~/.config/gnoblin/init.lua`; both reload on save.

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

Names use letters, numbers, `_` and `-`. Up to 256 command shortcuts are allowed.
Disabling one releases its binding; it does not stop a launched program.

## Disable a shortcut

`enable = false` disables an imported shortcut for this config load. Use it when a shell's config supplies a shortcut you do not want:

```lua
gnoblin.configure {
    shortcuts = {my_terminal = {enable = false}},
}
```

The config is rebuilt on every reload. This releases that entry's binding;
it does not change GNOME's built-in keybindings or shortcuts belonging to
other programs. Put the setting after the file that adds the shortcut;
[load order](/guides/files_and_load_order#override-or-append) matters.

Lua can inspect the shortcuts declared so far. For example, this disables
every named playback command in the bundled config:

```lua
for name, shortcut in pairs(gnoblin.configure.shortcuts) do
    if name:match("^media%-") then
        shortcut.enable = false
    end
end
```

## Key names

| Binding             | Keys                                         |
| ------------------- | -------------------------------------------- |
| `"<Super>Return"`   | Super + Enter                                |
| `"<Super><Shift>q"` | Super + Shift + Q                            |
| `"<Alt>F8"`         | Alt + F8                                     |
| `"Super"`           | Super press and release, without another key |

Super is usually the Windows-logo key. Put modifiers in angle brackets and
the main key after them. This is [GTK accelerator syntax](https://docs.gtk.org/gtk4/func.accelerator_parse.html).
Held keys do not repeatedly launch commands.
Command shortcuts are inactive on the lock and login screens.

## Run a command on release

Command shortcuts run when the key combination is pressed. Set `trigger` to
`"release"` to launch after the combination is released:

```lua
gnoblin.configure {
    shortcuts = {
        launcher = {
            binding = "<Super>space",
            command = {"my-launcher"},
            trigger = "release",
        },
    },
}
```

This lets a launcher start after the shortcut chord is complete. Bare `"Super"`
already runs on release; Mutter waits to see whether another key joins the
chord before emitting its overlay-key event.

## Change a built-in action

```lua
gnoblin.configure {
    shortcuts = {
        close_window = {
            action = {
                schema = "org.gnome.desktop.wm.keybindings",
                key = "close",
            },
            binding = {"<Super>q"},
        },
    },
}
```

`action` names a GSettings schema and key. Use `gsettings list-keys SCHEMA`
to find keys and `gsettings describe SCHEMA KEY` to read what one does. For
example:

```sh
gsettings list-keys org.gnome.desktop.wm.keybindings
gsettings describe org.gnome.desktop.wm.keybindings close
```

Copy the schema ID and native key spelling into the `action` table. For
example, use `schema = "org.gnome.desktop.wm.keybindings"` and `key = "close"`.
The [shortcut reference](/config/configure/shortcuts) lists the GNOME 51 keys
with descriptions and maps each schema to its purpose. Other GNOME versions
may provide different keys. The Lua config shows Gnoblin's active bindings;
the list from GSettings shows available keys. Use an empty binding list to
disable an action. Removing the entry restores its built-in default on reload.
Commands and built-in actions share the same `shortcuts` map.

## Avoid conflicts

To override an imported shortcut, use the same map key. Only supplied fields
change. Different names must use different bindings. See the
[desktop setup example](/recipes/small-desktop).

An existing GNOME action can also own the key. Disable or rebind that action
first. Invalid or conflicting edits keep the previous working registrations.

## Commands and shell syntax

Commands run directly. `$HOME`, `~`, pipes and redirection are not expanded.
Use an absolute path, a program on PATH, or explicitly run a shell:

```lua
gnoblin.configure {
    shortcuts = {
        log_time = {
            binding = "<Super><Shift>t",
            command = {"sh", "-c", "date >> \"$HOME/shortcut.log\""},
        },
    },
}
```

This appends the current time to `~/shortcut.log` when you press Super+Shift+T.

## Buffer typing while a popup opens

A shortcut can set `capture_input = true` to buffer typing between activation
and popup focus. The popup must implement the
[input handoff protocol](/compositor-bridge). This is separate from choosing
whether the command runs on press or release.

See also [restore-or-minimise bindings](/guides/window_state_shortcuts).
