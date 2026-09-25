# gnoblin.configure.window_management

Configure this part of `gnoblin.configure` with the `window_management` key.

Put these fields inside `gnoblin.configure {window_management = {...}}`.
Changes apply on configuration reload. Omitted fields use the defaults below.

| Key                            | Values                                       | Default             | Effect                                                        |
| ------------------------------ | -------------------------------------------- | ------------------- | ------------------------------------------------------------- |
| `focus_mode`                   | `"click"`, `"sloppy"`, `"mouse"`             | `"click"`           | Selects when pointer or click input changes focus. See below. |
| `focus_new_windows`            | `"smart"`, `"strict"`                        | `"smart"`           | See focus behavior below.                                     |
| `raise_on_click`               | Boolean                                      | `true`              | Raise a window when clicked.                                  |
| `auto_raise`                   | Boolean                                      | `false`             | Raise the focused window automatically.                       |
| `focus_change_on_pointer_rest` | Boolean                                      | `false`             | Change focus when the pointer stops over another window.      |
| `auto_raise_delay`             | 0–10000 ms                                   | `500`               | Delay before automatic raise.                                 |
| `action_double_click_titlebar` | Titlebar action below                        | `"toggle-maximize"` | Action for a titlebar double-click.                           |
| `action_middle_click_titlebar` | Titlebar action below                        | `"lower"`           | Action for a titlebar middle-click.                           |
| `action_right_click_titlebar`  | Titlebar action below                        | `"menu"`            | Action for a titlebar right-click.                            |
| `dynamic_workspaces`           | Boolean                                      | `false`             | Create and remove workspaces as needed.                       |
| `workspaces_only_on_primary`   | Boolean                                      | `false`             | Keep workspaces on the primary monitor.                       |
| `edge_tiling`                  | Boolean                                      | `false`             | Enable Mutter's edge tiling.                                  |
| `num_workspaces`               | 1–36                                         | `4`                 | Fixed workspace count when dynamic workspaces are disabled.   |
| `workspace_names`              | Up to 36 strings, each at most 80 characters | `{}`                | Display labels by workspace position.                         |
| `workspace_ids`                | Ordered array of up to 36 unique IDs         | `{}`                | Stable IDs assigned by workspace position.                    |
| `center_new_windows`           | Boolean                                      | `false`             | Center newly created windows.                                 |
| `attach_modal_dialogs`         | Boolean                                      | `false`             | Place modal dialogs with their parent window.                 |
| `constrain_drag_to_work_area`  | Boolean                                      | `true`              | Keep interactive window moves inside the work area.           |

## Focus behavior

`focus_mode` controls how pointer movement changes focus. See
[GNOME's focus-mode schema entry](https://github.com/GNOME/gsettings-desktop-schemas/blob/main/schemas/org.gnome.desktop.wm.preferences.gschema.xml.in#L809-L828)
for the upstream setting description.

- `"click"` focuses a window when you click it. Moving the pointer does not
  change focus.
- `"sloppy"` focuses a window when the pointer enters it. If the pointer
  leaves all windows, focus returns to the most recently used eligible window.
- `"mouse"` also focuses on pointer entry, but clears focus when the pointer
  leaves all windows.

Set `focus_change_on_pointer_rest = true` to change focus only after the
pointer rests briefly.

`focus_new_windows` controls when app activation requests can focus a window.
Mutter's standard policy has two choices. The [GNOME Shell team's
focus-stealing overview](https://blogs.gnome.org/shell-dev/2024/09/20/understanding-gnome-shells-focus-stealing-prevention/)
explains why the modes differ.

- `"smart"` lets applications activate their windows, even if you have
  interacted with another window since the app opened them. This is Gnoblin's
  default behavior.
- `"strict"` enables Mutter's focus-stealing prevention. Activation requests
  need recent launch or user activity. A newly opened window must also be a
  transient descendant of the focused window, such as a dialog opened by that
  app. Requests without valid recent activity leave the window unfocused.

In a Gnoblin session, `"strict"` makes Mutter use its focus-stealing checks.
See [Gnoblin's Mutter patch](https://github.com/kierandrewett/gnoblin/blob/main/patches/mutter/52-focus-transfer/0001-honour-app-activation.patch)
for this session-specific behavior, or read
[Mutter's focus-stealing overview](https://blogs.gnome.org/shell-dev/2024/09/20/understanding-gnome-shells-focus-stealing-prevention/).
See the [focus prevention guide](/focus-transfer) for a ready-to-use config
example.

Workspace IDs must be unique and match
`^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`. Gnoblin generates a session-only ID for
each position without a configured ID. This works with fixed and dynamic
workspaces. See the [window rules guide](/guides/window_rules#workspaces) for
matching and placement examples.

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

## Type definition

This is schema pseudocode in Lua table form. `?` marks an optional field;
`|` separates accepted alternatives.

```lua
gnoblin.configure {
    window_management = {
        focus_mode = "click" | "sloppy" | "mouse"?,
        focus_new_windows = "smart" | "strict"?,
        raise_on_click = boolean?,
        auto_raise = boolean?,
        focus_change_on_pointer_rest = boolean?,
        auto_raise_delay = integer?, -- 0–10000 ms
        action_double_click_titlebar = TitlebarAction?,
        action_middle_click_titlebar = TitlebarAction?,
        action_right_click_titlebar = TitlebarAction?,
        dynamic_workspaces = boolean?,
        workspaces_only_on_primary = boolean?,
        edge_tiling = boolean?,
        num_workspaces = integer?, -- 1–36
        workspace_names = {string, ...}?, -- up to 36, max 80 characters each
        workspace_ids = {string, ...}?, -- up to 36 unique IDs
        center_new_windows = boolean?,
        attach_modal_dialogs = boolean?,
        constrain_drag_to_work_area = boolean?,
    },
}

-- TitlebarAction = "toggle-maximize" | "toggle-maximize-horizontally"
--                | "toggle-maximize-vertically" | "minimize" | "lower"
--                | "menu" | "none"
```
