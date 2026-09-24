# config

[reference](/config/reference) · [recipes](/recipes)

On first login, `gnoblin-session` copies the packaged reference config to
`~/.config/gnoblin/init.lua` when there is no Lua or legacy config already
there. Gnoblin loads that file on the first shell start. The seed step never
replaces an existing config. Edit the file to change shortcuts, window rules,
effects and desktop preferences.
Configure the bar, dock and launcher in your desktop shell.

## 1. Find your config

Run this inside Gnoblin:

```sh
gnoblinctl config path
```

Edit the file it prints, creating it and its parent directory if needed.
Keep any `gnoblin.load(...)` lines already there: they load settings supplied
by your desktop shell or other config files.

## 2. Make a change

For an existing config, add this **after** its includes:

```lua
gnoblin.configure {
    shell = {
        minimize_duration = 150,
    },
}
```

This makes minimise and restore animations take 150 milliseconds.

If your shell installer supplied config files, load them before your own
settings. For example:

```lua
gnoblin.load("/usr/share/gnoblin/conf.d/*.lua")
gnoblin.load("conf.d/**/*.lua")

gnoblin.configure {
    shell = {
        minimize_duration = 150,
    },
}
```

The first line loads installed settings; the second loads your own files under
`~/.config/gnoblin/conf.d/`. Use the paths supplied by your installer if they
differ. A pattern that finds no files is ignored. If you have no extra config
files, the first example works on its own.

## 3. Save and check

Valid edits reload automatically. To apply now and see errors:

```sh
gnoblinctl config reload
```

Try minimising a window. Invalid edits keep the last working configuration.

## Reading the examples

- Strings use quotes: `"fade"`.
- Booleans are `true` or `false`, without quotes.
- Braces group settings: `shell = {minimize_duration = 150}`.
- Braces also hold lists: `{"ptyxis", "--new-window"}`.
- Separate entries with commas; a comma after the last entry is allowed.
- Setting names use underscores: `minimize_duration`.
- Comments begin with `--`.

Use `gnoblin.configure` for settings and named shortcuts, and
`gnoblin.window_rule` for ordered rules. The outer braces belong to the
function call; inner braces group related options. `gnoblin` is available in every config file;
you do not need to import it. See the [recipes](/recipes).

## Sizes and window types

Sizes use **logical pixels**, before display scaling. At 200% scale, a
10-pixel border occupies 20 physical screen pixels. Durations use milliseconds:
200 ms is one fifth of a second.

A **window** is an application window. A **layer surface** is a bar, dock,
launcher or other desktop panel using the Wayland layer-shell protocol. Rules
use `type = "window"` or `type = "layer"` to distinguish them.

## Start from an example

Use the [complete starter config](/recipes#complete-starter-config)
for a small working file. The same page shows how to override imported shortcuts,
combine window rules and split settings into files.

## What next?

- [Shortcuts](/config/shortcuts) and [autostart](/config/autostart)
- [Window rules](/config/window_rules), [effects](/config/window_effects) and [titlebars](/config/window_frames)
- [Animations](/config/animations) and [session settings](/config/session_settings)
- [Split your config into files](/config/files_and_load_order)
- [All settings](/config/reference)

## Reload or log out?

Most appearance settings and shortcuts reload on save.

**Log out and back in** after upgrading the compositor, changing protocol
availability, or changing `layer_shell.preserve_active_window`.

Some settings persist after you remove them from Lua. See
[reload and persistence](/config/files_and_load_order#reload-and-persistence).
If an edit does nothing, start with [troubleshooting](/troubleshooting).
