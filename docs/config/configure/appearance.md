# gnoblin.configure.appearance

Configure this part of `gnoblin.configure` with the `appearance` key.

Set the colour scheme and accent colour that other toolkits are told about:

```lua
gnoblin.configure {
    appearance = {
        ["color-scheme"] = "prefer-dark",
        accent = "blue",
    },
}
```

| Setting        | Accepted value                           | Default   |
| -------------- | ---------------------------------------- | --------- |
| `color-scheme` | `default`, `prefer-dark`, `prefer-light` | `default` |
| `accent`       | A named accent, `#rgb` or `#rrggbb`      | Unset     |

`color-scheme` of `default` states no preference, so each application keeps its
own choice. `prefer-dark` and `prefer-light` ask applications for that scheme.

`accent` is one of the nine named accents:

- `blue`, `teal`, `green`, `yellow`, `orange`
- `red`, `pink`, `purple`, `slate`

A hex colour is accepted without an alpha channel.
GTK takes only the named accents, so a hex colour is published as the nearest
named accent. When `accent` is unset, toolkits keep their own accent.

## Type definition

This is schema pseudocode in Lua table form. `?` marks an optional field.

```lua
gnoblin.configure {
    appearance = {
        ["color-scheme"] = "default" | "prefer-dark" | "prefer-light"?,
        accent = string?, -- a named accent, #rgb or #rrggbb
    },
}
```
