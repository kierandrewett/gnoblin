# Files and load order

[Configuration reference](configuration-reference.md)

Start with one `init.lua`. Split it into modules when that makes it easier
to read.

## Which file is loaded?

`gnoblinctl config path` shows the running session's root file.

The normal root is `~/.config/gnoblin/init.lua`, or
`$XDG_CONFIG_HOME/gnoblin/init.lua` when that variable is set.
The compositor's `GNOBLIN_CONFIG` environment variable overrides it.

If `init.lua` is absent, the loader checks `gnoblin.toml`, then
`gnoblin.conf`. These are alternatives, not extra includes.
Use a `.lua` suffix for Lua; other suffixes select TOML.

A missing root starts with defaults. Setting `GNOBLIN_CONFIG` in a terminal
does not change the already-running compositor's environment.

## Include a file

```lua
gnoblin.load("appearance.lua")
```

The path is relative to the calling file. Absolute and `~/` paths also work.
The included file uses the same API:

```lua
gnoblin.configure {
    shell = {minimize_duration = 150},
}
```

## Include a directory

```lua
gnoblin.load("conf.d/**/*.lua")
```

Files load in bytewise path order. Names such as `10-appearance.lua` and
`90-local.lua` make that order obvious.

- `*`, `?` and `[abc]` match within one path segment.
- `**` also searches subdirectories, without following directory symlinks.
- Hidden names need an explicit dot in the pattern.
- An unmatched glob is allowed; a missing exact filename is an error.

Files and searched directories are watched, including newly added matches.

## Override or append?

Load component files first, then your changes.

`gnoblin.configure` merges maps recursively. Later scalar values win.
A nonempty list replaces the earlier list, including `window-rules`,
`shortcuts`, `autostart` and permission `rules`.

To preserve imported rules, append:

```lua
gnoblin.window_rule {
    match = {type = "window", focused = false},
    opacity = 0.95,
}
```

`gnoblin.shortcut` and `gnoblin.autostart` merge entries with the same name.
Use `gnoblin.remove_shortcut(name)` or `gnoblin.remove_autostart(name)` to
remove an imported entry. Removing an autostart entry does not stop a process.

To replace a whole list, use `gnoblin.configure {shortcuts = {...}}`.
Use `shortcuts = {}` to clear it. Prefer declarations when extending imports.

## Use a Lua module

```lua
gnoblin.configure(require("appearance"))
```

`require("appearance")` searches `appearance.lua`, then
`lua/appearance.lua`, relative to its caller. It runs once per reload.
A returned table is only applied when passed to `gnoblin.configure`.

Dots in module names are not converted to directories. For explicit subdirectories,
use `gnoblin.load("parts/motion.lua")`. Avoid loading the same file through both a glob
and `require`.

## API

`gnoblin` is available globally in every loaded file and module.
See the [function reference](configuration-reference.md#lua-api).

Setting names use `snake_case`. The API converts them to Gnoblin's internal
hyphenated names. String values, shader uniform names and renderer names stay
literal. Do not supply both spellings of the same key in one table.

Reload starts a fresh Lua state. Standard table, string, math and UTF-8 helpers
are available; file I/O, process execution and native modules are not.
Use shortcuts or autostart to launch programs.

Evaluation is limited to 8 MiB of Lua memory, one million instructions and
32 nested files.

## Existing configs

Configs using `require("gnoblin")`, `g.set`, `g.config`, returned tables
and TOML still work.
They retain their original hyphenated keys and merge behaviour.

For example, this existing config:

```lua
local g = require("gnoblin")
g.set {
    shell = {["minimize-duration"] = 150},
}
```

is equivalent to:

```lua
gnoblin.configure {
    shell = {minimize_duration = 150},
}
```

You can use `gnoblin.configure` after existing component includes without
rewriting the included files.

If `gnoblin` or `configure` is reported as `nil`, see
[configuration compatibility](troubleshooting.md#gnoblin-or-configure-is-nil).

## Reload and persistence

| Change                       | Applies                     | When removed                             |
| ---------------------------- | --------------------------- | ---------------------------------------- |
| Rules and animations         | On reload                   | Earlier rules/defaults apply             |
| SSD policy                   | On reload and client commit | Earlier rules/defaults apply             |
| Command shortcuts            | On reload                   | Binding released; launched process stays |
| Built-in keybindings         | On reload                   | Saved GSettings value stays              |
| Native feature booleans      | On reload                   | Saved GSettings value stays              |
| Autostart                    | New names start on reload   | Running process stays                    |
| Renderer services            | Restart on reload           | Enabled frames use native fallback       |
| Protocols                    | Next login                  | Default on next login                    |
| Exclusive-layer focus policy | Next login                  | Default on next login                    |
| Drag boundary                | Next drag after reload      | Defaults to enabled                      |

An already-started autostart name uses a changed command only on the next login.
Explicit file settings override later CLI changes on the next config reload.

`gnoblinctl reload` also reloads the theme and user scripts. Neither reload
command replaces native libraries or restarts your separate desktop shell.
