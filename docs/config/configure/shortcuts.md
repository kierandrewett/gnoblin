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

Action names use `group.action`. Groups are `wm`, `mutter`, `wayland`, and
`gnome:shell` for GNOME Shell actions. Use `gsettings list-keys` with the
matching schema to find action names:

```sh
gsettings list-keys org.gnome.desktop.wm.keybindings
gsettings list-keys org.gnome.shell.keybindings
gsettings list-keys org.gnome.mutter.keybindings
gsettings list-keys org.gnome.mutter.wayland.keybindings
```

The schema order is `wm`, `gnome:shell`, `mutter`, then `wayland`.

For example, `close` in the window-manager schema is `wm.close`; Shell's
`show-screenshot-ui` key is `gnome:shell.show_screenshot_ui`. Gnoblin writes
the configured bindings to its compositor; `gsettings get` does not show the
active Gnoblin override.

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
