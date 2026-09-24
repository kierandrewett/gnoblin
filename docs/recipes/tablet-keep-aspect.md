# Keep tablet drawing proportions

Use absolute mapping and preserve the tablet's aspect ratio. Replace the
example ID with the tablet's vendor and product IDs:

```lua
gnoblin.configure {
    input = {
        tablets = {
            ["1234:5678"] = {
                mapping = "absolute",
                keep_aspect = true,
            },
        },
    },
}
```

Absolute mapping ties pen position to a fixed tablet area. Keeping the aspect
ratio avoids stretching when the tablet and display have different shapes. See
the [input reference](/config/configure/input).
