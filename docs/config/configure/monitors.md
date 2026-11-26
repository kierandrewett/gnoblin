# Monitor settings

A monitor's mode, scale, transform, position and power are the `settings` of an `OutputProfile`. A profile selects the
hardware with its match, the first argument, and the shell applies it whenever that output is connected, so it survives
unplugging and hotplug.

```ts
import { OutputProfile } from "gnoblin-std";

const main = new OutputProfile(
    { connector: "DP-1" },
    {
        settings: {
            mode: "3440x1440@144",
            scale: 1.25,
            transform: "normal",
            position: { kind: "absolute", x: 0, y: 0 },
        },
    },
);
const side = new OutputProfile(
    { connector: "HDMI-A-1" },
    { settings: { mode: "preferred", position: { kind: "right_of", profile: main, align: "top" } } },
);

gnoblin.outputs.profiles.add(main);
gnoblin.outputs.profiles.add(side);
```

| Key         | Values                                                                                                              |
| ----------- | ------------------------------------------------------------------------------------------------------------------- |
| match       | Required, the first argument. One or more of `connector`, `make`, `model` and `serial`; every key given must match. |
| `id`        | In the options: the profile's name. It defaults to the keys of the match.                                           |
| `settings`  | In the options: an object holding the rest of this table. Each setting is optional.                                 |
| `mode`      | In `settings`: `"preferred"` or `"WIDTHxHEIGHT@HERTZ"`, such as `"3440x1440@144"`.                                  |
| `scale`     | In `settings`: a number from 0.5 to 4.                                                                              |
| `transform` | In `settings`: `"normal"`. The compositor draws no rotation, so the others are refused by name.                     |
| `position`  | In `settings`: `{ kind: "absolute", x, y }`, or `{ kind, profile, align }` to sit against another profile.          |
| `enabled`   | In `settings`: `false` switches the output off.                                                                     |
| `vrr`       | In `settings`, accepted and reported as not applied: the backends have no variable refresh rate.                    |

A relative `position` has a `kind` of `"right_of"`, `"left_of"`, `"above"` or `"below"`, and the `profile` it sits
against. `align` places it along the shared edge: `"top"`, `"center"` or `"bottom"` beside another output, and `"left"`,
`"center"` or `"right"` above or below one.

A mode change on a hardware (DRM) output is off until the session starts with `GNOBLIN_DRM_MODESET=1`; see
[what each backend applies](/shell-api/outputs#what-each-backend-applies).

To change a connected output once, without keeping a profile, call `output.apply({ ... })` on it, and to mirror or
span outputs build a `Display`. Both are in [outputs](/shell-api/outputs#configure-an-output).

The global scale step and the privacy screen (`monitors`) have no TypeScript home yet: see
[settings with no TypeScript home yet](/js-settings#dropped-settings).
