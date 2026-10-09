# gnoblin.configure

Set Gnoblin's compositor, input, session, and window-management options with
`gnoblin.configure`:

```lua
gnoblin.configure {
    window_management = {focus_mode = "click"},
}
```

Defaults apply before your config loads. Included files merge maps, replace
lists, and later values win. Shell clients keep their own presentation
settings; Gnoblin config controls compositor and session behavior.

Sizes use logical pixels. Window rules distinguish application windows
(`type = "window"`) from layer surfaces such as bars and docks
(`type = "layer"`).

## Settings

Each entry below is a real top-level key accepted by `gnoblin.configure`.
A top-level key that is not in this list does nothing. The reload still
succeeds, and Gnoblin writes `gnoblin.configure: unknown section "name" is
ignored` to the session log. Check the log when a setting seems to have no
effect, for example after a spelling mistake in a section name.

- [`gnoblin.configure.keybindings`](/config/configure/keybindings)
- [`gnoblin.configure.window_management`](/config/configure/window_management)
- [`gnoblin.configure.compositor`](/config/configure/compositor)
- [`gnoblin.configure.input`](/config/configure/input)
- [`gnoblin.configure.input_sources`](/config/configure/input_sources)
- [`gnoblin.configure.monitors`](/config/configure/monitors)
- [`gnoblin.configure.touchpad_gestures`](/config/configure/touchpad_gestures)
- [`gnoblin.configure.permissions`](/config/configure/permissions)
- [`gnoblin.configure.location`](/config/configure/location)
- [`gnoblin.configure.prompts`](/config/configure/prompts)
- [`gnoblin.configure.auth`](/config/configure/auth)
- [`gnoblin.configure.portals`](/config/configure/portals)
- [`gnoblin.configure.layer_shell`](/config/configure/layer_shell)
- [`gnoblin.configure.protocols`](/config/configure/protocols)
- [`gnoblin.configure.frame_renderers`](/config/configure/frame_renderers)
- [`gnoblin.configure.cursor`](/config/configure/cursor)
- [`gnoblin.configure.xwayland`](/config/configure/xwayland)
- [`gnoblin.configure.shortcuts`](/config/configure/shortcuts)
- [`gnoblin.configure.autostart`](/config/configure/autostart)

Use [`gnoblin.snapshot()`](/config/snapshot) to inspect the config assembled
so far.

Use [Lua events](/config/lua-events) to set values from the window under the
pointer when the config loads.

To follow GNOME's light or dark preference in window rules, see
[light and dark appearance](/guides/theming).

See [recipes](/recipes/) for complete examples and
[file loading](/guides/files_and_load_order) for include order and reload
behavior.

## Type definition

Schema pseudocode: `?` marks an optional section. See each page for its
nested fields and accepted values.

```lua
gnoblin.configure {
    keybindings = {...}?,
    window_management = {...}?,
    compositor = {...}?,
    input = {...}?,
    input_sources = {...}?,
    monitors = {...}?,
    touchpad_gestures = {...}?,
    permissions = {...}?,
    location = {...}?,
    prompts = {...}?,
    auth = {...}?,
    portals = {...}?,
    layer_shell = {...}?,
    protocols = {...}?,
    frame_renderers = {...}?,
    cursor = {...}?,
    xwayland = {...}?,
    shortcuts = {...}?,
    autostart = {...}?,
}
```
