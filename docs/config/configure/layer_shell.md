# gnoblin.configure.layer_shell

Configure how Gnoblin handles an active window when a layer-shell client opens.
Changes apply at the next login.

| Setting                  | Values  | Default | Effect                                                                                                              |
| ------------------------ | ------- | ------- | ------------------------------------------------------------------------------------------------------------------- |
| `preserve_active_window` | Boolean | `true`  | Keeps the active application window focused when a layer-shell surface opens. Set `false` to allow focus to change. |

```lua
gnoblin.configure {
    layer_shell = {preserve_active_window = true},
}
```

See the [layer-shell protocol catalog](/wayland-protocols).

![GNOME Files beneath a Quickshell panel in a clean Gnoblin session](../../images/gnoblin-quickshell-files.png)

_Quickshell draws the panel as a layer surface; Files remains a regular app window._

## Type definition

This is schema pseudocode in Lua table form. `?` marks an optional field.

```lua
gnoblin.configure {
    layer_shell = {
        preserve_active_window = boolean?,
    },
}
```
