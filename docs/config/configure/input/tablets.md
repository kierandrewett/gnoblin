# gnoblin.configure.input.tablets

Set per-tablet overrides under a vendor and product ID in
`gnoblin.configure.input.tablets`:

```lua
gnoblin.configure {
    input = {
        tablets = {
            ["1234:5678"] = {
                mapping = "absolute",
                left_handed = false,
                keep_aspect = true,
            },
        },
    },
}
```

The device ID uses four hexadecimal digits for the vendor and product, joined
by a colon. Each override accepts:

| Field                        | Values                       |
| ---------------------------- | ---------------------------- |
| `mapping`                    | `"absolute"` or `"relative"` |
| `left_handed`, `keep_aspect` | Boolean                      |

Only devices listed in this table are overridden. Unlisted tablets keep their
current system settings.
