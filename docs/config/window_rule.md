# Window rule reference

[Shell files](/user-shell)

A window rule is one table in the list `seat.data.window_rules`, set in `~/.config/gnoblin/shell.lua`. The
[window rules guide](/guides/window_rules) has usage patterns.

```lua
seat.data.window_rules = {
    {
        match = { app_id = "^org%.example%.Editor$", focused = false },
        workspace = { id = "3" },
        maximised = true,
        opacity = 0.9,
    },
}
```

## Rule fields {#rule-fields}

| Field        | Type    | Effect                                                       |
| ------------ | ------- | ------------------------------------------------------------ |
| `match`      | Table   | The conditions a window must meet. Every key given must hold |
| `workspace`  | Table   | `{ id = "3" }`: the workspace, by name, the window opens on  |
| `maximised`  | Boolean | `true` maximises the window, `false` unmaximises it          |
| `fullscreen` | Boolean | `true` or `false`                                            |
| `secure`     | Boolean | `true` masks the window in screen capture                    |
| `opacity`    | Number  | From 0 to 1                                                  |

A rule with an unknown field, or a field of the wrong type, is refused. So is an `opacity` outside 0 to 1.

## Match keys

| Key            | Type    | Matches                                                  |
| -------------- | ------- | -------------------------------------------------------- |
| `app_id`       | Pattern | The application ID, or the WM class when the ID is empty |
| `title`        | Pattern | The current title                                        |
| `focused`      | Boolean | Whether the window has keyboard focus                    |
| `workspace_id` | String  | The name of the window's workspace                       |

A pattern is a [Lua pattern](https://www.lua.org/manual/5.4/manual.html#6.4.1), case-sensitive, matched anywhere in the
text. A malformed pattern refuses the whole list.

## Merging

Rules merge in list order. The last matching rule wins for each field, and a field a later rule does not set keeps the
earlier value. A field that no matching rule sets falls back to its default.

## Removed fields {#refused-keys}

The stock shell draws blur and shadow only, so these fields from earlier versions are refused. A rule that names one is
an unknown field.

- `corners` {#corners}
- `blur`, `blur-ignore-shadows`, `shader`, `shader-uniforms` and `animation` {#animation-fields}
- `frame` {#frame-fields}
- `match.type`, `match.layer`, `match.client_type` and `match.workspace_number`: use `match.app_id` or
  `match.workspace_id` instead.
