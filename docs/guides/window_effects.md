# window_effects

This page says which window effects work today and which do not. The source of
truth is the loader, not this page: a window rule that names an unsupported key
is refused at load with a reason. The list of refused keys is in the
[window rule reference](/config/window_rule).

## What works now

| Effect                 | Where it works                                  | Set from config?                  |
| ---------------------- | ----------------------------------------------- | --------------------------------- |
| Opacity, scale, offset | Animated tweens and open/close fades            | Not as a window rule (refused)    |
| Rounded corners        | Software, GLES and Vulkan renderers (see below) | No. No config path sets a radius  |
| Tint                   | Software, GLES and Vulkan renderers             | No window rule key                |
| Blur behind a window   | GLES renderer only                              | No. Refused as `blur`             |
| Shadow                 | GLES renderer only                              | No. Refused as `corners.shadow`   |
| User fragment shaders  | GLES renderer mechanism only                    | No. Refused as `shader`           |
| Native window frames   | Frame protocol, any renderer                    | Via `frame-renderers`, not a rule |

"No config path" means the renderer can draw the effect, but nothing a user writes
in `init.lua` reaches it. Rounded corners are drawn by the software renderer
(clipped to the radius) and by GLES and Vulkan, but no production code sets a
radius. The full per-renderer status, with test names, is in
[`design/visuals-status.md`](../../design/visuals-status.md).

## Window rules that do not apply

These keys are refused when the config loads. Remove them from your rules; they
have no effect today:

- `blur`, `blur-ignore-shadows`
- `opacity`
- `corners` (radius, smoothing, mode, `remove_csd`, border, shadow, and the
  rest of the table)
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
None of this is configurable yet. Shaped windows are not implemented.
