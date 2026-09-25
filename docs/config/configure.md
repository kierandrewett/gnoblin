# gnoblin.configure

Set Gnoblin's compositor, input, session, and window-management options with a
Lua table:

```lua
gnoblin.configure {shell = {minimize_duration = 150}}
```

Defaults apply before your config loads. Files supplied by your desktop shell
can change them. Map values merge, lists replace, and later values win.

Sizes use logical pixels. Window rules distinguish application windows
(`type = "window"`) from layer surfaces such as bars and docks
(`type = "layer"`).

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
- `window_rules` replaces the full ordered list; use [`gnoblin.window_rule`](/config/window_rule) to append a rule.

Use [`gnoblin.snapshot()`](/config/snapshot) to inspect the config assembled
so far.

Use [Lua events](/config/lua-events) to react to window and input activity at
runtime. The event page lists payload fields and gives a Chromium scroll-speed
example.

See [recipes](/recipes/) for complete examples and
[file loading](/guides/files_and_load_order) for include order and reload
behavior.

## Type definition

Each top-level section is optional. See its page for the full set of nested
fields and accepted values.

```lua
gnoblin.configure {
    shell = {...}?,
    keybindings = {...}?,
    window_management = {...}?,
    compositor = {...}?,
    input = {...}?,
    input_sources = {...}?,
    permissions = {...}?,
    layer_shell = {...}?,
    protocols = {...}?,
    frame_renderers = {...}?,
    cursor = {...}?,
    shortcuts = {...}?,
    autostart = {...}?,
    window_rules = {WindowRule, ...}?, -- replaces the ordered list
}
```

`WindowRule` is the table shape accepted by [`gnoblin.window_rule`](/config/window_rule).
