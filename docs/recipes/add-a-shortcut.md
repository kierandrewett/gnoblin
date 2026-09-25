# Add a shortcut

Add a named command shortcut without replacing the rest of the list:

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

| Field     | Accepted values            | Effect                                                                                                     |
| --------- | -------------------------- | ---------------------------------------------------------------------------------------------------------- |
| `binding` | One GTK accelerator string | Runs the command when the key is pressed.                                                                  |
| `command` | Array of strings           | Starts the first item as the program and passes the remaining items as arguments, without shell expansion. |

Changes apply on config reload. The [shortcut reference](/config/configure/shortcuts)
documents built-in actions and other fields.

To disable an imported shortcut:

```lua
gnoblin.configure {
    shortcuts = {terminal = {enable = false}},
}
```

Use one array item per program argument. See the [shortcut reference](/config/configure/shortcuts)
for built-in actions and [key names and conflict handling](/guides/shortcuts).
