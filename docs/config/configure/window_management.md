# gnoblin.configure.window_management

Configure this part of `gnoblin.configure` with the `window_management` key.

Put these fields inside `gnoblin.configure {window_management = {...}}`.
Changes apply on configuration reload. Omitted fields use the defaults below.

| Key                            | Values                                       | Default             | Effect                                                      |
| ------------------------------ | -------------------------------------------- | ------------------- | ----------------------------------------------------------- |
| `focus_mode`                   | `"click"`, `"sloppy"`, `"mouse"`             | `"click"`           | Focus on click, pointer entry, or pointer movement.         |
| `focus_new_windows`            | `"smart"`, `"strict"`                        | `"smart"`           | Controls whether newly opened windows may take focus.       |
| `raise_on_click`               | Boolean                                      | `true`              | Raise a window when clicked.                                |
| `auto_raise`                   | Boolean                                      | `false`             | Raise the focused window automatically.                     |
| `focus_change_on_pointer_rest` | Boolean                                      | `false`             | Change focus when the pointer stops over another window.    |
| `auto_raise_delay`             | 0–10000 ms                                   | `500`               | Delay before automatic raise.                               |
| `action_double_click_titlebar` | Titlebar action below                        | `"toggle-maximize"` | Action for a titlebar double-click.                         |
| `action_middle_click_titlebar` | Titlebar action below                        | `"lower"`           | Action for a titlebar middle-click.                         |
| `action_right_click_titlebar`  | Titlebar action below                        | `"menu"`            | Action for a titlebar right-click.                          |
| `dynamic_workspaces`           | Boolean                                      | `false`             | Create and remove workspaces as needed.                     |
| `workspaces_only_on_primary`   | Boolean                                      | `false`             | Keep workspaces on the primary monitor.                     |
| `edge_tiling`                  | Boolean                                      | `false`             | Enable Mutter's edge tiling.                                |
| `num_workspaces`               | 1–36                                         | `4`                 | Fixed workspace count when dynamic workspaces are disabled. |
| `workspace_names`              | Up to 36 strings, each at most 80 characters | `{}`                | Display labels by workspace position.                       |
| `workspace_ids`                | Ordered array of up to 36 unique IDs         | `{}`                | Stable IDs assigned by each workspace's initial position.   |
| `center_new_windows`           | Boolean                                      | `false`             | Center newly created windows.                               |
| `attach_modal_dialogs`         | Boolean                                      | `false`             | Place modal dialogs with their parent window.               |
| `constrain_drag_to_work_area`  | Boolean                                      | `true`              | Keep interactive window moves inside the work area.         |

Workspace IDs must be unique and match
`^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`. A configured ID is assigned by initial
position, then follows its workspace if order changes. Gnoblin generates a
session-only ID for each unconfigured workspace; do not save those IDs in
rules or scripts. This works with fixed and dynamic workspaces. See the
[window rules guide](/guides/window_rules#workspaces) for matching and
placement examples.

For example, use pointer-follow focus and name the first two workspaces:

```lua
gnoblin.configure {
    window_management = {
        focus_mode = "sloppy",
        workspace_names = {"Main", "Chat"},
        workspace_ids = {"main", "chat"},
    },
}
```

## Titlebar actions

The three titlebar click fields accept these values:

| Value                          | Action                                  |
| ------------------------------ | --------------------------------------- |
| `toggle-maximize`              | Toggle maximize.                        |
| `toggle-maximize-horizontally` | Toggle horizontal maximize.             |
| `toggle-maximize-vertically`   | Toggle vertical maximize.               |
| `minimize`                     | Minimize the window.                    |
| `lower`                        | Lower the window in the stacking order. |
| `menu`                         | Open the window menu.                   |
| `none`                         | Do nothing.                             |

These settings control Mutter policy and Gnoblin's built-in fallback SSD.
Applications that draw their own client-side titlebars handle those clicks
themselves.

GTK applications usually draw their own titlebars and read GNOME's window
manager settings directly. This Lua section does not change those apps' CSD
behavior. A custom SSD renderer implements its own titlebar click behavior.

See [titlebars](/guides/window_frames).
