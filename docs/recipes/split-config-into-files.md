# Split a config into files

Load files from `init.lua` in the order you want them applied:

```lua
gnoblin.load("conf.d/**/*.lua")
gnoblin.load("appearance.lua")
gnoblin.load("bindings.lua")
```

In `appearance.lua`:

```lua
gnoblin.configure {
    shell = {minimize_duration = 120, layer_duration = 180},
}
```

In `bindings.lua`:

```lua
gnoblin.configure {shortcuts = {launcher = {binding = "<Super>d", command = {"fuzzel"}}}}
```

Files share the same API. You do not need `require("gnoblin")` or a return
statement. See [files and load order](/guides/files_and_load_order) for how
later config changes interact with earlier values.
