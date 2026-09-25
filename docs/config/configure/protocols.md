# gnoblin.configure.protocols

Enable or disable a Wayland protocol that Gnoblin exposes to clients. Changes
apply at the next login because the compositor registers protocols at startup.

| Setting          | Values  | Default | Effect                                                                  |
| ---------------- | ------- | ------- | ----------------------------------------------------------------------- |
| `protocols.NAME` | Boolean | `true`  | Exposes the named protocol (`true`) or hides it from clients (`false`). |

Use one of the protocol names below as the `NAME` key:

- **Shell surfaces:** `wlr_layer_shell`.
- **Window management:** `ext_foreign_toplevel_list`,
  `wlr_foreign_toplevel_management`, `xdg_decoration`,
  `window_frame_renderer`.
- **Capture and effects:** `wlr_screencopy`, `ext_background_effect_v1`,
  `blur_fade`.
- **Session controls:** `ext_data_control`, `ext_idle_notify`,
  `wlr_gamma_control`, `wlr_output_power_management`.

For example, disable screen capture protocol advertisement:

```lua
gnoblin.configure {protocols = {wlr_screencopy = false}}
```

See the [protocol catalog](/wayland-protocols).
