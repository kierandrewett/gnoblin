# gnoblin.permission_rule

Append a portal permission rule. Use
[`gnoblin.configure`](/config/configure#settings) to set the global
fallback.

Any matching deny wins. Otherwise, the last matching rule wins as a whole.
See the [permissions guide](/guides/permissions) for examples and diagnostics.

```lua
gnoblin.permission_rule {
    name = "allow-example-remote-desktop",
    match = "^app%-id:org%.example%.Remote$",
    capabilities = {"remote-desktop", "screen-cast"},
    level = "allow",
    monitors = {"primary"},
}
```

| Field          | Accepted values                                                                                   | Default and meaning                                                      |
| -------------- | ------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| `name`         | Nonempty string                                                                                   | Required rule label.                                                     |
| `match`        | Nonempty Lua 5.4 pattern, 1–512 bytes, against the complete verified identity                     | Required. Identities begin with `app-id:` or `host-exe:`.                |
| `capabilities` | One or more of `"screen-cast"`, `"remote-desktop"`, `"input-capture"`, `"screenshot"`, `"access"` | Required permissions this rule controls.                                 |
| `level`        | `"default"`, `"ask"`, `"allow"`, `"deny"`                                                         | Required permission decision.                                            |
| `monitors`     | `"primary"` or exact monitor connector names                                                      | Applies to `screen-cast` and `remote-desktop`; unset means all monitors. |
| `devices`      | Any of `"keyboard"`, `"pointer"`, `"touchscreen"`                                                 | Applies to `remote-desktop`; unset means no devices.                     |
| `clipboard`    | Boolean                                                                                           | Applies to `remote-desktop`; defaults to `false`.                        |

For example, `^app%-id:org%.example%.Remote$` matches that complete
application identity. Lua patterns use `%` to escape pattern characters.
They are not regular expressions, and `|` is not alternation.

Convert regex `^app-id:org\.example$` to Lua pattern
`^app%-id:org%.example$`. Constructs such as `{m,n}`, `\d`, and lookarounds
need rewriting. See the
[Lua 5.4 pattern reference](https://www.lua.org/manual/5.4/manual.html#6.4.1).
Use `gnoblinctl` permission inspection to find the identity Gnoblin reports;
see [Inspect a decision](/guides/permissions#inspect-a-decision).

`monitors` only applies to screen capture or remote desktop. `devices` and
`clipboard` only apply to remote desktop. Other combinations are rejected.

## Type definition

`?` marks optional fields; `|` separates accepted alternatives. A rule needs
`name`, `match`, `capabilities`, and `level`.

```lua
gnoblin.permission_rule {
    name = string,
    match = string, -- Lua pattern, 1-512 bytes, over the complete identity
    capabilities = {"screen-cast" | "remote-desktop" | "input-capture"
        | "screenshot" | "access", ...}, -- nonempty
    level = "default" | "ask" | "allow" | "deny",
    monitors = {"primary" | string, ...}?,
    devices = {"keyboard" | "pointer" | "touchscreen", ...}?,
    clipboard = boolean?,
}
```
