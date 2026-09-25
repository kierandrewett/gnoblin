# Organize a growing config

Keep the entry point short and group settings by the kind of change you make.
For example, let `init.lua` load site defaults first, then your local
appearance and shortcut overrides:

```lua
gnoblin.load("/usr/share/gnoblin/conf.d/*.lua")
gnoblin.load("conf.d/**/*.lua")
gnoblin.load("appearance.lua")
gnoblin.load("bindings.lua")
```

Put window rules in `appearance.lua`:

```lua
gnoblin.window_rule {
    match = {type = "window"},
    corners = {radius = 12, smoothing = 0.5},
}
```

Put shortcuts in `bindings.lua`:

```lua
gnoblin.configure {
    shortcuts = {
        launcher = {binding = "<Super>d", command = {"fuzzel"}},
    },
}
```

Loaded files share the same Gnoblin API; they do not need `require` or a return
statement. Keep includes before your personal overrides. Calls such as
`gnoblin.window_rule` append rules, while supplying a complete `window_rules`
list replaces the previous list. Arrays such as `xkb_options` also replace the
active array, so list all values you intend to keep.

Use `gnoblinctl config path` to find the active entry point and
`gnoblinctl config reload` to apply changes. Read [file loading and order](/guides/files_and_load_order)
before adding overlapping files or globs.
