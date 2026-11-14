# Monitor settings

The `monitors` key sets preferences that apply to every connected monitor.

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    monitors: { globalScale: 2 },
});
```

| Setting         | Values                                      | Default     | Effect                                                     |
| --------------- | ------------------------------------------- | ----------- | ---------------------------------------------------------- |
| `globalScale`   | An integer from 0 through 4, or `"inherit"` | `"inherit"` | Scales every output by this step. `0` follows the system.  |
| `privacyScreen` | `true`, `false`, or `"inherit"`             | `"inherit"` | Requests the privacy screen state. Not applied: see below. |

`globalScale` applies on a reload.

`privacyScreen` is accepted and checked, but no output backend applies it, so it has no effect on hardware. `true`
requests that privacy screens turn on, `false` that they turn off, and `"inherit"` follows the system privacy screen
preference.

A monitor's own mode and scale, by connector name, are the `outputs` key of `defineConfig`. That key is accepted and
checked, and it does not change a screen: see [`style` and `outputs`](/typescript-config#style-and-outputs).
