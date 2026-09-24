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

`theme` is an installed cursor theme name. Gnoblin currently renders it with
Hyprcursor. `size` is an integer from 1 to 256 logical pixels (default `24`).
Changes apply on config reload. Guide: [cursor themes](/guides/cursors).
