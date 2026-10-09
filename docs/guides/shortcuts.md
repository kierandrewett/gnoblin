# Keyboard shortcuts

[Configuration reference](/config/configure)

Gnoblin reads global shortcuts from its Lua config. Use
`gnoblin.configure.keybindings.keyboard` for a binding and its Lua callback.
The callback can run a command, manage a window, or return `"forward"` to
leave the input for the focused app. Add the examples to
`~/.config/gnoblin/init.lua`; changes apply when the config reloads.

## Launch a command

Add this after any `gnoblin.load(...)` lines. It opens a terminal with
Super+Enter:

```lua
gnoblin.configure {
    keybindings = {
        keyboard = {
            my_terminal = {
                binding = "<Super>Return",
                callback = function()
                    gnoblin.commands.run({"ptyxis", "--new-window"})
                end,
            },
        },
    },
}
```

Replace `ptyxis` with an installed terminal. Each command argument is a separate
string. Spaces inside a string stay in that argument.

Binding names use lowercase letters, numbers and `_`.

### Callback fields

| Field | Accepted value | Default | Effect |
| --- | --- | --- | --- |
| `binding` | One GTK accelerator string | Required | Chooses the key combination. |
| `callback` | Lua function | Required | Runs when the binding matches. |
| `trigger` | `"press"` or `"release"` | `"press"` | Chooses key press or release. |
| `enable` | `true` or `false` | `true` | `false` disables an imported named binding. |

The callback can return `"consume"` to keep the event in Gnoblin or
`"forward"` to route it to the focused app. A missing return is `"consume"`.

Release triggers consume the whole pair, including the reserved press.
Use a press trigger when the callback must choose whether to forward input.

Run `gnoblinctl reload` after changing the config.

## Open an application launcher

Bind Fuzzel to Super+D:

```lua
gnoblin.configure {
    keybindings = {
        keyboard = {
            launcher = {
                binding = "<Super>d",
                callback = function()
                    gnoblin.commands.run({"fuzzel"})
                end,
            },
        },
    },
}
```

![Fuzzel searching for Firefox on a clean Waybar desktop, with the pointer visible](../images/gnoblin-waybar-launcher.png)

_The shortcut opens Fuzzel; Firefox is the selected search result._

## Disable a Lua binding

Set `enable = false` to disable a named binding added by another config file:

```lua
gnoblin.configure {
    keybindings = {
        keyboard = {
            my_terminal = {enable = false},
        },
    },
}
```

The config is rebuilt on every reload. Disabling a binding releases its key.
Shortcuts registered by other programs are unaffected.

Put the removal after the file that adds the shortcut. See
[load order](/guides/files_and_load_order#override-or-append).

## Key names

| Binding             | Keys                                         |
| ------------------- | -------------------------------------------- |
| `"<Super>Return"`   | Super + Enter                                |
| `"<Super><Shift>q"` | Super + Shift + Q                            |
| `"<Alt>F8"`         | Alt + F8                                     |
| `"Super"`           | Super press and release, without another key |

Super is usually the Windows-logo key. Put modifiers in angle brackets and
the main key after them. This is
[GTK accelerator syntax](https://docs.gtk.org/gtk4/func.accelerator_parse.html).

Run `gnoblinctl shortcut capture` in a Gnoblin terminal and press a key
combination. It prints the GTK accelerator for `binding`; bare Super prints
Gnoblin’s special `Super` binding. Escape cancels.

The command grabs the keyboard while it waits and consumes the captured
combination. It times out after 30 seconds by default. Use `--timeout SECONDS`
for 1–60 seconds.

## Declarative shortcuts and built-in actions

Use `gnoblin.configure.shortcuts` for a declarative command shortcut when a
Lua callback is unnecessary. Use `gnoblin.commands.run` inside a callback when
the action needs Lua logic.

Use `keybindings.wm`, `keybindings.mutter` or `keybindings.wayland` to override
an existing Mutter action:

```lua
gnoblin.configure {
    keybindings = {
        wm = {close = {"<Super>q"}},
    },
}
```

Built-in actions accept a list of accelerator strings. Use an empty list to
disable one. The group must be `wm`, `mutter`, or `wayland`; each group maps to
a Mutter GSettings keybinding schema. The action names and defaults depend on
your installed desktop schemas; Gnoblin does not define a fixed list for every
version.

Find the schema and action names on your system with
`gsettings list-schemas` and `gsettings list-keys SCHEMA`. Run
`gsettings describe SCHEMA KEY` to read an action's description. The
[keybinding reference](/config/configure/keybindings) lists the three supported
Mutter groups and gives examples.

Desktop Settings edits do not change Gnoblin's active bindings. Removing an
override restores the default on reload.

## Media keys

Media keys use Lua keyboard callbacks. The starter config includes volume and
microphone mute through WirePlumber's `wpctl`, and playback through
`playerctl`. Install those commands if you use the matching keys. Brightness
keys are not handled by the standalone session; bind them to a command such as
`brightnessctl` if your hardware and permissions support it.

Gnoblin handles shortcuts inside the compositor. It does not require systemd
or GNOME Settings Daemon’s MediaKeys service.

Run `gnoblinctl config path` to see the config file used by your session. Run
`gnoblinctl config default` to print the packaged starter config. For example,
a volume key can run `wpctl` directly:

```lua
gnoblin.configure {
    keybindings = {
        keyboard = {
            volume_up = {
                binding = "XF86AudioRaiseVolume",
                callback = function()
                    gnoblin.commands.run({"wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%+"})
                end,
            },
        },
    },
}
```

Edit or remove these entries in your Lua config to change the media-key
behavior. The commands they call must be installed.

## Avoid conflicts

To override an imported callback binding, use the same map key and supply
its complete declaration. Use different bindings for independent shortcuts. See the
[override example](/recipes/add-a-shortcut).

A Mutter action may already use the key. Change or disable it through
`gnoblin.configure.keybindings` before assigning the same key to a command.
Legacy command declarations reject duplicate bindings and keep the previous
working registrations if reload fails.

Lua input callbacks run in registration
order until one consumes the event; a later callback can use the same selector
when earlier handlers return forward.

## Commands and shell syntax

Commands run directly. `$HOME`, `~`, pipes and redirection are not expanded.
Use an absolute path, a program on PATH, or explicitly run a shell:

```lua
gnoblin.configure {
    shortcuts = {
        ["log-time"] = {
            binding = "<Super><Shift>t",
            command = {"sh", "-c", "date >> \"$HOME/shortcut.log\""},
        },
    },
}
```

This appends the current time to `~/shortcut.log` when you press Super+Shift+T.
The declarative form is retained for configurations that do not need a Lua
callback.

## Popups that capture typing

For a shell popup that needs keys typed after bare Super, configure a named
shortcut with `capture_input = true`, `binding = "Super"`, and
`trigger = "release"`. Handle `gnoblin.shortcut.session.key` events in Lua.
Only one command-free, action-free bare Super capture binding can be active at
a time. See the [shortcut configuration reference](/config/configure/shortcuts)
and [shortcut state and capture](/config/runtime-api#shortcut-state-and-capture)
for the event fields and session behavior.

See also [restore-or-minimise bindings](/guides/window_state_shortcuts).
