# session_settings

[Configuration reference](/config/configure)

Configure how layer-shell surfaces affect focus, how windows respond to
pointer input, and which Wayland interfaces are available. Add the examples to
`~/.config/gnoblin/init.lua`.

## Shell UI

Panels, notifications, launchers, and keyboard-layout popups belong to the
desktop shell you install. Configure them there. Gnoblin's Lua configuration
controls compositor and session behavior; it does not enable GNOME Shell
features.

See [choose a shell](/bring-your-own-shell) for shell options and
[shortcuts](/guides/shortcuts#avoid-conflicts) to resolve keybinding conflicts.

## Layer-shell keyboard focus

```lua
gnoblin.configure {
    layer_shell = {
        preserve_active_window = true,
    },
}
```

Default: `true`. **Requires logout and login.**

When a launcher requests exclusive keyboard focus, typing goes to the launcher.
With this setting enabled, the application underneath keeps its active
appearance: its titlebar and effects do not switch to their unfocused style.

The launcher still receives all typing.

Set `false` to let exclusive layers deactivate the application beneath them.
On-demand focus, passive bars and normal application menus are unchanged.

## Window drag boundary

```lua
gnoblin.configure {
    window_management = {
        constrain_drag_to_work_area = true,
    },
}
```

Default: `true`. The compositor accepts this setting but does not read it yet,
so `true` and `false` behave the same. See the
[window management reference](/config/configure/window_management).

This keeps the dragged frame below the monitor's reserved top area, including
space reserved by a layer-shell panel. Set `false` to allow overlap.

## Protocol settings

Wayland protocols let desktop tools request features such as screen capture,
clipboard access and display power control. The interfaces below are enabled
by default. Most users can leave them alone.

For example, this disables the `wlr_screencopy` screen-capture interface:

```lua
gnoblin.configure {
    protocols = {
        wlr_screencopy = false,
    },
}
```

Changes apply on config reload. Disabling an interface stops new clients from
binding to it; clients that already bound it keep their connection until they
disconnect.

Disabling `ext_session_lock` while the session is locked makes the reload fail
and keeps the previous configuration. Unlock before changing that setting.

| Name                              | Used for                           |
| --------------------------------- | ---------------------------------- |
| `wlr_layer_shell`                 | Bars, docks and launchers          |
| `wlr_screencopy`                  | Screen capture                     |
| `ext_foreign_toplevel_list`       | Window enumeration                 |
| `wlr_foreign_toplevel_management` | Window controls and dock targets   |
| `ext_data_control`                | Clipboard managers                 |
| `ext_idle_notify`                 | Idle detection                     |
| `wlr_gamma_control`               | Gamma and colour temperature       |
| `wlr_output_power_management`     | Display power                      |
| `ext_background_effect_v1`        | Client-requested background blur   |
| `xdg_decoration`                  | Client/server titlebar negotiation |
| `window_frame_renderer`           | External frame renderer service    |
| `blur_fade`                       | Per-item blur fade metadata        |

Each protocol name accepts `true` or `false` and defaults to `true`. Set a
protocol to `false` to keep Gnoblin from advertising that interface to new
clients.

This does not block all screen sharing: apps using the desktop portal follow
[portal permissions](/guides/permissions) instead.
Disabling protocols your shell uses can prevent its features from working.
See the [protocol catalog](/wayland-protocols) for the advertised interface
names, related guides and interfaces that are not yet supported.
