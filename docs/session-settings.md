# Session settings

[Configuration reference](configuration-reference.md)

These options control native UI, layer focus and protocol availability.
They belong in `init.lua`.

## Native features

```lua
gnoblin.configure {
    shell = {
        notifications = true,
        input_source_switcher = false,
        window_switcher = false,
    },
}
```

| Option                  | Default       | Enables                            |
| ----------------------- | ------------- | ---------------------------------- |
| `notifications`         | Initially off | GNOME's notification service       |
| `input_source_switcher` | Initially off | Native keyboard-layout popup       |
| `window_switcher`       | Off           | GNOME's app/window/group switchers |

Notifications and the layout-popup setting persist in GSettings.
Removing them from Lua leaves the saved value. Window-switcher removal returns
to its default, but disabling it does not release existing shortcut bindings.

Disabling the layout popup does not disable keyboard layouts.
GNOME's OSD and screenshot UI cannot be re-enabled; legacy keys do not restore them.

## Layer-shell keyboard focus

```lua
gnoblin.configure {
    layer_shell = {
        preserve_active_window = true,
    },
}
```

Default: `true`. **Requires logout and login.**

An exclusive-keyboard layer, such as a launcher, receives typing while the
application underneath stays active. Keys are not sent to both.

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

Default: `true`. Applies at the next drag after reload.

This keeps the dragged frame below the monitor's reserved top area, including
space reserved by a layer-shell panel. Set `false` to allow overlap.

## Protocol settings

Implemented gates are enabled by default in Gnoblin. To disable one:

```lua
gnoblin.configure {
    protocols = {
        wlr_screencopy = false,
    },
}
```

**Requires logout and login.** Reload cannot remove advertised Wayland globals.

| Name                              | Used for                         |
| --------------------------------- | -------------------------------- |
| `wlr_layer_shell`                 | Bars, docks and launchers        |
| `wlr_screencopy`                  | Screen capture                   |
| `ext_foreign_toplevel_list`       | Window enumeration               |
| `wlr_foreign_toplevel_management` | Window controls and dock targets |
| `ext_data_control`                | Clipboard managers               |
| `ext_idle_notify`                 | Idle detection                   |
| `wlr_gamma_control`               | Gamma and colour temperature     |
| `wlr_output_power_management`     | Display power                    |
| `ext_background_effect_v1`        | Client-requested background blur |

Protocol availability and [portal permissions](permissions.md) are separate.
Disabling protocols your shell uses can prevent its features from working.
