# Route new app windows to named workspaces

Give workspaces stable IDs in `init.lua`, then place each matching new window on its workspace when it opens from
`shell.lua`. Replace the example app IDs with the values reported by `gnoblinctl window list --json`.

```lua
-- ~/.config/gnoblin/init.lua
gnoblin.configure {
    workspaces = {
        { id = "code", name = "Code" },
        { id = "web", name = "Web" },
        { id = "chat", name = "Chat" },
    },
}
```

```lua
-- ~/.config/gnoblin/shell.lua
seat.data.window_rules = {
    { match = { app_id = [[^org%.gnome%.TextEditor$]] }, workspace = { id = "code" } },
    { match = { app_id = [[^org%.mozilla%.firefox$]] }, workspace = { id = "web" } },
    { match = { app_id = [[^org%.gnome%.Fractal$]] }, workspace = { id = "chat" } },
}
```

The stock desk builds one workspace for each declared workspace and names it by its `id`, so a rule targets a workspace
by that ID. IDs must be unique and match `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`; names must be nonempty and at most 80
characters. The shell reads the declaration when the session starts.

| Option         | Values and default                                | Effect                                                 |
| -------------- | ------------------------------------------------- | ------------------------------------------------------ |
| `match.app_id` | Case-sensitive Lua pattern; unset matches any app | Selects windows by application ID.                     |
| `workspace`    | `{ id = "code" }`; unset                          | Places a matching new normal window on that workspace. |

App ID patterns use Lua 5.4 `string.find` syntax and search anywhere by default. Use `^` and `$` to match the whole ID.
Escape literal dots with `%`.

Restart the session to apply these settings.

Placement happens once; it does not lock a window to its workspace. You can move it afterward, and the shell leaves it
there.

- If an app reuses an existing window, the shell does not place it again.
- Dialogs stay with their parent window.
- If the target workspace does not exist, the shell logs one error and the new window stays where it opened.

See [workspace rules](/guides/window_rules#workspaces) and the
[window-management reference](/config/configure/window_management).
