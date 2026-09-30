# Lua event handlers

Put live compositor behavior in Lua modules loaded by your Gnoblin
configuration. The standalone session does not load GJS modules or expose
GNOME Shell objects.

Create `~/.config/gnoblin/scripts/window-log.lua`:

```lua
gnoblin.events.on("gnoblin.window.created", function(event)
    local window = event.window
    print(window.app_id, window.title)
end)
```

Load the module from `~/.config/gnoblin/init.lua`:

```lua
gnoblin.load("scripts/**/*.lua")
```

Run `gnoblinctl config reload` to activate it. The runtime replaces the old
callbacks after a successful reload; a failed reload leaves the active config
running. See the [Lua event reference](/config/lua-events) for event names and
payload fields, and the [runtime API](/config/runtime-api) for operations that
control windows, workspaces, input, and session state.

Lua configuration runs in a restricted runtime. It cannot access arbitrary
files, start processes, load native modules, or use GNOME Shell's in-process
objects. Use configured shortcuts or autostart entries to launch programs.
Panels, launchers, notifications, and other visible UI belong in separate
Wayland clients; Lua supplies their compositor state and control API.
