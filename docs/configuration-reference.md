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

| Function                  | Input                      | Behaviour                                                                  |
| ------------------------- | -------------------------- | -------------------------------------------------------------------------- |
| `configure { ... }`       | Settings table             | Merge maps; replace supplied lists; later values win                       |
| `window_rule { ... }`     | Match and effect fields    | Append a rule; later matching fields win                                   |
| `permission_rule { ... }` | Identity and policy fields | Append a policy rule; any matching deny wins                               |
| `shortcut { ... }`        | Named command              | Add or update by name; omitted fields stay unchanged                       |
| `autostart { ... }`       | Named command              | Add or update by name; run once per name per login                         |
| `remove_shortcut(name)`   | String                     | Remove a named shortcut; missing names do nothing                          |
| `remove_autostart(name)`  | String                     | Remove an entry; does not stop its process                                 |
| `load(path)`              | File or glob               | Evaluate now, relative to the calling file                                 |
| `require(name)`           | Local module name          | Return a module result; once per reload; Lua global, not `gnoblin.require` |

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
| `name`          | Shortcut, autostart | Required string; identifies the entry for overrides                  |
| `command`       | Shortcut, autostart | Argument list, e.g. `{"ptyxis", "--new-window"}`; no shell expansion |
| `binding`       | Shortcut            | GTK accelerator, e.g. `"<Super>Return"`                              |
| `capture_input` | Shortcut            | Boolean; buffers popup typing; default `false`                       |

Shortcut names use letters, digits, `_` and `-`. Limit: 256 command shortcuts.
A new shortcut needs a binding and command. An override needs only its name and
changed fields. Different shortcut names must not claim the same binding.

Built-in bindings go in `configure {keybindings = {GROUP = {ACTION = {KEYS}}}}`.
Groups: `shell`, `wm`, `mutter`, `wayland`, `media`. Empty action lists disable
the binding. These values persist in GSettings.

Guides: [shortcuts](shortcuts.md), [autostart](autostart.md),
[restore or minimise](window-state-shortcuts.md).

### Keybinding groups

Use these schemas to look up action names with `gsettings list-recursively SCHEMA`:

| Lua group | GSettings schema                               |
| --------- | ---------------------------------------------- |
| `shell`   | `org.gnome.shell.keybindings`                  |
| `wm`      | `org.gnome.desktop.wm.keybindings`             |
| `mutter`  | `org.gnome.mutter.keybindings`                 |
| `wayland` | `org.gnome.mutter.wayland.keybindings`         |
| `media`   | `org.gnome.settings-daemon.plugins.media-keys` |

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
`wlr_gamma_control`, `wlr_output_power_management`, `ext_background_effect_v1`.

Guide: [session settings](session-settings.md).

## Cursor

GSettings schema: `org.gnome.desktop.interface`.
Keys: `cursor-theme` (string), `cursor-size` (integer).
These are GSettings, not Lua fields. Guide: [cursor themes](cursors.md).

## Files and reload

Root: `~/.config/gnoblin/init.lua`. Override: `GNOBLIN_CONFIG`.
Inspect: `gnoblinctl config path`. Apply: `gnoblinctl config reload`.

Valid edits reload automatically. A reload starts a fresh Lua state.
Native library upgrades require a new session.
See [load order](configuration-loading.md) and
[reload and persistence](configuration-loading.md#reload-and-persistence).
