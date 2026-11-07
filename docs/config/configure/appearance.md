# gnoblin.configure.appearance

Configure this part of `gnoblin.configure` with the `appearance` key.

Set the colour scheme, accent colour and accessibility preferences that other
toolkits are told about:

```lua
gnoblin.configure {
    appearance = {
        ["color-scheme"] = "prefer-dark",
        accent = "blue",
        ["reduced-motion"] = true,
        ["high-contrast"] = true,
    },
}
```

| Setting          | Accepted value                           | Default   |
| ---------------- | ---------------------------------------- | --------- |
| `color-scheme`   | `default`, `prefer-dark`, `prefer-light` | `default` |
| `accent`         | A named accent, `#rgb` or `#rrggbb`      | Unset     |
| `reduced-motion` | A boolean                                | `false`   |
| `high-contrast`  | A boolean                                | `false`   |

`color-scheme` of `default` states no preference, so each application keeps its
own choice. `prefer-dark` and `prefer-light` ask applications for that scheme.

`accent` is one of the nine named accents:

- `blue`, `teal`, `green`, `yellow`, `orange`
- `red`, `pink`, `purple`, `slate`

A hex colour is accepted without an alpha channel.
GTK takes only the named accents, so a hex colour is published as the nearest
named accent. When `accent` is unset, toolkits keep their own accent.

`reduced-motion = true` asks applications to stop animating. The compositor also
suppresses its own animations, so window changes land at once. `false` states
no preference.

`high-contrast = true` asks applications for a high-contrast appearance.
`false` states no preference.

Both preferences reach applications through the Settings portal and the dconf
layer that GTK reads. The keys each consumer sees are listed in the
[portals reference](/config/configure/portals#settings-interface). Qt and
KDE applications do not read them unless they have their own bridge.

## Type definition

This is schema pseudocode in Lua table form. `?` marks an optional field.

```lua
gnoblin.configure {
    appearance = {
        ["color-scheme"] = "default" | "prefer-dark" | "prefer-light"?,
        accent = string?, -- a named accent, #rgb or #rrggbb
        ["reduced-motion"] = boolean?,
        ["high-contrast"] = boolean?,
    },
}
```
