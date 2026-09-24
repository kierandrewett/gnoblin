# Configuration reader

Gnoblin evaluates one Lua root file in a fresh restricted Lua state. The
default root is `$XDG_CONFIG_HOME/gnoblin/init.lua`. `GNOBLIN_CONFIG` selects
an explicit root. Use a `.lua` suffix for Lua; other suffixes select TOML.

`gnoblin` is available globally. Use `gnoblin.configure { ... }` for settings,
`gnoblin.window_rule { ... }` and `gnoblin.permission_rule { ... }` for ordered
rules. `gnoblin.shortcut { ... }` and `gnoblin.autostart { ... }` merge entries
by name. `remove_shortcut(name)` and `remove_autostart(name)` remove them.
`gnoblin.animation { ... }` registers reusable transitions; use
`remove_animation(name)` to remove one. Animation declarations merge by name
and accept `duration`, `ease`, `origin`, `from`, `to`, `keyframes`, and an
optional `target`. See the [animation guide](../../docs/guides/animations.md)
for supported events, properties, presets, and preview controls.
For named overrides, `gnoblin.configure {shortcuts = {terminal = {command = {...}}}}`
merges fields, and `terminal = {enable = false}` disables that entry. The same
form works for autostart. The removal functions remain for older configs.
`gnoblin.snapshot()` copies the config accumulated at that point in the file,
so Lua code can inspect it while declaring more settings.
For an existing named entry, `gnoblin.configure.autostart.waybar.enable = false`
or `gnoblin.configure.shortcuts.terminal.command = {"ptyxis"}` changes it
directly. `pairs(gnoblin.configure.shortcuts)` iterates loaded command shortcuts.
Assigning a table to `gnoblin.configure.shortcuts.NAME` or
`gnoblin.configure.autostart.NAME` adds or merges that named entry.

Declarations copy their input and convert snake_case setting names to the
internal hyphenated form. Renderer names and shader uniform names stay literal.
Old `require('gnoblin')`, `g.set`, `g.config` and returned settings tables keep
their existing semantics.

Use `gnoblin.load('conf.d/**/*.lua')` for sorted fragments in the same state.
`require()` evaluates a module once per reload and returns its cached result.

The reader records loaded files and glob directories. A missing root gives an
empty document. A missing file named by `gnoblin.load()` is an error.

When `init.lua` is absent, the loader checks legacy `gnoblin.toml`, then
`gnoblin.conf`. New configurations should use Lua. See the
[user guide](../../docs/config.md) for selection, merge and reload behavior.

Run `./tests/test-config.sh` for the native Lua and glob tests.
