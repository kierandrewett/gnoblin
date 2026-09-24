# Configuration reference

Use this page to look up names, values and defaults. For a first config, start
with [Configure Gnoblin](configuration.md); for complete examples, see
[Recipes](configuration-recipes.md).

Defaults apply before your config loads. Files supplied by your desktop shell
can change them. Sizes are [logical pixels](configuration.md#sizes-and-window-types).

## Lua API

`gnoblin` is available in each config file without an import. Call its functions
with a settings table, for example `gnoblin.configure {shell = {minimize_duration = 150}}`.
Setting names use underscores (`snake_case`). String values are used as written.
Each reload builds the config afresh; these functions describe that config,
not one-off commands to change the running desktop.

| Function | Input | Behaviour |
| --- | --- | --- |
| [`configure { ... }`](configuration.md#2-make-a-change) | Settings table | Merge maps; replace supplied lists; later values win |
| [`window_rule { ... }`](window-rules.md#add-a-rule) | Match and effect fields | Append a rule; later matching fields win |
| [`permission_rule { ... }`](permissions.md#example-allow-a-remote-desktop-app) | Identity and policy fields | Append a policy rule; any matching deny wins |
| [`shortcut { ... }`](shortcuts.md#function-form) | Named command | Add or update by name; `enable = false` disables it |
| [`autostart { ... }`](autostart.md#function-form) | Named command | Add or update by name; `enable = false` disables it |
| [`remove_shortcut(name)`](shortcuts.md#remove-a-shortcut) | Shortcut name | Older form of disabling an earlier named shortcut |
| [`remove_autostart(name)`](autostart.md#remove-an-entry) | Entry name | Older form of disabling an earlier named autostart |
| [`load(path)`](configuration-loading.md#include-a-file) | File or glob | Evaluate now, relative to the calling file |
| [`require(name)`](configuration-loading.md#use-a-lua-module) | Local module name | Return a module result; once per reload; Lua global, not `gnoblin.require` |
| [`snapshot()`](configuration-loading.md#inspect-loaded-settings) | None | Copy the config assembled so far |
| [`configure.shortcuts.NAME`](shortcuts.md#remove-a-shortcut) | Existing shortcut name | Read or change a loaded command shortcut directly |
| [`configure.autostart.NAME`](autostart.md#remove-an-entry) | Existing autostart name | Read or change a loaded login command directly |

Call functions as `gnoblin.window_rule { ... }`, for example.
Tables passed to declarations are copied. Later changes to your table do not
change the declaration. Shader uniform keys and renderer names stay literal.

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

Guides: [animations](animations.md), [native features](session-settings.md),
[window menu](window-menu.md).

## Commands

| Field           | Used by             | Value                                                                |
| --------------- | ------------------- | -------------------------------------------------------------------- |
| `name`          | Shortcut, autostart | Required in function calls; named maps use the map key instead        |
| `command`       | Shortcut, autostart | Argument list, e.g. `{"ptyxis", "--new-window"}`; no shell expansion |
| `binding`       | Shortcut            | GTK accelerator, e.g. `"<Super>Return"`                              |
| `capture_input` | Shortcut            | Boolean; buffers popup typing; default `false`                       |

Shortcut names use letters, digits, `_` and `-`. Limit: 256 command shortcuts.
A new shortcut needs a binding and command. Set `configure {shortcuts = {
NAME = {binding = "...", command = {...}}}}` to define one, or set
`NAME = {enable = false}` to disable an imported entry. Named map entries merge
with earlier entries; omitted fields stay unchanged. Different shortcut names
must not claim the same binding.

`autostart` accepts the same named map form. Its entries run once per name per
login; disabling one does not stop a process that already started.

Built-in bindings go in `configure {keybindings = {GROUP = {ACTION = {KEYS}}}}`.
Use underscores in action names; hyphenated config names are rejected. For
example, configure `show_screenshot_ui` for GSettings' `show-screenshot-ui`.
Groups: `shell`, `wm`, `mutter`, `wayland`. Empty action lists disable the
binding. Gnoblin applies these to Mutter's native keybinding table on reload;
the Lua file owns persistence. Removing an override restores the built-in
default. Media keys are [command shortcuts in Lua](shortcuts.md#media-keys),
not built-in `keybindings` actions.

Guides: [shortcuts](shortcuts.md), [autostart](autostart.md),
[restore or minimise](window-state-shortcuts.md).

### Keybinding groups

Use these schemas to look up action names with `gsettings list-keys SCHEMA`:

| Lua group | GSettings schema                               |
| --------- | ---------------------------------------------- |
| `shell`   | `org.gnome.shell.keybindings`                  |
| `wm`      | `org.gnome.desktop.wm.keybindings`             |
| `mutter`  | `org.gnome.mutter.keybindings`                 |
| `wayland` | `org.gnome.mutter.wayland.keybindings`         |

These schemas still provide the built-in action catalogue and default bindings.
Gnoblin does not save the configured overrides to them.

## Window management

Inside `gnoblin.configure {window_management = {...}}`. Values apply on reload
and omitted fields return to the Gnoblin defaults below.

| Key | Values | Default |
| --- | --- | --- |
| `focus_mode` | `"click"`, `"sloppy"`, `"mouse"` | `"click"` |
| `focus_new_windows` | `"smart"`, `"strict"` | `"smart"` |
| `raise_on_click`, `auto_raise`, `focus_change_on_pointer_rest` | Boolean | `true`, `false`, `false` |
| `auto_raise_delay` | 0–10000 ms | `500` |
| `action_double_click_titlebar` | Titlebar action below | `"toggle-maximize"` |
| `action_middle_click_titlebar` | Titlebar action below | `"lower"` |
| `action_right_click_titlebar` | Titlebar action below | `"menu"` |
| `dynamic_workspaces`, `workspaces_only_on_primary`, `edge_tiling` | Boolean | `false` |
| `num_workspaces` | 1–36; used when dynamic workspaces are off | `4` |
| `workspace_names` | Array of up to 36 strings, each at most 80 characters | `{}` |
| `center_new_windows`, `attach_modal_dialogs` | Boolean | `false` |
| `constrain_drag_to_work_area` | Boolean | `true` |

Titlebar actions: `toggle-maximize`, `toggle-maximize-horizontally`,
`toggle-maximize-vertically`, `minimize`, `lower`, `menu`, `none`.
GTK apps usually draw their own titlebars and read GNOME's window-manager
settings directly; this Lua section does not change those app settings. It
controls Mutter policy and Gnoblin's built-in fallback SSD, which implements
these actions. A custom SSD renderer must implement its own titlebar click
behavior.
See [titlebars](window-frames.md).

## Compositor interaction

Inside `gnoblin.configure {compositor = {...}}`. Values apply on reload.
These settings cover compositor interaction preferences; accessibility
settings remain separate.

| Key | Values | Default |
| --- | --- | --- |
| `enable_animations` | Boolean | `true` |
| `locate_pointer` | Boolean | `false` |
| `visual_bell`, `audible_bell` | Boolean | `false`, `true` |
| `visual_bell_type` | `"fullscreen-flash"`, `"frame-flash"` | `"fullscreen-flash"` |

Guide: [session settings](session-settings.md).

## Input

Input settings go in `gnoblin.configure {input = {...}}` and apply on reload.
Each group is optional. Fields you omit continue to use the corresponding
GNOME/Mutter setting. Global settings use `mouse`, `touchpad`, and `keyboard`;
tablet and stylus overrides are selected by device identifier.

| Group | Fields | Values |
| --- | --- | --- |
| `mouse` | `speed` | Number from -1 to 1 |
| | `left_handed`, `natural_scroll` | Boolean |
| | `accel_profile` | `"default"`, `"flat"`, `"adaptive"` |
| `touchpad` | `speed` | Number from -1 to 1 |
| | `left_handed` | `"right"`, `"left"`, `"mouse"` |
| | `natural_scroll`, `tap_to_click`, `tap_and_drag`, `tap_and_drag_lock`, `disable_while_typing`, `edge_scrolling_enabled`, `two_finger_scrolling_enabled` | Boolean |
| | `accel_profile` | `"default"`, `"flat"`, `"adaptive"` |
| | `tap_button_map` | `"default"`, `"lrm"`, `"lmr"` |
| | `click_method` | `"default"`, `"none"`, `"areas"`, `"fingers"` |
| `keyboard` | `repeat`, `remember_numlock_state`, `numlock_state` | Boolean |
| | `delay`, `repeat_interval` | 1–10000 ms |
| | `xkb_options` | Array of XKB option strings |

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

## Window matches

Inside `window_rule {match = {...}, ...}`. All specified conditions must match.

| Key       | Value                                                         |
| --------- | ------------------------------------------------------------- |
| `type`    | `"window"` or `"layer"`                                       |
| `focused` | Boolean                                                       |
| `app_id`  | JavaScript regex against GTK app ID, falling back to WM class |
| `title`   | JavaScript regex against the window title                     |
| `layer`   | Layer-shell namespace matcher                                 |

Use anchors for exact regex matches. Rules apply in order; later matching rules
override only supplied fields. Guide: [window rules](window-rules.md).

## Effects

Fields inside `window_rule {...}`.

| Key                   | Value                                               |
| --------------------- | --------------------------------------------------- |
| `blur`                | Integer 0–100; 0 disables blur                      |
| `opacity`             | Number 0–1; affects content and text                |
| `blur_ignore_shadows` | Boolean; default `false`                            |
| `corners`             | Corner fields below                                 |
| `borders`             | Border fields below                                 |
| `shader`              | GLSL file path; `""` clears it                      |
| `shader_uniforms`     | Map of literal uniform names to numeric values      |
| `animation`           | `"slide"`, `"fade"`, `"none"`, or a per-phase table |
| `frame`               | Frame fields below                                  |

Guides: [effects](window-effects.md), [shaders](shaders.md).

### Corners

| Field                           | Default        | Values                                                        |
| ------------------------------- | -------------- | ------------------------------------------------------------- |
| `radius`                        | `0`            | 0–200 logical pixels                                          |
| `smoothing`                     | `0`            | 0–1; circular to a squarer curve                              |
| `mode`                          | `"auto"`       | `auto`, `force`, `off`                                        |
| `padding`                       | `{0, 0, 0, 0}` | Top/right/bottom/left inset, −128–128 logical pixels          |
| `keep_maximized`                | `true`         | Keep rounding when maximised                                  |
| `keep_fullscreen`, `keep_tiled` | `false`        | Keep rounding in those states                                 |
| `skip_libadwaita`               | `true`         | Preserve libadwaita corners in auto mode                      |
| `skip_libhandy`                 | `false`        | Skip libhandy windows in auto mode                            |
| `remove_csd`                    | `false`        | Reconstruct supported client corner gaps                      |
| `shadow`                        | `false`        | A shadow table or 1–4 shadow layers                           |
| `keep_shadow`                   | `false`        | Keep replacement shadows in maximised/fullscreen/tiled states |

### Borders and shadows

| Field                                                                     | Value / default                                                               |
| ------------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| `borders.inner_width`, `borders.outer_width`                              | 0–40 logical pixels; default 0                                                |
| `borders.inner_color`, `borders.outer_color`                              | `#RRGGBB` or `#RRGGBBAA`                                                      |
| `borders.radius`, `borders.smoothing`, `borders.padding`                  | Inherit corners; same ranges                                                  |
| `borders.keep_maximized`, `borders.keep_fullscreen`, `borders.keep_tiled` | Boolean; `false` hides borders in that state                                  |
| `corners.shadow`                                                          | Table or 1–4 layer tables with `x`, `y`, `blur`, `spread`, `opacity`, `color` |
| `corners.shadow_animation.duration`                                       | 0–2000 ms; default 0                                                          |
| `corners.shadow_animation.easing`                                         | Same easing values as shell animations                                        |

### Layer animation overrides

| Field                              | Value                                  |
| ---------------------------------- | -------------------------------------- |
| `animation["in"]`, `animation.out` | `"slide"`, `"fade"`, `"none"`          |
| `animation.duration`               | 0–5000 ms                              |
| `animation.easing`                 | Same easing values as shell animations |

Omitted fields inherit shell settings. `in` needs brackets because it is a Lua
keyword. Guide: [animations](animations.md).

## Frames

Inside a rule's `frame` table.

| Field                 | Default                             | Values                                               |
| --------------------- | ----------------------------------- | ---------------------------------------------------- |
| `mode`                | `"off"`                             | `"off"`, `"auto"`, `"prefer-server"`, `"replace"`    |
| `extents`             | `{32, 1, 1, 1}`                     | Top/right/bottom/left frame size; integers 0–256     |
| `crop`                | `{0, 0, 0, 0}`                      | Removed client margins; same order and range         |
| `renderer`            | `"native"`                          | Registered service name                              |
| `style`               | `"default"`                         | Style understood by that renderer                    |
| `background`          | `"#242424"`                         | Active background colour                             |
| `foreground`          | `"#eeeeee"`                         | Foreground colour                                    |
| `inactive_background` | `"#303030"`                         | Unfocused background colour                          |
| `button_layout`       | `{"minimize", "maximize", "close"}` | Ordered buttons, without duplicates; `{}` hides them |

`auto` supplies SSD only for explicit client requests. `replace` crops client
pixels and adds a frame. Register services with
`configure {frame_renderers = {NAME = {COMMAND}}}`. Guide: [frames](window-frames.md).

## Permissions

Global fallback: `configure {permissions = {default = "default"}}`.
Allowed fallbacks: `"default"`, `"ask"`, `"deny"`.

| Rule field     | Value                                                                                    |
| -------------- | ---------------------------------------------------------------------------------------- |
| `name`         | Rule name                                                                                |
| `match`        | JavaScript regex against `app-id:ID` or `host-exe:PATH`                                  |
| `capabilities` | List: `"screen-cast"`, `"remote-desktop"`, `"input-capture"`, `"screenshot"`, `"access"` |
| `level`        | `"default"`, `"ask"`, `"allow"`, `"deny"`                                                |
| `monitors`     | List of connectors or `"primary"`                                                        |
| `devices`      | List: `"keyboard"`, `"pointer"`, `"touchscreen"`; default empty                          |
| `clipboard`    | Boolean; default `false`                                                                 |

Any matching deny wins. Otherwise the last matching rule wins as a whole.
Guide: [portal permissions](permissions.md).

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

Guides: [session settings](session-settings.md), [protocol catalog](wayland-protocols.md).

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
Changes apply on config reload. Guide: [cursor themes](cursors.md).

## Files and reload

Root: `~/.config/gnoblin/init.lua`. Override: `GNOBLIN_CONFIG`.
Inspect: `gnoblinctl config path`. Apply: `gnoblinctl config reload`.

Valid edits reload automatically. A reload starts a fresh Lua state.
Native library upgrades require a new session.
See [load order](configuration-loading.md) and
[reload and persistence](configuration-loading.md#reload-and-persistence).
