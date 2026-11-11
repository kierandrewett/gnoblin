# Organize a growing config

Keep the entry point short and group settings by the kind of change you make.
For example, let `init.lua` load your local fragments first, then your
appearance and shortcut overrides. Packages that ship system defaults need no
line here; Gnoblin loads them before `init.lua`
(see [system drop-ins](/guides/files_and_load_order#system-drop-ins)):

```lua
gnoblin.load("conf.d/**/*.lua")
gnoblin.load("appearance.lua")
gnoblin.load("bindings.lua")
```

Put window rules in `shell.lua`, which the shell loads itself. `init.lua` does not load it:

```lua
-- ~/.config/gnoblin/shell.lua
seat.data.window_rules = {
    { match = { app_id = [[^org%.gnome%.TextEditor$]] }, maximised = true },
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
