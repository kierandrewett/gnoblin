# gnoblin.configure.window_management

Configure this part of `gnoblin.configure` with the `window_management` key.

Inside `gnoblin.configure {window_management = {...}}`. Values apply on reload
and omitted fields return to the Gnoblin defaults below.

| Key                                                               | Values                                                | Default                  |
| ----------------------------------------------------------------- | ----------------------------------------------------- | ------------------------ |
| `focus_mode`                                                      | `"click"`, `"sloppy"`, `"mouse"`                      | `"click"`                |
| `focus_new_windows`                                               | `"smart"`, `"strict"`                                 | `"smart"`                |
| `raise_on_click`, `auto_raise`, `focus_change_on_pointer_rest`    | Boolean                                               | `true`, `false`, `false` |
| `auto_raise_delay`                                                | 0–10000 ms                                            | `500`                    |
| `action_double_click_titlebar`                                    | Titlebar action below                                 | `"toggle-maximize"`      |
| `action_middle_click_titlebar`                                    | Titlebar action below                                 | `"lower"`                |
| `action_right_click_titlebar`                                     | Titlebar action below                                 | `"menu"`                 |
| `dynamic_workspaces`, `workspaces_only_on_primary`, `edge_tiling` | Boolean                                               | `false`                  |
| `num_workspaces`                                                  | 1–36; used when dynamic workspaces are off            | `4`                      |
| `workspace_names`                                                 | Array of up to 36 strings, each at most 80 characters | `{}`                     |
| `center_new_windows`, `attach_modal_dialogs`                      | Boolean                                               | `false`                  |
| `constrain_drag_to_work_area`                                     | Boolean                                               | `true`                   |

Titlebar actions: `toggle-maximize`, `toggle-maximize-horizontally`,
`toggle-maximize-vertically`, `minimize`, `lower`, `menu`, `none`.
GTK apps usually draw their own titlebars and read GNOME's window-manager
settings directly; this Lua section does not change those app settings. It
controls Mutter policy and Gnoblin's built-in fallback SSD, which implements
these actions. A custom SSD renderer must implement its own titlebar click
behavior.
See [titlebars](/guides/window_frames).
