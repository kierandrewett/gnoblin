# gnoblin.configure.shortcuts

Define and edit command shortcuts, including media keys, by name. Entries merge with the same name; fields you omit keep their previous values. Set `enable = false` to disable an imported shortcut.

```lua
gnoblin.configure {
    shortcuts = {
        terminal = {
            binding = "<Super>Return",
            command = {"ptyxis", "--new-window"},
        },
    },
}
```

See the [shortcuts guide](/guides/shortcuts) for key names, conflicts and command behavior.
The native compositor preview accepts command shortcuts at startup. It
supports release triggers and bare Super on release.

It also applies named actions from the `wm`, `mutter`, and `wayland` groups.

![Fuzzel searching for Firefox on a clean Waybar desktop, with the pointer visible](../../images/gnoblin-waybar-launcher.png)

_The Gnoblin shortcut opens Fuzzel; Firefox is the selected result._

## Run a built-in action

An `action` uses the `group.key` form for a built-in Mutter action. Available
names depend on the Mutter version in your Gnoblin build.

Accepted groups are `wm`, `mutter`, and `wayland`.

List actions with `gnoblinctl shortcut actions` while a Gnoblin session is
running, or read them from Lua with `gnoblin.shortcuts.actions(group?)`.

| Field     | Accepted values                                           | Meaning                                                                       |
| --------- | --------------------------------------------------------- | ----------------------------------------------------------------------------- |
| `action`  | `"wm.KEY"`, `"mutter.KEY"`, or `"wayland.KEY"`            | Selects a built-in action from that schema. Use underscores in Lua key names. |
| `binding` | GTK accelerator string for a command; array for an action | Required. An empty action list disables its current binding.                  |
| `command` | Nonempty array of strings                                 | Alternative to `action`; runs the program directly without shell expansion.   |
| `trigger` | `"press"` or `"release"`                                  | `"press"` by default; selects which key edge launches a command.              |

Input capture is available to transient runtime bindings, not named config
entries. See [shortcut state and capture](/config/runtime-api#shortcut-state-and-capture).

Set exactly one of `action` or `command`:

```lua
gnoblin.configure {
    shortcuts = {
        close_window = {
            action = "wm.close",
            binding = {"<Super>q"},
        },
    },
}
```

This binds Mutter's `close` action. See the
[keybinding reference](/config/configure/keybindings) for group names and how
to list available actions.

In the standalone session, `wm`, `mutter`, and `wayland` actions are applied by
Mutter at startup.

The named view lets later files inspect and edit imported shortcuts. Each
entry exposes public `snake_case` fields. `pairs` visits the names already
loaded:

```lua
for name, shortcut in pairs(gnoblin.configure.shortcuts) do
    print(name, shortcut.binding or "built-in action")
end
```

The bundled config supplies these names:

| Keys     | Shortcut names                                               |
| -------- | ------------------------------------------------------------ |
| Volume   | `volume-up`, `volume-down`, `volume-mute`, `microphone-mute` |
| Playback | `media-play-pause`, `media-next`, `media-previous`           |
| Files    | `files`                                                      |

The standalone session does not assign brightness keys by default. Bind them
to a command such as `brightnessctl` with
[`gnoblin.configure.shortcuts`](/config/configure/shortcuts).

To disable one after loading the bundled config:

```lua
gnoblin.configure.shortcuts["volume-up"].enable = false
```

Run `gnoblinctl shortcut capture` to print the GTK accelerator for a key combination. Escape cancels; capture times out after 30 seconds by default. See the [shortcuts guide](/guides/shortcuts) for timeout options and behavior.

## Type definition

Every named entry uses exactly one of `command` or `action`. `?` marks
optional fields; `|` separates alternatives.

```lua
gnoblin.configure {
    shortcuts = {
        ["shortcut-name"] = {
            binding = string | {string, ...} | {},
            command = {string, ...}?,
            action = string | {schema = string, key = string}?,
            trigger = "press" | "release"?,
            enable = boolean?,
        }, ...,
    },
}
```
