# gnoblin.configure.shortcuts

Define command shortcuts and change built-in actions by name. Entries merge with the same name; fields you omit keep their previous values. Set `enable = false` to disable an imported shortcut.

```lua
gnoblin.configure {
    shortcuts = {
        terminal = {
            binding = "<Super>Return",
            command = {"ptyxis", "--new-window"},
        },
        screenshot = {
            action = "gnome:shell.show_screenshot_ui",
            binding = {"Print"},
        },
    },
}
```

Use `command` for a program shortcut or `action` for a built-in GNOME or
Mutter action. Built-in actions require a list of bindings; use an empty list
to disable the action. Command shortcuts use one binding string and an argv
array. See the [shortcuts guide](/guides/shortcuts) for key names, conflicts
and command behavior.

Action names use `group.action`. The group selects the GNOME keybinding schema:

| Group         | GSettings schema                       | Example                          |
| ------------- | -------------------------------------- | -------------------------------- |
| `wm`          | `org.gnome.desktop.wm.keybindings`     | `wm.close`                       |
| `gnome:shell` | `org.gnome.shell.keybindings`          | `gnome:shell.show_screenshot_ui` |
| `mutter`      | `org.gnome.mutter.keybindings`         | `mutter.toggle_tiled_left`       |
| `wayland`     | `org.gnome.mutter.wayland.keybindings` | `wayland.restore_shortcuts`      |

The available actions depend on the installed GNOME version. List the keys in
the matching schema to find actions:

```sh
gsettings list-keys org.gnome.desktop.wm.keybindings
gsettings list-keys org.gnome.shell.keybindings
gsettings list-keys org.gnome.mutter.keybindings
gsettings list-keys org.gnome.mutter.wayland.keybindings
```

GSettings prints native keys with hyphens. Use underscores in the action name;
for example, `show-screenshot-ui` becomes
`gnome:shell.show_screenshot_ui`. To read an action's description, run
`gsettings describe SCHEMA KEY`, such as:

```sh
gsettings describe org.gnome.desktop.wm.keybindings close
gsettings describe org.gnome.shell.keybindings show-screenshot-ui
```

These commands list GNOME actions and descriptions; they do not show the
binding currently configured by Gnoblin. Gnoblin applies active bindings from
the Lua config.

Existing configs can continue using the older `keybindings` field. See the
[migration note](/config/configure/keybindings) to convert one to `shortcuts`.

The bundled config supplies these names:

| Keys       | Shortcut names                                               |
| ---------- | ------------------------------------------------------------ |
| Volume     | `volume-up`, `volume-down`, `volume-mute`, `microphone-mute` |
| Brightness | `brightness-up`, `brightness-down`                           |
| Playback   | `media-play-pause`, `media-next`, `media-previous`           |
| Files      | `files`                                                      |
| Built-in   | `screenshot`, `close_window`                                 |

To disable one after loading the bundled config:

```lua
gnoblin.configure.shortcuts["volume-up"].enable = false
```
