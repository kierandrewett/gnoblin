# gnoblin.configure.keybindings

Configure this part of `gnoblin.configure` with the `keybindings` key.

Override a built-in action by group and action name. For example, bind the
window-manager `close` action to Super+Q:

```lua
gnoblin.configure {
    keybindings = {
        wm = {close = {"<Super>q"}},
    },
}
```

Group names are `shell`, `wm`, `mutter` and `wayland`. Use underscore names
for actions. Give an action a list of accelerators; an empty list disables its
binding. Gnoblin applies overrides on reload. Removing an override restores the
built-in default.

Use [`shortcuts`](/config/configure/shortcuts) to launch commands from keys,
including media keys.

| Group     | GSettings schema                       |
| --------- | -------------------------------------- |
| `shell`   | `org.gnome.shell.keybindings`          |
| `wm`      | `org.gnome.desktop.wm.keybindings`     |
| `mutter`  | `org.gnome.mutter.keybindings`         |
| `wayland` | `org.gnome.mutter.wayland.keybindings` |

Use `gsettings list-keys SCHEMA` to look up native action names. For example, `show_screenshot_ui` maps to GSettings' `show-screenshot-ui`. See the [shortcuts guide](/guides/shortcuts).
