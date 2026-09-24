# gnoblin.permission_rule

Append a portal permission rule. Use [`gnoblin.configure`](/config/configure#permissions) to set the global fallback. Any matching deny wins; otherwise the last matching rule wins as a whole. See the [permissions guide](/guides/permissions) for examples and diagnostics.

```lua
gnoblin.permission_rule {
    name = "allow-example-remote-desktop",
    match = "app-id:^org\\.example\\.Remote$",
    capabilities = {"remote-desktop", "screen-cast"},
    level = "allow",
    monitors = {"primary"},
}
```

| Field          | Values                                                                             |
| -------------- | ---------------------------------------------------------------------------------- |
| `name`         | Nonempty rule name                                                                 |
| `match`        | JavaScript regular expression against `app-id:ID` or `host-exe:PATH`               |
| `capabilities` | `"screen-cast"`, `"remote-desktop"`, `"input-capture"`, `"screenshot"`, `"access"` |
| `level`        | `"default"`, `"ask"`, `"allow"`, `"deny"`                                          |
| `monitors`     | Connector names or `"primary"`                                                     |
| `devices`      | `"keyboard"`, `"pointer"`, `"touchscreen"`; default empty                          |
| `clipboard`    | Boolean; default `false`                                                           |
