# Use a different keyboard layout per window

List the layouts you use and enable per-window source tracking:

```lua
gnoblin.configure {
    input_sources = {
        sources = {
            {type = "xkb", id = "us"},
            {type = "xkb", id = "gb"},
        },
        per_window = true,
    },
}
```

Replace `us` and `gb` with the XKB source IDs you use. The list replaces the
active input sources when the config reloads; include every layout you want to
keep. With `per_window = true`, Gnoblin tracks the active source separately
for each window. See [input sources](/config/configure/input_sources).
