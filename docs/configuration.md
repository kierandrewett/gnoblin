# Configure Gnoblin

[Configuration reference](configuration-reference.md)

Edit `~/.config/gnoblin/init.lua` to change shortcuts, window rules and effects.
Configure the bar, dock and launcher in your desktop shell.

**Unreleased API:** the declaration functions below are new. Older installed
builds use the [legacy API](configuration-loading.md#existing-configs).

## 1. Find your config

Run this inside Gnoblin:

```sh
gnoblinctl config path
```

Edit the file it prints. Keep existing include lines from your shell installer.

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

Starting from an empty file? Load your installed component settings first:

```lua
gnoblin.load("/usr/share/gnoblin/conf.d/*.lua")
gnoblin.load("conf.d/**/*.lua")

gnoblin.configure {
    shell = {
        minimize_duration = 150,
    },
}
```

Use the system include path printed by your shell installer; source installs
may use a different prefix. An unmatched glob loads nothing.

## 3. Save and check

Valid edits reload automatically. To apply now and see errors:

```sh
gnoblinctl config reload
```

Try minimising a window. Invalid edits keep the last working configuration.

## Lua in a minute

- Strings use quotes: `"fade"`.
- Booleans are `true` or `false`, without quotes.
- Lists use braces: `{"ptyxis", "--new-window"}`.
- Setting names use underscores: `minimize_duration`.
- Comments begin with `--`.

Use `gnoblin.configure` for settings, `gnoblin.window_rule` for rules,
and `gnoblin.shortcut` for commands. `gnoblin` is available in every config file;
you do not need to import it. See the [recipes](configuration-recipes.md).

## Start from an example

Use the [complete starter config](configuration-recipes.md#complete-starter-config)
for a small working file. The same page shows how to override imported shortcuts,
combine window rules and split settings into files.

## What next?

- [Shortcuts](shortcuts.md) and [autostart](autostart.md)
- [Window rules](window-rules.md), [effects](window-effects.md) and [titlebars](window-frames.md)
- [Animations](animations.md) and [session settings](session-settings.md)
- [Split your config into files](configuration-loading.md)
- [All settings](configuration-reference.md)

## Reload or log out?

Most appearance settings and shortcuts reload on save.

**Log out and back in** after upgrading the compositor, changing protocol
availability, or changing `layer_shell.preserve_active_window`.

Some settings persist after you remove them from Lua. See
[reload and persistence](configuration-loading.md#reload-and-persistence).
If an edit does nothing, start with [troubleshooting](troubleshooting.md).
