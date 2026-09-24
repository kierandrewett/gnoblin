# gnoblin.configure.input_sources

Set the active input sources with `gnoblin.configure {input_sources = {...}}`. This replaces the
active source list in memory on reload; removing the table restores GNOME's
session sources. `sources` is required and contains records with `type` set to
`"xkb"` or `"ibus"` and a nonempty `id`. Set `per_window` to `true` to track
the active source per window; it defaults to `false`.

```lua
gnoblin.configure {
    input_sources = {
        sources = {
            {type = "xkb", id = "us"},
            {type = "xkb", id = "gb"},
        },
        per_window = false,
    },
}
```
