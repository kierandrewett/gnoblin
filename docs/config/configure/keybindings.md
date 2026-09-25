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

An action is a key in a GSettings schema. Write it as a group and action name;
the names available depend on the installed GNOME version. For example,
`wm.close` means the `close` key in Mutter's window-manager schema.

The group selects one of these schemas:

| Group     | Schema                                 |
| --------- | -------------------------------------- |
| `shell`   | `org.gnome.shell.keybindings`          |
| `wm`      | `org.gnome.desktop.wm.keybindings`     |
| `mutter`  | `org.gnome.mutter.keybindings`         |
| `wayland` | `org.gnome.mutter.wayland.keybindings` |

Use the GSettings key with underscores in Lua. Give each action a list of
accelerators. An empty list disables the action, and removing the override
restores its default binding on reload.

### Find an action

List the keys in the schema for the group you want. For example, these commands
list window-manager and Shell actions:

```sh
gsettings list-keys org.gnome.desktop.wm.keybindings
gsettings list-keys org.gnome.shell.keybindings
```

GSettings prints names with hyphens; use underscores in Lua. To read an
action's description, pass its schema and native key to `gsettings describe`:

```sh
gsettings describe org.gnome.desktop.wm.keybindings close
gsettings describe org.gnome.shell.keybindings show-screenshot-ui
```

For example, `show-screenshot-ui` becomes `show_screenshot_ui` under the
`shell` group. To bind it to Print:

```lua
gnoblin.configure {
    keybindings = {
        shell = {show_screenshot_ui = {"Print"}},
    },
}
```

Examples from the four groups:

- `shell.show_screenshot_ui`
- `wm.close`
- `mutter.toggle_tiled_left`
- `wayland.restore_shortcuts`

Use `gsettings list-keys` to confirm an action exists on your GNOME version.
These commands show available GNOME actions, not Gnoblin's current bindings.
GSettings schemas define each key's type and default; see the official
[Gio.Settings reference](https://docs.gtk.org/gio/class.Settings.html) and
[GSettings schema API](https://docs.gtk.org/gio/struct.SettingsSchema.html).

See the [shortcuts guide](/guides/shortcuts) to bind commands, use media keys,
and resolve conflicts between shortcuts.
