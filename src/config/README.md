# Configuration reader

Gnoblin evaluates one Lua root file in a fresh restricted Lua state. The
default root is `$XDG_CONFIG_HOME/gnoblin/init.lua`. `GNOBLIN_CONFIG` selects
an explicit root. Use a `.lua` suffix for Lua; other suffixes select TOML.

`gnoblin` is available globally. Its main declarations are:

| Declaration                       | Input                              | Purpose and reference                                                                     |
| --------------------------------- | ---------------------------------- | ----------------------------------------------------------------------------------------- |
| `gnoblin.configure { ... }`       | Settings table                     | Sets runtime options. See the [configuration API](/config/configure).                     |
| `gnoblin.window_rule { ... }`     | Match and effect tables            | Adds an ordered window rule. See the [window rules guide](/guides/window_rules).          |
| `gnoblin.permission_rule { ... }` | Match, capabilities, and decision  | Adds an ordered permission rule. See the [permission rule API](/config/permission_rule).  |
| `gnoblin.animation { ... }`       | Name, event, timing, and keyframes | Registers a named transition. See the [animation guide](../../docs/guides/animations.md). |

Use `gnoblin.configure.shortcuts` and `gnoblin.configure.autostart` for named
entries. The older `gnoblin.shortcut`, `gnoblin.autostart`,
`gnoblin.remove_shortcut`, and `gnoblin.remove_autostart` functions remain for
compatibility and are deprecated.

The first animation registered for an event supplies its default. Settings and
window rules can select another registered name. Set `enable = false` on a
named animation declaration to disable it:

- `gnoblin.animation {name = "soft-open", enable = false}` disables that animation.

For example, a settings declaration and a window rule use this shape:

```lua
gnoblin.configure {window_management = {focus_mode = "click"}}

gnoblin.window_rule {
    match = {type = "window", app_id = "^org.example.Editor$"},
    animation = {open = "gnome-open"},
}
```

Animation declarations merge by `name` during one config load.

| Animation field              | Purpose                                               |
| ---------------------------- | ----------------------------------------------------- |
| `duration`, `ease`, `origin` | Set timing, interpolation curve, and transform pivot. |
| `from`, `to`                 | Set the start and end property values.                |
| `keyframes`                  | Set ordered intermediate values.                      |
| `target`                     | Add a label used by inspection tools.                 |

Actor events accept `x`, `y`, `scale`, `scale_x`, `scale_y`, `rotation`, and
`opacity`. Tile-preview events also accept `width` and `height`. Workspace,
resize, shadow, and dialog-dimming events use `progress`.

The supported event names are:

- Window: `minimize`, `restore`, `open`, `close`.
- Dialog: `dialog-open`, `dialog-close`, `dialog-dim`, `dialog-undim`.
- Layer surface: `layer-open`, `layer-close`, `layer-companion-close`.
- Workspace and shell: `workspace-switch`, `console-open`, `console-close`.
- Effects: `shadow-change`, `resize`, `tile-preview-open`,
  `tile-preview-close`.

See the [animation guide](../../docs/guides/animations.md) for presets,
layer-shell use, and CLI preview commands.

Declarations copy their input. Gnoblin converts setting names from
`snake_case` to its internal hyphenated form. Keybinding action names stay
`snake_case` until the shell maps them to GSettings. Renderer and shader
uniform names stay literal.

Use `gnoblin.load('conf.d/**/*.lua')` to load sorted fragments in the same
state. `require()` evaluates each module once per reload and returns the cached
result on later calls.

The reader records loaded files and glob directories. A missing root produces
an empty document. A missing file named by `gnoblin.load()` is an error.

The selected root is evaluated in a fresh Lua state on each reload. See the
[configuration guide](../../docs/config/configure) for settings and the
[file loading guide](../../docs/guides/files_and_load_order) for fragments,
includes and reload behavior.

Run `./tests/test-config.sh` for the native Lua and glob tests.
