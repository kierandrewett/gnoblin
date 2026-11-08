# window_effects

This page says which window effects work today and which do not. The source of
truth is the loader, not this page: a window rule that names an unsupported key
is refused at load with a reason. The list of refused keys is in the
[window rule reference](/config/window_rule).

## What works now

| Effect                 | Where it works                                  | Set from config?                       |
| ---------------------- | ----------------------------------------------- | -------------------------------------- |
| Opacity, scale, offset | Animated tweens and open/close fades            | `opacity` rule: yes. Scale, offset: no |
| Rounded corners        | Software, GLES and Vulkan renderers (see below) | `corners = {radius = n}` rule: yes     |
| Tint                   | Software, GLES and Vulkan renderers             | No window rule key                     |
| Blur behind a window   | GLES renderer only                              | No. Refused as `blur`                  |
| Shadow                 | GLES renderer only                              | No. Refused as `corners.shadow`        |
| User fragment shaders  | GLES renderer mechanism only                    | No. Refused as `shader`                |
| Native window frames   | Frame protocol, any renderer                    | Via `frame-renderers`, not a rule      |

"No config path" means the renderer can draw the effect, but nothing a user writes
in `init.lua` reaches it. Opacity and corner radius from a window rule reach the
window's compositor node. Rounded corners are drawn by the software renderer
(clipped to the radius) and by GLES and Vulkan. The full per-renderer status, with
test names, is in [`design/visuals-status.md`](../../design/visuals-status.md).

## Window rules that do not apply

These keys are refused when the config loads. Remove them from your rules; they
have no effect today:

- `blur`, `blur-ignore-shadows`
- `corners` keys other than `radius`, such as `corners.shadow` and `smoothing`
- `shader`, `shader-uniforms`
- `animation`
- `frame`
- `match.layer`

A rule that names one of them is refused, and the config reports an error at load.
The reason for each key is in the [refused table](/config/window_rule).

## Background blur from applications {#blur-and-opacity}

Applications and shells can ask for blur behind a region they draw, through the
`ext_background_effect` protocol. That path is separate from window rules and is
described in [background effects](/background-effects).

## Planned

The design in [`design/rust-compositor.md`](../../design/rust-compositor.md)
(D24 and D26) plans an ordered per-window effect stack, where blur follows the
window's opacity and shadow, border and blur follow a window's alpha silhouette.
Blur, shadow, border and shader are not configurable yet. Shaped windows are not
implemented.
