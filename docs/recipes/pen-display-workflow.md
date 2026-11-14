# Set up a pen display

Map the tablet to an absolute drawing area, preserve its proportions, and use the stylus side button to move the
pointer between monitors. Replace `1234:5678` with the vendor and product IDs reported for your tablet.

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    input: {
        tablets: {
            "1234:5678": {
                mapping: "absolute",
                keepAspect: true,
            },
        },
        styluses: {
            "default-1234:5678": {
                secondaryButtonAction: "switch-monitor",
            },
        },
    },
});
```

The tablet and stylus settings are accepted and checked, and no device reads them: the compositor logs
`not applied: ...` for `input.tablets` and `input.styluses`, so the pen keeps its system behaviour.

Absolute mapping ties pen position to the mapped display area. `keepAspect` prevents circles from becoming ovals when
the tablet and display have different shapes; it may leave unused space at one edge. The `default-` stylus entry
applies to matching devices without a more specific serial-number mapping.

When a setting applies, check that the pen reaches the full display and that the secondary button changes the active
monitor. If it selects the wrong display, check the device ID and monitor arrangement before changing the mapping.

See [input configuration](/config/configure/input) for tablet IDs and supported stylus actions.
