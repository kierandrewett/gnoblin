# gnoblin.configure.keybindings

Use `keybindings.keyboard` and `keybindings.pointer` to run Lua callbacks
from keyboard and mouse bindings. Both use the shared Lua input handler.
You can also override Mutter keyboard actions by schema group and key. For named command shortcuts or named built-in actions, use
[`gnoblin.configure.shortcuts`](/config/configure/shortcuts).

The callback groups are `keyboard` and `pointer`. The compatibility groups
`wm`, `mutter`, and `wayland` select existing Mutter actions. Shell-owned actions are configured in the shell.
Changes apply on config reload.

## Keyboard callbacks

Put this in a file loaded by `~/.config/gnoblin/init.lua`:

```lua
gnoblin.configure {
    keybindings = {
        keyboard = {
            terminal = {
                binding = "<Super>Return",
                callback = function(event)
                    gnoblin.commands.run({"ptyxis", "--new-window"})
                end,
            },
        },
    },
}
```

Install `ptyxis`, save the file, and run `gnoblinctl reload`. Super+Enter then
queues a terminal launch. Commands are argument arrays; shell expansion happens
only if you explicitly run a shell.

Use snake_case names. A later declaration with the same name replaces its
callback. Set `enable = false` to disable a binding.

The callback options are:

- `trigger`: `"press"` by default, or `"release"` to run on key release.
- Return value: nil consumes the event. Return `"forward"` for normal routing
  or `"consume"` to withhold delivery.

Release triggers reserve and consume the physical press/release pair.
Returning forward cannot undo that reservation; use a press trigger for
conditional forwarding.

For device selectors and lower-level events, see [Input handlers](/shell-api/input#input-handlers).

## Mutter action overrides

Override a built-in action by group and action name. For example, bind the
window-manager `close` action to Super+Q:

```lua
gnoblin.configure {
    keybindings = {
        wm = {close = {"<Super>q"}},
    },
}
```

An action must have an executable handler in the Mutter build used by Gnoblin.
Write it as a group and action name; for example, `wm.close` selects Mutter's
window-manager close action. Schema keys without a compositor handler cannot
be overridden. The available names depend on the Mutter version in your Gnoblin
build.

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

## Pointer bindings

Pointer bindings replace `window_management.mouse_button_modifier` and
`window_management.resize_with_right_button`. Remove those fields when
migrating an older config.

The embedded config declares pointer callbacks in `config/50-pointer.lua`.
Each named binding supplies a button accelerator and a Lua callback:

```lua
gnoblin.configure {
    keybindings = {
        pointer = {
            resize = {
                binding = "<Super>Button3",
                callback = function(event)
                    local window = event.window
                    if not window.resizable then return end
                    local frame = window.frame
                    local vertical = event.pointer.y < frame.y + frame.height / 2
                        and "north" or "south"
                    local horizontal = event.pointer.x < frame.x + frame.width / 2
                        and "west" or "east"
                    window:begin_resize(vertical .. "_" .. horizontal, event.focus_context)
                end,
            },
        },
    },
}
```

The default callback selects the nearest corner in Lua. The native compositor
starts the requested resize and enforces the window's size limits. Change the
callback to select another edge or perform another supported window operation.

`Button1` is left, `Button2` middle and `Button3` right. Prefix the button with
one or more modifier tokens. Ordinary clicks continue to reach apps.

Modifier tokens are case-insensitive: `<Super>`, `<Alt>`, `<Meta>`, `<Hyper>`,
`<Mod1>` through `<Mod5>`, `<Control>` (also `<Ctrl>`, `<Ctl>` or `<Primary>`),
and `<Shift>` (also `<Shft>`). Combine tokens without spaces. Conflicting
button and modifier combinations are rejected.

Names identify callbacks, not built-in actions. Without a binding, the
compositor supplies no gesture. Save and run `gnoblinctl config reload` to
replace the bindings and callbacks.

To disable an imported binding, set its named declaration to
`{enable = false}`. A later callback declaration with the same name replaces
the earlier callback.

The event contains the window under the pointer, its frame geometry, and the
pointer's stage coordinates `pointer.x` and `pointer.y`. Its one-use `focus_context` authorizes
a move or resize of that window during the trusted input interaction.

These gestures operate on Wayland and Xwayland application windows,
independently of decorations. Fixed-size windows cannot be resized. Shortcut
inhibition is respected.

`pointer` actions are independent of the keyboard action catalog returned by
`gnoblinctl shortcut actions`.

## Type definition

Only the actions you want to override need to be included. Their names and
available groups depend on the Mutter version in your Gnoblin build.

```lua
gnoblin.configure {
    keybindings = {
        pointer = {["binding_name"] = {binding = string, callback = function(event) end}},
        wm = {["action_name"] = ({string, ...} | {})?},
        mutter = {["action_name"] = ({string, ...} | {})?},
        wayland = {["action_name"] = ({string, ...} | {})?},
    },
}
```
