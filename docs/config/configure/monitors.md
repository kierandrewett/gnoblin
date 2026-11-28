# Monitor settings

A monitor's mode, scale, transform, position and power are the `settings` of an `OutputProfile`. A profile selects the
hardware with its match, the first argument in every language, and the shell applies it whenever that output is connected,
so it survives unplugging and hotplug.

::: tabs

```typescript
import { gnoblin } from "gnoblin-core";
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

```python
from gnoblin.std import OutputCollectionAugment, OutputMatch, OutputPosition, OutputProfile, PositionKind, ProfileSettings, OutputTransform

main = OutputProfile(
    OutputMatch(connector="DP-1"),
    settings=ProfileSettings(
        mode="3440x1440@144",
        scale=1.25,
        transform=OutputTransform.NORMAL,
        position=OutputPosition(PositionKind.ABSOLUTE, x=0, y=0),
    ),
)
side = OutputProfile(
    OutputMatch(connector="HDMI-A-1"),
    settings=ProfileSettings(
        mode="preferred",
        position=OutputPosition(PositionKind.RIGHT_OF, profile=main, align="top"),
    ),
)

OutputCollectionAugment.get().profiles.add(main)
OutputCollectionAugment.get().profiles.add(side)
```

```rust
use gnoblin_core::Gnoblin;
use gnoblin_std::{
    OutputMatch, OutputPosition, OutputProfile, OutputProfileOptions, OutputTransform, PositionKind, ProfileSettings,
};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = Gnoblin::get(&g.objects()?)?;
    let objects = g.objects()?.std()?;
    let outputs = model.outputs()?;
    let profiles = outputs.profiles()?;

    let main = OutputProfile::new_with(
        &objects,
        &OutputMatch {
            connector: Some("DP-1".to_owned()),
            ..OutputMatch::default()
        },
        OutputProfileOptions {
            settings: Some(ProfileSettings {
                mode: Some("3440x1440@144".to_owned()),
                scale: Some(1.25),
                transform: Some(OutputTransform::Normal),
                position: Some(OutputPosition {
                    x: Some(0),
                    y: Some(0),
                    ..OutputPosition::new(PositionKind::Absolute)
                }),
                ..ProfileSettings::default()
            }),
            ..OutputProfileOptions::default()
        },
    )?;
    profiles.add(&main)?;

    let side = OutputProfile::new_with(
        &objects,
        &OutputMatch {
            connector: Some("HDMI-A-1".to_owned()),
            ..OutputMatch::default()
        },
        OutputProfileOptions {
            settings: Some(ProfileSettings {
                mode: Some("preferred".to_owned()),
                position: Some(OutputPosition {
                    profile: Some(main),
                    align: Some("top".to_owned()),
                    ..OutputPosition::new(PositionKind::RightOf)
                }),
                ..ProfileSettings::default()
            }),
            ..OutputProfileOptions::default()
        },
    )?;
    profiles.add(&side)?;
    Ok(())
}
```

:::

| Key         | Values                                                                                                              |
| ----------- | ------------------------------------------------------------------------------------------------------------------- |
| match       | Required, the first argument. One or more of `connector`, `make`, `model` and `serial`; every key given must match. |
| `id`        | An option: the profile's name. It defaults to the keys of the match.                                                |
| `settings`  | An option: the rest of this table. Each setting is optional.                                                        |
| `mode`      | In `settings`: `"preferred"` or `"WIDTHxHEIGHT@HERTZ"`, such as `"3440x1440@144"`.                                  |
| `scale`     | In `settings`: a number from 0.5 to 4.                                                                              |
| `transform` | In `settings`: `normal`. The compositor draws no rotation, so the others are refused by name.                       |
| `position`  | In `settings`: an absolute place, or a place against another profile.                                               |
| `enabled`   | In `settings`: `false` switches the output off.                                                                     |
| `vrr`       | In `settings`, accepted and reported as not applied: the backends have no variable refresh rate.                    |

The `position` takes one of two forms:

- An absolute place takes `x` and `y`, whole numbers for the top-left corner.
- A relative place has a `kind` of `right_of`, `left_of`, `above` or `below`, and the `profile` it sits against. `align`
  places it along the shared edge: `top`, `center` or `bottom` beside another output, and `left`, `center` or `right`
  above or below one.

A mode change on a hardware (DRM) output is off until the session starts with `GNOBLIN_DRM_MODESET=1`; see
[what each backend applies](/shell-api/outputs#what-each-backend-applies).

To change a connected output once, without keeping a profile, call `apply` on it, and to mirror or span outputs build a
`Display`. Both are in [outputs](/shell-api/outputs#configure-an-output).

The global scale step and the privacy screen (`monitors`) have no home in a shell yet: see
[settings with no object yet](/js-settings#dropped-settings).
