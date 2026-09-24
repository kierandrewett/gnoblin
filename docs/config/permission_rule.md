# gnoblin.permission_rule

Append a portal permission rule. Use [`gnoblin.configure`](/config/configure/permissions) to set the global fallback. Any matching deny wins; otherwise the last matching rule wins as a whole. See the [permissions guide](/guides/permissions) for examples and diagnostics.

```lua
gnoblin.permission_rule {
    name = "allow-example-remote-desktop",
    match = "app-id:^org\\.example\\.Remote$",
    capabilities = {"remote-desktop", "screen-cast"},
    level = "allow",
    monitors = {"primary"},
}
```

| Field          | Values                                                                                    |
| -------------- | ----------------------------------------------------------------------------------------- |
| `name`         | Unique 1–80 character name using letters, digits, `_`, `.`, or `-`; max 256 rules         |
| `match`        | JavaScript regular expression (max 512 characters) against `app-id:ID` or `host-exe:PATH` |
| `capabilities` | `"screen-cast"`, `"remote-desktop"`, `"input-capture"`, `"screenshot"`, `"access"`        |
| `level`        | `"default"`, `"ask"`, `"allow"`, `"deny"`                                                 |
| `monitors`     | Nonempty connector names or `"primary"`; only for screen cast or remote desktop           |
| `devices`      | `"keyboard"`, `"pointer"`, `"touchscreen"`; only for remote desktop                       |
| `clipboard`    | Boolean; only for remote desktop; default `false`                                         |

Rule names must be unique. Any matching deny wins; if none deny, the last
matching rule decides the request.
