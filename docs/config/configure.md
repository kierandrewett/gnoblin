# gnoblin.configure

Configure compositor, shell, input and window-management settings with this function. For a first config, see [config](/config); for complete examples, see [recipes](/recipes).

Defaults apply before your config loads. Files supplied by your desktop shell
can change them. See [sizes and window types](#sizes-and-window-types) for the units used below.

## Usage {#2-make-a-change}

Pass one table of settings. Calls merge maps, replace supplied lists, and later values win. Setting names use `snake_case`.
Named shortcuts and autostart entries are also available as `gnoblin.configure.shortcuts.NAME` and `gnoblin.configure.autostart.NAME`. Read or update an entry through these views, or assign a table to add one. See the [shortcuts](/guides/shortcuts) and [autostart](/guides/autostart) guides. Use [`gnoblin.snapshot()`](/config/snapshot) to inspect a copy of the config assembled so far.

```lua
gnoblin.configure {shell = {minimize_duration = 150}}
```

## Sizes and window types {#sizes-and-window-types}

Sizes use logical pixels before display scaling. A window is an application window; a layer surface is a bar, dock, launcher or other Wayland layer-shell panel. Window rules distinguish them with `type = "window"` or `type = "layer"`.

## Shell

Inside `gnoblin.configure {shell = {...}}`.

| Key                     | Values                                                                   | Default                                   |
| ----------------------- | ------------------------------------------------------------------------ | ----------------------------------------- |
| `minimize_animation`    | `"zoom"`, `"fade"`, `"none"`, `"gnome"`                                  | `"zoom"`                                  |
| `minimize_duration`     | 0–5000 ms                                                                | `200`                                     |
| `minimize_target`       | `{x, y}` in desktop logical pixels                                       | Dock target, then bottom centre           |
| `layer_animation`       | `"slide"`, `"fade"`, `"none"`                                            | `"slide"`                                 |
| `layer_duration`        | 0–5000 ms                                                                | `220`                                     |
| `layer_easing`          | `"linear"`, `"ease-out-quad"`, `"ease-out-cubic"`, `"ease-in-out-cubic"` | `"ease-out-cubic"`                        |
| `window_menu`           | Command argument list                                                    | Empty                                     |
| `window_switcher`       | Boolean                                                                  | `false`                                   |
| `notifications`         | Boolean                                                                  | Initially disabled; persists in GSettings |
| `input_source_switcher` | Boolean                                                                  | Initially disabled; persists in GSettings |

Guides: [animations](/guides/animations), [native features](/guides/session_settings),
[window menu](/guides/window_menu).

## Keybindings {#keybinding-groups}

Override Mutter's built-in keybindings with `keybindings = {GROUP = {ACTION = {KEYS}}}`. Use underscore names for actions. Empty action lists disable that binding. Gnoblin applies overrides to Mutter's native keybinding table on reload and keeps persistence in the Lua config. Removing an override restores the built-in default. GNOME Settings Daemon media keys use a separate service.

| Group     | GSettings schema                       |
| --------- | -------------------------------------- |
| `shell`   | `org.gnome.shell.keybindings`          |
| `wm`      | `org.gnome.desktop.wm.keybindings`     |
| `mutter`  | `org.gnome.mutter.keybindings`         |
| `wayland` | `org.gnome.mutter.wayland.keybindings` |

Use `gsettings list-keys SCHEMA` to look up native action names. For example, `show_screenshot_ui` maps to GSettings' `show-screenshot-ui`. See the [shortcuts guide](/guides/shortcuts).

## Window management

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

## Compositor interaction

Inside `gnoblin.configure {compositor = {...}}`. Values apply on reload.
These settings cover compositor interaction preferences; accessibility
settings remain separate.

| Key                           | Values                                | Default              |
| ----------------------------- | ------------------------------------- | -------------------- |
| `enable_animations`           | Boolean                               | `true`               |
| `locate_pointer`              | Boolean                               | `false`              |
| `visual_bell`, `audible_bell` | Boolean                               | `false`, `true`      |
| `visual_bell_type`            | `"fullscreen-flash"`, `"frame-flash"` | `"fullscreen-flash"` |

Guide: [session settings](/guides/session_settings).

## Input

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

### Input sources

Set sources with `gnoblin.configure {input_sources = {...}}`. This replaces the
active source list in memory on reload; removing the table restores GNOME's
session sources. `sources` is required and contains records with `type` set to
`"xkb"` or `"ibus"` and a nonempty `id`. Set `per_window` to `true` to track
the active source per window; it defaults to `false`.

```lua
gnoblin.configure {
    input_sources = {
        sources = {
            {type = "xkb", id = "us"},
            {type = "xkb", id = "gb"},
        },
        per_window = false,
    },
}
```

## Permissions {#permissions}

Set the fallback used when no explicit portal permission rule decides a request.

| Setting               | Values                         | Default     |
| --------------------- | ------------------------------ | ----------- |
| `permissions.default` | `"default"`, `"ask"`, `"deny"` | `"default"` |

Add ordered per-application rules with [`gnoblin.permission_rule`](/config/permission_rule).

## Session and protocols

Inside `gnoblin.configure {...}`.

| Setting                                         | Value   | Default | Applies    |
| ----------------------------------------------- | ------- | ------- | ---------- |
| `layer_shell.preserve_active_window`            | Boolean | `true`  | Next login |
| `window_management.constrain_drag_to_work_area` | Boolean | `true`  | Next drag  |
| `protocols.NAME`                                | Boolean | `true`  | Next login |

Protocol keys: `wlr_layer_shell`, `wlr_screencopy`, `ext_foreign_toplevel_list`,
`wlr_foreign_toplevel_management`, `ext_data_control`, `ext_idle_notify`,
`wlr_gamma_control`, `wlr_output_power_management`, `ext_background_effect_v1`,
`xdg_decoration`, `window_frame_renderer`, `blur_fade`.

Guides: [session settings](/guides/session_settings), [protocol catalog](/wayland-protocols).

## Frame renderers

Register named renderer commands with `frame_renderers`. Each value is an argument list; use an absolute executable path. `native` is reserved. Select the registered name in a `frame` field on [`gnoblin.window_rule`](/config/window_rule#frame-fields).

```lua
gnoblin.configure {
    frame_renderers = {
        cairo = {"/absolute/path/gnoblin-frame-cairo"},
    },
}
```

## Cursor

Configure the compositor cursor theme and size in Gnoblin's config:

```lua
gnoblin.configure {
    cursor = {
        theme = "Adwaita-Hyprcursor",
        size = 24,
    },
}
```

`theme` is an installed cursor theme name. Gnoblin currently renders it with
Hyprcursor. `size` is an integer from 1 to 256 logical pixels (default `24`).
Changes apply on config reload. Guide: [cursor themes](/guides/cursors).

## Reload

Root: `~/.config/gnoblin/init.lua`. Override: `GNOBLIN_CONFIG`.
Inspect: `gnoblinctl config path`. Apply: `gnoblinctl config reload`.

Valid edits reload automatically. A reload starts a fresh Lua state.
Native library upgrades require a new session.
See [load order](/guides/files_and_load_order) and [reload and persistence](/guides/files_and_load_order#reload-and-persistence).
