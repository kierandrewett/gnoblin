# gnoblin.configure.keybindings

Configure this part of `gnoblin.configure` with the `keybindings` key.

Override Mutter's built-in keybindings with `keybindings = {GROUP = {ACTION = {KEYS}}}`. Use underscore names for actions. Empty action lists disable that binding. Gnoblin applies overrides to Mutter's native keybinding table on reload and keeps persistence in the Lua config. Removing an override restores the built-in default. Configure media-key commands through [`shortcuts`](/config/configure/shortcuts).

| Group     | GSettings schema                       |
| --------- | -------------------------------------- |
| `shell`   | `org.gnome.shell.keybindings`          |
| `wm`      | `org.gnome.desktop.wm.keybindings`     |
| `mutter`  | `org.gnome.mutter.keybindings`         |
| `wayland` | `org.gnome.mutter.wayland.keybindings` |

Use `gsettings list-keys SCHEMA` to look up native action names. For example, `show_screenshot_ui` maps to GSettings' `show-screenshot-ui`. See the [shortcuts guide](/guides/shortcuts).
