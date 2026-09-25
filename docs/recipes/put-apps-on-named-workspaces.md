# Route new app windows to named workspaces

Give workspaces stable IDs, then place each matching new window on its
workspace when it opens. Replace the example app IDs with the values reported by
`gnoblinctl window list --json`.

```lua
gnoblin.configure {
    window_management = {
        dynamic_workspaces = false,
        num_workspaces = 3,
        workspace_names = {"Code", "Web", "Chat"},
        workspace_ids = {"code", "web", "chat"},
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

| Option               | Values and default                                    | Effect                                                 |
| -------------------- | ----------------------------------------------------- | ------------------------------------------------------ |
| `dynamic_workspaces` | Boolean; default `false`                              | Keeps the configured workspace count fixed.            |
| `num_workspaces`     | Integer 1–36; default `4`                             | Sets the number of workspaces.                         |
| `workspace_names`    | Up to 36 strings, max 80 chars each; default `{}`     | Labels workspaces by position.                         |
| `workspace_ids`      | Up to 36 unique IDs; default `{}`                     | Keeps rule targets stable if positions change.         |
| `match.app_id`       | JavaScript regular expression; unset matches any app  | Selects windows by application ID.                     |
| `workspace`          | Declared `{id = "code"}` or `{number = 1..36}`; unset | Places a matching new normal window on that workspace. |

Workspace IDs start with a letter or digit. They may also contain `.`, `_` or
`-`. The example declares its three IDs in workspace order; each placement ID
must be declared there. Numeric targets are one-based positions.

App ID patterns are case-sensitive JavaScript regular expressions. Use `^`
and `$` to match the whole ID. Reload the configuration to apply these settings.

Placement happens once; it does not lock a window to its workspace. You can
move it afterward, and Gnoblin will leave it there.

- If an app reuses an existing window, Gnoblin does not place it again.
- Dialogs stay with their parent window.
- If the target workspace is unavailable, the new window stays where it opened.

See [workspace rules](/guides/window_rules#workspaces) and the
[window-management reference](/config/configure/window_management).
