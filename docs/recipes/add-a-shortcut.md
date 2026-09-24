# Add a shortcut

Add a named shortcut without replacing the rest of the shortcut list:

```lua
gnoblin.configure {shortcuts = {my_terminal = {binding = "<Super>Return", command = {"ptyxis", "--new-window"}}}}
```

Use an installed terminal. To change an imported shortcut, use its existing
name. Only fields you supply change, so this keeps its current binding:

```lua
gnoblin.configure {
    shortcuts = {
        terminal = {command = {"ptyxis", "--new-window"}},
    },
}
```

To disable an imported shortcut:

```lua
gnoblin.configure {
    shortcuts = {terminal = {enable = false}},
}
```

See the [shortcuts guide](/guides/shortcuts) for key names and conflict handling.
