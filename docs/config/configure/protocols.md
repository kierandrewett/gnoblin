# gnoblin.configure.protocols

Enable or disable a Wayland protocol that Gnoblin exposes to clients. Changes
apply after `gnoblinctl config reload`. Disabling a protocol stops new clients
from discovering it; clients already bound to it keep their existing object
until they disconnect.

| Setting          | Values  | Default                       | Effect                                                                  |
| ---------------- | ------- | ----------------------------- | ----------------------------------------------------------------------- |
| `protocols.NAME` | Boolean | `true` in the Gnoblin session | Exposes the named protocol (`true`) or hides it from clients (`false`). |

Use one of the protocol names below as the `NAME` key:

- **Shell surfaces:** `wlr_layer_shell`.
- **Window management:** `ext_foreign_toplevel_list`,
  `wlr_foreign_toplevel_management`, `xdg_decoration`,
  `kde_server_decoration`, `window_frame_renderer`.
- **Capture and effects:** `wlr_screencopy`, `ext_background_effect_v1`,
  `blur_fade`.
- **Session controls:** `ext_data_control`, `ext_idle_notify`,
  `wlr_gamma_control`, `wlr_output_power_management`, `ext_session_lock`.

For example, disable screen capture protocol advertisement:

```lua
gnoblin.configure {protocols = {wlr_screencopy = false}}
```

See the [protocol catalog](/wayland-protocols).

A name that is not in the list above does nothing. The reload still succeeds, and
Gnoblin writes `gnoblin.configure: unknown protocol "name" is ignored` to the
session log.

## Type definition

Protocol fields are optional; list only the protocols you want to override.

```lua
gnoblin.configure {
    protocols = {
        wlr_layer_shell = boolean?,
        ext_foreign_toplevel_list = boolean?,
        wlr_foreign_toplevel_management = boolean?,
        xdg_decoration = boolean?,
        kde_server_decoration = boolean?,
        window_frame_renderer = boolean?,
        wlr_screencopy = boolean?,
        ext_background_effect_v1 = boolean?,
        blur_fade = boolean?,
        ext_data_control = boolean?,
        ext_idle_notify = boolean?,
        wlr_gamma_control = boolean?,
        wlr_output_power_management = boolean?,
        ext_session_lock = boolean?,
    },
}
```
