# Make the cursor larger

Set a larger compositor cursor using the installed Adwaita Hyprcursor theme:

```lua
gnoblin.configure {
    cursor = {
        theme = "Adwaita-Hyprcursor",
        size = 32,
    },
}
```

Cursor size is in logical pixels and accepts integers from 1 to 256. Install a
theme before selecting it by name. See the [cursor guide](/guides/cursors) for
theme installation.
