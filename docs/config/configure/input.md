# gnoblin.configure.input

Configure this part of `gnoblin.configure` with the `input` key.

Input settings go in `gnoblin.configure {input = {...}}` and apply on reload.
Each group is optional. Fields you omit continue to use the corresponding
GNOME/Mutter setting. Global settings use `mouse`, `touchpad`, and `keyboard`;
tablet and stylus overrides are selected by device identifier.

| Group      | Fields                                                                                                                                                  | Values                                        |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------- |
| `mouse`    | `speed`                                                                                                                                                 | Number from -1 to 1                           |
|            | `left_handed`, `natural_scroll`                                                                                                                         | Boolean                                       |
|            | `accel_profile`                                                                                                                                         | `"default"`, `"flat"`, `"adaptive"`           |
| `touchpad` | `speed`                                                                                                                                                 | Number from -1 to 1                           |
|            | `left_handed`                                                                                                                                           | `"right"`, `"left"`, `"mouse"`                |
|            | `natural_scroll`, `tap_to_click`, `tap_and_drag`, `tap_and_drag_lock`, `disable_while_typing`, `edge_scrolling_enabled`, `two_finger_scrolling_enabled` | Boolean                                       |
|            | `accel_profile`                                                                                                                                         | `"default"`, `"flat"`, `"adaptive"`           |
|            | `tap_button_map`                                                                                                                                        | `"default"`, `"lrm"`, `"lmr"`                 |
|            | `click_method`                                                                                                                                          | `"default"`, `"none"`, `"areas"`, `"fingers"` |
| `keyboard` | `repeat`, `remember_numlock_state`, `numlock_state`                                                                                                     | Boolean                                       |
|            | `delay`, `repeat_interval`                                                                                                                              | 1–10000 ms                                    |
|            | `xkb_options`                                                                                                                                           | Array of XKB option strings                   |

`numlock_state` is kept in memory while Gnoblin's config is active. Tablet keys
use four-hex-digit vendor and product IDs such as `"1234:5678"`; their fields
are `mapping` (`"absolute"` or `"relative"`), `left_handed`, and `keep_aspect`.
Stylus keys use a device serial or `"default-1234:5678"`. Stylus fields are
`button_action`, `secondary_button_action`, and `tertiary_button_action`
(`"default"`, `"middle"`, `"right"`, `"back"`, `"forward"`,
`"switch-monitor"`, or `"keybinding"`), plus the matching
`*_button_keybinding` string fields.

Orientation lock uses the boolean `input.orientation_lock` field. Removing it
restores GNOME's orientation-lock setting.
