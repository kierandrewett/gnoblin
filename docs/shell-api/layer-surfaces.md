# Layer surfaces

`gnoblin.layers.list(filter?)` returns read-only records from the latest native
layer-surface snapshot. Each record includes its state revision. Layer records
have no mutating methods; shell clients continue to own their surfaces.

The optional filter accepts exact string matches for `monitor_id`, `namespace`,
and `layer`. Unknown fields and non-string values raise a Lua error. The read
is available only in the native Mutter runtime and raises a Lua error while its
snapshot is unavailable.

Shells that also use the control socket can find its request format in the
[compositor bridge](/compositor-bridge).

Layer records expose these fields:

| Field                              | Type and values                                                                |
| ---------------------------------- | ------------------------------------------------------------------------------ |
| `id`                               | Stable string ID.                                                              |
| `title`, `namespace`, `monitor_id` | Optional strings.                                                              |
| `layer`                            | `"background"`, `"bottom"`, `"top"`, or `"overlay"`.                           |
| `keyboard_interactive`             | `"none"`, `"on_demand"`, or `"exclusive"`.                                     |
| `exclusive_zone`                   | Integer.                                                                       |
| `anchor`                           | Array containing zero or more of `"top"`, `"bottom"`, `"left"`, and `"right"`. |
| `geometry`                         | Rectangle.                                                                     |
| `mapped`                           | Boolean.                                                                       |
| `revision`                         | Integer state revision.                                                        |

Mutter currently omits `app_id` because layer-shell does not provide one.

The `layer` field controls stacking order:

- `background` sits behind desktop content.
- `bottom` sits above background surfaces.
- `top` sits above normal windows.
- `overlay` sits above the other layer levels.

`keyboard_interactive` controls keyboard focus:

- `none` does not request keyboard focus.
- `on_demand` requests focus when the surface needs keyboard input.
- `exclusive` keeps keyboard focus while the surface is mapped.

`gnoblin.layers.animation_policy(namespace)` returns the effective enter and
exit phases and `window_shadow` for a layer namespace. The namespace must be 1
to 128 UTF-8 bytes. Without an animation override, each phase uses its first enabled event
registration, or `none` if no event is registered.

`window_shadow` defaults to `false` unless a matching default-window rule
supplies a shadow value. The method raises a Lua error if the committed policy
cannot be read.

`gnoblin.capabilities.list()` returns read-only native compositor and protocol
capability records. It takes no arguments and has no filters or mutators. The
read raises a Lua error if its snapshot is unavailable.

Capability records expose these fields:

- `id`: the capability name.
- `description`: what the capability provides.
- `available`: whether its current requirements are met.
- `revision`: the compositor state revision.
- `reason`: the cause when the capability is unavailable; omitted otherwise.

The standalone runtime advertises these optional capabilities:

- `window-thumbnails`: bounded window previews.
- `session-activity`: native idle-monitor state.
- `microphone-monitor`: PipeWire microphone activity monitoring.
- `camera-monitor`: PipeWire camera activity monitoring.

`microphone-monitor` and `camera-monitor` are available only when this Mutter
build includes remote-desktop support and the PipeWire monitor is connected.
When unavailable, `reason` is one of:

- `remote_desktop_disabled`: the Mutter build lacks remote-desktop support.
- `pipewire_unavailable`: the monitor cannot connect.

Camera activity follows running PipeWire nodes whose media role is `Camera`.
The activity remains available for 500 ms after the last camera node stops to
avoid flickering.

Subscribe to `gnoblin.capability.changed` to receive the updated record when
availability changes. Check the capability snapshot before using an optional
capability.
