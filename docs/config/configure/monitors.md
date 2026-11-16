# Monitor settings

A monitor's mode, scale, transform, position and power are an `OutputProfile`. A profile selects the hardware it is for
with `match`, and the shell applies it whenever that output is connected, so it survives unplugging and hotplug.

```ts
import { OutputProfile } from "gnoblin-std";

const main = new OutputProfile({
    match: { connector: "DP-1" },
    mode: { width: 3440, height: 1440, refreshRate: 144 },
    scale: 1.25,
    transform: "normal",
    position: { x: 0, y: 0 },
});
const side = new OutputProfile({
    match: { connector: "HDMI-A-1" },
    mode: "preferred",
    position: { rightOf: main, align: "top" },
});

gnoblin.outputs.profiles.add(main);
gnoblin.outputs.profiles.add(side);
```

| Option      | Values                                                                                             |
| ----------- | -------------------------------------------------------------------------------------------------- |
| `match`     | Required. One or more of `connector`, `make`, `model` and `serial`; every key given must match.    |
| `mode`      | `"preferred"`, `"3440x1440@144"` or `{ width, height, refreshRate }`.                              |
| `scale`     | A number from 0.5 to 4.                                                                            |
| `transform` | `"normal"`. The compositor draws no rotation, so the others are refused by name.                   |
| `position`  | `{ x, y }`, or `{ rightOf, leftOf, above, below: profile, align }` to sit against another profile. |
| `enabled`   | `false` switches the output off.                                                                   |
| `vrr`       | Accepted and reported as not applied: the backends have no variable refresh rate.                  |

A mode change on a hardware (DRM) output is off until the session starts with `GNOBLIN_DRM_MODESET=1`; see
[what each backend applies](/shell-api/outputs#what-each-backend-applies).

To change a connected output once, without keeping a profile, call `output.apply({ ... })` on it, and to mirror or
span outputs build a `Display`. Both are in [outputs](/shell-api/outputs#configure-an-output).

The global scale step and the privacy screen (`monitors`) have no TypeScript home yet: see
[settings with no TypeScript home yet](/js-settings#dropped-settings).
