# files_and_load_order

[Configuration reference](/config/configure)

Start with one `init.lua`. Split it into modules when that makes it easier
to read.

## Which file is loaded?

`gnoblinctl config path` prints the main config file used by your running session.

The usual path is `~/.config/gnoblin/init.lua`, or
`$XDG_CONFIG_HOME/gnoblin/init.lua` when that variable is set.
The compositor's `GNOBLIN_CONFIG` environment variable overrides it.

Without that override, Gnoblin checks `init.lua`, `gnoblin.toml`, then
`gnoblin.conf`. The TOML names are kept for existing installations. New configs
should use `init.lua`; any selected file without a `.lua` suffix is parsed as TOML.

For packaged logins, `gnoblin-session` copies
`/usr/share/gnoblin/init.lua.example` to `init.lua` when no user config exists.
The shell loads that file when its config starts.

If the example is unavailable, or you run a build directly, Gnoblin uses its
defaults. Setting `GNOBLIN_CONFIG` in a terminal does not change the
environment of an already-running compositor.

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

Lists replace earlier values. This applies to `window_rules`, `shortcuts`,
`autostart` and permission `rules`.

To add a window rule while keeping previous rules, use:

```lua
gnoblin.window_rule {
    match = {type = "window", focused = false},
    opacity = 0.95,
}
```

The `shortcuts` and `autostart` maps merge entries with the same name. Set an
entry's `enable` field to `false` to disable an imported shortcut or autostart.
Disabling an autostart entry does not stop a running process.

To remove every shortcut loaded so far:

```lua
gnoblin.configure {shortcuts = {}}
```

Use `gnoblin.configure.shortcuts` to add or change individual shortcuts
without clearing the others.

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

`require("appearance")` searches `appearance.lua`, then `lua/appearance.lua`,
relative to its caller. It runs once per reload.

Returning a table does not apply its settings by itself. Pass it to
`gnoblin.configure` as shown above.

Dots in module names are not converted to directories. For explicit subdirectories,
use `gnoblin.load("parts/motion.lua")`. Avoid loading the same file through both a glob
and `require`.

## Available Lua functions

`gnoblin` is available globally in every loaded file and module.
See the [function reference](/config#api-reference).

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

## Reload and persistence

| Change                             | Applies                                        | When removed                             |
| ---------------------------------- | ---------------------------------------------- | ---------------------------------------- |
| Rules and animations               | On reload                                      | Earlier rules/defaults apply             |
| Titlebar policy                    | After reload and the app's next surface update | Earlier rules/defaults apply             |
| Command shortcuts                  | On reload                                      | Binding released; launched process stays |
| Built-in keybindings               | On reload                                      | Built-in default applies                 |
| Window-management preferences      | On reload                                      | Gnoblin default applies                  |
| Compositor interaction preferences | On reload                                      | Gnoblin default applies                  |
| Input preferences                  | On reload                                      | GNOME/Mutter settings apply              |
| Input sources                      | On reload                                      | GNOME session sources apply              |
| Orientation lock                   | On reload                                      | GNOME orientation setting applies        |
| Cursor theme and size              | On reload                                      | Adwaita at 24 logical pixels             |
| Notifications and layout popup     | On reload                                      | Saved GSettings value stays              |
| Autostart                          | New names start on reload                      | Running process stays                    |
| Renderer services                  | Restart on reload                              | Enabled frames use native fallback       |
| Protocols                          | Next login                                     | Default on next login                    |
| Launcher focus behaviour           | Next login                                     | Default on next login                    |
| Drag boundary                      | Next drag after reload                         | Defaults to enabled                      |

An already-started autostart name uses a changed command only on the next login.
If you change a setting with the CLI, a value written in your config file
will replace that change on the next config reload.

`gnoblinctl reload` also reloads the theme and user scripts. Neither reload
command replaces native libraries or restarts your separate desktop shell.
