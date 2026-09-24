# Ask before undecided portal access

Require consent when a portal request has no matching permission rule:

```lua
gnoblin.configure {
    permissions = {default = "ask"},
}
```

This sets the fallback only. Explicit matching rules can still allow or deny a
request. It applies to Gnoblin's patched portal backend; see the
[permissions guide](/guides/permissions) for rule matching and supported
capabilities.
