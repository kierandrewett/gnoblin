# files_and_load_order

[Configuration reference](/config/reference)

Start with one `init.lua`. Split it into modules when that makes it easier
to read.

## Which file is loaded?

`gnoblinctl config path` prints the main config file used by your running session.

The usual path is `~/.config/gnoblin/init.lua`, or
`$XDG_CONFIG_HOME/gnoblin/init.lua` when that variable is set.
The compositor's `GNOBLIN_CONFIG` environment variable overrides it.

If `init.lua` is absent, the loader checks `gnoblin.toml`, then
`gnoblin.conf`. These are alternatives, not extra includes.
Use a `.lua` suffix for Lua; other suffixes select TOML.

For packaged logins, `gnoblin-session` seeds `init.lua` from
`/usr/share/gnoblin/init.lua.example` before starting gnome-session when no
supported user config exists. The shell then loads that file on its first
config load. The seed step preserves existing `init.lua`, TOML, and legacy
config files. If the example is unavailable, or when running a build directly,
Gnoblin uses its defaults. Setting `GNOBLIN_CONFIG` in a terminal does not
change the already-running compositor's environment.

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

Matching files load in filename order (byte order, independent of your language
settings). Use numbered names such as `10-appearance.lua` and `90-local.lua`
to make the order predictable.

- `*`, `?` and `[abc]` match within one path segment.
- `**` also searches subdirectories, without following directory symlinks.
- Hidden names need an explicit dot in the pattern.
- An unmatched glob is allowed; a missing exact filename is an error.

Files and searched directories are watched, including newly added matches.

## Override or append?

Put `gnoblin.load(...)` calls before your personal settings so your changes
are applied last.

Repeated `gnoblin.configure` calls keep settings you have not changed:

```lua
gnoblin.configure {shell = {minimize_animation = "fade", minimize_duration = 200}}
gnoblin.configure {shell = {minimize_duration = 150}}
```

The result is a fade lasting 150 milliseconds. Changing the duration does not
remove the animation choice.

Lists behave differently: supplying `window_rules`, `shortcuts`, `autostart`
or permission `rules` replaces that list. To add a window rule while keeping
previous rules, use:

```lua
gnoblin.window_rule {
    match = {type = "window", focused = false},
    opacity = 0.95,
}
```

`gnoblin.shortcut` and `gnoblin.autostart` merge entries with the same name.
Use `gnoblin.remove_shortcut(name)` or `gnoblin.remove_autostart(name)` to
remove an imported entry. Removing an autostart entry does not stop a process.

To remove every command shortcut loaded so far:

```lua
gnoblin.configure {shortcuts = {}}
```

Use `gnoblin.shortcut` to add or change individual shortcuts without clearing
the others.

## Use a Lua module

Use a module when you want a file to return settings for another file to use.
For ordinary config files, `gnoblin.load` is enough.

In `appearance.lua`:

```lua
return {shell = {minimize_duration = 150}}
```

In `init.lua`:

```lua
gnoblin.configure(require("appearance"))
```

`require("appearance")` searches `appearance.lua`, then
`lua/appearance.lua`, relative to its caller. It runs once per reload.
A returned table is only applied when passed to `gnoblin.configure`.

Dots in module names are not converted to directories. For explicit subdirectories,
use `gnoblin.load("parts/motion.lua")`. Avoid loading the same file through both a glob
and `require`.

## Available Lua functions

`gnoblin` is available globally in every loaded file and module.
See the [function reference](/config/reference#lua-api).

Setting names use `snake_case`. The API converts them to Gnoblin's internal
hyphenated names. String values, shader uniform names and renderer names stay
literal. Do not supply both spellings of the same key in one table.

Each reload evaluates the files again; Lua variables do not survive it.
You can use Lua's table, string, math and UTF-8 helpers. Config code cannot
read arbitrary files, run processes or load native modules.
Use shortcuts or autostart to launch programs.

Evaluation is limited to 8 MiB of Lua memory, one million instructions and
32 nested files.

Lua configuration cannot run arbitrary JavaScript. For custom live automation,
see [user scripts](/user-scripts); most settings and desktop behavior should
stay in the supported configuration API.

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
[configuration compatibility](/troubleshooting#gnoblin-or-configure-is-nil).

## Reload and persistence

| Change                         | Applies                                        | When removed                             |
| ------------------------------ | ---------------------------------------------- | ---------------------------------------- |
| Rules and animations           | On reload                                      | Earlier rules/defaults apply             |
| Titlebar policy                | After reload and the app's next surface update | Earlier rules/defaults apply             |
| Command shortcuts              | On reload                                      | Binding released; launched process stays |
| Built-in keybindings           | On reload                                      | Built-in default applies                 |
| Window-management preferences | On reload                                      | Gnoblin default applies                  |
| Compositor interaction preferences | On reload                                  | Gnoblin default applies                  |
| Input preferences              | On reload                                      | GNOME/Mutter settings apply              |
| Input sources                  | On reload                                      | GNOME session sources apply              |
| Orientation lock               | On reload                                      | GNOME orientation setting applies        |
| Cursor theme and size          | On reload                                      | Adwaita at 24 logical pixels              |
| Notifications and layout popup | On reload                                      | Saved GSettings value stays              |
| Autostart                      | New names start on reload                      | Running process stays                    |
| Renderer services              | Restart on reload                              | Enabled frames use native fallback       |
| Protocols                      | Next login                                     | Default on next login                    |
| Launcher focus behaviour       | Next login                                     | Default on next login                    |
| Drag boundary                  | Next drag after reload                         | Defaults to enabled                      |

An already-started autostart name uses a changed command only on the next login.
If you change a setting with the CLI, a value written in your config file
will replace that change on the next config reload.

`gnoblinctl reload` also reloads the theme and user scripts. Neither reload
command replaces native libraries or restarts your separate desktop shell.
