# gnoblin.configure.cursor

Configure this part of `gnoblin.configure` with the `cursor` key.

Configure the compositor cursor theme and size in Gnoblin's config:

```lua
gnoblin.configure {
    cursor = {
        theme = "Adwaita-Hyprcursor",
        size = 24,
    },
}
```

| Setting | Accepted value                       | Default              |
| ------- | ------------------------------------ | -------------------- |
| `theme` | Installed Hyprcursor theme name      | `Adwaita-Hyprcursor` |
| `size`  | Integer from 1 to 256 logical pixels | `24`                 |

Changes apply on config reload. See the [cursor themes guide](/guides/cursors)
for installation paths and theme details.
