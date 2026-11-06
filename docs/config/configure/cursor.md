# gnoblin.configure.cursor

Configure this part of `gnoblin.configure` with the `cursor` key.

Configure the compositor cursor theme, size and image providers in Gnoblin's config:

```lua
gnoblin.configure {
    cursor = {
        theme = "default",
        size = 24,
        providers = { "hyprcursor", "kde-gnome-svg", "xcursor" },
    },
}
```

| Setting     | Accepted value                                           | Default                                        |
| ----------- | -------------------------------------------------------- | ---------------------------------------------- |
| `theme`     | Installed Xcursor or Hyprcursor theme name               | `default`                                      |
| `size`      | Integer from 1 to 256 logical pixels                     | `24`                                           |
| `providers` | Ordered list of `hyprcursor`, `kde-gnome-svg`, `xcursor` | `{ "hyprcursor", "kde-gnome-svg", "xcursor" }` |

`providers` sets the order in which cursor image sources are searched for each
shape. The built-in arrow is always the last resort, so an empty list shows only
the arrow. An unknown name is refused, and the error lists the accepted values.

- `hyprcursor`: compiled Hyprcursor themes.
- `kde-gnome-svg`: `cursors_scalable` SVG themes with a `metadata.json` per shape.
- `xcursor`: Xcursor themes, following `index.theme` inheritance.

Changes apply on config reload. See the [cursor themes guide](/guides/cursors)
for installation paths and theme details.

## Type definition

This is schema pseudocode in Lua table form. `?` marks an optional field.

```lua
gnoblin.configure {
    cursor = {
        theme = string?,
        size = integer?, -- 1–256 logical pixels
        providers = { string?, ... }, -- each: hyprcursor, kde-gnome-svg, xcursor
    },
}
```
