# gnoblin.configure.shortcuts

Define and edit command shortcuts by name. Entries merge with the same name; fields you omit keep their previous values. Set `enable = false` to disable an imported shortcut.

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
