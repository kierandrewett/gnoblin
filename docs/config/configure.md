# gnoblin.configure

Set Gnoblin's compositor, input, session and window-management options. Pass a Lua table; map values merge, lists replace, and later values win.

```lua
gnoblin.configure {shell = {minimize_duration = 150}}
```

Defaults apply before your config loads. Files supplied by your desktop shell can change them. Sizes use logical pixels. Window rules distinguish application windows (`type = "window"`) from layer surfaces such as bars and docks (`type = "layer"`).

## Settings

Each entry below is a real top-level key accepted by `gnoblin.configure`.

- [`gnoblin.configure.shell`](/config/configure/shell)
- [`gnoblin.configure.keybindings`](/config/configure/keybindings)
- [`gnoblin.configure.window_management`](/config/configure/window_management)
- [`gnoblin.configure.compositor`](/config/configure/compositor)
- [`gnoblin.configure.input`](/config/configure/input)
- [`gnoblin.configure.input_sources`](/config/configure/input_sources)
- [`gnoblin.configure.permissions`](/config/configure/permissions)
- [`gnoblin.configure.layer_shell`](/config/configure/layer_shell)
- [`gnoblin.configure.protocols`](/config/configure/protocols)
- [`gnoblin.configure.frame_renderers`](/config/configure/frame_renderers)
- [`gnoblin.configure.cursor`](/config/configure/cursor)
- [`gnoblin.configure.shortcuts`](/config/configure/shortcuts)
- [`gnoblin.configure.autostart`](/config/configure/autostart)

Use [`gnoblin.snapshot()`](/config/snapshot) to inspect a copy of the config assembled so far. See [recipes](/recipes) for complete examples and [file loading](/guides/files_and_load_order) for include order and reload behavior.
