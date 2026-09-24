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

See the [shortcuts guide](/guides/shortcuts) for key names, conflicts and command behavior. The older [`gnoblin.shortcut`](/config/shortcut) declaration form is also available.

The bundled config supplies these names:

| Keys       | Shortcut names                                               |
| ---------- | ------------------------------------------------------------ |
| Volume     | `volume-up`, `volume-down`, `volume-mute`, `microphone-mute` |
| Brightness | `brightness-up`, `brightness-down`                           |
| Playback   | `media-play-pause`, `media-next`, `media-previous`           |
| Files      | `files`                                                      |

To disable one after loading the bundled config:

```lua
gnoblin.configure.shortcuts["volume-up"].enable = false
```
