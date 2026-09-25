# gnoblin.animation

Register a named animation for one supported event. Declarations with the same
name merge during a config load. Disable an imported animation by registering
its name with `enable = false`. The [animation guide](/guides/animations)
covers events, presets and keyframe examples.

```lua
gnoblin.animation {
    name = "soft-open",
    event = "open",
    duration = 280,
    ease = "ease-out-cubic",
    origin = "center",
    from = {y = 18, scale = 0.94, opacity = 0},
    to = {y = 0, scale = 1, opacity = 1},
}
```

| Field        | Accepted values                                                                           | Default and effect                                                                                              |
| ------------ | ----------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| `name`       | Required unique name; letters, digits, `_` and `-`; maximum 80 characters                 | Selects this registration from settings and window rules.                                                       |
| `enable`     | Boolean                                                                                   | `true`; set to `false` to disable a registration by name.                                                       |
| `event`      | One event listed in the [animation guide](/guides/animations#register-a-custom-animation) | Selects the transition this animation controls.                                                                 |
| `duration`   | Integer milliseconds from `0` to `10000`                                                  | Uses the event preset's duration when omitted; `0` completes immediately.                                       |
| `ease`       | Named curve or `{type = "cubic-bezier", x1, y1, x2, y2}`                                  | Uses the event preset's curve when omitted. See [curve values](/guides/animations#properties-and-keyframes).    |
| `from`, `to` | Property-value maps supported by the event                                                | Start and end state. Supply these or `keyframes`.                                                               |
| `keyframes`  | Ordered frames from `at = 0` through `at = 1`                                             | Defines intermediate states instead of endpoint interpolation.                                                  |
| `origin`     | Named pivot or normalized `{x, y}` pair                                                   | `"center"`; sets the transform pivot. See accepted [pivot values](/guides/animations#properties-and-keyframes). |
| `target`     | Optional string label                                                                     | `"none"`; displayed by inspection tools.                                                                        |

Provide `from` and/or `to`, or provide `keyframes`. The animation guide lists
all supported events, properties, curves, pivots, and event-specific defaults.
