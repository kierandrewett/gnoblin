# Route new app windows to named workspaces

Give workspaces stable IDs, then place each matching new window on its
workspace when it opens. Replace the example app IDs with the values reported by
`gnoblinctl window list --json`.

```lua
gnoblin.configure {
    workspaces = {
        {id = "code", name = "Code"},
        {id = "web", name = "Web"},
        {id = "chat", name = "Chat"},
    },
}

gnoblin.window_rule {
    match = {type = "window", app_id = [[^org.gnome.TextEditor$]]},
    workspace = {id = "code"},
}

gnoblin.window_rule {
    match = {type = "window", app_id = [[^org.mozilla.firefox$]]},
    workspace = {id = "web"},
}

gnoblin.window_rule {
    match = {type = "window", app_id = [[^org.gnome.Fractal$]]},
    workspace = {id = "chat"},
}
```

Each declared workspace remains available when empty. The declaration order
sets the initial positions, but rules target workspaces by stable ID. IDs must
be unique and match `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`; names must be
nonempty and no longer than 80 characters. Runtime-created workspaces are
temporary and are not part of this list.

| Option         | Values and default                                   | Effect                                                 |
| -------------- | ---------------------------------------------------- | ------------------------------------------------------ |
| `match.app_id` | JavaScript regular expression; unset matches any app | Selects windows by application ID.                     |
| `workspace`    | `{id = "code"}` or `{number = 1..1024}`; unset       | Places a matching new normal window on that workspace. |

App ID patterns are case-sensitive JavaScript regular expressions. Use `^`
and `$` to match the whole ID. Reload the configuration to apply these settings.

Placement happens once; it does not lock a window to its workspace. You can
move it afterward, and Gnoblin will leave it there.

- If an app reuses an existing window, Gnoblin does not place it again.
- Dialogs stay with their parent window.
- If the target workspace is unavailable, the new window stays where it opened.

See [workspace rules](/guides/window_rules#workspaces) and the
[window-management reference](/config/configure/window_management).
