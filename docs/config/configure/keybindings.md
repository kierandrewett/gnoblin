# gnoblin.configure.keybindings

Use `keybindings` to override a Mutter action directly by its schema group and
key. For named command shortcuts or named built-in actions, use
[`gnoblin.configure.shortcuts`](/config/configure/shortcuts).

The standalone session accepts only the `wm`, `mutter`, and `wayland` groups.
Shell-owned actions are configured in the shell. Restart the compositor after
changing these settings.

Override a built-in action by group and action name. For example, bind the
window-manager `close` action to Super+Q:

```lua
gnoblin.configure {
    keybindings = {
        wm = {close = {"<Super>q"}},
    },
}
```

An action is a built-in Mutter action. Write it as a group and action name;
the available names depend on the Mutter version in your Gnoblin build. For
example, `wm.close` selects Mutter's window-manager close action.

The group selects one of these schemas:

| Group     | Schema                                 |
| --------- | -------------------------------------- |
| `wm`      | `org.gnome.desktop.wm.keybindings`     |
| `mutter`  | `org.gnome.mutter.keybindings`         |
| `wayland` | `org.gnome.mutter.wayland.keybindings` |

Use the action's `key` with underscores in Lua. Give each action a list of
accelerators. An empty list disables the action, and removing the override
restores its default binding on reload.

### Find an action

List the built-in actions available in the running Gnoblin session:

```sh
gnoblinctl shortcut actions wm
```

Pass `mutter` or `wayland` to list another group. Add `--json` to keep the full
action records in a pipe. Each record includes its `id`, description, and
default bindings. The same records are available to Lua through
`gnoblin.shortcuts.actions(group?)`.

Use an action's `id` in a named shortcut. Use its `key` with underscores under
`keybindings`; for example, `toggle_tiled_left` is the Lua key for the
`mutter.toggle_tiled_left` action. The available actions and defaults come
from the schemas installed with this Mutter version.

See the [shortcuts guide](/guides/shortcuts) to bind commands, use media keys,
and resolve conflicts between shortcuts.

## Type definition

Only the actions you want to override need to be included. Their names and
available groups depend on the Mutter version in your Gnoblin build.

```lua
gnoblin.configure {
    keybindings = {
        wm = {["action_name"] = ({string, ...} | {})?},
        mutter = {["action_name"] = ({string, ...} | {})?},
        wayland = {["action_name"] = ({string, ...} | {})?},
    },
}
```
