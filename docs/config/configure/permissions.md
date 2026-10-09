# gnoblin.configure.permissions

Configure this part of `gnoblin.configure` with the `permissions` key.

Set the fallback used when no explicit portal permission rule decides a request.

| Setting               | Values                         | Default     |
| --------------------- | ------------------------------ | ----------- |
| `permissions.default` | `"inherit"`, `"ask"`, `"deny"` | `"inherit"` |

| Value       | Effect                                                                      |
| ----------- | --------------------------------------------------------------------------- |
| `"inherit"` | Normal portal behavior, including restore tokens. No extra prompt is added. |
| `"ask"`     | Ask for consent or a selection every time.                                  |
| `"deny"`    | Reject the request without a dialog.                                        |

`"default"` is the old name for `"inherit"`. It still loads, and Gnoblin reports it as `"inherit"`.
It will be removed in a later release.

Add ordered per-application rules with [`gnoblin.permission_rule`](/config/permission_rule).

## Type definition

This is schema pseudocode in Lua table form. `?` marks an optional field.

```lua
gnoblin.configure {
    permissions = {
        default = "inherit" | "ask" | "deny"?,
    },
}
```
