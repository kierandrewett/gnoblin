# gnoblin.animation

Register a named animation for one compositor event. The first registration
for an event is its default; a shell setting or window rule can select another
registered name. A later declaration with the same name updates the earlier
one. Each name must be unique in the assembled configuration.

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

## Fields

| Field        | Required    | Values                                                 |
| ------------ | ----------- | ------------------------------------------------------ |
| `name`       | Yes         | 1–80 letters, digits, `_` or `-`                       |
| `event`      | Yes         | One supported event below                              |
| `duration`   | No          | Integer milliseconds, 0–10000                          |
| `ease`       | No          | Easing name or cubic Bézier table                      |
| `from`, `to` | One or both | Start and end property maps                            |
| `keyframes`  | Alternative | 2–128 ordered frames from `at = 0` to `at = 1`         |
| `origin`     | No          | Named pivot or normalized two-number array from 0 to 1 |
| `target`     | No          | Label of up to 80 letters, digits, `_` or `-`          |

An animation needs `from`, `to`, or `keyframes`. Each keyframe has an `at`
position and one or more properties; its optional `ease` controls the segment
ending at that frame. Supported properties depend on the event: window and
layer transitions use `x`, `y`, `scale`, `scale_x`, `scale_y`, `rotation` and
`opacity`; tile previews use `x`, `y`, `width`, `height` and `opacity`; scalar
transitions such as workspace switching and shadow changes use `progress`.

Events are `minimize`, `restore`, `open`, `close`, `dialog-open`,
`dialog-close`, `layer-open`, `layer-close`, `workspace-switch`, `console-open`,
`console-close`, `shadow-change`, `layer-companion-close`, `resize`,
`tile-preview-open`, `tile-preview-close`, `dialog-dim` and `dialog-undim`.

Names may be used in [`gnoblin.configure.shell`](/config/configure/shell) and
[`gnoblin.window_rule`](/config/window_rule). Shell settings that cover two
events need a map if the selected custom animations are event-specific. See
the [animation guide](/guides/animations) for built-in names, practical
examples, easing choices and preview commands.
