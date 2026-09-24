# gnoblin.configure.protocols

Enable or disable Wayland protocols exposed by Gnoblin. Changes apply at the next login.

| Setting          | Value   | Default | Applies    |
| ---------------- | ------- | ------- | ---------- |
| `protocols.NAME` | Boolean | `true`  | Next login |

Protocol names: `wlr_layer_shell`, `wlr_screencopy`, `ext_foreign_toplevel_list`,
`wlr_foreign_toplevel_management`, `ext_data_control`, `ext_idle_notify`,
`wlr_gamma_control`, `wlr_output_power_management`, `ext_background_effect_v1`,
`xdg_decoration`, `window_frame_renderer`, `blur_fade`, `ext_session_lock`.

See the [protocol catalog](/wayland-protocols).
