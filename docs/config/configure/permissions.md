# gnoblin.configure.permissions

Configure this part of `gnoblin.configure` with the `permissions` key.

Set the fallback used when no explicit portal permission rule decides a request.

| Setting               | Values                         | Default     |
| --------------------- | ------------------------------ | ----------- |
| `permissions.default` | `"default"`, `"ask"`, `"deny"` | `"default"` |

Add ordered per-application rules with [`gnoblin.permission_rule`](/config/permission_rule).

## Type definition

This is schema pseudocode in Lua table form. `?` marks an optional field.

```lua
gnoblin.configure {
    permissions = {
        default = "default" | "ask" | "deny"?,
    },
}
```
