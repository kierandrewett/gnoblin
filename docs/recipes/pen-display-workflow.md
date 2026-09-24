# Set up a pen display

Map the tablet to an absolute drawing area, preserve its proportions, and use
the stylus side button to move the pointer between monitors. Replace
`1234:5678` with the vendor and product IDs reported for your tablet.

```lua
gnoblin.configure {
    input = {
        tablets = {
            ["1234:5678"] = {
                mapping = "absolute",
                keep_aspect = true,
            },
        },
        styluses = {
            ["default-1234:5678"] = {
                secondary_button_action = "switch-monitor",
            },
        },
    },
}
```

Absolute mapping ties pen position to the mapped display area. `keep_aspect`
prevents circles from becoming ovals when the tablet and display have different
shapes; it may leave unused space at one edge. The `default-` stylus entry
applies to matching devices without a more specific serial-number mapping.

After reloading, check that the pen reaches the full display and that the
secondary button changes the active monitor. If it selects the wrong display,
check the device ID and monitor arrangement before changing the mapping.

See [input configuration](/config/configure/input) for tablet IDs and supported
stylus actions.
